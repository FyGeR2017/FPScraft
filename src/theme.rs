use eframe::egui::{Color32, Stroke};

// === Neutral Palette & Signals ===
pub const BG_SETTINGS: Color32 = Color32::from_rgb(16, 18, 23); // #101217
pub const SURFACE_ALT: Color32 = Color32::from_rgb(22, 25, 32); // #161920
pub const BORDER_HAIRLINE: Color32 = Color32::from_rgb(38, 43, 54); // #262B36
pub const DIVIDER_LINE: Color32 = Color32::from_rgb(30, 34, 43); // #1E222B

pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(236, 239, 244); // #ECEFF4
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(154, 162, 177); // #9AA2B1
pub const TEXT_MUTED: Color32 = Color32::from_rgb(94, 102, 117); // #5E6675

pub const ACCENT: Color32 = Color32::from_rgb(56, 189, 248); // #38BDF8 (default accent)
pub const SIGNAL_WARN: Color32 = Color32::from_rgb(234, 179, 8); // #EAB308 (yellow)
pub const SIGNAL_CRIT: Color32 = Color32::from_rgb(239, 68, 68); // #EF4444 (red)

// === Typography: 3 Sizes, Monospace Tabular ===
pub const FONT_LARGE: f32 = 28.0;
pub const FONT_MEDIUM: f32 = 13.0;
pub const FONT_SMALL: f32 = 10.5;

// === Geometry ===
pub const WINDOW_RADIUS: u8 = 2;
pub const CONTROL_RADIUS: u8 = 2;
pub const BORDER_WIDTH: f32 = 1.0;
pub const GRAPH_LINE_WIDTH: f32 = 1.2;
pub const FULL_WIDTH: f32 = 340.0;
pub const SCREEN_MARGIN: f32 = 16.0;
pub const DEFAULT_BG_OPACITY: f32 = 0.91;

// === Signal Thresholds (Defaults) ===
pub const TEMP_WARN_C: f32 = 75.0;
pub const TEMP_CRIT_C: f32 = 85.0;
pub const LOAD_WARN_PCT: f32 = 80.0;
pub const LOAD_CRIT_PCT: f32 = 90.0;
pub const FPS_WARN: f32 = 60.0;
pub const FPS_CRIT: f32 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    Ru,
    En,
}

impl Language {
    pub fn tr(self, ru: &'static str, en: &'static str) -> &'static str {
        match self {
            Self::Ru => ru,
            Self::En => en,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePreset {
    Graphite,
    OledBlack,
    AmberRace,
    EmeraldBtop,
    Titanium,
}

impl ThemePreset {
    pub const ALL: [Self; 5] = [
        Self::Graphite,
        Self::OledBlack,
        Self::AmberRace,
        Self::EmeraldBtop,
        Self::Titanium,
    ];

    pub fn label(self, lang: Language) -> &'static str {
        match (self, lang) {
            (Self::Graphite, Language::Ru) => "Графит — Технический",
            (Self::Graphite, Language::En) => "Graphite — Technical",
            (Self::OledBlack, Language::Ru) => "OLED Чёрный — Монохром",
            (Self::OledBlack, Language::En) => "OLED Black — Monochrome",
            (Self::AmberRace, Language::Ru) => "Гоночная телеметрия — Янтарь",
            (Self::AmberRace, Language::En) => "Race Telemetry — Amber",
            (Self::EmeraldBtop, Language::Ru) => "Терминал — Изумруд",
            (Self::EmeraldBtop, Language::En) => "Terminal — Emerald",
            (Self::Titanium, Language::Ru) => "Титан — Стальной",
            (Self::Titanium, Language::En) => "Titanium — Steel",
        }
    }

    pub fn colors(self) -> ThemeColors {
        match self {
            Self::Graphite => ThemeColors {
                bg_rgb: (14, 16, 20),
                surface_alt: Color32::from_rgb(22, 25, 32),
                border_rgb: (68, 78, 98),
                divider: Color32::from_rgb(30, 34, 43),
                grid_line: Color32::from_rgb(28, 32, 40),
                accent: ACCENT,
            },
            Self::OledBlack => ThemeColors {
                bg_rgb: (5, 5, 7),
                surface_alt: Color32::from_rgb(16, 17, 21),
                border_rgb: (72, 76, 92),
                divider: Color32::from_rgb(26, 28, 34),
                grid_line: Color32::from_rgb(24, 26, 32),
                accent: Color32::from_rgb(236, 239, 244),
            },
            Self::AmberRace => ThemeColors {
                bg_rgb: (18, 16, 13),
                surface_alt: Color32::from_rgb(30, 26, 20),
                border_rgb: (110, 88, 56),
                divider: Color32::from_rgb(42, 36, 27),
                grid_line: Color32::from_rgb(36, 31, 24),
                accent: Color32::from_rgb(245, 158, 11),
            },
            Self::EmeraldBtop => ThemeColors {
                bg_rgb: (11, 18, 16),
                surface_alt: Color32::from_rgb(18, 29, 25),
                border_rgb: (56, 98, 82),
                divider: Color32::from_rgb(25, 42, 36),
                grid_line: Color32::from_rgb(22, 37, 32),
                accent: Color32::from_rgb(16, 185, 129),
            },
            Self::Titanium => ThemeColors {
                bg_rgb: (24, 26, 32),
                surface_alt: Color32::from_rgb(34, 38, 47),
                border_rgb: (96, 108, 132),
                divider: Color32::from_rgb(44, 49, 60),
                grid_line: Color32::from_rgb(38, 43, 53),
                accent: Color32::from_rgb(148, 163, 184),
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ThemeColors {
    pub bg_rgb: (u8, u8, u8),
    pub surface_alt: Color32,
    pub border_rgb: (u8, u8, u8),
    pub divider: Color32,
    pub grid_line: Color32,
    pub accent: Color32,
}

#[derive(Debug, Clone)]
pub struct ThemeConfig {
    pub preset: ThemePreset,
    pub bg_opacity: f32,
    pub border_opacity: f32,
    pub border_width: f32,
    pub show_dividers: bool,
    pub corner_radius: u8,
    pub ui_scale: f32,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            preset: ThemePreset::Graphite,
            bg_opacity: DEFAULT_BG_OPACITY,
            border_opacity: 1.0,
            border_width: BORDER_WIDTH,
            show_dividers: true,
            corner_radius: WINDOW_RADIUS,
            ui_scale: 1.0,
        }
    }
}

impl ThemeConfig {
    pub fn colors(&self) -> ThemeColors {
        self.preset.colors()
    }

    pub fn panel_bg(&self) -> Color32 {
        let (r, g, b) = self.colors().bg_rgb;
        let alpha = (self.bg_opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
        Color32::from_rgba_unmultiplied(r, g, b, alpha)
    }

    pub fn surface_alt_bg(&self) -> Color32 {
        let c = self.colors().surface_alt;
        let alpha = (self.bg_opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
    }

    pub fn border_color(&self) -> Color32 {
        let (r, g, b) = self.colors().border_rgb;
        let alpha = (self.border_opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
        Color32::from_rgba_unmultiplied(r, g, b, alpha)
    }

    pub fn effective_border_stroke(&self) -> Stroke {
        if self.border_opacity <= 0.01 || self.border_width <= 0.05 {
            Stroke::NONE
        } else {
            Stroke::new(self.border_width.clamp(0.0, 3.0), self.border_color())
        }
    }
}

pub fn value_temp_color(temp_c: f32, warn: f32, crit: f32) -> Color32 {
    if temp_c >= crit {
        SIGNAL_CRIT
    } else if temp_c >= warn {
        SIGNAL_WARN
    } else {
        TEXT_PRIMARY
    }
}

pub fn value_load_color(pct: f32, warn: f32, crit: f32) -> Color32 {
    if pct >= crit {
        SIGNAL_CRIT
    } else if pct >= warn {
        SIGNAL_WARN
    } else {
        TEXT_PRIMARY
    }
}

pub fn value_fps_color(fps: f32) -> Color32 {
    if fps < FPS_CRIT {
        SIGNAL_CRIT
    } else if fps < FPS_WARN {
        SIGNAL_WARN
    } else {
        TEXT_PRIMARY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_border_stroke_handles_zero_and_width() {
        let mut cfg = ThemeConfig::default();
        assert_eq!(cfg.effective_border_stroke().width, BORDER_WIDTH);

        cfg.border_opacity = 0.0;
        assert_eq!(cfg.effective_border_stroke(), Stroke::NONE);

        cfg.border_opacity = 1.0;
        cfg.border_width = 0.0;
        assert_eq!(cfg.effective_border_stroke(), Stroke::NONE);

        cfg.border_width = 2.5;
        let s = cfg.effective_border_stroke();
        assert!((s.width - 2.5).abs() < f32::EPSILON);
    }

    #[test]
    fn bg_opacity_scales_only_panel_and_surface_alpha() {
        let mut cfg = ThemeConfig::default();
        cfg.bg_opacity = 0.0;
        assert_eq!(cfg.panel_bg().a(), 0);
        assert_eq!(cfg.surface_alt_bg().a(), 0);
        assert_eq!(TEXT_PRIMARY.a(), 255);

        cfg.bg_opacity = 0.5;
        assert_eq!(cfg.panel_bg().a(), 128);
        assert_eq!(cfg.surface_alt_bg().a(), 128);
    }
}
