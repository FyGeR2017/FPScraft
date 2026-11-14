use std::fs;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use crate::settings::{
    Corner, HotkeyConfig, KeyBind, OverlayMode, OverlaySettings, StatsVisibility, ThresholdConfig,
};
use crate::theme::{Language, ThemeConfig, ThemePreset};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const SETTINGS_KEY: &str = r"HKCU\Software\FPScraft";
const LANGUAGE_VALUE: &str = "Language";

/// Snapshot of all user-configurable fields that are persisted across restarts.
#[derive(Debug, Clone, PartialEq)]
pub struct PersistedSettings {
    pub lang: Language,
    pub mode: OverlayMode,
    pub corner: Corner,
    pub custom_pos: Option<[f32; 2]>,
    pub theme: ThemeConfig,
    pub stats: StatsVisibility,
    pub hotkeys: HotkeyConfig,
    pub thresholds: ThresholdConfig,
}

impl PersistedSettings {
    pub fn from_settings(s: &OverlaySettings) -> Self {
        Self {
            lang: s.lang,
            mode: s.mode,
            corner: s.corner,
            custom_pos: s.custom_pos,
            theme: s.theme.clone(),
            stats: s.stats.clone(),
            hotkeys: s.hotkeys.clone(),
            thresholds: s.thresholds.clone(),
        }
    }
}

/// Returns the path to `%APPDATA%\FPScraft\config.ini` (falling back to `%LOCALAPPDATA%`).
pub fn config_file_path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA").or_else(|| std::env::var_os("LOCALAPPDATA"))?;
    Some(PathBuf::from(base).join("FPScraft").join("config.ini"))
}

/// Loads persisted settings from `config.ini`, falling back to registry language on first run.
pub fn load_settings(settings: &mut OverlaySettings) {
    let mut loaded_lang = false;
    if let Some(path) = config_file_path() {
        if let Ok(content) = fs::read_to_string(&path) {
            loaded_lang = apply_config_str(settings, &content);
        }
    }
    if !loaded_lang {
        if let Some(lang) = load_language() {
            settings.lang = lang;
        }
    }
}

/// Saves all persisted settings to `config.ini`.
pub fn save_settings(settings: &OverlaySettings) -> Result<(), String> {
    let path = config_file_path().ok_or_else(|| String::from("APPDATA/LOCALAPPDATA not set"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create_dir_all: {e}"))?;
    }
    let content = serialize_settings(settings);
    let tmp = path.with_extension("ini.tmp");
    fs::write(&tmp, &content).map_err(|e| format!("write tmp: {e}"))?;
    if let Err(_e) = fs::rename(&tmp, &path) {
        fs::write(&path, &content).map_err(|e| format!("write config: {e}"))?;
        let _ = fs::remove_file(&tmp);
    }
    Ok(())
}

/// Returns the language saved by the installer or the last in-app selection.
pub fn load_language() -> Option<Language> {
    let out = Command::new("reg")
        .args(["query", SETTINGS_KEY, "/v", LANGUAGE_VALUE])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_language(&String::from_utf8_lossy(&out.stdout))
}

/// Persists the UI language so the next launch (including autostart) keeps it.
pub fn save_language(lang: Language) -> Result<(), String> {
    let code = match lang {
        Language::Ru => "ru",
        Language::En => "en",
    };
    let out = Command::new("reg")
        .args([
            "add",
            SETTINGS_KEY,
            "/v",
            LANGUAGE_VALUE,
            "/t",
            "REG_SZ",
            "/d",
            code,
            "/f",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("reg add: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(())
}

fn parse_language(text: &str) -> Option<Language> {
    let value = text
        .lines()
        .find(|line| line.contains(LANGUAGE_VALUE) && line.contains("REG_SZ"))?
        .split_whitespace()
        .last()?;
    parse_lang_code(value)
}

fn parse_lang_code(value: &str) -> Option<Language> {
    match value.trim().to_ascii_lowercase().as_str() {
        "ru" | "russian" => Some(Language::Ru),
        "en" | "english" => Some(Language::En),
        _ => None,
    }
}

fn serialize_settings(s: &OverlaySettings) -> String {
    let mut lines = Vec::with_capacity(56);
    lines.push(format!(
        "lang={}",
        match s.lang {
            Language::Ru => "ru",
            Language::En => "en",
        }
    ));
    lines.push(format!(
        "mode={}",
        match s.mode {
            OverlayMode::Full => "full",
            OverlayMode::Corner => "corner",
        }
    ));
    lines.push(format!(
        "corner={}",
        match s.corner {
            Corner::TopLeft => "top_left",
            Corner::TopRight => "top_right",
            Corner::BottomRight => "bottom_right",
            Corner::BottomLeft => "bottom_left",
        }
    ));
    lines.push(match s.custom_pos {
        Some([x, y]) => format!("custom_pos={x:.1},{y:.1}"),
        None => String::from("custom_pos=none"),
    });

    lines.push(format!(
        "theme.preset={}",
        match s.theme.preset {
            ThemePreset::Graphite => "graphite",
            ThemePreset::OledBlack => "oled_black",
            ThemePreset::AmberRace => "amber_race",
            ThemePreset::EmeraldBtop => "emerald_btop",
            ThemePreset::Titanium => "titanium",
        }
    ));
    lines.push(format!("theme.bg_opacity={:.4}", s.theme.bg_opacity));
    lines.push(format!("theme.border_opacity={:.4}", s.theme.border_opacity));
    lines.push(format!("theme.border_width={:.4}", s.theme.border_width));
    lines.push(format!("theme.show_dividers={}", s.theme.show_dividers));
    lines.push(format!("theme.corner_radius={}", s.theme.corner_radius));
    lines.push(format!("theme.ui_scale={:.4}", s.theme.ui_scale));

    lines.push(format!(
        "thresholds.temp_warn_c={:.2}",
        s.thresholds.temp_warn_c
    ));
    lines.push(format!(
        "thresholds.temp_crit_c={:.2}",
        s.thresholds.temp_crit_c
    ));
    lines.push(format!(
        "thresholds.load_warn_pct={:.2}",
        s.thresholds.load_warn_pct
    ));
    lines.push(format!(
        "thresholds.load_crit_pct={:.2}",
        s.thresholds.load_crit_pct
    ));

    lines.push(format!(
        "hotkeys.toggle_hidden={}",
        format_keybind(s.hotkeys.toggle_hidden)
    ));
    lines.push(format!(
        "hotkeys.toggle_click_through={}",
        format_keybind(s.hotkeys.toggle_click_through)
    ));
    lines.push(format!(
        "hotkeys.cycle_mode={}",
        format_keybind(s.hotkeys.cycle_mode)
    ));
    lines.push(format!(
        "hotkeys.cycle_corner={}",
        format_keybind(s.hotkeys.cycle_corner)
    ));
    lines.push(format!(
        "hotkeys.toggle_settings={}",
        format_keybind(s.hotkeys.toggle_settings)
    ));
    lines.push(format!(
        "hotkeys.show_help={}",
        format_keybind(s.hotkeys.show_help)
    ));
    lines.push(format!(
        "hotkeys.quit_app={}",
        format_keybind(s.hotkeys.quit_app)
    ));

    let st = &s.stats;
    for (name, val) in [
        ("corner_show_fps", st.corner_show_fps),
        ("corner_show_1pct_low", st.corner_show_1pct_low),
        ("corner_show_frametime", st.corner_show_frametime),
        ("corner_show_cpu_temp", st.corner_show_cpu_temp),
        ("corner_show_cpu_usage", st.corner_show_cpu_usage),
        ("corner_show_gpu_temp", st.corner_show_gpu_temp),
        ("corner_show_gpu_usage", st.corner_show_gpu_usage),
        ("corner_show_vram", st.corner_show_vram),
        ("corner_show_ram", st.corner_show_ram),
        ("corner_show_net", st.corner_show_net),
        ("show_fps", st.show_fps),
        ("show_1pct_low", st.show_1pct_low),
        ("show_frametime_ms", st.show_frametime_ms),
        ("show_fps_graph", st.show_fps_graph),
        ("show_cpu_section", st.show_cpu_section),
        ("show_cpu_temp", st.show_cpu_temp),
        ("show_cpu_usage", st.show_cpu_usage),
        ("show_cpu_clock", st.show_cpu_clock),
        ("show_cpu_power", st.show_cpu_power),
        ("show_cpu_cores", st.show_cpu_cores),
        ("show_gpu_section", st.show_gpu_section),
        ("show_gpu_temp", st.show_gpu_temp),
        ("show_gpu_usage", st.show_gpu_usage),
        ("show_gpu_clock", st.show_gpu_clock),
        ("show_gpu_power", st.show_gpu_power),
        ("show_gpu_fan", st.show_gpu_fan),
        ("show_gpu_vram", st.show_gpu_vram),
        ("show_ram_section", st.show_ram_section),
        ("show_network", st.show_network),
        ("show_uptime", st.show_uptime),
        ("show_disk", st.show_disk),
    ] {
        lines.push(format!("stats.{name}={val}"));
    }

    lines.join("\n") + "\n"
}

fn apply_config_str(s: &mut OverlaySettings, text: &str) -> bool {
    let mut has_lang = false;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim();

        match key {
            "lang" => {
                if let Some(lang) = parse_lang_code(val) {
                    s.lang = lang;
                    has_lang = true;
                }
            }
            "mode" => match val {
                "full" => s.mode = OverlayMode::Full,
                "corner" => s.mode = OverlayMode::Corner,
                _ => {}
            },
            "corner" => match val {
                "top_left" => s.corner = Corner::TopLeft,
                "top_right" => s.corner = Corner::TopRight,
                "bottom_right" => s.corner = Corner::BottomRight,
                "bottom_left" => s.corner = Corner::BottomLeft,
                _ => {}
            },
            "custom_pos" => {
                if val == "none" {
                    s.custom_pos = None;
                } else if let Some((xs, ys)) = val.split_once(',') {
                    if let (Ok(x), Ok(y)) = (xs.trim().parse::<f32>(), ys.trim().parse::<f32>()) {
                        if x.is_finite() && y.is_finite() {
                            s.custom_pos = Some([x, y]);
                        }
                    }
                }
            }
            "theme.preset" => match val {
                "graphite" => s.theme.preset = ThemePreset::Graphite,
                "oled_black" => s.theme.preset = ThemePreset::OledBlack,
                "amber_race" => s.theme.preset = ThemePreset::AmberRace,
                "emerald_btop" => s.theme.preset = ThemePreset::EmeraldBtop,
                "titanium" => s.theme.preset = ThemePreset::Titanium,
                _ => {}
            },
            "theme.bg_opacity" => {
                if let Ok(v) = val.parse::<f32>() {
                    if v.is_finite() {
                        s.theme.bg_opacity = v.clamp(0.15, 1.0);
                    }
                }
            }
            "theme.border_opacity" => {
                if let Ok(v) = val.parse::<f32>() {
                    if v.is_finite() {
                        s.theme.border_opacity = v.clamp(0.0, 1.0);
                    }
                }
            }
            "theme.border_width" => {
                if let Ok(v) = val.parse::<f32>() {
                    if v.is_finite() {
                        s.theme.border_width = v.clamp(0.0, 4.0);
                    }
                }
            }
            "theme.show_dividers" => {
                if let Ok(v) = val.parse::<bool>() {
                    s.theme.show_dividers = v;
                }
            }
            "theme.corner_radius" => {
                if let Ok(v) = val.parse::<u8>() {
                    s.theme.corner_radius = v.min(24);
                }
            }
            "theme.ui_scale" => {
                if let Ok(v) = val.parse::<f32>() {
                    if v.is_finite() {
                        s.theme.ui_scale = v.clamp(0.75, 1.6);
                    }
                }
            }
            "thresholds.temp_warn_c" => {
                if let Ok(v) = val.parse::<f32>() {
                    if v.is_finite() {
                        s.thresholds.temp_warn_c = v.clamp(30.0, 110.0);
                    }
                }
            }
            "thresholds.temp_crit_c" => {
                if let Ok(v) = val.parse::<f32>() {
                    if v.is_finite() {
                        s.thresholds.temp_crit_c = v.clamp(35.0, 120.0);
                    }
                }
            }
            "thresholds.load_warn_pct" => {
                if let Ok(v) = val.parse::<f32>() {
                    if v.is_finite() {
                        s.thresholds.load_warn_pct = v.clamp(10.0, 100.0);
                    }
                }
            }
            "thresholds.load_crit_pct" => {
                if let Ok(v) = val.parse::<f32>() {
                    if v.is_finite() {
                        s.thresholds.load_crit_pct = v.clamp(15.0, 100.0);
                    }
                }
            }
            "hotkeys.toggle_hidden" => {
                if let Some(bind) = parse_keybind(val) {
                    s.hotkeys.toggle_hidden = bind;
                }
            }
            "hotkeys.toggle_click_through" => {
                if let Some(bind) = parse_keybind(val) {
                    s.hotkeys.toggle_click_through = bind;
                }
            }
            "hotkeys.cycle_mode" => {
                if let Some(bind) = parse_keybind(val) {
                    s.hotkeys.cycle_mode = bind;
                }
            }
            "hotkeys.cycle_corner" => {
                if let Some(bind) = parse_keybind(val) {
                    s.hotkeys.cycle_corner = bind;
                }
            }
            "hotkeys.toggle_settings" => {
                if let Some(bind) = parse_keybind(val) {
                    s.hotkeys.toggle_settings = bind;
                }
            }
            "hotkeys.show_help" => {
                if let Some(bind) = parse_keybind(val) {
                    s.hotkeys.show_help = bind;
                }
            }
            "hotkeys.quit_app" => {
                if let Some(bind) = parse_keybind(val) {
                    s.hotkeys.quit_app = bind;
                }
            }
            _ if key.starts_with("stats.") => {
                if let Ok(v) = val.parse::<bool>() {
                    set_stat_flag(&mut s.stats, &key["stats.".len()..], v);
                }
            }
            _ => {}
        }
    }
    has_lang
}

fn format_keybind(bind: Option<KeyBind>) -> String {
    match bind {
        None => String::from("none"),
        Some(b) => format!(
            "{}:{}:{}:{}",
            u8::from(b.ctrl),
            u8::from(b.alt),
            u8::from(b.shift),
            b.vk
        ),
    }
}

/// Outer `Option` indicates valid parse; inner `Option` represents bound vs unbound (`none`).
fn parse_keybind(val: &str) -> Option<Option<KeyBind>> {
    if val == "none" {
        return Some(None);
    }
    let mut parts = val.split(':');
    let ctrl = parts.next()?.parse::<u8>().ok()? != 0;
    let alt = parts.next()?.parse::<u8>().ok()? != 0;
    let shift = parts.next()?.parse::<u8>().ok()? != 0;
    let vk = parts.next()?.parse::<u16>().ok()?;
    if parts.next().is_some() || vk == 0 {
        return None;
    }
    Some(Some(KeyBind {
        ctrl,
        alt,
        shift,
        vk,
    }))
}

fn set_stat_flag(st: &mut StatsVisibility, field: &str, v: bool) {
    match field {
        "corner_show_fps" => st.corner_show_fps = v,
        "corner_show_1pct_low" => st.corner_show_1pct_low = v,
        "corner_show_frametime" => st.corner_show_frametime = v,
        "corner_show_cpu_temp" => st.corner_show_cpu_temp = v,
        "corner_show_cpu_usage" => st.corner_show_cpu_usage = v,
        "corner_show_gpu_temp" => st.corner_show_gpu_temp = v,
        "corner_show_gpu_usage" => st.corner_show_gpu_usage = v,
        "corner_show_vram" => st.corner_show_vram = v,
        "corner_show_ram" => st.corner_show_ram = v,
        "corner_show_net" => st.corner_show_net = v,
        "show_fps" => st.show_fps = v,
        "show_1pct_low" => st.show_1pct_low = v,
        "show_frametime_ms" => st.show_frametime_ms = v,
        "show_fps_graph" => st.show_fps_graph = v,
        "show_cpu_section" => st.show_cpu_section = v,
        "show_cpu_temp" => st.show_cpu_temp = v,
        "show_cpu_usage" => st.show_cpu_usage = v,
        "show_cpu_clock" => st.show_cpu_clock = v,
        "show_cpu_power" => st.show_cpu_power = v,
        "show_cpu_cores" => st.show_cpu_cores = v,
        "show_gpu_section" => st.show_gpu_section = v,
        "show_gpu_temp" => st.show_gpu_temp = v,
        "show_gpu_usage" => st.show_gpu_usage = v,
        "show_gpu_clock" => st.show_gpu_clock = v,
        "show_gpu_power" => st.show_gpu_power = v,
        "show_gpu_fan" => st.show_gpu_fan = v,
        "show_gpu_vram" => st.show_gpu_vram = v,
        "show_ram_section" => st.show_ram_section = v,
        "show_network" => st.show_network = v,
        "show_uptime" => st.show_uptime = v,
        "show_disk" => st.show_disk = v,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_registry_language_values() {
        let en = "\r\nHKEY_CURRENT_USER\\Software\\FPScraft\r\n    Language    REG_SZ    english\r\n\r\n";
        let ru =
            "\r\nHKEY_CURRENT_USER\\Software\\FPScraft\r\n    Language    REG_SZ    ru\r\n\r\n";
        assert_eq!(parse_language(en), Some(Language::En));
        assert_eq!(parse_language(ru), Some(Language::Ru));
        assert_eq!(parse_language("ERROR: not found"), None);
    }

    #[test]
    fn round_trips_full_settings_config() {
        let mut orig = OverlaySettings::default();
        orig.lang = Language::En;
        orig.mode = OverlayMode::Corner;
        orig.corner = Corner::BottomRight;
        orig.theme.preset = ThemePreset::AmberRace;
        orig.theme.bg_opacity = 0.72;
        orig.theme.border_opacity = 0.45;
        orig.theme.border_width = 1.5;
        orig.theme.show_dividers = false;
        orig.theme.corner_radius = 6;
        orig.theme.ui_scale = 1.25;
        orig.thresholds.temp_warn_c = 70.0;
        orig.thresholds.temp_crit_c = 88.0;
        orig.thresholds.load_warn_pct = 75.0;
        orig.thresholds.load_crit_pct = 92.0;
        orig.hotkeys.toggle_hidden = Some(KeyBind {
            ctrl: true,
            alt: false,
            shift: true,
            vk: 0x79,
        });
        orig.hotkeys.quit_app = None;
        orig.stats.corner_show_net = true;
        orig.stats.show_fps_graph = false;
        orig.stats.show_disk = false;

        let serialized = serialize_settings(&orig);
        let mut loaded = OverlaySettings::default();
        assert!(apply_config_str(&mut loaded, &serialized));

        assert_eq!(
            PersistedSettings::from_settings(&loaded),
            PersistedSettings::from_settings(&orig)
        );
    }
}
