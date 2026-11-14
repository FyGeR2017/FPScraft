use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame,
    Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, ViewportCommand,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowLongW, IsWindowVisible, SetLayeredWindowAttributes, SetWindowLongW, ShowWindow,
    GWL_EXSTYLE, LWA_ALPHA, SW_HIDE, SW_SHOWNA, WS_EX_LAYERED,
};

use crate::autostart;
use crate::config;
use crate::metrics::{MetricsCollector, MetricsSnapshot};
use crate::settings::{
    format_hotkey, render_settings_viewport, Corner, HotkeyPoller, KeyBind, OverlayMode,
    OverlaySettings,
};
use crate::theme::{
    value_fps_color, value_load_color, value_temp_color, Language, BORDER_WIDTH, FONT_LARGE,
    FONT_MEDIUM, FONT_SMALL, FULL_WIDTH, GRAPH_LINE_WIDTH, SIGNAL_CRIT, SIGNAL_WARN, TEXT_MUTED,
    TEXT_PRIMARY, TEXT_SECONDARY,
};
use crate::tray::{Tray, TrayAction};

struct HiddenWatcherState {
    hwnd: isize,
    hidden: bool,
    wait_for_release: bool,
    toggle_hidden: Option<KeyBind>,
}

/// Main eframe application rendering the precision telemetry overlay and Settings window.
pub struct OverlayApp {
    collector: MetricsCollector,
    last_frame_instant: Instant,
    settings: OverlaySettings,
    click_through: bool,
    hidden: bool,
    help_pinned: bool,
    poller: HotkeyPoller,
    last_viewport_size: Vec2,
    overlay_hwnd: isize,
    last_applied_alpha: u8,
    watcher_state: Arc<Mutex<HiddenWatcherState>>,
    tray: Option<Tray>,
    autostart_enabled: bool,
    last_saved_settings: config::PersistedSettings,
    dirty_since: Option<Instant>,
    initial_pos_applied: bool,
    last_known_pos: Option<Pos2>,
    programmatic_move_frames: u8,
}

impl OverlayApp {
    /// Initializes the overlay, installs tabular monospace fonts, and sets dark neutral visuals.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_tabular_fonts(&cc.egui_ctx);
        configure_visuals(&cc.egui_ctx);

        let mut settings = OverlaySettings::default();
        config::load_settings(&mut settings);
        let last_saved_settings = config::PersistedSettings::from_settings(&settings);
        let initial_hwnd = extract_win32_hwnd(cc);
        let watcher_state = Arc::new(Mutex::new(HiddenWatcherState {
            hwnd: initial_hwnd,
            hidden: false,
            wait_for_release: false,
            toggle_hidden: settings.hotkeys.toggle_hidden,
        }));

        spawn_unhide_watcher(cc.egui_ctx.clone(), Arc::clone(&watcher_state));

        let autostart_enabled = autostart::is_enabled();
        let tray = Tray::new(settings.lang, false, autostart_enabled);

        Self {
            collector: MetricsCollector::new(),
            last_frame_instant: Instant::now(),
            settings,
            click_through: false,
            hidden: false,
            help_pinned: false,
            poller: HotkeyPoller::default(),
            last_viewport_size: Vec2::new(FULL_WIDTH, 340.0),
            overlay_hwnd: initial_hwnd,
            last_applied_alpha: 255,
            watcher_state,
            tray,
            autostart_enabled,
            last_saved_settings,
            dirty_since: None,
            initial_pos_applied: false,
            last_known_pos: None,
            programmatic_move_frames: 0,
        }
    }

    fn ensure_overlay_hwnd(&mut self, frame: &eframe::Frame) -> isize {
        if self.overlay_hwnd != 0 {
            return self.overlay_hwnd;
        }
        let hwnd = extract_win32_hwnd(frame);
        if hwnd != 0 {
            self.overlay_hwnd = hwnd;
            if let Ok(mut st) = self.watcher_state.lock() {
                st.hwnd = hwnd;
            }
        }
        hwnd
    }

    fn sync_overlay_opacity(&mut self) {
        if self.overlay_hwnd == 0 || self.hidden {
            return;
        }
        let target_alpha = overlay_window_alpha(self.settings.theme.bg_opacity);
        if target_alpha == 255 && self.last_applied_alpha == 255 {
            return;
        }

        let hwnd = self.overlay_hwnd as HWND;
        // SAFETY: hwnd is a valid Win32 window handle for FPScraft Overlay.
        let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) };
        let has_layered = (ex_style & WS_EX_LAYERED as i32) != 0;

        if target_alpha != self.last_applied_alpha || (target_alpha < 255 && !has_layered) {
            // SAFETY: hwnd is valid; adding WS_EX_LAYERED preserves existing extended styles
            // (including WS_EX_TRANSPARENT when click-through is active).
            unsafe {
                if !has_layered {
                    SetWindowLongW(hwnd, GWL_EXSTYLE, ex_style | WS_EX_LAYERED as i32);
                }
                SetLayeredWindowAttributes(hwnd, 0, target_alpha, LWA_ALPHA);
            }
            self.last_applied_alpha = target_alpha;
        }
    }

    fn apply_hidden_state(&mut self, ctx: &egui::Context, hidden: bool) {
        self.hidden = hidden;
        let hwnd = self.overlay_hwnd;
        if let Ok(mut st) = self.watcher_state.lock() {
            st.hwnd = hwnd;
            st.hidden = hidden;
            st.wait_for_release = hidden;
            st.toggle_hidden = self.settings.hotkeys.toggle_hidden;
        }
        if hwnd != 0 {
            // SAFETY: hwnd is a valid Win32 window handle obtained from HasWindowHandle.
            unsafe {
                ShowWindow(hwnd as HWND, if hidden { SW_HIDE } else { SW_SHOWNA });
            }
        }
        // IMPORTANT: never send window-flag commands (e.g. `MousePassthrough`) while hidden.
        // winit keeps its own copy of the window flags and still believes the window is visible,
        // so any flag change makes it call `ShowWindow(SW_SHOW)` again and resurrect the window
        // as a stale, fully black rectangle instead of leaving it hidden.
        if !hidden {
            ctx.send_viewport_cmd(ViewportCommand::MousePassthrough(self.click_through));
        }
    }

    fn handle_global_hotkeys(&mut self, ctx: &egui::Context) {
        if self.settings.capturing_action.is_some() {
            return;
        }

        let hk = self.settings.hotkeys.clone();

        if self.poller.is_edge_pressed(hk.toggle_click_through) {
            self.click_through = !self.click_through;
            // Only touch window flags while the window is really on screen (see `apply_hidden_state`).
            if !self.hidden {
                ctx.send_viewport_cmd(ViewportCommand::MousePassthrough(self.click_through));
            }
        }
        if self.poller.is_edge_pressed(hk.cycle_mode) {
            self.set_mode(ctx, self.settings.mode.toggled());
        }
        if self.poller.is_edge_pressed(hk.toggle_hidden) {
            let next_hidden = !self.hidden;
            self.apply_hidden_state(ctx, next_hidden);
        }
        if self.poller.is_edge_pressed(hk.cycle_corner) {
            self.settings.corner = self.settings.corner.next();
            self.settings.custom_pos = None;
            self.apply_corner_position(ctx);
        }
        if self.poller.is_edge_pressed(hk.toggle_settings) {
            self.settings.open = !self.settings.open;
        }
        if self.poller.is_edge_pressed(hk.show_help) {
            self.help_pinned = !self.help_pinned;
        }
        if self.poller.is_edge_pressed(hk.quit_app) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn handle_tray_action(&mut self, ctx: &egui::Context, action: TrayAction) {
        match action {
            TrayAction::ToggleOverlay => {
                let next_hidden = !self.hidden;
                self.apply_hidden_state(ctx, next_hidden);
            }
            TrayAction::OpenSettings => {
                self.settings.open = true;
                ctx.request_repaint();
            }
            TrayAction::ToggleAutostart(enable) => {
                self.autostart_enabled = match autostart::set_enabled(enable) {
                    Ok(()) => enable,
                    Err(_) => autostart::is_enabled(),
                };
            }
            TrayAction::Quit => {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
    }

    fn set_mode(&mut self, _ctx: &egui::Context, mode: OverlayMode) {
        if self.settings.mode != mode {
            self.settings.mode = mode;
            self.last_viewport_size = Vec2::ZERO;
        }
    }

    fn apply_corner_position(&mut self, ctx: &egui::Context) {
        let monitor_size = ctx
            .input(|i| i.viewport().monitor_size)
            .unwrap_or(Vec2::new(1920.0, 1080.0));
        let win_size = if self.last_viewport_size.x > 10.0 {
            self.last_viewport_size
        } else {
            Vec2::new(FULL_WIDTH * self.settings.theme.ui_scale, 28.0)
        };
        let pos = self.settings.corner.position(monitor_size, win_size);
        self.programmatic_move_frames = 3;
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos));
    }

    fn sync_viewport_size(&mut self, ctx: &egui::Context, measured: Vec2) {
        let rounded = Vec2::new(measured.x.ceil().max(40.0), measured.y.ceil().max(20.0));
        if (rounded.x - self.last_viewport_size.x).abs() > 1.0
            || (rounded.y - self.last_viewport_size.y).abs() > 1.0
        {
            self.last_viewport_size = rounded;
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(rounded));
            if self.settings.custom_pos.is_none() && self.settings.corner != Corner::TopLeft {
                self.apply_corner_position(ctx);
            }
        }
    }
}

impl eframe::App for OverlayApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame_instant).as_secs_f32();
        self.last_frame_instant = now;

        self.ensure_overlay_hwnd(frame);

        if !self.initial_pos_applied {
            self.initial_pos_applied = true;
            if let Some([x, y]) = self.settings.custom_pos {
                let mut pos = Pos2::new(x, y);
                if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
                    pos.x = pos.x.clamp(0.0, (monitor.x - 40.0).max(0.0));
                    pos.y = pos.y.clamp(0.0, (monitor.y - 20.0).max(0.0));
                }
                self.programmatic_move_frames = 3;
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos));
            } else if self.settings.corner != Corner::TopLeft {
                self.apply_corner_position(ctx);
            }
        }

        // The watcher thread un-hides the overlay while the window receives no input, so the
        // mouse passthrough state is restored here, on the UI thread, in the very frame that
        // repaints the panel.
        let unhidden_by_watcher = match self.watcher_state.lock() {
            Ok(mut st) => {
                st.toggle_hidden = self.settings.hotkeys.toggle_hidden;
                self.hidden && !st.hidden
            }
            Err(_) => false,
        };
        if unhidden_by_watcher {
            if let Some(bind) = self.settings.hotkeys.toggle_hidden {
                self.poller.reset_state(bind);
            }
            self.apply_hidden_state(ctx, false);
        }

        self.handle_global_hotkeys(ctx);

        if let Some(action) = self.tray.as_mut().and_then(Tray::poll) {
            self.handle_tray_action(ctx, action);
        }
        if let Some(tray) = self.tray.as_mut() {
            tray.sync(self.settings.lang, self.hidden, self.autostart_enabled);
        }

        self.collector.on_frame(dt);

        let settings_actions = render_settings_viewport(ctx, &mut self.settings, &mut self.poller);
        if settings_actions.language_changed {
            let _ = config::save_language(self.settings.lang);
        }
        if settings_actions.mode_changed {
            self.last_viewport_size = Vec2::ZERO;
        } else if settings_actions.corner_changed {
            self.settings.custom_pos = None;
            self.apply_corner_position(ctx);
        }

        if self.hidden {
            self.persist_settings_if_dirty(false);
            // Keep the tray menu responsive while the overlay window is hidden.
            ctx.request_repaint_after(Duration::from_millis(120));
            return;
        }

        self.sync_overlay_opacity();

        let snap = self.collector.snapshot().clone();
        let mode = self.settings.mode;
        let theme = self.settings.theme.clone();
        let scale = theme.ui_scale;

        let panel_fill = theme.panel_bg();
        let border_stroke = theme.effective_border_stroke();
        let corner_rad = CornerRadius::same(theme.corner_radius);

        let full_margin = (10.0 * scale).round().clamp(6.0, 20.0) as i8;
        let corner_h_margin = (7.0 * scale).round().clamp(4.0, 14.0) as i8;
        let corner_v_margin = (3.0 * scale).round().clamp(2.0, 8.0) as i8;

        let outer_frame = Frame::NONE
            .fill(panel_fill)
            .stroke(Stroke::NONE)
            .corner_radius(corner_rad)
            .inner_margin(match mode {
                OverlayMode::Full => Margin::same(full_margin),
                OverlayMode::Corner => Margin::symmetric(corner_h_margin, corner_v_margin),
            });

        let target_w = (FULL_WIDTH * scale).round();
        let mut measured_size = Vec2::new(target_w, 28.0);
        let mut paint_border_rect: Option<Rect> = None;

        egui::CentralPanel::default()
            .frame(Frame::NONE)
            .show(ctx, |ui| {
                ui.style_mut().interaction.selectable_labels = false;
                match mode {
                    OverlayMode::Full => {
                        let mut header_bottom = 0.0_f32;
                        let mut panel_rect = Rect::NOTHING;
                        ui.allocate_ui_with_layout(
                            Vec2::new(target_w, 10.0),
                            Layout::top_down(Align::Min),
                            |ui| {
                                ui.set_width(target_w);
                                ui.set_max_width(target_w);
                                let frame_resp = outer_frame.show(ui, |ui| {
                                    header_bottom = self.render_full(ui, ctx, &snap);
                                });
                                panel_rect = frame_resp.response.rect;
                            },
                        );
                        measured_size = Vec2::new(target_w, panel_rect.height());
                        paint_border_rect = Some(panel_rect);

                        // Drag ONLY from body below the header so header buttons always receive clicks.
                        if !self.click_through && panel_rect.max.y > header_bottom + 4.0 {
                            let body_rect = Rect::from_min_max(
                                Pos2::new(panel_rect.min.x, header_bottom + 4.0),
                                panel_rect.max,
                            );
                            let drag_resp = ui.interact(
                                body_rect,
                                ui.id().with("overlay_body_drag"),
                                Sense::drag(),
                            );
                            if drag_resp.drag_started() {
                                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                            }
                        }
                    }
                    OverlayMode::Corner => {
                        ui.horizontal(|ui| {
                            let frame_resp = outer_frame.show(ui, |ui| {
                                self.render_corner(ui, ctx, &snap);
                            });
                            let rect = frame_resp.response.rect;
                            measured_size = rect.size();
                            paint_border_rect = Some(rect);
                        });
                    }
                }

                if let Some(rect) = paint_border_rect {
                    if border_stroke != Stroke::NONE {
                        ui.painter().rect_stroke(
                            rect,
                            corner_rad,
                            border_stroke,
                            StrokeKind::Inside,
                        );
                    }
                }
            });

        self.sync_viewport_size(ctx, measured_size);
        let outer_pos = ctx.input(|i| i.viewport().outer_rect).map(|r| r.min);
        if self.programmatic_move_frames > 0 {
            self.programmatic_move_frames -= 1;
            if let Some(pos) = outer_pos {
                self.last_known_pos = Some(pos);
            }
        } else if let Some(pos) = outer_pos {
            if self
                .last_known_pos
                .is_some_and(|prev| (prev.x - pos.x).abs() > 1.0 || (prev.y - pos.y).abs() > 1.0)
            {
                self.settings.custom_pos = Some([pos.x, pos.y]);
            }
            self.last_known_pos = Some(pos);
        }
        self.persist_settings_if_dirty(false);
        ctx.request_repaint_after(Duration::from_millis(16));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.persist_settings_if_dirty(true);
    }
}

impl OverlayApp {
    fn persist_settings_if_dirty(&mut self, force: bool) {
        let current = config::PersistedSettings::from_settings(&self.settings);
        if current == self.last_saved_settings {
            self.dirty_since = None;
            return;
        }
        let should_save = force
            || self
                .dirty_since
                .is_some_and(|t| t.elapsed() >= Duration::from_millis(300));
        if should_save {
            let _ = config::save_settings(&self.settings);
            self.last_saved_settings = current;
            self.dirty_since = None;
        } else if self.dirty_since.is_none() {
            self.dirty_since = Some(Instant::now());
        }
    }

    fn render_full(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        snap: &MetricsSnapshot,
    ) -> f32 {
        let lang = self.settings.lang;
        let theme = self.settings.theme.clone();
        let colors = theme.colors();
        let surface_alt_bg = theme.surface_alt_bg();
        let scale = theme.ui_scale;
        let f_large = FONT_LARGE * scale;
        let f_medium = FONT_MEDIUM * scale;
        let f_small = FONT_SMALL * scale;

        ui.spacing_mut().item_spacing = Vec2::new(
            (6.0 * scale).round().max(3.0),
            (4.0 * scale).round().max(2.0),
        );

        let header_bottom = self.render_header(ui, ctx, scale, f_medium, f_small);
        let show_help_hint =
            self.help_pinned || self.poller.is_held(self.settings.hotkeys.show_help);

        let stats = self.settings.stats.clone();
        let thresholds = self.settings.thresholds.clone();

        // 1. FPS + 1% Low + Frame time + Graph
        if stats.has_any_fps_row() {
            if theme.show_dividers {
                draw_hairline_divider(ui, colors.divider);
            }

            ui.horizontal(|ui| {
                if stats.show_fps {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        ui.label(
                            RichText::new(format!("{:.0}", snap.fps))
                                .strong()
                                .size(f_large)
                                .color(value_fps_color(snap.fps)),
                        );
                        ui.vertical(|ui| {
                            ui.add_space(13.0 * scale);
                            ui.label(RichText::new("FPS").size(f_small).color(TEXT_MUTED));
                        });
                    });
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        if stats.show_1pct_low {
                            ui.horizontal(|ui| {
                                ui.set_width(134.0 * scale);
                                ui.label(RichText::new("1% LOW").size(f_small).color(TEXT_MUTED));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(
                                        RichText::new(format!("{:>4.0}", snap.fps_1pct_low))
                                            .size(f_medium)
                                            .color(TEXT_PRIMARY),
                                    );
                                });
                            });
                        }
                        if stats.show_frametime_ms {
                            ui.horizontal(|ui| {
                                ui.set_width(134.0 * scale);
                                ui.label(
                                    RichText::new(lang.tr("КАДР", "FRAME"))
                                        .size(f_small)
                                        .color(TEXT_MUTED),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "{:>4.1} {}",
                                            snap.frametime_ms,
                                            lang.tr("мс", "ms")
                                        ))
                                        .size(f_medium)
                                        .color(TEXT_PRIMARY),
                                    );
                                });
                            });
                        }
                    });
                });
            });

            if stats.show_fps_graph {
                ui.add_space(2.0);
                let graph_border = if theme.border_opacity > 0.05 {
                    theme.border_color()
                } else {
                    colors.divider
                };
                draw_fps_graph(
                    ui,
                    &snap.fps_history,
                    snap.fps_1pct_low,
                    scale,
                    f_small,
                    colors.grid_line,
                    graph_border,
                    colors.accent,
                    theme.corner_radius.min(4),
                );
            }
        }

        // 2. CPU Section
        if stats.show_cpu_section {
            if theme.show_dividers {
                draw_hairline_divider(ui, colors.divider);
            }

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("CPU")
                        .strong()
                        .size(f_small)
                        .color(TEXT_MUTED),
                );
                ui.label(
                    RichText::new(truncate_str(&snap.cpu_name, 20))
                        .size(f_small)
                        .color(TEXT_SECONDARY),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if stats.show_cpu_usage {
                        let col = value_load_color(
                            snap.cpu_usage,
                            thresholds.load_warn_pct,
                            thresholds.load_crit_pct,
                        );
                        ui.label(
                            RichText::new(format!("{:>3.0}%", snap.cpu_usage))
                                .size(f_medium)
                                .color(col),
                        );
                    }
                    if stats.show_cpu_temp {
                        let (txt, col) = match snap.cpu_temp_c {
                            Some(t) => (
                                format!("{t:.0} °C"),
                                value_temp_color(t, thresholds.temp_warn_c, thresholds.temp_crit_c),
                            ),
                            None => (String::from("N/A"), TEXT_MUTED),
                        };
                        ui.label(RichText::new(txt).size(f_medium).color(col));
                    }
                });
            });

            if stats.show_cpu_clock || stats.show_cpu_power {
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if stats.show_cpu_power {
                            if let Some(pwr) = snap.cpu_power_w {
                                ui.label(
                                    RichText::new(format!("{pwr:.0} {}", lang.tr("Вт", "W")))
                                        .size(f_small)
                                        .color(TEXT_SECONDARY),
                                );
                            }
                        }
                        if stats.show_cpu_clock {
                            ui.label(
                                RichText::new(format!(
                                    "{} {}",
                                    snap.cpu_clock_mhz,
                                    lang.tr("МГц", "MHz")
                                ))
                                .size(f_small)
                                .color(TEXT_SECONDARY),
                            );
                        }
                    });
                });
            }

            if stats.show_cpu_cores && !snap.cpu_cores_usage.is_empty() {
                ui.add_space(1.0);
                draw_cpu_core_bars(
                    ui,
                    &snap.cpu_cores_usage,
                    scale,
                    thresholds.load_warn_pct,
                    thresholds.load_crit_pct,
                    surface_alt_bg,
                );
            }
        }

        // 3. GPU Section
        if stats.show_gpu_section {
            if theme.show_dividers {
                draw_hairline_divider(ui, colors.divider);
            }

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("GPU")
                        .strong()
                        .size(f_small)
                        .color(TEXT_MUTED),
                );
                ui.label(
                    RichText::new(truncate_str(&snap.gpu_name, 20))
                        .size(f_small)
                        .color(TEXT_SECONDARY),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if stats.show_gpu_usage {
                        let gpu_pct = snap.gpu_usage.unwrap_or(0.0);
                        let col = value_load_color(
                            gpu_pct,
                            thresholds.load_warn_pct,
                            thresholds.load_crit_pct,
                        );
                        ui.label(
                            RichText::new(format!("{:>3.0}%", gpu_pct))
                                .size(f_medium)
                                .color(col),
                        );
                    }
                    if stats.show_gpu_temp {
                        let (txt, col) = match snap.gpu_temp_c {
                            Some(t) => (
                                format!("{t:.0} °C"),
                                value_temp_color(t, thresholds.temp_warn_c, thresholds.temp_crit_c),
                            ),
                            None => (String::from("N/A"), TEXT_MUTED),
                        };
                        ui.label(RichText::new(txt).size(f_medium).color(col));
                    }
                });
            });

            if stats.show_gpu_clock || stats.show_gpu_power || stats.show_gpu_fan {
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if stats.show_gpu_fan {
                            if let Some(fan) = snap.gpu_fan_pct {
                                ui.label(
                                    RichText::new(format!("{} {fan}%", lang.tr("КУЛЕР", "FAN")))
                                        .size(f_small)
                                        .color(TEXT_MUTED),
                                );
                            }
                        }
                        if stats.show_gpu_power {
                            if let Some(pwr) = snap.gpu_power_w {
                                ui.label(
                                    RichText::new(format!("{pwr:.0} {}", lang.tr("Вт", "W")))
                                        .size(f_small)
                                        .color(TEXT_SECONDARY),
                                );
                            }
                        }
                        if stats.show_gpu_clock {
                            if let Some(clk) = snap.gpu_clock_mhz {
                                ui.label(
                                    RichText::new(format!("{clk} {}", lang.tr("МГц", "MHz")))
                                        .size(f_small)
                                        .color(TEXT_SECONDARY),
                                );
                            }
                        }
                    });
                });
            }

            if stats.show_gpu_vram {
                let vram_pct = if snap.vram_total_gb > 0.0 {
                    (snap.vram_used_gb / snap.vram_total_gb) * 100.0
                } else {
                    0.0
                };
                let vram_col =
                    value_load_color(vram_pct, thresholds.load_warn_pct, thresholds.load_crit_pct);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("VRAM").size(f_small).color(TEXT_MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{:.1} / {:.1} {} ({:.0}%)",
                                snap.vram_used_gb,
                                snap.vram_total_gb,
                                lang.tr("ГБ", "GB"),
                                vram_pct
                            ))
                            .size(f_small)
                            .color(if vram_col == TEXT_PRIMARY {
                                TEXT_SECONDARY
                            } else {
                                vram_col
                            }),
                        );
                    });
                });
            }
        }

        // 4. RAM Section
        if stats.show_ram_section {
            if theme.show_dividers {
                draw_hairline_divider(ui, colors.divider);
            }

            let ram_col = value_load_color(
                snap.ram_usage_pct,
                thresholds.load_warn_pct,
                thresholds.load_crit_pct,
            );
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(lang.tr("ОЗУ", "RAM"))
                        .strong()
                        .size(f_small)
                        .color(TEXT_MUTED),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!(
                            "{:.1} / {:.0} {} ({:.0}%)",
                            snap.ram_used_gb,
                            snap.ram_total_gb,
                            lang.tr("ГБ", "GB"),
                            snap.ram_usage_pct
                        ))
                        .size(f_medium)
                        .color(ram_col),
                    );
                });
            });
        }

        // 5. Network, Uptime & Disk
        if stats.has_any_system_footer() {
            if theme.show_dividers {
                draw_hairline_divider(ui, colors.divider);
            }

            if stats.show_network {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(lang.tr("СЕТЬ", "NET"))
                            .size(f_small)
                            .color(TEXT_MUTED),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "↓ {}  ↑ {}",
                                format_net_speed(snap.net_down_kbps, lang),
                                format_net_speed(snap.net_up_kbps, lang)
                            ))
                            .size(f_small)
                            .color(TEXT_SECONDARY),
                        );
                    });
                });
            }

            if stats.show_uptime || stats.show_disk {
                ui.horizontal(|ui| {
                    if stats.show_uptime {
                        ui.label(
                            RichText::new(format!(
                                "{} {}",
                                lang.tr("АПТАЙМ", "UPTIME"),
                                format_uptime(snap.uptime_secs)
                            ))
                            .size(f_small)
                            .color(TEXT_MUTED),
                        );
                    }
                    if stats.show_disk {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{} C: {:.0} {}",
                                    lang.tr("ДИСК", "DISK"),
                                    snap.disk_free_gb,
                                    lang.tr("ГБ", "GB")
                                ))
                                .size(f_small)
                                .color(TEXT_MUTED),
                            );
                        });
                    }
                });
            }
        }

        // 6. On-demand Hotkey Help (when ShowHelp hotkey is bound and pressed)
        if show_help_hint {
            if theme.show_dividers {
                draw_hairline_divider(ui, colors.divider);
            }
            let hk = &self.settings.hotkeys;
            let line1 = format!(
                "{} {}  {} {}  {} {}  {} {}",
                format_hotkey(hk.toggle_click_through, lang),
                lang.tr("сквозь", "pass"),
                format_hotkey(hk.cycle_mode, lang),
                lang.tr("вид", "mode"),
                format_hotkey(hk.toggle_hidden, lang),
                lang.tr("скрыть", "hide"),
                format_hotkey(hk.cycle_corner, lang),
                lang.tr("угол", "corner"),
            );
            let line2 = format!(
                "{} {}  {} {}  {} {}",
                format_hotkey(hk.toggle_settings, lang),
                lang.tr("настр.", "cfg"),
                format_hotkey(hk.show_help, lang),
                lang.tr("подсказка", "help"),
                format_hotkey(hk.quit_app, lang),
                lang.tr("выход", "quit"),
            );
            ui.label(RichText::new(line1).size(f_small).color(TEXT_MUTED));
            ui.label(RichText::new(line2).size(f_small).color(TEXT_MUTED));
        }

        header_bottom
    }

    fn render_header(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        scale: f32,
        f_medium: f32,
        f_small: f32,
    ) -> f32 {
        let lang = self.settings.lang;
        let colors = self.settings.theme.colors();
        let surface_alt_bg = self.settings.theme.surface_alt_bg();

        let resp = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = (6.0 * scale).round().max(3.0);

            let title_resp = ui.add(
                egui::Label::new(
                    RichText::new("FPScraft")
                        .strong()
                        .size(f_medium)
                        .color(TEXT_PRIMARY),
                )
                .sense(Sense::drag()),
            );
            if title_resp.drag_started() && !self.click_through {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }

            if !self.click_through {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = (6.0 * scale).round().max(3.0);

                    // Neutral close button (red ONLY on hover)
                    let btn_h = (16.0 * scale).round();
                    let (close_rect, close_resp) =
                        ui.allocate_exact_size(Vec2::new(btn_h, btn_h), Sense::click());
                    let close_col = if close_resp.hovered() {
                        SIGNAL_CRIT
                    } else {
                        TEXT_SECONDARY
                    };
                    ui.painter().text(
                        close_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "×",
                        FontId::monospace(f_medium),
                        close_col,
                    );
                    if close_resp
                        .on_hover_text(tooltip_with_hotkey(
                            lang.tr("Свернуть в трей", "Hide to tray"),
                            self.settings.hotkeys.toggle_hidden,
                        ))
                        .clicked()
                    {
                        self.apply_hidden_state(ctx, true);
                    }

                    // Settings gear button
                    let gear_resp = ui.add(
                        egui::Button::new(RichText::new("⚙").size(f_small + 1.5 * scale).color(
                            if self.settings.open {
                                colors.accent
                            } else {
                                TEXT_SECONDARY
                            },
                        ))
                        .frame(false)
                        .min_size(Vec2::new((18.0 * scale).round(), btn_h)),
                    );
                    if gear_resp
                        .on_hover_text(tooltip_with_hotkey(
                            lang.tr("Настройки", "Settings"),
                            self.settings.hotkeys.toggle_settings,
                        ))
                        .clicked()
                    {
                        self.settings.open = !self.settings.open;
                    }

                    // Segmented mode toggle: [Полный | Угол] / [Full | Corner]
                    if let Some(new_mode) = render_mode_segment(
                        ui,
                        self.settings.mode,
                        lang,
                        scale,
                        f_small,
                        colors.accent,
                        surface_alt_bg,
                        self.settings.theme.border_color(),
                    ) {
                        self.set_mode(ctx, new_mode);
                    }
                });
            }
        });

        resp.response.rect.max.y
    }

    fn render_corner(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snap: &MetricsSnapshot) {
        let lang = self.settings.lang;
        let thresholds = self.settings.thresholds.clone();
        let stats = self.settings.stats.clone();
        let theme = self.settings.theme.clone();
        let colors = theme.colors();
        let surface_alt_bg = theme.surface_alt_bg();
        let scale = theme.ui_scale;
        let f_val = 12.0 * scale;
        let f_lbl = FONT_SMALL * scale;
        let sep_h = (12.0 * scale).round();
        let gap = (5.0 * scale).round().max(3.0);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;

            let drag_area = ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let mut has_prev_group = false;

                // Group 1: FPS / 1% Low / Frametime
                let any_perf = stats.corner_show_fps
                    || stats.corner_show_1pct_low
                    || stats.corner_show_frametime
                    || !stats.has_any_corner_item();
                if any_perf {
                    if stats.corner_show_fps || !stats.has_any_corner_item() {
                        ui.label(
                            RichText::new(format!("{:.0}", snap.fps))
                                .strong()
                                .size(f_val)
                                .color(value_fps_color(snap.fps)),
                        );
                        ui.label(RichText::new("FPS").size(f_lbl).color(TEXT_MUTED));
                    }
                    if stats.corner_show_1pct_low {
                        ui.label(
                            RichText::new(format!("1% {:.0}", snap.fps_1pct_low))
                                .size(f_lbl)
                                .color(TEXT_SECONDARY),
                        );
                    }
                    if stats.corner_show_frametime {
                        ui.label(
                            RichText::new(format!(
                                "{:.1}{}",
                                snap.frametime_ms,
                                lang.tr("мс", "ms")
                            ))
                            .size(f_lbl)
                            .color(TEXT_SECONDARY),
                        );
                    }
                    has_prev_group = true;
                }

                // Group 2: CPU Temp / Usage
                if stats.corner_show_cpu_temp || stats.corner_show_cpu_usage {
                    if has_prev_group {
                        draw_vertical_sep(ui, colors.divider, sep_h);
                    }
                    ui.label(RichText::new("CPU").size(f_lbl).color(TEXT_MUTED));
                    if stats.corner_show_cpu_temp {
                        let (cpu_txt, cpu_col) = match snap.cpu_temp_c {
                            Some(t) => (
                                format!("{t:.0}°C"),
                                value_temp_color(t, thresholds.temp_warn_c, thresholds.temp_crit_c),
                            ),
                            None => (String::from("N/A"), TEXT_MUTED),
                        };
                        ui.label(RichText::new(cpu_txt).size(f_val).color(cpu_col));
                    }
                    if stats.corner_show_cpu_usage {
                        let col = value_load_color(
                            snap.cpu_usage,
                            thresholds.load_warn_pct,
                            thresholds.load_crit_pct,
                        );
                        ui.label(
                            RichText::new(format!("{:.0}%", snap.cpu_usage))
                                .size(f_val)
                                .color(col),
                        );
                    }
                    has_prev_group = true;
                }

                // Group 3: GPU Temp / Usage / VRAM
                if stats.corner_show_gpu_temp
                    || stats.corner_show_gpu_usage
                    || stats.corner_show_vram
                {
                    if has_prev_group {
                        draw_vertical_sep(ui, colors.divider, sep_h);
                    }
                    ui.label(RichText::new("GPU").size(f_lbl).color(TEXT_MUTED));
                    if stats.corner_show_gpu_temp {
                        let (gpu_txt, gpu_col) = match snap.gpu_temp_c {
                            Some(t) => (
                                format!("{t:.0}°C"),
                                value_temp_color(t, thresholds.temp_warn_c, thresholds.temp_crit_c),
                            ),
                            None => (String::from("N/A"), TEXT_MUTED),
                        };
                        ui.label(RichText::new(gpu_txt).size(f_val).color(gpu_col));
                    }
                    if stats.corner_show_gpu_usage {
                        let gpu_pct = snap.gpu_usage.unwrap_or(0.0);
                        let col = value_load_color(
                            gpu_pct,
                            thresholds.load_warn_pct,
                            thresholds.load_crit_pct,
                        );
                        ui.label(
                            RichText::new(format!("{gpu_pct:.0}%"))
                                .size(f_val)
                                .color(col),
                        );
                    }
                    if stats.corner_show_vram {
                        let vram_pct = if snap.vram_total_gb > 0.0 {
                            (snap.vram_used_gb / snap.vram_total_gb) * 100.0
                        } else {
                            0.0
                        };
                        let col = value_load_color(
                            vram_pct,
                            thresholds.load_warn_pct,
                            thresholds.load_crit_pct,
                        );
                        ui.label(
                            RichText::new(format!(
                                "VRAM {:.1}{}",
                                snap.vram_used_gb,
                                lang.tr("Г", "G")
                            ))
                            .size(f_lbl)
                            .color(if col == TEXT_PRIMARY {
                                TEXT_SECONDARY
                            } else {
                                col
                            }),
                        );
                    }
                    has_prev_group = true;
                }

                // Group 4: RAM
                if stats.corner_show_ram {
                    if has_prev_group {
                        draw_vertical_sep(ui, colors.divider, sep_h);
                    }
                    let ram_col = value_load_color(
                        snap.ram_usage_pct,
                        thresholds.load_warn_pct,
                        thresholds.load_crit_pct,
                    );
                    ui.label(
                        RichText::new(lang.tr("ОЗУ", "RAM"))
                            .size(f_lbl)
                            .color(TEXT_MUTED),
                    );
                    ui.label(
                        RichText::new(format!(
                            "{:.1}/{:.0}{}",
                            snap.ram_used_gb,
                            snap.ram_total_gb,
                            lang.tr("Г", "G")
                        ))
                        .size(f_val)
                        .color(ram_col),
                    );
                    has_prev_group = true;
                }

                // Group 5: Network
                if stats.corner_show_net {
                    if has_prev_group {
                        draw_vertical_sep(ui, colors.divider, sep_h);
                    }
                    ui.label(
                        RichText::new(format!(
                            "↓{} ↑{}",
                            format_net_speed(snap.net_down_kbps, lang),
                            format_net_speed(snap.net_up_kbps, lang)
                        ))
                        .size(f_lbl)
                        .color(TEXT_SECONDARY),
                    );
                }
            });

            if !self.click_through {
                let drag_resp = ui.interact(
                    drag_area.response.rect,
                    ui.id().with("corner_drag_area"),
                    Sense::drag(),
                );
                if drag_resp.drag_started() {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }

                draw_vertical_sep(ui, colors.divider, sep_h);

                let btn_h = (16.0 * scale).round();
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(lang.tr("Полн", "Full"))
                                .size(f_lbl)
                                .color(colors.accent),
                        )
                        .fill(surface_alt_bg)
                        .stroke(Stroke::new(BORDER_WIDTH, theme.border_color()))
                        .corner_radius(CornerRadius::same(2))
                        .min_size(Vec2::new((34.0 * scale).round(), btn_h)),
                    )
                    .on_hover_text(tooltip_with_hotkey(
                        lang.tr("Полный режим", "Full mode"),
                        self.settings.hotkeys.cycle_mode,
                    ))
                    .clicked()
                {
                    self.set_mode(ctx, OverlayMode::Full);
                }

                if ui
                    .add(
                        egui::Button::new(RichText::new("⚙").size(f_lbl + 1.0 * scale).color(
                            if self.settings.open {
                                colors.accent
                            } else {
                                TEXT_SECONDARY
                            },
                        ))
                        .frame(false)
                        .min_size(Vec2::new(btn_h, btn_h)),
                    )
                    .on_hover_text(tooltip_with_hotkey(
                        lang.tr("Настройки", "Settings"),
                        self.settings.hotkeys.toggle_settings,
                    ))
                    .clicked()
                {
                    self.settings.open = !self.settings.open;
                }
            }
        });
    }
}

fn tooltip_with_hotkey(label: &str, bind: Option<KeyBind>) -> String {
    match bind {
        Some(b) => format!("{label} ({})", b.label()),
        None => label.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_mode_segment(
    ui: &mut egui::Ui,
    current: OverlayMode,
    lang: Language,
    scale: f32,
    font_size: f32,
    accent: Color32,
    surface_alt: Color32,
    border_col: Color32,
) -> Option<OverlayMode> {
    let mut clicked_mode = None;
    Frame::NONE
        .stroke(Stroke::new(BORDER_WIDTH, border_col))
        .corner_radius(CornerRadius::same(2))
        .inner_margin(Margin::same(0))
        .show(ui, |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (mode, label) in [
                    (OverlayMode::Full, lang.tr("Полный", "Full")),
                    (OverlayMode::Corner, lang.tr("Угол", "Corner")),
                ] {
                    let active = current == mode;
                    let btn =
                        egui::Button::new(RichText::new(label).size(font_size).color(if active {
                            accent
                        } else {
                            TEXT_MUTED
                        }))
                        .fill(if active {
                            surface_alt
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(Stroke::NONE)
                        .corner_radius(CornerRadius::ZERO)
                        .min_size(Vec2::new((42.0 * scale).round(), (16.0 * scale).round()));

                    if ui.add(btn).clicked() && !active {
                        clicked_mode = Some(mode);
                    }
                }
            });
        });
    clicked_mode
}

fn draw_hairline_divider(ui: &mut egui::Ui, divider_col: Color32) {
    ui.add_space(2.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::ZERO, divider_col);
    ui.add_space(2.0);
}

fn draw_vertical_sep(ui: &mut egui::Ui, divider_col: Color32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, height), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::ZERO, divider_col);
}

fn draw_cpu_core_bars(
    ui: &mut egui::Ui,
    cores: &[f32],
    scale: f32,
    warn_pct: f32,
    crit_pct: f32,
    track_col: Color32,
) {
    let height = (10.0 * scale).round().max(6.0);
    let desired = Vec2::new(ui.available_width(), height);
    let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
    let painter = ui.painter();

    let count = cores.len() as f32;
    let gap = (2.0 * scale).max(1.0);
    let bar_w = ((rect.width() - gap * (count - 1.0)) / count).max(2.0);

    for (idx, &usage) in cores.iter().enumerate() {
        let x = rect.min.x + idx as f32 * (bar_w + gap);
        let bg = Rect::from_min_size(Pos2::new(x, rect.min.y), Vec2::new(bar_w, height));
        painter.rect_filled(bg, CornerRadius::ZERO, track_col);

        let frac = (usage / 100.0).clamp(0.05, 1.0);
        let fill_h = height * frac;
        let fg = Rect::from_min_size(Pos2::new(x, rect.max.y - fill_h), Vec2::new(bar_w, fill_h));
        let col = if usage >= crit_pct {
            SIGNAL_CRIT
        } else if usage >= warn_pct {
            SIGNAL_WARN
        } else {
            TEXT_SECONDARY
        };
        painter.rect_filled(fg, CornerRadius::ZERO, col);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_fps_graph(
    ui: &mut egui::Ui,
    history: &[f32],
    low_1pct_fps: f32,
    scale: f32,
    font_small: f32,
    grid_col: Color32,
    border_col: Color32,
    accent_col: Color32,
    corner_radius: u8,
) {
    let height = (54.0 * scale).round();
    let desired = Vec2::new(ui.available_width(), height);
    let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
    let painter = ui.painter();

    painter.rect_stroke(
        rect,
        CornerRadius::same(corner_radius),
        Stroke::new(BORDER_WIDTH, border_col),
        StrokeKind::Inside,
    );

    let peak_fps = history.iter().copied().fold(240.0_f32, f32::max);
    let y_max = (peak_fps * 1.1).max(260.0);
    let plot_left = rect.min.x + 28.0 * scale;
    let plot_right = rect.max.x - 4.0 * scale;
    let plot_top = rect.min.y + 4.0 * scale;
    let plot_bottom = rect.max.y - 4.0 * scale;
    let plot_h = (plot_bottom - plot_top).max(1.0);

    let y_for_fps = |fps: f32| -> f32 {
        let norm = (fps / y_max).clamp(0.0, 1.0);
        plot_bottom - norm * plot_h
    };

    // Grid lines + scale labels (60 / 144 / 240)
    for (ref_fps, label) in [(240.0, "240"), (144.0, "144"), (60.0, "60")] {
        let y = y_for_fps(ref_fps);
        painter.line_segment(
            [Pos2::new(plot_left, y), Pos2::new(plot_right, y)],
            Stroke::new(1.0, grid_col),
        );
        painter.text(
            Pos2::new(plot_left - 3.0, y),
            egui::Align2::RIGHT_CENTER,
            label,
            FontId::monospace(font_small),
            TEXT_MUTED,
        );
    }

    // 1% Low threshold line
    if low_1pct_fps > 0.0 {
        let low_y = y_for_fps(low_1pct_fps);
        let warn_muted =
            Color32::from_rgba_unmultiplied(SIGNAL_WARN.r(), SIGNAL_WARN.g(), SIGNAL_WARN.b(), 120);
        painter.line_segment(
            [Pos2::new(plot_left, low_y), Pos2::new(plot_right, low_y)],
            Stroke::new(1.0, warn_muted),
        );
    }

    // Thin FPS polyline (no area fill, no glow)
    if history.len() < 2 {
        return;
    }

    let step_x = (plot_right - plot_left) / (history.len() - 1) as f32;
    let points: Vec<Pos2> = history
        .iter()
        .enumerate()
        .map(|(i, &fps)| Pos2::new(plot_left + i as f32 * step_x, y_for_fps(fps)))
        .collect();

    painter.add(egui::Shape::line(
        points,
        Stroke::new(GRAPH_LINE_WIDTH, accent_col),
    ));
}

fn format_net_speed(kbps: f32, lang: Language) -> String {
    if kbps >= 1024.0 {
        format!("{:.1} {}", kbps / 1024.0, lang.tr("МБ/с", "MB/s"))
    } else {
        format!("{:.0} {}", kbps, lang.tr("КБ/с", "KB/s"))
    }
}

fn format_uptime(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn install_tabular_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let local_fonts = std::env::var("LOCALAPPDATA")
        .map(|d| {
            PathBuf::from(d)
                .join("Microsoft")
                .join("Windows")
                .join("Fonts")
        })
        .ok();

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(ref local) = local_fonts {
        candidates.push(local.join("JetBrainsMono-Regular.ttf"));
        candidates.push(local.join("IBMPlexMono-Regular.ttf"));
    }
    candidates.push(PathBuf::from(r"C:\Windows\Fonts\JetBrainsMono-Regular.ttf"));
    candidates.push(PathBuf::from(r"C:\Windows\Fonts\IBMPlexMono-Regular.ttf"));
    candidates.push(PathBuf::from(r"C:\Windows\Fonts\CascadiaMono.ttf"));
    candidates.push(PathBuf::from(r"C:\Windows\Fonts\consola.ttf"));

    if let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) {
        fonts.font_data.insert(
            "tabular_mono".to_owned(),
            FontData::from_owned(bytes).into(),
        );
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "tabular_mono".to_owned());
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, "tabular_mono".to_owned());
    }

    if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
        fonts
            .font_data
            .insert("ui_fallback".to_owned(), FontData::from_owned(bytes).into());
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .push("ui_fallback".to_owned());
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push("ui_fallback".to_owned());
    }

    ctx.set_fonts(fonts);
}

fn configure_visuals(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = Color32::TRANSPARENT;
    visuals.panel_fill = Color32::TRANSPARENT;
    visuals.override_text_color = Some(TEXT_PRIMARY);
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(56, 189, 248, 60);
    visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(56, 189, 248));
    ctx.set_visuals(visuals);
}

fn extract_win32_hwnd(source: &impl HasWindowHandle) -> isize {
    let Ok(wh) = source.window_handle() else {
        return 0;
    };
    match wh.as_raw() {
        RawWindowHandle::Win32(h) => h.hwnd.get(),
        _ => 0,
    }
}

fn spawn_unhide_watcher(ctx: egui::Context, state: Arc<Mutex<HiddenWatcherState>>) {
    std::thread::spawn(move || {
        let mut poller = HotkeyPoller::default();
        loop {
            std::thread::sleep(Duration::from_millis(25));
            let (hwnd, hidden, wait_for_release, bind) = match state.lock() {
                Ok(st) => (st.hwnd, st.hidden, st.wait_for_release, st.toggle_hidden),
                Err(_) => break,
            };

            if !hidden || hwnd == 0 {
                continue;
            }

            // Self-healing guard: anything that makes the window visible again while it is
            // supposed to be hidden would leave a black, non-interactive rectangle on screen.
            // SAFETY: hwnd is a valid window handle for FPScraft Overlay.
            if unsafe { IsWindowVisible(hwnd as HWND) } != 0 {
                unsafe {
                    ShowWindow(hwnd as HWND, SW_HIDE);
                }
            }

            let Some(key) = bind else {
                continue;
            };

            if wait_for_release {
                if !HotkeyPoller::is_combo_down(key) {
                    if let Ok(mut st) = state.lock() {
                        st.wait_for_release = false;
                    }
                    let _ = poller.is_edge_pressed(Some(key));
                }
                continue;
            }

            if poller.is_edge_pressed(Some(key)) {
                if let Ok(mut st) = state.lock() {
                    st.hidden = false;
                }
                // A hidden window receives no paint events, so it has to be shown here before
                // the UI thread can repaint it. The mouse passthrough state is restored in
                // `OverlayApp::update` once the fresh frame is on screen.
                // SAFETY: hwnd is a valid window handle for FPScraft Overlay.
                unsafe {
                    ShowWindow(hwnd as HWND, SW_SHOWNA);
                }
                ctx.request_repaint();
            }
        }
    });
}

/// Computes the Win32 `LWA_ALPHA` byte for the overlay window.
/// At ~0% background opacity, `panel_bg()` is already `TRANSPARENT`, so we keep
/// the window alpha at 255 to preserve crisp HUD text.
fn overlay_window_alpha(bg_opacity: f32) -> u8 {
    if bg_opacity <= 0.005 {
        255
    } else {
        (bg_opacity.clamp(0.15, 1.0) * 255.0).round() as u8
    }
}

#[cfg(test)]
mod tests {
    use super::overlay_window_alpha;

    #[test]
    fn overlay_window_alpha_handles_hud_zero_and_clamped_range() {
        assert_eq!(overlay_window_alpha(0.0), 255);
        assert_eq!(overlay_window_alpha(0.05), 38);
        assert_eq!(overlay_window_alpha(0.5), 128);
        assert_eq!(overlay_window_alpha(1.0), 255);
    }
}
