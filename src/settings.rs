//! Application preferences shared by the GIS and Civiel workbenches.

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

#[cfg(test)]
thread_local! {
    static TEST_FILE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn path() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_FILE.with(|file| file.borrow().clone()) {
        return path;
    }
    crate::gis::servers::config_dir().join("settings.json")
}

fn lock_imports(settings: &Value) -> bool {
    settings["lock_imported_layers"].as_bool().unwrap_or(true)
}

pub(crate) fn lock_imported_layers() -> bool {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .is_none_or(|settings| lock_imports(&settings))
}

pub(crate) fn save_lock_imported_layers(on: bool) -> Result<(), String> {
    update(&path(), |settings| {
        settings.insert("lock_imported_layers".into(), on.into());
    })
}

pub(crate) fn update(path: &Path, edit: impl FnOnce(&mut Map<String, Value>)) -> Result<(), String> {
    let mut settings = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<Map<String, Value>>(&text).map_err(|e| e.to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(e) => return Err(e.to_string()),
    };
    edit(&mut settings);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Import workers can read the file while the UI saves a preference.
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let json = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
    let result = std::fs::write(&temporary, json).and_then(|()| std::fs::rename(&temporary, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e| e.to_string())
}

#[cfg(test)]
pub(crate) fn with_test_file(test: impl FnOnce()) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ois-import-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            TEST_FILE.with(|file| file.replace(None));
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    TEST_FILE.with(|file| file.replace(Some(dir.join("settings.json"))));
    test();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_locks_default_on_for_existing_preferences() {
        assert!(lock_imports(&serde_json::json!({"language":"nl","region":"NL"})));
        assert!(lock_imports(&serde_json::json!({"lock_imported_layers":true})));
        assert!(!lock_imports(&serde_json::json!({"lock_imported_layers":false})));
    }
}
