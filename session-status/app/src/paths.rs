//! Shared filesystem locations and small JSON helpers.

use serde_json::Value;
use std::path::PathBuf;

pub fn home() -> PathBuf {
    #[cfg(windows)]
    let v = std::env::var("USERPROFILE");
    #[cfg(not(windows))]
    let v = std::env::var("HOME");
    PathBuf::from(v.unwrap_or_default())
}

pub fn base() -> PathBuf { home().join(".claude").join("session-status") }
pub fn state_dir() -> PathBuf { base().join("state") }
pub fn labels_path() -> PathBuf { base().join("labels.json") }
pub fn mutes_path() -> PathBuf { base().join("mutes.json") }
pub fn request_path() -> PathBuf { base().join("focus-request.json") }
#[cfg(unix)]
pub fn tab_names_dir() -> PathBuf { base().join("tab-names") }
pub fn config_path() -> PathBuf { base().join("config.json") }
pub fn sessions_dir() -> PathBuf { home().join(".claude").join("sessions") }
pub fn settings_path() -> PathBuf { home().join(".claude").join("settings.json") }

pub fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn str_of(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}
pub fn i64_of(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}
pub fn f64_of(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

/// Load a JSON file, tolerating a UTF-8 BOM; returns Null on any failure.
pub fn load_json(path: &std::path::Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(s.trim_start_matches('\u{feff}')).ok())
        .unwrap_or(Value::Null)
}
