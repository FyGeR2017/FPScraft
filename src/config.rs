use std::os::windows::process::CommandExt;
use std::process::Command;

use crate::theme::Language;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const SETTINGS_KEY: &str = r"HKCU\Software\FPScraft";
const LANGUAGE_VALUE: &str = "Language";

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
    match value.to_ascii_lowercase().as_str() {
        "ru" | "russian" => Some(Language::Ru),
        "en" | "english" => Some(Language::En),
        _ => None,
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
}
