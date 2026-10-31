//! Interface language: Dutch or English.
//!
//! Open Infra started as a Dutch tool; it is used worldwide, so new parts of
//! the interface come in both languages. The choice is `language` in
//! `<config>/OpenCADStudio/open-infra/settings.json` ("nl" or "en"); without
//! it the system locale decides (`nl*` → Dutch, anything else → English).

use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Nl,
    En,
}

fn settings_file() -> std::path::PathBuf {
    crate::gis::servers::config_dir().join("settings.json")
}

fn from_settings() -> Option<Lang> {
    let text = std::fs::read_to_string(settings_file()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    parse(v.get("language")?.as_str()?)
}

fn parse(code: &str) -> Option<Lang> {
    let c = code.trim().to_ascii_lowercase();
    if c.starts_with("nl") {
        Some(Lang::Nl)
    } else if c.starts_with("en") {
        Some(Lang::En)
    } else {
        None
    }
}

fn from_locale() -> Lang {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                return if v.to_ascii_lowercase().starts_with("nl") { Lang::Nl } else { Lang::En };
            }
        }
    }
    // Windows has no LANG; its user locale name sits in the registry, which a
    // plugin should not read. English is the safe default there.
    Lang::En
}

/// Open CAD Studio's interface language; `None` for "system" or unknown.
fn from_host() -> Option<Option<Lang>> {
    let path = crate::gis::servers::config_dir().parent()?.join("settings.json");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let code = v.get("language")?.as_str()?;
    Some(if code.eq_ignore_ascii_case("system") { None } else { Some(if code.to_ascii_lowercase().starts_with("nl") { Lang::Nl } else { Lang::En }) })
}

/// The interface language, decided once per session.
pub fn current() -> Lang {
    static L: OnceLock<Lang> = OnceLock::new();
    *L.get_or_init(|| from_settings().or_else(|| from_host().flatten()).unwrap_or_else(from_locale))
}

/// Store the language for the next session (the ribbon is built at load).
pub fn save(lang: Lang) -> Result<(), String> {
    let path = settings_file();
    let mut v: serde_json::Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({}));
    v["language"] = serde_json::Value::String(if lang == Lang::Nl { "nl" } else { "en" }.into());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Compatibility check for the application's regional ribbon profile.
/// The drawing's project region is selected separately in the workbench.
pub fn dutch() -> bool {
    crate::localisation::current() == crate::localisation::Region::Netherlands
}

/// Compatibility entry point; new interfaces select an explicit regional profile.
pub fn save_dutch(on: bool) -> Result<(), String> {
    crate::localisation::save(if on {crate::localisation::Region::Netherlands} else {crate::localisation::Region::International})
}

/// Pick the text for the current language.
pub fn tr<'a>(nl: &'a str, en: &'a str) -> &'a str {
    match current() {
        Lang::Nl => nl,
        Lang::En => en,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes() {
        assert_eq!(parse("nl_NL.UTF-8"), Some(Lang::Nl));
        assert_eq!(parse("EN"), Some(Lang::En));
        assert_eq!(parse("de_DE"), None);
    }
}
