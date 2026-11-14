use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, WAIT_OBJECT_0};
use windows_sys::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, VirtualQuery, FILE_MAP_READ,
    MEMORY_BASIC_INFORMATION,
};
use windows_sys::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhExpandWildCardPathW,
    PdhGetFormattedCounterValue, PdhOpenQueryW, PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE,
};
use windows_sys::Win32::System::Threading::{OpenMutexW, ReleaseMutex, WaitForSingleObject};

const PDH_MORE_DATA: u32 = 0x8000_07D2;
const PDH_CSTATUS_VALID_DATA: u32 = 0x0000_0000;
const PDH_CSTATUS_NEW_DATA: u32 = 0x0000_0001;

#[derive(Debug, Default, Clone, Copy)]
pub struct ExtraCpuSensors {
    pub cpu_clock_mhz: Option<u32>,
    pub cpu_power_w: Option<f32>,
    pub cpu_temp_c: Option<f32>,
}

/// Polls Windows PDH counters for real boosted CPU frequency, RAPL package power,
/// and ACPI thermal zones without requiring administrator privileges or kernel drivers.
pub struct PdhSensors {
    query: isize,
    perf_pct_counter: Option<isize>,
    base_freq_counter: Option<isize>,
    pkg_power_counter: Option<isize>,
    core_power_counters: Vec<isize>,
    thermal_hp_counters: Vec<isize>,
    thermal_counters: Vec<isize>,
    primed: bool,
}

impl PdhSensors {
    pub fn new() -> Option<Self> {
        let mut query: isize = 0;
        // SAFETY: PdhOpenQueryW writes a valid query handle to `query` on ERROR_SUCCESS.
        let status = unsafe { PdhOpenQueryW(ptr::null(), 0, &mut query) };
        if status != ERROR_SUCCESS || query == 0 {
            return None;
        }

        let perf_pct_counter =
            add_counter(query, r"\Processor Information(_Total)\% Processor Performance");
        let base_freq_counter =
            add_counter(query, r"\Processor Information(_Total)\Processor Frequency");

        let pkg_power_counter = add_counter(query, r"\Energy Meter(rapl_package0_pkg)\Power");
        let mut core_power_counters = Vec::new();
        if pkg_power_counter.is_none() {
            for path in expand_wildcard(r"\Energy Meter(*)\Power") {
                let lower = path.to_ascii_lowercase();
                if lower.contains("rapl_package0") && !lower.contains("_total") {
                    if let Some(c) = add_counter(query, &path) {
                        core_power_counters.push(c);
                    }
                }
            }
        }

        let thermal_hp_counters: Vec<isize> =
            expand_wildcard(r"\Thermal Zone Information(*)\High Precision Temperature")
                .into_iter()
                .filter_map(|p| add_counter(query, &p))
                .collect();
        let thermal_counters: Vec<isize> = if thermal_hp_counters.is_empty() {
            expand_wildcard(r"\Thermal Zone Information(*)\Temperature")
                .into_iter()
                .filter_map(|p| add_counter(query, &p))
                .collect()
        } else {
            Vec::new()
        };

        // Prime rate-based PDH counters (`% Processor Performance`) with an initial sample.
        // SAFETY: `query` is a valid open PDH query handle.
        let primed = unsafe { PdhCollectQueryData(query) == ERROR_SUCCESS };

        Some(Self {
            query,
            perf_pct_counter,
            base_freq_counter,
            pkg_power_counter,
            core_power_counters,
            thermal_hp_counters,
            thermal_counters,
            primed,
        })
    }

    pub fn poll(&mut self, fallback_base_mhz: u32) -> ExtraCpuSensors {
        let mut out = ExtraCpuSensors::default();
        if self.query == 0 {
            return out;
        }

        // SAFETY: `self.query` is a valid open PDH query handle.
        let status = unsafe { PdhCollectQueryData(self.query) };
        if status != ERROR_SUCCESS {
            return out;
        }
        if !self.primed {
            self.primed = true;
            return out;
        }

        let base_mhz = self
            .base_freq_counter
            .and_then(read_counter_double)
            .filter(|&v| v > 100.0 && v < 10_000.0)
            .unwrap_or(fallback_base_mhz as f64);

        if base_mhz > 0.0 {
            if let Some(perf_pct) = self.perf_pct_counter.and_then(read_counter_double) {
                let effective = (base_mhz * (perf_pct / 100.0)).round();
                if effective >= 200.0 && effective <= 10_000.0 {
                    out.cpu_clock_mhz = Some(effective as u32);
                }
            }
        }

        if let Some(mw) = self.pkg_power_counter.and_then(read_counter_double) {
            let watts = (mw / 1000.0) as f32;
            if watts.is_finite() && watts > 0.0 && watts < 1000.0 {
                out.cpu_power_w = Some(watts);
            }
        } else if !self.core_power_counters.is_empty() {
            let mut sum_mw = 0.0_f64;
            let mut any = false;
            for &c in &self.core_power_counters {
                if let Some(mw) = read_counter_double(c) {
                    if mw > 0.0 {
                        sum_mw += mw;
                        any = true;
                    }
                }
            }
            if any {
                let watts = (sum_mw / 1000.0) as f32;
                if watts.is_finite() && watts > 0.0 && watts < 1000.0 {
                    out.cpu_power_w = Some(watts);
                }
            }
        }

        for &c in &self.thermal_hp_counters {
            if let Some(raw) = read_counter_double(c) {
                let celsius = (raw / 10.0 - 273.15) as f32;
                if celsius.is_finite() && celsius > 5.0 && celsius < 130.0 {
                    out.cpu_temp_c = Some(match out.cpu_temp_c {
                        Some(prev) => prev.max(celsius),
                        None => celsius,
                    });
                }
            }
        }
        if out.cpu_temp_c.is_none() {
            for &c in &self.thermal_counters {
                if let Some(raw) = read_counter_double(c) {
                    let celsius = (raw - 273.15) as f32;
                    if celsius.is_finite() && celsius > 5.0 && celsius < 130.0 {
                        out.cpu_temp_c = Some(match out.cpu_temp_c {
                            Some(prev) => prev.max(celsius),
                            None => celsius,
                        });
                    }
                }
            }
        }

        out
    }
}

impl Drop for PdhSensors {
    fn drop(&mut self) {
        if self.query != 0 {
            // SAFETY: `self.query` is owned by this struct and closed once on drop.
            unsafe {
                let _ = PdhCloseQuery(self.query);
            }
            self.query = 0;
        }
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn add_counter(query: isize, path: &str) -> Option<isize> {
    let w = to_wide(path);
    let mut counter: isize = 0;
    // SAFETY: `query` is a valid PDH query handle and `w` is NUL-terminated UTF-16.
    let status = unsafe { PdhAddEnglishCounterW(query, w.as_ptr(), 0, &mut counter) };
    (status == ERROR_SUCCESS && counter != 0).then_some(counter)
}

fn expand_wildcard(pattern: &str) -> Vec<String> {
    let w = to_wide(pattern);
    let mut len: u32 = 0;
    // SAFETY: First call queries required buffer size in UTF-16 code units.
    let st = unsafe { PdhExpandWildCardPathW(ptr::null(), w.as_ptr(), ptr::null_mut(), &mut len, 0) };
    if (st != PDH_MORE_DATA && st != ERROR_SUCCESS) || len == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u16; len as usize];
    // SAFETY: `buf` has capacity `len` UTF-16 code units.
    let st2 = unsafe {
        PdhExpandWildCardPathW(ptr::null(), w.as_ptr(), buf.as_mut_ptr(), &mut len, 0)
    };
    if st2 != ERROR_SUCCESS {
        return Vec::new();
    }
    let slice = &buf[..len.min(buf.len() as u32) as usize];
    slice
        .split(|&ch| ch == 0)
        .filter(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

fn read_counter_double(counter: isize) -> Option<f64> {
    let mut val: PDH_FMT_COUNTERVALUE = unsafe { std::mem::zeroed() };
    // SAFETY: `counter` is a valid PDH counter handle and `val` is stack-allocated.
    let status = unsafe { PdhGetFormattedCounterValue(counter, PDH_FMT_DOUBLE, ptr::null_mut(), &mut val) };
    if status != ERROR_SUCCESS {
        return None;
    }
    if val.CStatus != PDH_CSTATUS_VALID_DATA && val.CStatus != PDH_CSTATUS_NEW_DATA {
        return None;
    }
    // SAFETY: `PDH_FMT_DOUBLE` populates `Anonymous.doubleValue`.
    let d = unsafe { val.Anonymous.doubleValue };
    d.is_finite().then_some(d)
}

/// Reads live CPU sensors from HWiNFO64 shared memory (`Global\HWiNFO_SENS_SM2`) if active.
pub fn read_hwinfo_shared_memory() -> Option<ExtraCpuSensors> {
    let map_name = to_wide(r"Global\HWiNFO_SENS_SM2");
    let mutex_name = to_wide(r"Global\HWiNFO_SM2_MUTEX");

    // SAFETY: Opens read-only shared memory mapping, optionally waits briefly on HWiNFO's
    // mutex, queries actual mapped region size via VirtualQuery, and unmaps/closes handles.
    unsafe {
        let handle = OpenFileMappingW(FILE_MAP_READ, 0, map_name.as_ptr());
        if handle.is_null() {
            return None;
        }
        let view = MapViewOfFile(handle, FILE_MAP_READ, 0, 0, 0);
        if view.Value.is_null() {
            CloseHandle(handle);
            return None;
        }

        let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
        let queried = VirtualQuery(
            view.Value,
            &mut mbi,
            std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        );
        if queried == 0 || mbi.RegionSize < 44 {
            UnmapViewOfFile(view);
            CloseHandle(handle);
            return None;
        }

        let mutex = OpenMutexW(0x0010_0001, 0, mutex_name.as_ptr()); // SYNCHRONIZE | MUTEX_MODIFY_STATE
        let locked = !mutex.is_null() && WaitForSingleObject(mutex, 15) == WAIT_OBJECT_0;

        let buf = std::slice::from_raw_parts(view.Value as *const u8, mbi.RegionSize);
        let parsed = parse_hwinfo_view(buf);

        if locked {
            ReleaseMutex(mutex);
        }
        if !mutex.is_null() {
            CloseHandle(mutex);
        }
        UnmapViewOfFile(view);
        CloseHandle(handle);
        parsed
    }
}

fn read_u32_le(buf: &[u8], off: usize) -> Option<u32> {
    let bytes: [u8; 4] = buf.get(off..off + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn read_f64_le(buf: &[u8], off: usize) -> Option<f64> {
    let bytes: [u8; 8] = buf.get(off..off + 8)?.try_into().ok()?;
    let v = f64::from_le_bytes(bytes);
    v.is_finite().then_some(v)
}

fn read_cstr(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).trim().to_string()
}

fn parse_hwinfo_view(buf: &[u8]) -> Option<ExtraCpuSensors> {
    // Signature "HWiS" at offset 0..4
    if buf.get(0..4) != Some(b"HWiS") {
        return None;
    }

    // HWiNFO_SENSORS_SHARED_MEM2 (#pragma pack(1)):
    // 0: dwSignature(4), 4: dwVersion(4), 8: dwRevision(4), 12: poll_time(8),
    // 20: dwOffsetOfSensorSection(4), 24: dwSizeOfSensorElement(4), 28: dwNumSensorElements(4),
    // 32: dwOffsetOfReadingSection(4), 36: dwSizeOfReadingElement(4), 40: dwNumReadingElements(4)
    let off_readings = read_u32_le(buf, 32)? as usize;
    let size_reading = read_u32_le(buf, 36)? as usize;
    let num_readings = read_u32_le(buf, 40)? as usize;

    // Minimum size of HWiNFO_SENSORS_READING_ELEMENT:
    // tReading(4) + dwSensorIndex(4) + dwReadingID(4) + szLabelOrig(128) + szLabelUser(128)
    // + szUnit(16) + Value(8) + ValueMin(8) + ValueMax(8) + ValueAvg(8) = 316 bytes
    if size_reading < 316 || num_readings == 0 || num_readings > 4096 {
        return None;
    }
    let end_readings = off_readings.checked_add(num_readings.checked_mul(size_reading)?)?;
    if end_readings > buf.len() {
        return None;
    }

    let mut out = ExtraCpuSensors::default();
    let mut temp_prio = 0u8;
    let mut power_prio = 0u8;
    let mut clock_prio = 0u8;

    for i in 0..num_readings {
        let base = off_readings + i * size_reading;
        let t_reading = read_u32_le(buf, base)?;
        let label_orig = read_cstr(&buf[base + 12..base + 140]).to_ascii_lowercase();
        let value = match read_f64_le(buf, base + 284) {
            Some(v) => v,
            None => continue,
        };

        match t_reading {
            // SENSOR_TYPE_TEMP = 1
            1 if value > 5.0 && value < 130.0 => {
                let prio = if label_orig.contains("tctl/tdie") {
                    5
                } else if label_orig.contains("cpu die") || label_orig.contains("cpu (tctl") {
                    4
                } else if label_orig.contains("cpu package") {
                    3
                } else if label_orig.contains("core temperatures") || label_orig == "cpu" {
                    2
                } else {
                    0
                };
                if prio > temp_prio {
                    temp_prio = prio;
                    out.cpu_temp_c = Some(value as f32);
                }
            }
            // SENSOR_TYPE_POWER = 5
            5 if value > 0.0 && value < 1000.0 => {
                let prio = if label_orig.contains("cpu package power") {
                    4
                } else if label_orig.contains("cpu ppt") {
                    3
                } else if label_orig.contains("cpu power") || label_orig.contains("package power") {
                    2
                } else {
                    0
                };
                if prio > power_prio {
                    power_prio = prio;
                    out.cpu_power_w = Some(value as f32);
                }
            }
            // SENSOR_TYPE_CLOCK = 6
            6 if value >= 200.0 && value <= 10_000.0 => {
                let prio = if label_orig.contains("core clocks")
                    || label_orig.contains("core 0 t0 effective clock")
                    || label_orig.contains("average effective clock")
                {
                    4
                } else if label_orig.contains("core 0") && label_orig.contains("clock") {
                    3
                } else if label_orig.contains("cpu clock") {
                    2
                } else {
                    0
                };
                if prio > clock_prio {
                    clock_prio = prio;
                    out.cpu_clock_mhz = Some(value.round() as u32);
                }
            }
            _ => {}
        }
    }

    Some(out)
}

/// Reads live CPU sensors from AIDA64 shared memory (`AIDA64_SensorValues`) if active.
pub fn read_aida64_shared_memory() -> Option<ExtraCpuSensors> {
    let map_name = to_wide("AIDA64_SensorValues");

    // SAFETY: Opens read-only mapping of AIDA64 XML sensor buffer, queries size via
    // VirtualQuery, copies out bytes, and unmaps/closes handles.
    unsafe {
        let handle = OpenFileMappingW(FILE_MAP_READ, 0, map_name.as_ptr());
        if handle.is_null() {
            return None;
        }
        let view = MapViewOfFile(handle, FILE_MAP_READ, 0, 0, 0);
        if view.Value.is_null() {
            CloseHandle(handle);
            return None;
        }
        let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
        let queried = VirtualQuery(
            view.Value,
            &mut mbi,
            std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        );
        let parsed = if queried != 0 && mbi.RegionSize > 0 {
            let max_len = mbi.RegionSize.min(256 * 1024);
            let raw = std::slice::from_raw_parts(view.Value as *const u8, max_len);
            let text = read_cstr(raw);
            parse_aida64_xml(&text)
        } else {
            None
        };
        UnmapViewOfFile(view);
        CloseHandle(handle);
        parsed
    }
}

fn parse_aida64_xml(xml: &str) -> Option<ExtraCpuSensors> {
    if xml.is_empty() {
        return None;
    }
    let mut out = ExtraCpuSensors::default();
    let mut pos = 0usize;
    while let Some(id_start) = xml[pos..].find("<id>") {
        let id_from = pos + id_start + 4;
        let Some(id_end_rel) = xml[id_from..].find("</id>") else {
            break;
        };
        let id = &xml[id_from..id_from + id_end_rel];
        let after_id = id_from + id_end_rel + 5;
        let Some(val_start) = xml[after_id..].find("<value>") else {
            pos = after_id;
            continue;
        };
        let val_from = after_id + val_start + 7;
        let Some(val_end_rel) = xml[val_from..].find("</value>") else {
            break;
        };
        let val_str = xml[val_from..val_from + val_end_rel].trim();
        pos = val_from + val_end_rel + 8;

        if let Ok(val) = val_str.parse::<f32>() {
            match id {
                "TCPU" | "TCPUDIO" | "TCPUPKG" if out.cpu_temp_c.is_none() && val > 5.0 && val < 130.0 => {
                    out.cpu_temp_c = Some(val);
                }
                "PCPUPKG" | "PCPU" if out.cpu_power_w.is_none() && val > 0.0 && val < 1000.0 => {
                    out.cpu_power_w = Some(val);
                }
                "SCPUCLK" if out.cpu_clock_mhz.is_none() && val >= 200.0 && val <= 10_000.0 => {
                    out.cpu_clock_mhz = Some(val.round() as u32);
                }
                _ => {}
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdh_sensors_read_live_cpu_clock_and_power() {
        let mut pdh = PdhSensors::new().expect("PDH should initialize on Windows");
        std::thread::sleep(std::time::Duration::from_millis(120));
        let sample = pdh.poll(3700);
        println!(
            "PDH Live -> clock={:?} MHz, power={:?} W, temp={:?} C",
            sample.cpu_clock_mhz, sample.cpu_power_w, sample.cpu_temp_c
        );
        assert!(
            sample.cpu_clock_mhz.is_some_and(|c| c > 500),
            "PDH should report real CPU clock"
        );
    }

    #[test]
    fn parses_aida64_xml_cpu_sensors() {
        let xml = "<sys><id>SCPUCLK</id><label>CPU Clock</label><value>4450</value></sys>\
                   <temp><id>TCPU</id><label>CPU</label><value>61.5</value></temp>\
                   <pwr><id>PCPUPKG</id><label>CPU Package</label><value>54.2</value></pwr>";
        let parsed = parse_aida64_xml(xml).unwrap();
        assert_eq!(parsed.cpu_clock_mhz, Some(4450));
        assert_eq!(parsed.cpu_temp_c, Some(61.5));
        assert_eq!(parsed.cpu_power_w, Some(54.2));
    }
}
