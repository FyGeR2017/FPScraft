use std::collections::HashMap;

use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Layout, Margin, Pos2, RichText, Stroke, Vec2,
    ViewportBuilder, ViewportId,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_0, VK_9, VK_A, VK_BACK, VK_CONTROL, VK_DELETE, VK_END, VK_ESCAPE, VK_F1,
    VK_F12, VK_HOME, VK_INSERT, VK_MENU, VK_NEXT, VK_PAUSE, VK_PRIOR, VK_SHIFT, VK_Z,
};

use crate::theme::{
    Language, ThemeConfig, ThemePreset, ACCENT, BG_SETTINGS, BORDER_HAIRLINE, BORDER_WIDTH,
    CONTROL_RADIUS, DIVIDER_LINE, FONT_MEDIUM, FONT_SMALL, LOAD_CRIT_PCT, LOAD_WARN_PCT,
    SCREEN_MARGIN, SIGNAL_WARN, SURFACE_ALT, TEMP_CRIT_C, TEMP_WARN_C, TEXT_MUTED, TEXT_PRIMARY,
    TEXT_SECONDARY,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayMode {
    Full,
    Corner,
}

impl OverlayMode {
    pub fn toggled(self) -> Self {
        match self {
            Self::Full => Self::Corner,
            Self::Corner => Self::Full,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

impl Corner {
    pub fn next(self) -> Self {
        match self {
            Self::TopLeft => Self::TopRight,
            Self::TopRight => Self::BottomRight,
            Self::BottomRight => Self::BottomLeft,
            Self::BottomLeft => Self::TopLeft,
        }
    }

    pub fn label(self, lang: Language) -> &'static str {
        match (self, lang) {
            (Self::TopLeft, Language::Ru) => "Верх-Л",
            (Self::TopLeft, Language::En) => "Top-L",
            (Self::TopRight, Language::Ru) => "Верх-П",
            (Self::TopRight, Language::En) => "Top-R",
            (Self::BottomRight, Language::Ru) => "Низ-П",
            (Self::BottomRight, Language::En) => "Bot-R",
            (Self::BottomLeft, Language::Ru) => "Низ-Л",
            (Self::BottomLeft, Language::En) => "Bot-L",
        }
    }

    pub fn position(self, monitor: Vec2, window: Vec2) -> Pos2 {
        let max_x = (monitor.x - window.x - SCREEN_MARGIN).max(SCREEN_MARGIN);
        let max_y = (monitor.y - window.y - SCREEN_MARGIN).max(SCREEN_MARGIN);
        match self {
            Self::TopLeft => Pos2::new(SCREEN_MARGIN, SCREEN_MARGIN),
            Self::TopRight => Pos2::new(max_x, SCREEN_MARGIN),
            Self::BottomRight => Pos2::new(max_x, max_y),
            Self::BottomLeft => Pos2::new(SCREEN_MARGIN, max_y),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    ToggleClickThrough,
    CycleMode,
    ToggleHidden,
    CycleCorner,
    ToggleSettings,
    ShowHelp,
    QuitApp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyBind {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub vk: u16,
}

impl KeyBind {
    pub const fn alt_key(vk: u16) -> Self {
        Self {
            ctrl: false,
            alt: true,
            shift: false,
            vk,
        }
    }

    pub fn label(self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl {
            parts.push(String::from("CTRL"));
        }
        if self.alt {
            parts.push(String::from("ALT"));
        }
        if self.shift {
            parts.push(String::from("SHIFT"));
        }
        parts.push(vk_name(self.vk));
        parts.join("+")
    }
}

pub fn format_hotkey(bind: Option<KeyBind>, lang: Language) -> String {
    bind.map(|b| b.label())
        .unwrap_or_else(|| String::from(lang.tr("НЕТ", "NONE")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyConfig {
    pub toggle_hidden: Option<KeyBind>,
    pub toggle_click_through: Option<KeyBind>,
    pub cycle_mode: Option<KeyBind>,
    pub cycle_corner: Option<KeyBind>,
    pub toggle_settings: Option<KeyBind>,
    pub show_help: Option<KeyBind>,
    pub quit_app: Option<KeyBind>,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            toggle_hidden: Some(KeyBind::alt_key(b'T' as u16)),
            toggle_click_through: None,
            cycle_mode: None,
            cycle_corner: None,
            toggle_settings: None,
            show_help: None,
            quit_app: None,
        }
    }
}

impl HotkeyConfig {
    pub fn key_for(&self, action: HotkeyAction) -> Option<KeyBind> {
        match action {
            HotkeyAction::ToggleClickThrough => self.toggle_click_through,
            HotkeyAction::CycleMode => self.cycle_mode,
            HotkeyAction::ToggleHidden => self.toggle_hidden,
            HotkeyAction::CycleCorner => self.cycle_corner,
            HotkeyAction::ToggleSettings => self.toggle_settings,
            HotkeyAction::ShowHelp => self.show_help,
            HotkeyAction::QuitApp => self.quit_app,
        }
    }

    pub fn set_key(&mut self, action: HotkeyAction, bind: Option<KeyBind>) {
        match action {
            HotkeyAction::ToggleClickThrough => self.toggle_click_through = bind,
            HotkeyAction::CycleMode => self.cycle_mode = bind,
            HotkeyAction::ToggleHidden => self.toggle_hidden = bind,
            HotkeyAction::CycleCorner => self.cycle_corner = bind,
            HotkeyAction::ToggleSettings => self.toggle_settings = bind,
            HotkeyAction::ShowHelp => self.show_help = bind,
            HotkeyAction::QuitApp => self.quit_app = bind,
        }
    }
}

#[derive(Default)]
pub struct HotkeyPoller {
    prev_down: HashMap<KeyBind, bool>,
}

impl HotkeyPoller {
    pub fn is_combo_down(bind: KeyBind) -> bool {
        is_vk_down(VK_CONTROL) == bind.ctrl
            && is_vk_down(VK_MENU) == bind.alt
            && is_vk_down(VK_SHIFT) == bind.shift
            && is_vk_down(bind.vk)
    }

    pub fn is_edge_pressed(&mut self, bind: Option<KeyBind>) -> bool {
        let Some(b) = bind else {
            return false;
        };
        let down = Self::is_combo_down(b);
        let prev = self.prev_down.insert(b, down).unwrap_or(false);
        down && !prev
    }

    pub fn is_held(&self, bind: Option<KeyBind>) -> bool {
        bind.is_some_and(Self::is_combo_down)
    }

    pub fn reset_state(&mut self, bind: KeyBind) {
        self.prev_down.insert(bind, true);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsVisibility {
    // Элементы компактного режима «Угол»
    pub corner_show_fps: bool,
    pub corner_show_1pct_low: bool,
    pub corner_show_frametime: bool,
    pub corner_show_cpu_temp: bool,
    pub corner_show_cpu_usage: bool,
    pub corner_show_gpu_temp: bool,
    pub corner_show_gpu_usage: bool,
    pub corner_show_vram: bool,
    pub corner_show_ram: bool,
    pub corner_show_net: bool,

    // Кадры и график (полный режим)
    pub show_fps: bool,
    pub show_1pct_low: bool,
    pub show_frametime_ms: bool,
    pub show_fps_graph: bool,

    // Процессор (CPU)
    pub show_cpu_section: bool,
    pub show_cpu_temp: bool,
    pub show_cpu_usage: bool,
    pub show_cpu_clock: bool,
    pub show_cpu_power: bool,
    pub show_cpu_cores: bool,

    // Видеокарта (GPU)
    pub show_gpu_section: bool,
    pub show_gpu_temp: bool,
    pub show_gpu_usage: bool,
    pub show_gpu_clock: bool,
    pub show_gpu_power: bool,
    pub show_gpu_fan: bool,
    pub show_gpu_vram: bool,

    // Память и система
    pub show_ram_section: bool,
    pub show_network: bool,
    pub show_uptime: bool,
    pub show_disk: bool,
}

impl Default for StatsVisibility {
    fn default() -> Self {
        Self {
            corner_show_fps: true,
            corner_show_1pct_low: true,
            corner_show_frametime: false,
            corner_show_cpu_temp: true,
            corner_show_cpu_usage: false,
            corner_show_gpu_temp: true,
            corner_show_gpu_usage: false,
            corner_show_vram: false,
            corner_show_ram: false,
            corner_show_net: false,
            show_fps: true,
            show_1pct_low: true,
            show_frametime_ms: true,
            show_fps_graph: true,
            show_cpu_section: true,
            show_cpu_temp: true,
            show_cpu_usage: true,
            show_cpu_clock: true,
            show_cpu_power: true,
            show_cpu_cores: true,
            show_gpu_section: true,
            show_gpu_temp: true,
            show_gpu_usage: true,
            show_gpu_clock: true,
            show_gpu_power: true,
            show_gpu_fan: true,
            show_gpu_vram: true,
            show_ram_section: true,
            show_network: true,
            show_uptime: true,
            show_disk: true,
        }
    }
}

impl StatsVisibility {
    pub fn apply_full_all(&mut self) {
        self.show_fps = true;
        self.show_1pct_low = true;
        self.show_frametime_ms = true;
        self.show_fps_graph = true;
        self.show_cpu_section = true;
        self.show_cpu_temp = true;
        self.show_cpu_usage = true;
        self.show_cpu_clock = true;
        self.show_cpu_power = true;
        self.show_cpu_cores = true;
        self.show_gpu_section = true;
        self.show_gpu_temp = true;
        self.show_gpu_usage = true;
        self.show_gpu_clock = true;
        self.show_gpu_power = true;
        self.show_gpu_fan = true;
        self.show_gpu_vram = true;
        self.show_ram_section = true;
        self.show_network = true;
        self.show_uptime = true;
        self.show_disk = true;
    }

    pub fn apply_full_essential(&mut self) {
        self.show_fps = true;
        self.show_1pct_low = true;
        self.show_frametime_ms = true;
        self.show_fps_graph = false;
        self.show_cpu_section = true;
        self.show_cpu_temp = true;
        self.show_cpu_usage = true;
        self.show_cpu_clock = false;
        self.show_cpu_power = false;
        self.show_cpu_cores = false;
        self.show_gpu_section = true;
        self.show_gpu_temp = true;
        self.show_gpu_usage = true;
        self.show_gpu_clock = false;
        self.show_gpu_power = false;
        self.show_gpu_fan = false;
        self.show_gpu_vram = true;
        self.show_ram_section = true;
        self.show_network = false;
        self.show_uptime = false;
        self.show_disk = false;
    }

    pub fn apply_corner_all(&mut self) {
        self.corner_show_fps = true;
        self.corner_show_1pct_low = true;
        self.corner_show_frametime = true;
        self.corner_show_cpu_temp = true;
        self.corner_show_cpu_usage = true;
        self.corner_show_gpu_temp = true;
        self.corner_show_gpu_usage = true;
        self.corner_show_vram = true;
        self.corner_show_ram = true;
        self.corner_show_net = true;
    }

    pub fn apply_corner_defaults(&mut self) {
        self.corner_show_fps = true;
        self.corner_show_1pct_low = true;
        self.corner_show_frametime = false;
        self.corner_show_cpu_temp = true;
        self.corner_show_cpu_usage = false;
        self.corner_show_gpu_temp = true;
        self.corner_show_gpu_usage = false;
        self.corner_show_vram = false;
        self.corner_show_ram = false;
        self.corner_show_net = false;
    }

    pub fn has_any_corner_item(&self) -> bool {
        self.corner_show_fps
            || self.corner_show_1pct_low
            || self.corner_show_frametime
            || self.corner_show_cpu_temp
            || self.corner_show_cpu_usage
            || self.corner_show_gpu_temp
            || self.corner_show_gpu_usage
            || self.corner_show_vram
            || self.corner_show_ram
            || self.corner_show_net
    }

    pub fn has_any_fps_row(&self) -> bool {
        self.show_fps || self.show_1pct_low || self.show_frametime_ms || self.show_fps_graph
    }

    pub fn has_any_system_footer(&self) -> bool {
        self.show_network || self.show_uptime || self.show_disk
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThresholdConfig {
    pub temp_warn_c: f32,
    pub temp_crit_c: f32,
    pub load_warn_pct: f32,
    pub load_crit_pct: f32,
}

impl Default for ThresholdConfig {
    fn default() -> Self {
        Self {
            temp_warn_c: TEMP_WARN_C,
            temp_crit_c: TEMP_CRIT_C,
            load_warn_pct: LOAD_WARN_PCT,
            load_crit_pct: LOAD_CRIT_PCT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatsSubTab {
    #[default]
    Full,
    Corner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    Appearance,
    Stats,
    Hotkeys,
}

/// Complete runtime configuration shared between the overlay and the Settings viewport.
pub struct OverlaySettings {
    pub open: bool,
    pub lang: Language,
    pub active_tab: SettingsTab,
    pub stats_subtab: StatsSubTab,
    pub mode: OverlayMode,
    pub corner: Corner,
    pub theme: ThemeConfig,
    pub stats: StatsVisibility,
    pub hotkeys: HotkeyConfig,
    pub thresholds: ThresholdConfig,
    pub capturing_action: Option<HotkeyAction>,
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self {
            open: false,
            lang: Language::default(),
            active_tab: SettingsTab::Appearance,
            stats_subtab: StatsSubTab::default(),
            mode: OverlayMode::Full,
            corner: Corner::TopLeft,
            theme: ThemeConfig::default(),
            stats: StatsVisibility::default(),
            hotkeys: HotkeyConfig::default(),
            thresholds: ThresholdConfig::default(),
            capturing_action: None,
        }
    }
}

/// Result of one Settings window frame so the main overlay can apply viewport updates immediately.
#[derive(Default)]
pub struct SettingsFrameActions {
    pub mode_changed: bool,
    pub corner_changed: bool,
    pub layout_changed: bool,
    pub language_changed: bool,
}

/// Renders the separate Settings native window when `settings.open` is true.
pub fn render_settings_viewport(
    ctx: &egui::Context,
    settings: &mut OverlaySettings,
    poller: &mut HotkeyPoller,
) -> SettingsFrameActions {
    let mut actions = SettingsFrameActions::default();
    if !settings.open {
        settings.capturing_action = None;
        return actions;
    }

    let lang_before = settings.lang;

    if let Some(target_action) = settings.capturing_action {
        if is_vk_down(VK_ESCAPE) {
            settings.capturing_action = None;
        } else if is_vk_down(VK_BACK) {
            settings.hotkeys.set_key(target_action, None);
            settings.capturing_action = None;
        } else if let Some(vk) = poll_any_rebindable_key() {
            let bind = KeyBind {
                ctrl: is_vk_down(VK_CONTROL),
                alt: is_vk_down(VK_MENU),
                shift: is_vk_down(VK_SHIFT),
                vk,
            };
            settings.hotkeys.set_key(target_action, Some(bind));
            poller.reset_state(bind);
            settings.capturing_action = None;
        }
    }

    let viewport_id = ViewportId::from_hash_of("fpscraft_settings_window");
    let builder = ViewportBuilder::default()
        .with_title(
            settings
                .lang
                .tr("FPScraft — Настройки", "FPScraft — Settings"),
        )
        .with_inner_size([440.0, 640.0])
        .with_min_inner_size([380.0, 460.0])
        .with_always_on_top()
        .with_transparent(true)
        .with_decorations(true)
        .with_resizable(true)
        .with_taskbar(false);

    ctx.show_viewport_immediate(viewport_id, builder, |v_ctx, _class| {
        if v_ctx.input(|i| i.viewport().close_requested()) {
            settings.open = false;
            settings.capturing_action = None;
            return;
        }

        let settings_alpha =
            (settings.theme.bg_opacity.clamp(0.35, 1.0) * 255.0).round() as u8;
        let settings_bg = Color32::from_rgba_unmultiplied(
            BG_SETTINGS.r(),
            BG_SETTINGS.g(),
            BG_SETTINGS.b(),
            settings_alpha,
        );

        egui::CentralPanel::default()
            .frame(
                Frame::NONE
                    .fill(settings_bg)
                    .inner_margin(Margin::same(14)),
            )
            .show(v_ctx, |ui| {
                let outer_w = ui.available_width().max(320.0);
                render_settings_header(ui, settings, outer_w);
                draw_divider(ui, outer_w);
                render_category_tabs(ui, &mut settings.active_tab, settings.lang, outer_w);
                ui.add_space(10.0);

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let content_w = (ui.available_width() - 8.0).max(300.0);
                        ui.set_max_width(content_w);
                        match settings.active_tab {
                            SettingsTab::Stats => {
                                render_stats_tab(ui, settings, &mut actions, content_w);
                            }
                            SettingsTab::Hotkeys => render_hotkeys_tab(ui, settings, content_w),
                            SettingsTab::Appearance => {
                                render_appearance_tab(ui, settings, &mut actions, content_w);
                            }
                        }
                    });
            });
    });

    if settings.lang != lang_before {
        actions.language_changed = true;
    }

    ctx.request_repaint_of(ViewportId::ROOT);
    actions
}

fn render_settings_header(ui: &mut egui::Ui, settings: &mut OverlaySettings, width: f32) {
    let lang = settings.lang;
    ui.allocate_ui_with_layout(
        Vec2::new(width, 24.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.label(
                RichText::new(lang.tr("НАСТРОЙКИ FPSCRAFT", "FPSCRAFT SETTINGS"))
                    .strong()
                    .size(FONT_MEDIUM)
                    .color(TEXT_PRIMARY),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let close_btn = ui.add(
                    egui::Button::new(
                        RichText::new(lang.tr("ЗАКРЫТЬ", "CLOSE"))
                            .size(FONT_SMALL)
                            .color(TEXT_SECONDARY),
                    )
                    .fill(SURFACE_ALT)
                    .stroke(Stroke::new(BORDER_WIDTH, BORDER_HAIRLINE))
                    .corner_radius(CornerRadius::same(CONTROL_RADIUS)),
                );
                if close_btn.clicked() {
                    settings.open = false;
                    settings.capturing_action = None;
                }

                ui.add_space(6.0);

                for (target_lang, code) in [(Language::En, "EN"), (Language::Ru, "RU")] {
                    let active = settings.lang == target_lang;
                    let lang_btn =
                        egui::Button::new(RichText::new(code).size(FONT_SMALL).color(if active {
                            ACCENT
                        } else {
                            TEXT_MUTED
                        }))
                        .fill(if active {
                            SURFACE_ALT
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(Stroke::new(
                            BORDER_WIDTH,
                            if active { ACCENT } else { BORDER_HAIRLINE },
                        ))
                        .corner_radius(CornerRadius::same(CONTROL_RADIUS));

                    if ui.add_sized([28.0, 20.0], lang_btn).clicked() {
                        settings.lang = target_lang;
                    }
                }
            });
        },
    );
}

fn render_category_tabs(
    ui: &mut egui::Ui,
    active_tab: &mut SettingsTab,
    lang: Language,
    width: f32,
) {
    let tabs = [
        (SettingsTab::Appearance, lang.tr("ТЕМА", "THEME")),
        (SettingsTab::Stats, lang.tr("ПОКАЗАТЕЛИ", "METRICS")),
        (SettingsTab::Hotkeys, lang.tr("КЛАВИШИ", "HOTKEYS")),
    ];
    let gap = 4.0;
    let tab_w = ((width - gap * 2.0) / 3.0).floor().max(72.0);

    ui.allocate_ui_with_layout(
        Vec2::new(width, 26.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (tab, title) in tabs {
                let active = *active_tab == tab;
                let btn =
                    egui::Button::new(RichText::new(title).size(FONT_SMALL).color(if active {
                        ACCENT
                    } else {
                        TEXT_SECONDARY
                    }))
                    .fill(if active {
                        SURFACE_ALT
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::new(
                        BORDER_WIDTH,
                        if active { ACCENT } else { BORDER_HAIRLINE },
                    ))
                    .corner_radius(CornerRadius::same(CONTROL_RADIUS));

                if ui.add_sized([tab_w, 26.0], btn).clicked() {
                    *active_tab = tab;
                }
            }
        },
    );
}

fn render_stats_tab(
    ui: &mut egui::Ui,
    settings: &mut OverlaySettings,
    actions: &mut SettingsFrameActions,
    content_w: f32,
) {
    let lang = settings.lang;
    let before = settings.stats.clone();

    let sub_btn_w = ((content_w - 6.0) * 0.5).floor();
    ui.allocate_ui_with_layout(
        Vec2::new(content_w, 24.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (sub, title) in [
                (StatsSubTab::Full, lang.tr("ПОЛНЫЙ РЕЖИМ", "FULL MODE")),
                (StatsSubTab::Corner, lang.tr("РЕЖИМ «УГОЛ»", "CORNER MODE")),
            ] {
                let active = settings.stats_subtab == sub;
                let btn =
                    egui::Button::new(RichText::new(title).size(FONT_SMALL).color(if active {
                        ACCENT
                    } else {
                        TEXT_SECONDARY
                    }))
                    .fill(if active {
                        SURFACE_ALT
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::new(
                        BORDER_WIDTH,
                        if active { ACCENT } else { BORDER_HAIRLINE },
                    ))
                    .corner_radius(CornerRadius::same(CONTROL_RADIUS));

                if ui.add_sized([sub_btn_w, 24.0], btn).clicked() {
                    settings.stats_subtab = sub;
                }
            }
        },
    );

    ui.add_space(8.0);

    match settings.stats_subtab {
        StatsSubTab::Full => {
            render_full_stats_subtab(ui, &mut settings.stats, lang, content_w);
        }
        StatsSubTab::Corner => {
            render_corner_stats_subtab(ui, &mut settings.stats, lang, content_w);
        }
    }

    if settings.stats != before {
        actions.layout_changed = true;
    }
}

fn render_full_stats_subtab(
    ui: &mut egui::Ui,
    stats: &mut StatsVisibility,
    lang: Language,
    content_w: f32,
) {
    let preset_btn_w = ((content_w - 6.0) * 0.5).floor();
    ui.allocate_ui_with_layout(
        Vec2::new(content_w, 24.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if ui
                .add_sized(
                    [preset_btn_w, 24.0],
                    egui::Button::new(
                        RichText::new(lang.tr("ВКЛЮЧИТЬ ВСЕ", "ENABLE ALL"))
                            .size(FONT_SMALL)
                            .color(TEXT_PRIMARY),
                    )
                    .fill(SURFACE_ALT)
                    .stroke(Stroke::new(BORDER_WIDTH, BORDER_HAIRLINE))
                    .corner_radius(CornerRadius::same(CONTROL_RADIUS)),
                )
                .clicked()
            {
                stats.apply_full_all();
            }
            if ui
                .add_sized(
                    [preset_btn_w, 24.0],
                    egui::Button::new(
                        RichText::new(lang.tr("ТОЛЬКО ОСНОВНОЕ", "ESSENTIAL ONLY"))
                            .size(FONT_SMALL)
                            .color(TEXT_SECONDARY),
                    )
                    .fill(SURFACE_ALT)
                    .stroke(Stroke::new(BORDER_WIDTH, BORDER_HAIRLINE))
                    .corner_radius(CornerRadius::same(CONTROL_RADIUS)),
                )
                .clicked()
            {
                stats.apply_full_essential();
            }
        },
    );

    ui.add_space(10.0);

    section_title(ui, lang.tr("КАДРЫ И ГРАФИК", "FRAMES & GRAPH"));
    stat_checkbox_grid(
        ui,
        content_w,
        &mut [
            (&mut stats.show_fps, lang.tr("Текущий FPS", "Current FPS")),
            (&mut stats.show_1pct_low, "1% Low FPS"),
            (
                &mut stats.show_frametime_ms,
                lang.tr("Время кадра (мс)", "Frame time (ms)"),
            ),
            (
                &mut stats.show_fps_graph,
                lang.tr("График FPS", "FPS graph"),
            ),
        ],
    );

    draw_divider(ui, content_w);

    ui.allocate_ui_with_layout(
        Vec2::new(content_w, 20.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.label(
                RichText::new(lang.tr("ПРОЦЕССОР (CPU)", "PROCESSOR (CPU)"))
                    .strong()
                    .size(FONT_SMALL)
                    .color(TEXT_SECONDARY),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                stat_toggle(
                    ui,
                    &mut stats.show_cpu_section,
                    lang.tr("Показывать блок", "Show block"),
                );
            });
        },
    );
    ui.add_space(4.0);
    ui.add_enabled_ui(stats.show_cpu_section, |ui| {
        stat_checkbox_grid(
            ui,
            content_w,
            &mut [
                (
                    &mut stats.show_cpu_temp,
                    lang.tr("Температура (°C)", "Temperature (°C)"),
                ),
                (
                    &mut stats.show_cpu_usage,
                    lang.tr("Общая загрузка (%)", "Total usage (%)"),
                ),
                (
                    &mut stats.show_cpu_clock,
                    lang.tr("Частота (МГц)", "Clock (MHz)"),
                ),
                (
                    &mut stats.show_cpu_power,
                    lang.tr("Мощность (Вт)", "Power (W)"),
                ),
                (
                    &mut stats.show_cpu_cores,
                    lang.tr("Загрузка по ядрам", "Per-core usage"),
                ),
            ],
        );
    });

    draw_divider(ui, content_w);

    ui.allocate_ui_with_layout(
        Vec2::new(content_w, 20.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.label(
                RichText::new(lang.tr("ВИДЕОКАРТА (GPU)", "GRAPHICS (GPU)"))
                    .strong()
                    .size(FONT_SMALL)
                    .color(TEXT_SECONDARY),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                stat_toggle(
                    ui,
                    &mut stats.show_gpu_section,
                    lang.tr("Показывать блок", "Show block"),
                );
            });
        },
    );
    ui.add_space(4.0);
    ui.add_enabled_ui(stats.show_gpu_section, |ui| {
        stat_checkbox_grid(
            ui,
            content_w,
            &mut [
                (
                    &mut stats.show_gpu_temp,
                    lang.tr("Температура (°C)", "Temperature (°C)"),
                ),
                (
                    &mut stats.show_gpu_usage,
                    lang.tr("Загрузка ядра (%)", "Core usage (%)"),
                ),
                (
                    &mut stats.show_gpu_clock,
                    lang.tr("Частота ядра (МГц)", "Core clock (MHz)"),
                ),
                (
                    &mut stats.show_gpu_power,
                    lang.tr("Мощность (Вт)", "Power (W)"),
                ),
                (&mut stats.show_gpu_fan, lang.tr("Кулер (%)", "Fan (%)")),
                (
                    &mut stats.show_gpu_vram,
                    lang.tr("Видеопамять (VRAM)", "VRAM usage"),
                ),
            ],
        );
    });

    draw_divider(ui, content_w);

    section_title(ui, lang.tr("ПАМЯТЬ И СИСТЕМА", "MEMORY & SYSTEM"));
    stat_checkbox_grid(
        ui,
        content_w,
        &mut [
            (
                &mut stats.show_ram_section,
                lang.tr("ОЗУ (занято / всего)", "RAM (used / total)"),
            ),
            (
                &mut stats.show_network,
                lang.tr("Сеть (↓ / ↑)", "Network (↓ / ↑)"),
            ),
            (
                &mut stats.show_uptime,
                lang.tr("Аптайм системы", "System uptime"),
            ),
            (
                &mut stats.show_disk,
                lang.tr("Свободно на диске C:", "Free on drive C:"),
            ),
        ],
    );
}

fn render_corner_stats_subtab(
    ui: &mut egui::Ui,
    stats: &mut StatsVisibility,
    lang: Language,
    content_w: f32,
) {
    let preset_btn_w = ((content_w - 6.0) * 0.5).floor();
    ui.allocate_ui_with_layout(
        Vec2::new(content_w, 24.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if ui
                .add_sized(
                    [preset_btn_w, 24.0],
                    egui::Button::new(
                        RichText::new(lang.tr("ВКЛЮЧИТЬ ВСЕ", "ENABLE ALL"))
                            .size(FONT_SMALL)
                            .color(TEXT_PRIMARY),
                    )
                    .fill(SURFACE_ALT)
                    .stroke(Stroke::new(BORDER_WIDTH, BORDER_HAIRLINE))
                    .corner_radius(CornerRadius::same(CONTROL_RADIUS)),
                )
                .clicked()
            {
                stats.apply_corner_all();
            }
            if ui
                .add_sized(
                    [preset_btn_w, 24.0],
                    egui::Button::new(
                        RichText::new(lang.tr("ПО УМОЛЧАНИЮ", "DEFAULTS"))
                            .size(FONT_SMALL)
                            .color(TEXT_SECONDARY),
                    )
                    .fill(SURFACE_ALT)
                    .stroke(Stroke::new(BORDER_WIDTH, BORDER_HAIRLINE))
                    .corner_radius(CornerRadius::same(CONTROL_RADIUS)),
                )
                .clicked()
            {
                stats.apply_corner_defaults();
            }
        },
    );

    ui.add_space(10.0);

    section_title(
        ui,
        lang.tr("ЭЛЕМЕНТЫ КОМПАКТНОЙ СТРОКИ", "COMPACT BAR ITEMS"),
    );
    ui.add(
        egui::Label::new(
            RichText::new(lang.tr(
                "Отмеченные показатели отображаются в одну строку в углу экрана.",
                "Checked metrics appear in a single compact row in the corner.",
            ))
            .size(FONT_SMALL)
            .color(TEXT_MUTED),
        )
        .wrap(),
    );
    ui.add_space(8.0);

    stat_checkbox_grid(
        ui,
        content_w,
        &mut [
            (
                &mut stats.corner_show_fps,
                lang.tr("Текущий FPS", "Current FPS"),
            ),
            (&mut stats.corner_show_1pct_low, "1% Low FPS"),
            (
                &mut stats.corner_show_frametime,
                lang.tr("Время кадра (мс)", "Frame time (ms)"),
            ),
            (&mut stats.corner_show_cpu_temp, "CPU °C"),
            (&mut stats.corner_show_cpu_usage, "CPU %"),
            (&mut stats.corner_show_gpu_temp, "GPU °C"),
            (&mut stats.corner_show_gpu_usage, "GPU %"),
            (&mut stats.corner_show_vram, "VRAM"),
            (&mut stats.corner_show_ram, lang.tr("ОЗУ", "RAM")),
            (&mut stats.corner_show_net, lang.tr("Сеть", "Net")),
        ],
    );
}

fn render_hotkeys_tab(ui: &mut egui::Ui, settings: &mut OverlaySettings, content_w: f32) {
    let lang = settings.lang;
    let items = [
        (
            HotkeyAction::ToggleHidden,
            lang.tr("Скрыть / показать оверлей", "Hide / show overlay"),
            lang.tr(
                "Мгновенно убрать или вернуть панель с экрана",
                "Instantly hide or show the overlay panel",
            ),
        ),
        (
            HotkeyAction::ToggleClickThrough,
            lang.tr("Клик-сквозь", "Click-through"),
            lang.tr(
                "Пропускать клики в игру или перетаскивать оверлей",
                "Pass clicks through to game or allow dragging",
            ),
        ),
        (
            HotkeyAction::CycleMode,
            lang.tr("Режим: Полный / Угол", "Mode: Full / Corner"),
            lang.tr(
                "Переключить подробный вид и компактную строку",
                "Switch between detailed view and compact bar",
            ),
        ),
        (
            HotkeyAction::CycleCorner,
            lang.tr("Сменить угол экрана", "Cycle screen corner"),
            lang.tr(
                "Переместить оверлей по часовой стрелке",
                "Move the overlay clockwise around corners",
            ),
        ),
        (
            HotkeyAction::ToggleSettings,
            lang.tr("Окно настроек", "Settings window"),
            lang.tr(
                "Открыть или закрыть это окно параметров",
                "Open or close this configuration window",
            ),
        ),
        (
            HotkeyAction::ShowHelp,
            lang.tr("Подсказка клавиш", "Hotkey hint"),
            lang.tr(
                "Показать список горячих клавиш на оверлее",
                "Show assigned hotkeys on the overlay",
            ),
        ),
        (
            HotkeyAction::QuitApp,
            lang.tr("Выход из FPScraft", "Exit FPScraft"),
            lang.tr("Завершить работу утилиты", "Quit the telemetry utility"),
        ),
    ];

    let inner_w = (content_w - 22.0).max(240.0);

    for (action, title, desc) in items {
        let capturing = settings.capturing_action == Some(action);
        let current_bind = settings.hotkeys.key_for(action);

        Frame::NONE
            .fill(SURFACE_ALT)
            .stroke(Stroke::new(
                BORDER_WIDTH,
                if capturing { ACCENT } else { BORDER_HAIRLINE },
            ))
            .corner_radius(CornerRadius::same(CONTROL_RADIUS))
            .inner_margin(Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.set_width(inner_w);
                ui.set_max_width(inner_w);

                ui.allocate_ui_with_layout(
                    Vec2::new(inner_w, 22.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.label(RichText::new(title).size(FONT_MEDIUM).color(TEXT_PRIMARY));

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if current_bind.is_some() {
                                let clear_btn = egui::Button::new(
                                    RichText::new("×").size(FONT_SMALL).color(TEXT_MUTED),
                                )
                                .fill(BG_SETTINGS)
                                .stroke(Stroke::new(BORDER_WIDTH, BORDER_HAIRLINE))
                                .corner_radius(CornerRadius::same(CONTROL_RADIUS));

                                if ui
                                    .add(clear_btn)
                                    .on_hover_text(lang.tr("Убрать клавишу", "Clear binding"))
                                    .clicked()
                                {
                                    settings.hotkeys.set_key(action, None);
                                    if capturing {
                                        settings.capturing_action = None;
                                    }
                                }
                                ui.add_space(4.0);
                            }

                            let btn_text = if capturing {
                                RichText::new(lang.tr("НАЖМИТЕ...", "PRESS KEY..."))
                                    .size(FONT_SMALL)
                                    .color(SIGNAL_WARN)
                            } else {
                                RichText::new(lang.tr("НАЗНАЧИТЬ", "BIND"))
                                    .size(FONT_SMALL)
                                    .color(TEXT_SECONDARY)
                            };

                            let btn = egui::Button::new(btn_text)
                                .fill(BG_SETTINGS)
                                .stroke(Stroke::new(
                                    BORDER_WIDTH,
                                    if capturing {
                                        SIGNAL_WARN
                                    } else {
                                        BORDER_HAIRLINE
                                    },
                                ))
                                .corner_radius(CornerRadius::same(CONTROL_RADIUS));

                            if ui.add(btn).clicked() {
                                settings.capturing_action =
                                    if capturing { None } else { Some(action) };
                            }

                            ui.add_space(6.0);

                            Frame::NONE
                                .fill(BG_SETTINGS)
                                .stroke(Stroke::new(BORDER_WIDTH, BORDER_HAIRLINE))
                                .corner_radius(CornerRadius::same(CONTROL_RADIUS))
                                .inner_margin(Margin::symmetric(8, 3))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(format_hotkey(current_bind, lang))
                                            .strong()
                                            .size(FONT_SMALL)
                                            .color(if current_bind.is_some() {
                                                ACCENT
                                            } else {
                                                TEXT_MUTED
                                            }),
                                    );
                                });
                        });
                    },
                );

                ui.add_space(2.0);
                ui.add(
                    egui::Label::new(RichText::new(desc).size(FONT_SMALL).color(TEXT_MUTED)).wrap(),
                );
            });
        ui.add_space(4.0);
    }

    ui.add_space(8.0);
    if ui
        .add_sized(
            [content_w, 24.0],
            egui::Button::new(
                RichText::new(lang.tr("СБРОСИТЬ ПО УМОЛЧАНИЮ", "RESET TO DEFAULTS"))
                    .size(FONT_SMALL)
                    .color(TEXT_SECONDARY),
            )
            .fill(SURFACE_ALT)
            .stroke(Stroke::new(BORDER_WIDTH, BORDER_HAIRLINE))
            .corner_radius(CornerRadius::same(CONTROL_RADIUS)),
        )
        .clicked()
    {
        settings.hotkeys = HotkeyConfig::default();
        settings.capturing_action = None;
    }
}

fn render_appearance_tab(
    ui: &mut egui::Ui,
    settings: &mut OverlaySettings,
    actions: &mut SettingsFrameActions,
    content_w: f32,
) {
    let lang = settings.lang;
    let prev_scale = settings.theme.ui_scale;
    let prev_border_w = settings.theme.border_width;
    let prev_radius = settings.theme.corner_radius;
    let prev_opacity = settings.theme.bg_opacity;

    section_title(ui, lang.tr("ЦВЕТОВАЯ ТЕМА", "COLOR THEME"));
    ui.spacing_mut().item_spacing.y = 4.0;
    for preset in ThemePreset::ALL {
        let active = settings.theme.preset == preset;
        let cols = preset.colors();
        let (row_rect, row_resp) =
            ui.allocate_exact_size(Vec2::new(content_w, 26.0), egui::Sense::click());
        let painter = ui.painter();

        painter.rect_filled(
            row_rect,
            CornerRadius::same(CONTROL_RADIUS),
            if active { SURFACE_ALT } else { BG_SETTINGS },
        );
        painter.rect_stroke(
            row_rect,
            CornerRadius::same(CONTROL_RADIUS),
            Stroke::new(
                BORDER_WIDTH,
                if active {
                    cols.accent
                } else if row_resp.hovered() {
                    TEXT_MUTED
                } else {
                    BORDER_HAIRLINE
                },
            ),
            egui::StrokeKind::Inside,
        );

        let swatch_pos = Pos2::new(row_rect.min.x + 10.0, row_rect.center().y - 6.0);
        let swatch_bg = Color32::from_rgb(cols.bg_rgb.0, cols.bg_rgb.1, cols.bg_rgb.2);
        painter.rect_filled(
            egui::Rect::from_min_size(swatch_pos, Vec2::new(8.0, 12.0)),
            CornerRadius::ZERO,
            swatch_bg,
        );
        painter.rect_filled(
            egui::Rect::from_min_size(swatch_pos + Vec2::new(8.0, 0.0), Vec2::new(6.0, 12.0)),
            CornerRadius::ZERO,
            cols.accent,
        );

        painter.text(
            Pos2::new(row_rect.min.x + 32.0, row_rect.center().y),
            egui::Align2::LEFT_CENTER,
            preset.label(lang),
            egui::FontId::monospace(FONT_SMALL),
            if active { TEXT_PRIMARY } else { TEXT_SECONDARY },
        );

        if active {
            painter.text(
                Pos2::new(row_rect.max.x - 10.0, row_rect.center().y),
                egui::Align2::RIGHT_CENTER,
                lang.tr("АКТИВНА", "ACTIVE"),
                egui::FontId::monospace(FONT_SMALL),
                cols.accent,
            );
        }

        if row_resp.clicked() {
            settings.theme.preset = preset;
        }
    }

    draw_divider(ui, content_w);

    section_title(
        ui,
        lang.tr("ПРОЗРАЧНОСТЬ, РАМКА И РАЗМЕР", "OPACITY, BORDER & SCALE"),
    );

    threshold_slider(
        ui,
        content_w,
        lang.tr("Непрозрачность фона", "Background opacity"),
        &mut settings.theme.bg_opacity,
        0.0..=1.0,
        "%",
        100.0,
        0.01,
        0,
    );
    threshold_slider(
        ui,
        content_w,
        lang.tr("Непрозрачность рамки", "Border opacity"),
        &mut settings.theme.border_opacity,
        0.0..=1.0,
        "%",
        100.0,
        0.01,
        0,
    );
    threshold_slider(
        ui,
        content_w,
        lang.tr("Толщина рамки", "Border thickness"),
        &mut settings.theme.border_width,
        0.0..=3.0,
        "px",
        1.0,
        0.5,
        1,
    );

    let mut radius_f = settings.theme.corner_radius as f32;
    threshold_slider(
        ui,
        content_w,
        lang.tr("Скругление углов", "Corner radius"),
        &mut radius_f,
        0.0..=10.0,
        "px",
        1.0,
        1.0,
        0,
    );
    settings.theme.corner_radius = radius_f.round() as u8;

    threshold_slider(
        ui,
        content_w,
        lang.tr("Масштаб интерфейса", "UI scale"),
        &mut settings.theme.ui_scale,
        0.80..=1.35,
        "%",
        100.0,
        0.01,
        0,
    );

    ui.add_space(4.0);
    stat_toggle(
        ui,
        &mut settings.theme.show_dividers,
        lang.tr(
            "Показывать разделительные линии (1 px)",
            "Show 1 px divider lines",
        ),
    );

    draw_divider(ui, content_w);

    section_title(ui, lang.tr("РЕЖИМ И ПОЗИЦИЯ", "MODE & POSITION"));

    ui.label(
        RichText::new(lang.tr("Режим отображения", "Display mode"))
            .size(FONT_SMALL)
            .color(TEXT_SECONDARY),
    );
    ui.add_space(3.0);
    let mode_btn_w = ((content_w - 6.0) * 0.5).floor();
    ui.allocate_ui_with_layout(
        Vec2::new(content_w, 24.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (mode, label) in [
                (OverlayMode::Full, lang.tr("Полный", "Full")),
                (OverlayMode::Corner, lang.tr("Угол", "Corner")),
            ] {
                let active = settings.mode == mode;
                if ui
                    .add_sized(
                        [mode_btn_w, 24.0],
                        egui::Button::new(RichText::new(label).size(FONT_SMALL).color(if active {
                            ACCENT
                        } else {
                            TEXT_SECONDARY
                        }))
                        .fill(if active {
                            SURFACE_ALT
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(Stroke::new(
                            BORDER_WIDTH,
                            if active { ACCENT } else { BORDER_HAIRLINE },
                        ))
                        .corner_radius(CornerRadius::same(CONTROL_RADIUS)),
                    )
                    .clicked()
                    && !active
                {
                    settings.mode = mode;
                    actions.mode_changed = true;
                }
            }
        },
    );

    ui.add_space(8.0);

    ui.label(
        RichText::new(lang.tr("Угол привязки", "Anchor corner"))
            .size(FONT_SMALL)
            .color(TEXT_SECONDARY),
    );
    ui.add_space(3.0);
    let corner_btn_w = ((content_w - 18.0) / 4.0).floor();
    ui.allocate_ui_with_layout(
        Vec2::new(content_w, 24.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for corner in [
                Corner::TopLeft,
                Corner::TopRight,
                Corner::BottomRight,
                Corner::BottomLeft,
            ] {
                let active = settings.corner == corner;
                if ui
                    .add_sized(
                        [corner_btn_w, 24.0],
                        egui::Button::new(
                            RichText::new(corner.label(lang))
                                .size(FONT_SMALL)
                                .color(if active { ACCENT } else { TEXT_SECONDARY }),
                        )
                        .fill(if active {
                            SURFACE_ALT
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(Stroke::new(
                            BORDER_WIDTH,
                            if active { ACCENT } else { BORDER_HAIRLINE },
                        ))
                        .corner_radius(CornerRadius::same(CONTROL_RADIUS)),
                    )
                    .clicked()
                    && !active
                {
                    settings.corner = corner;
                    actions.corner_changed = true;
                }
            }
        },
    );

    draw_divider(ui, content_w);

    section_title(
        ui,
        lang.tr("СИГНАЛЬНЫЕ ПОРОГИ ЦВЕТА", "SIGNAL COLOR THRESHOLDS"),
    );
    ui.add(
        egui::Label::new(
            RichText::new(lang.tr(
                "Числа нейтральные в норме и подсвечиваются жёлтым / красным при превышении порога.",
                "Numbers stay neutral when normal and turn yellow / red above thresholds.",
            ))
            .size(FONT_SMALL)
            .color(TEXT_MUTED),
        )
        .wrap(),
    );
    ui.add_space(8.0);

    threshold_slider(
        ui,
        content_w,
        lang.tr("Температура: жёлтый порог (°C)", "Temperature: yellow (°C)"),
        &mut settings.thresholds.temp_warn_c,
        50.0..=95.0,
        "°C",
        1.0,
        1.0,
        0,
    );
    threshold_slider(
        ui,
        content_w,
        lang.tr("Температура: красный порог (°C)", "Temperature: red (°C)"),
        &mut settings.thresholds.temp_crit_c,
        55.0..=105.0,
        "°C",
        1.0,
        1.0,
        0,
    );
    if settings.thresholds.temp_crit_c <= settings.thresholds.temp_warn_c {
        settings.thresholds.temp_crit_c = (settings.thresholds.temp_warn_c + 2.0).min(105.0);
    }

    ui.add_space(6.0);

    threshold_slider(
        ui,
        content_w,
        lang.tr(
            "Загрузка / VRAM / ОЗУ: жёлтый (%)",
            "Load / VRAM / RAM: yellow (%)",
        ),
        &mut settings.thresholds.load_warn_pct,
        50.0..=95.0,
        "%",
        1.0,
        1.0,
        0,
    );
    threshold_slider(
        ui,
        content_w,
        lang.tr(
            "Загрузка / VRAM / ОЗУ: красный (%)",
            "Load / VRAM / RAM: red (%)",
        ),
        &mut settings.thresholds.load_crit_pct,
        60.0..=99.0,
        "%",
        1.0,
        1.0,
        0,
    );
    if settings.thresholds.load_crit_pct <= settings.thresholds.load_warn_pct {
        settings.thresholds.load_crit_pct = (settings.thresholds.load_warn_pct + 2.0).min(99.0);
    }

    ui.add_space(10.0);
    if ui
        .add_sized(
            [content_w, 24.0],
            egui::Button::new(
                RichText::new(lang.tr("СБРОСИТЬ ТЕМУ И ПОРОГИ", "RESET THEME & THRESHOLDS"))
                    .size(FONT_SMALL)
                    .color(TEXT_SECONDARY),
            )
            .fill(SURFACE_ALT)
            .stroke(Stroke::new(BORDER_WIDTH, BORDER_HAIRLINE))
            .corner_radius(CornerRadius::same(CONTROL_RADIUS)),
        )
        .clicked()
    {
        settings.theme = ThemeConfig::default();
        settings.thresholds = ThresholdConfig::default();
        actions.layout_changed = true;
    }

    if (settings.theme.ui_scale - prev_scale).abs() > 0.001
        || (settings.theme.border_width - prev_border_w).abs() > 0.01
        || settings.theme.corner_radius != prev_radius
        || (settings.theme.bg_opacity - prev_opacity).abs() > 0.001
    {
        actions.layout_changed = true;
    }
}

#[allow(clippy::too_many_arguments)]
fn threshold_slider(
    ui: &mut egui::Ui,
    content_w: f32,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    display_mult: f32,
    step: f64,
    decimals: usize,
) {
    let formatted = format!("{:.*} {suffix}", decimals, *value * display_mult);
    ui.allocate_ui_with_layout(
        Vec2::new(content_w, 18.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.label(RichText::new(label).size(FONT_SMALL).color(TEXT_SECONDARY));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(formatted)
                        .size(FONT_SMALL)
                        .color(TEXT_PRIMARY),
                );
            });
        },
    );
    ui.spacing_mut().slider_width = (content_w - 8.0).max(160.0);
    ui.add(
        egui::Slider::new(value, range)
            .show_value(false)
            .step_by(step),
    );
    ui.add_space(4.0);
}

fn section_title(ui: &mut egui::Ui, title: &str) {
    ui.label(
        RichText::new(title)
            .strong()
            .size(FONT_SMALL)
            .color(TEXT_SECONDARY),
    );
    ui.add_space(4.0);
}

fn draw_divider(ui: &mut egui::Ui, width: f32) {
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::ZERO, DIVIDER_LINE);
    ui.add_space(8.0);
}

fn stat_checkbox_grid(ui: &mut egui::Ui, content_w: f32, items: &mut [(&mut bool, &str)]) {
    let col_w = ((content_w - 12.0) * 0.5).floor().max(130.0);
    egui::Grid::new(ui.next_auto_id())
        .num_columns(2)
        .min_col_width(col_w)
        .max_col_width(col_w)
        .spacing([12.0, 5.0])
        .show(ui, |ui| {
            for pair in items.chunks_mut(2) {
                for (val, label) in pair.iter_mut() {
                    stat_toggle(ui, val, label);
                }
                ui.end_row();
            }
        });
}

fn stat_toggle(ui: &mut egui::Ui, value: &mut bool, label: &str) {
    ui.checkbox(
        value,
        RichText::new(label).size(FONT_SMALL).color(TEXT_PRIMARY),
    );
}

pub fn is_vk_down(vk: u16) -> bool {
    // SAFETY: GetAsyncKeyState queries asynchronous key state for a Win32 virtual-key code.
    unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

fn poll_any_rebindable_key() -> Option<u16> {
    let nav_keys = [
        VK_INSERT, VK_DELETE, VK_HOME, VK_END, VK_PRIOR, VK_NEXT, VK_PAUSE,
    ];
    for vk in nav_keys {
        if is_vk_down(vk) {
            return Some(vk);
        }
    }
    for vk in VK_F1..=VK_F12 {
        if is_vk_down(vk) {
            return Some(vk);
        }
    }
    for vk in VK_0..=VK_9 {
        if is_vk_down(vk) {
            return Some(vk);
        }
    }
    for vk in VK_A..=VK_Z {
        if is_vk_down(vk) {
            return Some(vk);
        }
    }
    None
}

pub fn vk_name(vk: u16) -> String {
    match vk {
        VK_F1..=VK_F12 => format!("F{}", vk - VK_F1 + 1),
        VK_0..=VK_9 => format!("{}", (vk as u8) as char),
        VK_A..=VK_Z => format!("{}", (vk as u8) as char),
        VK_INSERT => String::from("INS"),
        VK_DELETE => String::from("DEL"),
        VK_HOME => String::from("HOME"),
        VK_END => String::from("END"),
        VK_PRIOR => String::from("PGUP"),
        VK_NEXT => String::from("PGDN"),
        VK_PAUSE => String::from("PAUSE"),
        other => format!("0x{other:02X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corner_visibility_defaults_and_empty_check() {
        let mut vis = StatsVisibility::default();
        assert!(vis.has_any_corner_item());

        vis.corner_show_fps = false;
        vis.corner_show_1pct_low = false;
        vis.corner_show_cpu_temp = false;
        vis.corner_show_gpu_temp = false;
        assert!(!vis.has_any_corner_item());

        vis.corner_show_net = true;
        assert!(vis.has_any_corner_item());
    }

    #[test]
    fn default_hotkeys_only_bind_toggle_hidden_alt_t() {
        let hk = HotkeyConfig::default();
        assert_eq!(hk.toggle_hidden, Some(KeyBind::alt_key(b'T' as u16)));
        assert_eq!(format_hotkey(hk.toggle_hidden, Language::Ru), "ALT+T");
        assert_eq!(hk.toggle_click_through, None);
        assert_eq!(hk.cycle_mode, None);
        assert_eq!(hk.cycle_corner, None);
        assert_eq!(hk.toggle_settings, None);
        assert_eq!(hk.show_help, None);
        assert_eq!(hk.quit_app, None);
        assert_eq!(format_hotkey(hk.quit_app, Language::Ru), "НЕТ");
        assert_eq!(format_hotkey(hk.quit_app, Language::En), "NONE");
    }

    #[test]
    fn stats_subtab_presets_do_not_clobber_each_other() {
        let mut vis = StatsVisibility::default();
        vis.corner_show_net = true;
        vis.show_fps_graph = true;

        vis.apply_full_essential();
        assert!(!vis.show_fps_graph);
        assert!(vis.corner_show_net);

        vis.apply_corner_all();
        assert!(vis.corner_show_ram);
        assert!(!vis.show_fps_graph);

        vis.apply_corner_defaults();
        assert!(!vis.corner_show_net);
        assert!(!vis.show_fps_graph);

        vis.apply_full_all();
        assert!(vis.show_fps_graph);
        assert!(!vis.corner_show_net);
    }
}
