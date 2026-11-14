use std::collections::VecDeque;
use std::path::Path;
use std::time::Instant;

use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use nvml_wrapper::Nvml;
use sysinfo::{
    Components, CpuRefreshKind, Disks, MemoryRefreshKind, Networks, RefreshKind, System,
};
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingA, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_READ,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

const FRAMETIME_HISTORY_LEN: usize = 90;
const SENSOR_POLL_INTERVAL_SECS: f32 = 0.5;
const BYTES_PER_GIB: f32 = 1_073_741_824.0;
const MAHM_SIGNATURE: u32 = 0x4D41_484D; // 'MAHM'

/// Snapshot of all hardware, FPS, memory, and system telemetry rendered by the overlay.
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    // FPS & frametime metrics
    pub fps: f32,
    pub fps_1pct_low: f32,
    pub frametime_ms: f32,
    pub fps_history: Vec<f32>,
    pub fps_source: &'static str,
    pub active_app: String,

    // CPU metrics
    pub cpu_name: String,
    pub cpu_usage: f32,
    pub cpu_cores_usage: Vec<f32>,
    pub cpu_temp_c: Option<f32>,
    pub cpu_clock_mhz: u32,
    pub cpu_power_w: Option<f32>,

    // GPU metrics
    pub gpu_name: String,
    pub gpu_temp_c: Option<f32>,
    pub gpu_usage: Option<f32>,
    pub vram_used_gb: f32,
    pub vram_total_gb: f32,
    pub gpu_power_w: Option<f32>,
    pub gpu_clock_mhz: Option<u32>,
    pub gpu_fan_pct: Option<u32>,

    // RAM & swap metrics
    pub ram_free_gb: f32,
    pub ram_used_gb: f32,
    pub ram_total_gb: f32,
    pub ram_usage_pct: f32,
    pub swap_used_gb: f32,

    // Network, disk & uptime metrics
    pub net_down_kbps: f32,
    pub net_up_kbps: f32,
    pub disk_free_gb: f32,
    pub uptime_secs: u64,
}

impl Default for MetricsSnapshot {
    fn default() -> Self {
        Self {
            fps: 0.0,
            fps_1pct_low: 0.0,
            frametime_ms: 0.0,
            fps_history: Vec::with_capacity(FRAMETIME_HISTORY_LEN),
            fps_source: "Overlay",
            active_app: String::from("Desktop"),
            cpu_name: String::from("CPU"),
            cpu_usage: 0.0,
            cpu_cores_usage: Vec::new(),
            cpu_temp_c: None,
            cpu_clock_mhz: 0,
            cpu_power_w: None,
            gpu_name: String::from("GPU"),
            gpu_temp_c: None,
            gpu_usage: None,
            vram_used_gb: 0.0,
            vram_total_gb: 0.0,
            gpu_power_w: None,
            gpu_clock_mhz: None,
            gpu_fan_pct: None,
            ram_free_gb: 0.0,
            ram_used_gb: 0.0,
            ram_total_gb: 0.0,
            ram_usage_pct: 0.0,
            swap_used_gb: 0.0,
            net_down_kbps: 0.0,
            net_up_kbps: 0.0,
            disk_free_gb: 0.0,
            uptime_secs: 0,
        }
    }
}

#[derive(Debug, Default)]
struct MahmData {
    cpu_temp_c: Option<f32>,
    cpu_clock_mhz: Option<u32>,
    cpu_power_w: Option<f32>,
    cpu_usage: Option<f32>,
    gpu_temp_c: Option<f32>,
    gpu_usage: Option<f32>,
    vram_used_mb: Option<f32>,
    gpu_clock_mhz: Option<u32>,
    gpu_power_w: Option<f32>,
}

/// Collects system, GPU, CPU, memory, network, disk, and FPS telemetry.
pub struct MetricsCollector {
    sys: System,
    networks: Networks,
    disks: Disks,
    components: Components,
    nvml: Option<Nvml>,
    snapshot: MetricsSnapshot,
    frametime_samples_ms: VecDeque<f32>,
    fps_samples: VecDeque<f32>,
    last_sensor_poll: Instant,
    external_fps: Option<f32>,
}

impl MetricsCollector {
    /// Initializes system monitors, NVML (if present), and takes an initial sensor snapshot.
    pub fn new() -> Self {
        let refresh = RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything());
        let mut sys = System::new_with_specifics(refresh);
        sys.refresh_cpu_all();
        sys.refresh_memory();

        let networks = Networks::new_with_refreshed_list();
        let disks = Disks::new_with_refreshed_list();
        let components = Components::new_with_refreshed_list();
        let nvml = Nvml::init().ok();

        let cpu_name = clean_cpu_name(sys.cpus().first().map(|c| c.brand()).unwrap_or("CPU"));

        let gpu_name = nvml
            .as_ref()
            .and_then(|n| n.device_by_index(0).ok())
            .and_then(|d| d.name().ok())
            .map(|s| clean_gpu_name(&s))
            .unwrap_or_else(|| String::from("GPU"));

        let mut collector = Self {
            sys,
            networks,
            disks,
            components,
            nvml,
            snapshot: MetricsSnapshot {
                cpu_name,
                gpu_name,
                ..Default::default()
            },
            frametime_samples_ms: VecDeque::with_capacity(FRAMETIME_HISTORY_LEN),
            fps_samples: VecDeque::with_capacity(FRAMETIME_HISTORY_LEN),
            last_sensor_poll: Instant::now(),
            external_fps: None,
        };

        collector.refresh_sensors(0.5);
        collector
    }

    /// Records a rendered frame delta (in seconds) and refreshes hardware sensors every 500 ms.
    pub fn on_frame(&mut self, dt_secs: f32) {
        let clamped_dt = dt_secs.clamp(0.0002, 1.0);
        let ft_ms = clamped_dt * 1000.0;

        if self.frametime_samples_ms.len() >= FRAMETIME_HISTORY_LEN {
            self.frametime_samples_ms.pop_front();
        }
        self.frametime_samples_ms.push_back(ft_ms);

        let now = Instant::now();
        let elapsed = now.duration_since(self.last_sensor_poll).as_secs_f32();
        if elapsed >= SENSOR_POLL_INTERVAL_SECS {
            self.last_sensor_poll = now;
            self.refresh_sensors(elapsed);
        }

        self.update_fps_fields();
    }

    /// Forces an immediate hardware sensor poll (used in unit tests).
    #[cfg(test)]
    pub fn force_refresh(&mut self) {
        let now = Instant::now();
        let elapsed = now
            .duration_since(self.last_sensor_poll)
            .as_secs_f32()
            .max(0.05);
        self.last_sensor_poll = now;
        self.refresh_sensors(elapsed);
        self.update_fps_fields();
    }

    /// Returns the latest telemetry snapshot.
    pub fn snapshot(&self) -> &MetricsSnapshot {
        &self.snapshot
    }

    fn update_fps_fields(&mut self) {
        let (avg_ft_ms, low_1pct_fps) = if self.frametime_samples_ms.is_empty() {
            (0.0, 0.0)
        } else {
            let sum: f32 = self.frametime_samples_ms.iter().sum();
            let avg = sum / self.frametime_samples_ms.len() as f32;

            let mut sorted: Vec<f32> = self.frametime_samples_ms.iter().copied().collect();
            sorted.sort_by(|a, b| b.total_cmp(a));
            let top_count = (sorted.len() / 100).max(1);
            let worst_avg_ms: f32 = sorted.iter().take(top_count).sum::<f32>() / top_count as f32;
            let low_fps = if worst_avg_ms > 0.0 {
                1000.0 / worst_avg_ms
            } else {
                0.0
            };
            (avg, low_fps)
        };

        let sample_fps =
            if let Some(ext_fps) = self.external_fps.filter(|&v| v > 0.0 && v < 10_000.0) {
                self.snapshot.fps = ext_fps;
                self.snapshot.fps_source = "PresentMon";
                self.snapshot.frametime_ms = if avg_ft_ms > 0.0 {
                    avg_ft_ms
                } else {
                    1000.0 / ext_fps
                };
                self.snapshot.fps_1pct_low = if low_1pct_fps > 0.0 {
                    low_1pct_fps.min(ext_fps)
                } else {
                    ext_fps
                };
                ext_fps
            } else {
                let overlay_fps = if avg_ft_ms > 0.0 {
                    1000.0 / avg_ft_ms
                } else {
                    0.0
                };
                self.snapshot.fps = overlay_fps;
                self.snapshot.fps_source = "Overlay";
                self.snapshot.frametime_ms = avg_ft_ms;
                self.snapshot.fps_1pct_low = low_1pct_fps;
                self.frametime_samples_ms
                    .back()
                    .map(|&ft| if ft > 0.0 { 1000.0 / ft } else { overlay_fps })
                    .unwrap_or(overlay_fps)
            };

        if sample_fps > 0.0 {
            if self.fps_samples.len() >= FRAMETIME_HISTORY_LEN {
                self.fps_samples.pop_front();
            }
            self.fps_samples.push_back(sample_fps);
        }

        self.snapshot.fps_history.clear();
        self.snapshot
            .fps_history
            .extend(self.fps_samples.iter().copied());
    }

    fn refresh_sensors(&mut self, elapsed_secs: f32) {
        self.sys.refresh_cpu_all();
        self.sys.refresh_memory();
        self.networks.refresh(true);
        self.disks.refresh(true);
        self.components.refresh(true);

        let mahm = read_mahm_shared_memory().unwrap_or_default();
        self.external_fps = read_msi_trace_fps();

        if let Some(app) = read_foreground_process_name() {
            self.snapshot.active_app = app;
        }

        // CPU telemetry
        let sys_cpu_usage = self.sys.global_cpu_usage();
        self.snapshot.cpu_usage = mahm
            .cpu_usage
            .filter(|&v| v > 0.0 || sys_cpu_usage <= 0.0)
            .unwrap_or(sys_cpu_usage);

        self.snapshot.cpu_cores_usage = self.sys.cpus().iter().map(|c| c.cpu_usage()).collect();

        let fallback_clock = self
            .sys
            .cpus()
            .first()
            .map(|c| c.frequency() as u32)
            .unwrap_or(0);
        self.snapshot.cpu_clock_mhz = mahm.cpu_clock_mhz.unwrap_or(fallback_clock);
        self.snapshot.cpu_power_w = mahm.cpu_power_w;
        self.snapshot.cpu_temp_c = mahm.cpu_temp_c.or_else(|| self.read_fallback_cpu_temp());

        // GPU telemetry (NVML primary, MAHM fallback)
        self.refresh_gpu_sensors(&mahm);

        // RAM & swap telemetry
        let total_bytes = self.sys.total_memory() as f32;
        let avail_bytes = self.sys.available_memory() as f32;
        let used_bytes = self.sys.used_memory() as f32;

        self.snapshot.ram_total_gb = total_bytes / BYTES_PER_GIB;
        self.snapshot.ram_free_gb = avail_bytes / BYTES_PER_GIB;
        self.snapshot.ram_used_gb = used_bytes / BYTES_PER_GIB;
        self.snapshot.ram_usage_pct = if total_bytes > 0.0 {
            (used_bytes / total_bytes) * 100.0
        } else {
            0.0
        };
        self.snapshot.swap_used_gb = self.sys.used_swap() as f32 / BYTES_PER_GIB;

        // Network throughput (KB/s)
        let dt = elapsed_secs.max(0.05);
        let mut down_bytes: u64 = 0;
        let mut up_bytes: u64 = 0;
        for (name, data) in &self.networks {
            if is_virtual_net_interface(name) {
                continue;
            }
            down_bytes = down_bytes.saturating_add(data.received());
            up_bytes = up_bytes.saturating_add(data.transmitted());
        }
        self.snapshot.net_down_kbps = (down_bytes as f32 / 1024.0) / dt;
        self.snapshot.net_up_kbps = (up_bytes as f32 / 1024.0) / dt;

        // Primary disk free space (C:\ or first disk)
        let primary_disk = self
            .disks
            .iter()
            .find(|d| {
                d.mount_point()
                    .to_string_lossy()
                    .to_ascii_uppercase()
                    .starts_with("C:")
            })
            .or_else(|| self.disks.first());
        self.snapshot.disk_free_gb = primary_disk
            .map(|d| d.available_space() as f32 / BYTES_PER_GIB)
            .unwrap_or(0.0);

        self.snapshot.uptime_secs = System::uptime();
    }

    fn refresh_gpu_sensors(&mut self, mahm: &MahmData) {
        let nvml_device = self
            .nvml
            .as_ref()
            .and_then(|nvml| nvml.device_by_index(0).ok());

        if let Some(dev) = nvml_device {
            if let Ok(name) = dev.name() {
                self.snapshot.gpu_name = clean_gpu_name(&name);
            }
            self.snapshot.gpu_temp_c = dev
                .temperature(TemperatureSensor::Gpu)
                .ok()
                .map(|t| t as f32)
                .or(mahm.gpu_temp_c);
            self.snapshot.gpu_usage = dev
                .utilization_rates()
                .ok()
                .map(|u| u.gpu as f32)
                .or(mahm.gpu_usage);
            if let Ok(mem) = dev.memory_info() {
                self.snapshot.vram_used_gb = mem.used as f32 / BYTES_PER_GIB;
                self.snapshot.vram_total_gb = mem.total as f32 / BYTES_PER_GIB;
            } else if let Some(used_mb) = mahm.vram_used_mb {
                self.snapshot.vram_used_gb = used_mb / 1024.0;
            }
            self.snapshot.gpu_power_w = dev
                .power_usage()
                .ok()
                .map(|mw| mw as f32 / 1000.0)
                .or(mahm.gpu_power_w);
            self.snapshot.gpu_clock_mhz =
                dev.clock_info(Clock::Graphics).ok().or(mahm.gpu_clock_mhz);
            self.snapshot.gpu_fan_pct = dev.fan_speed(0).ok();
        } else {
            self.snapshot.gpu_temp_c = mahm.gpu_temp_c;
            self.snapshot.gpu_usage = mahm.gpu_usage;
            if let Some(used_mb) = mahm.vram_used_mb {
                self.snapshot.vram_used_gb = used_mb / 1024.0;
            }
            self.snapshot.gpu_power_w = mahm.gpu_power_w;
            self.snapshot.gpu_clock_mhz = mahm.gpu_clock_mhz;
            self.snapshot.gpu_fan_pct = None;
        }
    }

    fn read_fallback_cpu_temp(&self) -> Option<f32> {
        let mut best: Option<f32> = None;
        for comp in &self.components {
            let label = comp.label().to_ascii_lowercase();
            if label.contains("cpu") || label.contains("package") || label.contains("tctl") {
                if let Some(temp) = comp.temperature() {
                    if temp > 0.0 && temp < 150.0 {
                        return Some(temp);
                    }
                }
            }
            if best.is_none() {
                if let Some(temp) = comp.temperature() {
                    if temp > 0.0 && temp < 150.0 {
                        best = Some(temp);
                    }
                }
            }
        }
        best
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

fn clean_cpu_name(raw: &str) -> String {
    let trimmed = raw
        .replace("6-Core Processor", "")
        .replace("8-Core Processor", "")
        .replace("12-Core Processor", "")
        .replace("16-Core Processor", "")
        .replace("(R)", "")
        .replace("(TM)", "")
        .trim()
        .to_string();
    if trimmed.is_empty() {
        String::from("CPU")
    } else {
        trimmed
    }
}

fn clean_gpu_name(raw: &str) -> String {
    raw.replace("NVIDIA ", "").trim().to_string()
}

fn is_virtual_net_interface(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("loopback")
        || lower.contains("vethernet")
        || lower.contains("pseudo")
        || lower.contains("teredo")
}

/// Reads MSI Afterburner's `MAHMSharedMemory` shared memory block if running.
fn read_mahm_shared_memory() -> Option<MahmData> {
    const NAME: &[u8] = b"MAHMSharedMemory\0";

    // SAFETY: Win32 shared memory handle is opened read-only, validated before any pointer
    // dereference, and unconditionally unmapped/closed before returning.
    unsafe {
        let handle = OpenFileMappingA(FILE_MAP_READ, 0, NAME.as_ptr());
        if handle.is_null() {
            return None;
        }

        let view = MapViewOfFile(handle, FILE_MAP_READ, 0, 0, 0);
        if view.Value.is_null() {
            CloseHandle(handle);
            return None;
        }

        let result = parse_mahm_view(view.Value as *const u8);

        UnmapViewOfFile(view);
        CloseHandle(handle);
        result
    }
}

/// Parses the mapped `MAHMSharedMemory` header and sensor entries.
///
/// # Safety
/// `base_ptr` must point to a valid read-only mapped `MAHMSharedMemory` view.
unsafe fn parse_mahm_view(base_ptr: *const u8) -> Option<MahmData> {
    let header = std::slice::from_raw_parts(base_ptr, 24);
    let sig = u32::from_le_bytes(header[0..4].try_into().ok()?);
    if sig != MAHM_SIGNATURE {
        return None;
    }

    let header_size = u32::from_le_bytes(header[8..12].try_into().ok()?) as usize;
    let num_entries = u32::from_le_bytes(header[12..16].try_into().ok()?) as usize;
    let entry_size = u32::from_le_bytes(header[16..20].try_into().ok()?) as usize;

    if header_size < 24 || entry_size < 1304 || num_entries == 0 || num_entries > 256 {
        return None;
    }

    let total_bytes = header_size.checked_add(num_entries.checked_mul(entry_size)?)?;
    let buf = std::slice::from_raw_parts(base_ptr, total_bytes);

    let mut data = MahmData::default();
    for i in 0..num_entries {
        let entry_offset = header_size + i * entry_size;
        let name_slice = &buf[entry_offset..entry_offset + 260];
        let name_len = name_slice
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(name_slice.len());
        let name = match std::str::from_utf8(&name_slice[..name_len]) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let val_offset = entry_offset + 1300; // 260 * 5 (after 5 char[260] fields)
        let val_bytes: [u8; 4] = buf[val_offset..val_offset + 4].try_into().ok()?;
        let val = f32::from_le_bytes(val_bytes);

        if !val.is_finite() || val.abs() >= 1_000_000.0 {
            continue;
        }

        match name {
            "CPU temperature" => data.cpu_temp_c = Some(val),
            "CPU clock" => data.cpu_clock_mhz = Some(val.round().max(0.0) as u32),
            "CPU power" => data.cpu_power_w = Some(val),
            "CPU usage" => data.cpu_usage = Some(val),
            "GPU temperature" => data.gpu_temp_c = Some(val),
            "GPU usage" => data.gpu_usage = Some(val),
            "Memory usage" => data.vram_used_mb = Some(val),
            "Core clock" => data.gpu_clock_mhz = Some(val.round().max(0.0) as u32),
            "Power" => data.gpu_power_w = Some(val),
            _ => {}
        }
    }

    Some(data)
}

/// Reads live PresentMon FPS from MSI Companion's `SharedMemory_FPS_Engine` if active.
fn read_msi_trace_fps() -> Option<f32> {
    let wide_name: Vec<u16> = "SharedMemory_FPS_Engine\0".encode_utf16().collect();

    // SAFETY: Opens read-only mapping of up to 64 bytes, copies UTF-16 code units,
    // and unconditionally unmaps and closes the handle.
    unsafe {
        let handle = OpenFileMappingW(FILE_MAP_READ, 0, wide_name.as_ptr());
        if handle.is_null() {
            return None;
        }

        let view = MapViewOfFile(handle, FILE_MAP_READ, 0, 0, 64);
        if view.Value.is_null() {
            CloseHandle(handle);
            return None;
        }

        let units = std::slice::from_raw_parts(view.Value as *const u16, 32);
        let len = units.iter().position(|&c| c == 0).unwrap_or(units.len());
        let text = String::from_utf16_lossy(&units[..len]);

        UnmapViewOfFile(view);
        CloseHandle(handle);

        text.trim().parse::<f32>().ok().filter(|v| v.is_finite())
    }
}

/// Returns the executable file name of the current foreground window.
fn read_foreground_process_name() -> Option<String> {
    // SAFETY: Win32 foreground window and process query handles are checked for null/zero
    // and closed properly.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }

        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return None;
        }

        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }

        let mut buf = [0u16; 260];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut size);
        CloseHandle(process);

        if ok == 0 || size == 0 {
            return None;
        }

        let full_path = String::from_utf16_lossy(&buf[..size as usize]);
        Path::new(&full_path)
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn collects_live_system_and_hardware_metrics() {
        let mut collector = MetricsCollector::new();
        thread::sleep(Duration::from_millis(250));
        collector.on_frame(0.016);
        collector.force_refresh();

        let snap = collector.snapshot();
        println!("Live Telemetry Snapshot:");
        println!(
            "  FPS: {:.1} ({}) | 1% Low: {:.1} | Frame: {:.2} ms",
            snap.fps, snap.fps_source, snap.fps_1pct_low, snap.frametime_ms
        );
        println!("  Active App: {}", snap.active_app);
        println!(
            "  CPU: {} | Usage: {:.1}% | Temp: {:?} C | Clock: {} MHz | Power: {:?} W",
            snap.cpu_name, snap.cpu_usage, snap.cpu_temp_c, snap.cpu_clock_mhz, snap.cpu_power_w
        );
        println!("  GPU: {} | Usage: {:?}% | Temp: {:?} C | VRAM: {:.2}/{:.2} GB | Power: {:?} W | Clock: {:?} MHz | Fan: {:?}%", snap.gpu_name, snap.gpu_usage, snap.gpu_temp_c, snap.vram_used_gb, snap.vram_total_gb, snap.gpu_power_w, snap.gpu_clock_mhz, snap.gpu_fan_pct);
        println!(
            "  RAM Free: {:.2} GB | Used: {:.2}/{:.2} GB ({:.1}%) | Swap Used: {:.2} GB",
            snap.ram_free_gb,
            snap.ram_used_gb,
            snap.ram_total_gb,
            snap.ram_usage_pct,
            snap.swap_used_gb
        );
        println!(
            "  Net Down: {:.1} KB/s | Up: {:.1} KB/s | Disk C Free: {:.1} GB | Uptime: {}s",
            snap.net_down_kbps, snap.net_up_kbps, snap.disk_free_gb, snap.uptime_secs
        );

        assert!(snap.ram_total_gb > 1.0, "Total RAM should be positive");
        assert!(snap.ram_free_gb > 0.0, "Free RAM should be positive");
        assert!(
            !snap.cpu_cores_usage.is_empty(),
            "CPU threads should be detected"
        );
        assert!(snap.fps > 0.0, "FPS should be positive");
    }
}
