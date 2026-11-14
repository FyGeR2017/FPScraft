use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::theme::Language;

#[derive(Debug, Clone, Copy)]
pub enum TrayAction {
    ToggleOverlay,
    OpenSettings,
    ToggleAutostart(bool),
    Quit,
}

/// Owns the system tray icon and its native context menu.
pub struct Tray {
    icon: TrayIcon,
    toggle_item: MenuItem,
    settings_item: MenuItem,
    autostart_item: CheckMenuItem,
    quit_item: MenuItem,
    lang: Language,
    hidden: bool,
    autostart: bool,
}

impl Tray {
    pub fn new(lang: Language, hidden: bool, autostart: bool) -> Option<Self> {
        let menu = Menu::new();

        let toggle_item = MenuItem::new(toggle_label(lang, hidden), true, None);
        let settings_item = MenuItem::new(settings_label(lang), true, None);
        let autostart_item = CheckMenuItem::new(autostart_label(lang), true, autostart, None);
        let quit_item = MenuItem::new(quit_label(lang), true, None);

        menu.append_items(&[
            &toggle_item,
            &settings_item,
            &PredefinedMenuItem::separator(),
            &autostart_item,
            &PredefinedMenuItem::separator(),
            &quit_item,
        ])
        .ok()?;

        let icon = TrayIconBuilder::new()
            .with_id("fpscraft")
            .with_icon(make_icon()?)
            .with_tooltip(tooltip(lang))
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .ok()?;

        Some(Self {
            icon,
            toggle_item,
            settings_item,
            autostart_item,
            quit_item,
            lang,
            hidden,
            autostart,
        })
    }

    /// Applies tray labels and the autostart check state without rebuilding the menu.
    pub fn sync(&mut self, lang: Language, hidden: bool, autostart: bool) {
        if self.lang != lang {
            self.lang = lang;
            let _ = self.icon.set_tooltip(Some(tooltip(lang)));
            self.settings_item.set_text(settings_label(lang));
            self.quit_item.set_text(quit_label(lang));
            self.autostart_item.set_text(autostart_label(lang));
        }
        if self.hidden != hidden {
            self.hidden = hidden;
            self.toggle_item.set_text(toggle_label(lang, hidden));
        }
        if self.autostart != autostart {
            self.autostart = autostart;
            self.autostart_item.set_checked(autostart);
        }
    }

    /// Drains pending native tray/menu events and maps them to app actions.
    pub fn poll(&mut self) -> Option<TrayAction> {
        let mut action = None;

        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == *self.toggle_item.id() {
                action = Some(TrayAction::ToggleOverlay);
            } else if event.id == *self.settings_item.id() {
                action = Some(TrayAction::OpenSettings);
            } else if event.id == *self.autostart_item.id() {
                action = Some(TrayAction::ToggleAutostart(
                    self.autostart_item.is_checked(),
                ));
            } else if event.id == *self.quit_item.id() {
                action = Some(TrayAction::Quit);
            }
        }

        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                action = Some(TrayAction::ToggleOverlay);
            }
        }

        action
    }
}

fn toggle_label(lang: Language, hidden: bool) -> &'static str {
    match (lang, hidden) {
        (Language::Ru, false) => "Скрыть оверлей",
        (Language::Ru, true) => "Показать оверлей",
        (Language::En, false) => "Hide overlay",
        (Language::En, true) => "Show overlay",
    }
}

fn settings_label(lang: Language) -> &'static str {
    lang.tr("Настройки", "Settings")
}

fn autostart_label(lang: Language) -> &'static str {
    lang.tr("Запускать с Windows", "Run at Windows startup")
}

fn quit_label(lang: Language) -> &'static str {
    lang.tr("Выход", "Exit")
}

fn tooltip(lang: Language) -> &'static str {
    lang.tr("FPScraft — телеметрия", "FPScraft — telemetry")
}

fn make_icon() -> Option<Icon> {
    const SIZE: u32 = 32;
    const BG: [u8; 3] = [10, 12, 17];
    const ACCENT: [u8; 3] = [56, 189, 248];
    const GLYPH: [u8; 3] = [236, 239, 244];

    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];

    for y in 0..SIZE {
        for x in 0..SIZE {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;

            let mut color = [0u8, 0, 0, 0];
            if rounded_rect(px, py, 1.0, 6.0) {
                color = if rounded_rect(px, py, 2.2, 5.2) {
                    [BG[0], BG[1], BG[2], 255]
                } else {
                    [ACCENT[0], ACCENT[1], ACCENT[2], 255]
                };
            }

            let in_glyph = ((9.0..=13.0).contains(&px) && (8.0..=24.0).contains(&py))
                || ((9.0..=23.0).contains(&px) && (8.0..=11.5).contains(&py))
                || ((9.0..=20.0).contains(&px) && (14.5..=18.0).contains(&py));
            if in_glyph {
                color = [GLYPH[0], GLYPH[1], GLYPH[2], 255];
            }

            let idx = ((y * SIZE + x) * 4) as usize;
            rgba[idx..idx + 4].copy_from_slice(&color);
        }
    }

    Icon::from_rgba(rgba, SIZE, SIZE).ok()
}

fn rounded_rect(x: f32, y: f32, inset: f32, radius: f32) -> bool {
    let min = inset;
    let max = 32.0 - inset;
    if x < min || x > max || y < min || y > max {
        return false;
    }
    let cx = x.clamp(min + radius, max - radius);
    let cy = y.clamp(min + radius, max - radius);
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= radius * radius
}
