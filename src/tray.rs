use std::cell::RefCell;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{mpsc, Arc};

use eframe::egui;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, PostMessageW, RegisterClassW, HWND_MESSAGE,
    WM_APP, WNDCLASSW,
};

use crate::theme::Language;

const WM_APP_TRAY_WAKE: u32 = WM_APP + 42;

thread_local! {
    static TRAY_WAKE_CALLBACK: RefCell<Option<Box<dyn FnMut()>>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, Copy)]
pub enum TrayAction {
    ToggleOverlay,
    OpenSettings,
    ToggleAutostart(bool),
    RestartAsAdmin,
    Quit,
}

/// Owns the system tray icon and its native context menu.
pub struct Tray {
    icon: TrayIcon,
    toggle_item: MenuItem,
    settings_item: MenuItem,
    admin_item: MenuItem,
    autostart_item: CheckMenuItem,
    quit_item: MenuItem,
    menu_rx: mpsc::Receiver<MenuEvent>,
    icon_rx: mpsc::Receiver<TrayIconEvent>,
    quit_requested: Arc<AtomicBool>,
    msg_hwnd: HWND,
    wake_hwnd: Arc<AtomicIsize>,
    lang: Language,
    hidden: bool,
    autostart: bool,
    admin_needed: bool,
}

impl Tray {
    pub fn new(ctx: egui::Context, lang: Language, hidden: bool, autostart: bool) -> Option<Self> {
        let menu = Menu::new();

        let toggle_item = MenuItem::new(toggle_label(lang, hidden), true, None);
        let settings_item = MenuItem::new(settings_label(lang), true, None);
        let admin_item = MenuItem::new(admin_label(lang), false, None);
        let autostart_item = CheckMenuItem::new(autostart_label(lang), true, autostart, None);
        let quit_item = MenuItem::new(quit_label(lang), true, None);

        menu.append_items(&[
            &toggle_item,
            &settings_item,
            &admin_item,
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

        let msg_hwnd = create_tray_wake_window();
        let (menu_tx, menu_rx) = mpsc::channel();
        let (icon_tx, icon_rx) = mpsc::channel();
        let quit_requested = Arc::new(AtomicBool::new(false));
        let wake_hwnd = Arc::new(AtomicIsize::new(msg_hwnd as isize));

        let quit_id = quit_item.id().clone();
        let quit_flag = Arc::clone(&quit_requested);
        let menu_ctx = ctx.clone();
        let menu_wake_hwnd = Arc::clone(&wake_hwnd);
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == quit_id {
                quit_flag.store(true, Ordering::Release);
            }
            let _ = menu_tx.send(event);
            wake_ui(&menu_ctx, &menu_wake_hwnd);
        }));

        let icon_ctx = ctx;
        let icon_wake_hwnd = Arc::clone(&wake_hwnd);
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            let _ = icon_tx.send(event);
            wake_ui(&icon_ctx, &icon_wake_hwnd);
        }));

        Some(Self {
            icon,
            toggle_item,
            settings_item,
            admin_item,
            autostart_item,
            quit_item,
            menu_rx,
            icon_rx,
            quit_requested,
            msg_hwnd,
            wake_hwnd,
            lang,
            hidden,
            autostart,
            admin_needed: false,
        })
    }

    pub fn set_wake_callback<F: FnMut() + 'static>(cb: F) {
        TRAY_WAKE_CALLBACK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(cb));
        });
    }

    pub fn is_quit_requested(&self) -> bool {
        self.quit_requested.load(Ordering::Acquire)
    }

    /// Applies tray labels and the autostart check state without rebuilding the menu.
    pub fn sync(&mut self, lang: Language, hidden: bool, autostart: bool, admin_needed: bool) {
        if self.lang != lang {
            self.lang = lang;
            let _ = self.icon.set_tooltip(Some(tooltip(lang)));
            self.settings_item.set_text(settings_label(lang));
            self.admin_item.set_text(admin_label(lang));
            self.quit_item.set_text(quit_label(lang));
            self.autostart_item.set_text(autostart_label(lang));
            self.toggle_item.set_text(toggle_label(lang, hidden));
        }
        if self.hidden != hidden {
            self.hidden = hidden;
            self.toggle_item.set_text(toggle_label(lang, hidden));
        }
        if self.autostart != autostart {
            self.autostart = autostart;
            self.autostart_item.set_checked(autostart);
        }
        if self.admin_needed != admin_needed {
            self.admin_needed = admin_needed;
            self.admin_item.set_enabled(admin_needed);
        }
    }

    /// Drains pending native tray/menu events and maps them to app actions.
    pub fn poll(&mut self) -> Vec<TrayAction> {
        let mut actions = Vec::new();

        while let Ok(event) = self.menu_rx.try_recv() {
            if event.id == *self.quit_item.id() {
                actions.push(TrayAction::Quit);
            } else if event.id == *self.toggle_item.id() {
                actions.push(TrayAction::ToggleOverlay);
            } else if event.id == *self.settings_item.id() {
                actions.push(TrayAction::OpenSettings);
            } else if event.id == *self.admin_item.id() {
                actions.push(TrayAction::RestartAsAdmin);
            } else if event.id == *self.autostart_item.id() {
                actions.push(TrayAction::ToggleAutostart(
                    self.autostart_item.is_checked(),
                ));
            }
        }

        while let Ok(event) = self.icon_rx.try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                actions.push(TrayAction::ToggleOverlay);
            }
        }

        actions
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.wake_hwnd.store(0, Ordering::Relaxed);
        if !self.msg_hwnd.is_null() {
            // SAFETY: `self.msg_hwnd` was created by `create_tray_wake_window` on this thread.
            unsafe {
                let _ = DestroyWindow(self.msg_hwnd);
            }
            self.msg_hwnd = ptr::null_mut();
        }
    }
}

fn wake_ui(ctx: &egui::Context, wake_hwnd: &AtomicIsize) {
    ctx.request_repaint();
    let raw = wake_hwnd.load(Ordering::Relaxed);
    if raw != 0 {
        // SAFETY: `raw` is our dedicated HWND_MESSAGE window on the UI thread; posting
        // WM_APP_TRAY_WAKE wakes the UI message pump immediately even when the overlay is SW_HIDE.
        unsafe {
            let _ = PostMessageW(raw as HWND, WM_APP_TRAY_WAKE, 0, 0);
        }
    }
}

fn create_tray_wake_window() -> HWND {
    let class_name: Vec<u16> = "FPScraftTrayWakeWnd\0".encode_utf16().collect();
    // SAFETY: Registers a minimal message-only window class on the UI thread and creates
    // an HWND_MESSAGE window for immediate tray dispatch.
    unsafe {
        let hinstance = GetModuleHandleW(ptr::null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(tray_wake_wnd_proc),
            hInstance: hinstance,
            lpszClassName: class_name.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            ptr::null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            ptr::null_mut(),
            hinstance,
            ptr::null(),
        )
    }
}

unsafe extern "system" fn tray_wake_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_APP_TRAY_WAKE {
        TRAY_WAKE_CALLBACK.with(|slot| {
            if let Some(cb) = slot.borrow_mut().as_mut() {
                cb();
            }
        });
        return 0;
    }
    // SAFETY: Standard Win32 default message processing.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
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

fn admin_label(lang: Language) -> &'static str {
    lang.tr(
        "Перезапустить от администратора (для FPS)",
        "Restart as Administrator (for FPS)",
    )
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
