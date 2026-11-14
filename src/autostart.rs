use std::env;
use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const RUN_KEY_PATH: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE_NAME: &str = "FPScraft";

/// Returns true when the current user's Run key contains the FPScraft entry.
pub fn is_enabled() -> bool {
    Command::new("reg")
        .args(["query", RUN_KEY_PATH, "/v", RUN_VALUE_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Adds or removes the current executable from the per-user Windows autostart list.
pub fn set_enabled(enable: bool) -> Result<(), String> {
    if enable {
        let exe = env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        let quoted = format!("\"{}\"", exe.display());
        let out = Command::new("reg")
            .args([
                "add",
                RUN_KEY_PATH,
                "/v",
                RUN_VALUE_NAME,
                "/t",
                "REG_SZ",
                "/d",
                &quoted,
                "/f",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("reg add: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(())
    } else {
        if !is_enabled() {
            return Ok(());
        }
        let out = Command::new("reg")
            .args(["delete", RUN_KEY_PATH, "/v", RUN_VALUE_NAME, "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("reg delete: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(())
    }
}
