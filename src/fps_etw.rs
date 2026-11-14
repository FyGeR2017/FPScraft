use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_SUCCESS, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::Etw::{
    CloseTrace, ControlTraceW, EnableTraceEx2, OpenTraceW, ProcessTrace, StartTraceW,
    CONTROLTRACE_HANDLE, EVENT_CONTROL_CODE_ENABLE_PROVIDER, EVENT_RECORD,
    EVENT_TRACE_CONTROL_STOP, EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES,
    EVENT_TRACE_REAL_TIME_MODE, PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_REAL_TIME,
    TRACE_LEVEL_INFORMATION, WNODE_FLAG_TRACED_GUID,
};
use windows_sys::Win32::System::Performance::QueryPerformanceFrequency;

/// `Microsoft-Windows-DXGI` provider GUID: `{CA11C036-0102-4A2D-A6AD-F03CFED5D3C9}`.
const DXGI_PROVIDER: GUID = GUID::from_u128(0xCA11C036_0102_4A2D_A6AD_F03CFED5D3C9);
/// `Microsoft-Windows-D3D9` provider GUID: `{783ACA0A-790E-4D7F-8451-AA850511C6B9}`.
const D3D9_PROVIDER: GUID = GUID::from_u128(0x783ACA0A_790E_4D7F_8451_AA850511C6B9);

const DXGI_PRESENT_START_ID: u16 = 42;
const D3D9_PRESENT_START_ID: u16 = 1;

const SESSION_NAME: &str = "FPScraft_DXGI_Trace";
const MAX_TIMESTAMPS: usize = 512;

#[derive(Debug, Clone, Copy)]
pub struct EtwFpsSample {
    pub fps: f32,
    pub fps_1pct_low: f32,
    pub frametime_ms: f32,
}

struct SharedEtwState {
    target_pid: AtomicU32,
    active: AtomicBool,
    qpc_freq: i64,
    timestamps: Mutex<VecDeque<i64>>,
}

static GLOBAL_STATE: Mutex<Option<Arc<SharedEtwState>>> = Mutex::new(None);

/// Background ETW Present tracer for the current foreground window PID.
pub struct EtwFpsTracker {
    state: Arc<SharedEtwState>,
}

impl EtwFpsTracker {
    pub fn start() -> Self {
        let mut freq: i64 = 10_000_000;
        // SAFETY: QueryPerformanceFrequency writes to a valid stack i64.
        unsafe {
            let _ = QueryPerformanceFrequency(&mut freq);
        }
        let state = Arc::new(SharedEtwState {
            target_pid: AtomicU32::new(0),
            active: AtomicBool::new(false),
            qpc_freq: freq.max(1),
            timestamps: Mutex::new(VecDeque::with_capacity(MAX_TIMESTAMPS)),
        });

        if let Ok(mut guard) = GLOBAL_STATE.lock() {
            *guard = Some(Arc::clone(&state));
        }

        let bg_state = Arc::clone(&state);
        thread::spawn(move || run_etw_session(bg_state));

        Self { state }
    }

    pub fn set_target_pid(&self, pid: u32) {
        let prev = self.state.target_pid.swap(pid, Ordering::Relaxed);
        if prev != pid {
            if let Ok(mut ts) = self.state.timestamps.lock() {
                ts.clear();
            }
        }
    }

    pub fn sample(&self) -> Option<EtwFpsSample> {
        if !self.state.active.load(Ordering::Relaxed) {
            return None;
        }
        let Ok(ts) = self.state.timestamps.lock() else {
            return None;
        };
        if ts.len() < 2 {
            return None;
        }

        let freq = self.state.qpc_freq as f64;
        let newest = *ts.back()?;
        // Keep only frames within the last 1.5 seconds.
        let cutoff = newest - (freq * 1.5) as i64;
        let recent: Vec<i64> = ts.iter().copied().filter(|&t| t >= cutoff).collect();
        if recent.len() < 2 {
            return None;
        }

        let mut deltas_ms: Vec<f32> = recent
            .windows(2)
            .filter_map(|w| {
                let dt_ms = ((w[1] - w[0]) as f64 / freq * 1000.0) as f32;
                (dt_ms > 0.05 && dt_ms < 2000.0).then_some(dt_ms)
            })
            .collect();
        if deltas_ms.is_empty() {
            return None;
        }

        let avg_ft_ms = deltas_ms.iter().sum::<f32>() / deltas_ms.len() as f32;
        let fps = if avg_ft_ms > 0.0 {
            1000.0 / avg_ft_ms
        } else {
            0.0
        };

        deltas_ms.sort_by(|a, b| b.total_cmp(a));
        let top_n = (deltas_ms.len() as f32 * 0.01).ceil().max(1.0) as usize;
        let worst_avg_ms =
            deltas_ms.iter().take(top_n).sum::<f32>() / top_n.min(deltas_ms.len()) as f32;
        let fps_1pct_low = if worst_avg_ms > 0.0 {
            (1000.0 / worst_avg_ms).min(fps)
        } else {
            fps
        };

        Some(EtwFpsSample {
            fps,
            fps_1pct_low,
            frametime_ms: avg_ft_ms,
        })
    }
}

#[repr(C)]
struct TracePropertiesBuf {
    props: EVENT_TRACE_PROPERTIES,
    logger_name: [u16; 128],
}

fn new_trace_properties() -> TracePropertiesBuf {
    let mut buf: TracePropertiesBuf = unsafe { std::mem::zeroed() };
    buf.props.Wnode.BufferSize = std::mem::size_of::<TracePropertiesBuf>() as u32;
    buf.props.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
    buf.props.Wnode.ClientContext = 1; // QPC timestamps
    buf.props.LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
    buf.props.LoggerNameOffset = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
    buf
}

fn run_etw_session(state: Arc<SharedEtwState>) {
    let mut session_name_w: Vec<u16> = SESSION_NAME.encode_utf16().collect();
    session_name_w.push(0);

    // Stop any stale session from a previous run.
    let mut stop_buf = new_trace_properties();
    // SAFETY: valid zeroed TracePropertiesBuf and NUL-terminated UTF-16 session name.
    unsafe {
        let _ = ControlTraceW(
            CONTROLTRACE_HANDLE { Value: 0 },
            session_name_w.as_ptr(),
            &mut stop_buf.props,
            EVENT_TRACE_CONTROL_STOP,
        );
    }

    let mut start_buf = new_trace_properties();
    let mut session_handle = CONTROLTRACE_HANDLE { Value: 0 };
    // SAFETY: valid properties buffer and session name.
    let status = unsafe {
        StartTraceW(
            &mut session_handle,
            session_name_w.as_ptr(),
            &mut start_buf.props,
        )
    };
    if status != ERROR_SUCCESS && status != ERROR_ALREADY_EXISTS {
        return;
    }

    // Enable DXGI and D3D9 Present_Start providers.
    // SAFETY: session_handle is a valid trace handle from StartTraceW.
    unsafe {
        let _ = EnableTraceEx2(
            session_handle,
            &DXGI_PROVIDER,
            EVENT_CONTROL_CODE_ENABLE_PROVIDER,
            TRACE_LEVEL_INFORMATION as u8,
            0,
            0,
            0,
            std::ptr::null(),
        );
        let _ = EnableTraceEx2(
            session_handle,
            &D3D9_PROVIDER,
            EVENT_CONTROL_CODE_ENABLE_PROVIDER,
            TRACE_LEVEL_INFORMATION as u8,
            0,
            0,
            0,
            std::ptr::null(),
        );
    }

    let mut logfile: EVENT_TRACE_LOGFILEW = unsafe { std::mem::zeroed() };
    logfile.LoggerName = session_name_w.as_mut_ptr();
    logfile.Anonymous1.ProcessTraceMode =
        PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
    logfile.Anonymous2.EventRecordCallback = Some(event_record_callback);

    // SAFETY: logfile is properly initialized with a valid logger name and callback.
    let trace_handle = unsafe { OpenTraceW(&mut logfile) };
    if trace_handle.Value == INVALID_HANDLE_VALUE as u64 {
        return;
    }

    state.active.store(true, Ordering::Relaxed);
    let handles = [trace_handle];
    // SAFETY: ProcessTrace blocks on the valid trace_handle until the session stops.
    unsafe {
        let _ = ProcessTrace(handles.as_ptr(), 1, std::ptr::null(), std::ptr::null());
        let _ = CloseTrace(trace_handle);
    }
    state.active.store(false, Ordering::Relaxed);
}

unsafe extern "system" fn event_record_callback(event: *mut EVENT_RECORD) {
    if event.is_null() {
        return;
    }
    // SAFETY: event pointer is non-null and provided by ETW ProcessTrace.
    let rec = unsafe { &*event };
    let provider = rec.EventHeader.ProviderId;
    let id = rec.EventHeader.EventDescriptor.Id;

    let is_dxgi_present = guid_eq(provider, DXGI_PROVIDER) && id == DXGI_PRESENT_START_ID;
    let is_d3d9_present = guid_eq(provider, D3D9_PROVIDER) && id == D3D9_PRESENT_START_ID;
    if !is_dxgi_present && !is_d3d9_present {
        return;
    }

    let Ok(guard) = GLOBAL_STATE.try_lock() else {
        return;
    };
    let Some(state) = guard.as_ref() else {
        return;
    };
    let target_pid = state.target_pid.load(Ordering::Relaxed);
    if target_pid == 0 || rec.EventHeader.ProcessId != target_pid {
        return;
    }

    if let Ok(mut ts) = state.timestamps.try_lock() {
        if ts.len() >= MAX_TIMESTAMPS {
            ts.pop_front();
        }
        ts.push_back(rec.EventHeader.TimeStamp);
    };
}

fn guid_eq(a: GUID, b: GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}
