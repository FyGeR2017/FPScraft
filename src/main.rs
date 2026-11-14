#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod config;
mod fps_etw;
mod metrics;
mod settings;
mod theme;
mod tray;

use eframe::egui;

use app::OverlayApp;
use theme::{FULL_WIDTH, SCREEN_MARGIN};

fn main() -> eframe::Result {
    if !acquire_single_instance() {
        return Ok(());
    }

    let viewport = egui::ViewportBuilder::default()
        .with_title("FPScraft Overlay")
        .with_inner_size([FULL_WIDTH, 340.0])
        .with_min_inner_size([60.0, 20.0])
        .with_position([SCREEN_MARGIN, SCREEN_MARGIN])
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_resizable(false)
        .with_taskbar(false);

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "FPScraft Overlay",
        options,
        Box::new(|cc| Ok(Box::new(OverlayApp::new(cc)))),
    )
}

/// Keeps a second launch from creating a duplicate tray icon / overlay.
fn acquire_single_instance() -> bool {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let name: Vec<u16> = "FpscraftOverlayMutex\0".encode_utf16().collect();
    // SAFETY: the name is a valid NUL-terminated UTF-16 string; a null security
    // descriptor means the default per-user security attributes.
    let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
    if handle.is_null() {
        return true;
    }
    // SAFETY: GetLastError reads the calling thread's last-error value.
    unsafe { GetLastError() != ERROR_ALREADY_EXISTS }
}
