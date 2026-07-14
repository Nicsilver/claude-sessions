//! Shared bits for the `record` and `dashboard` binaries — the small surface that
//! both the recorder (bin/record.py) and the terminal dashboard (bin/dashboard.py)
//! had in common: locating the shared runtime data, liveness checks, age formatting,
//! and the state-ordering used to sort sessions.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// `$HOME` (falls back to `.` so we never panic).
pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `~/.claude/session-status` — the shared runtime data dir (state/, focus-request.json …).
pub fn base_dir() -> PathBuf {
    home().join(".claude").join("session-status")
}

/// `~/.claude/session-status/state` — one JSON file per session.
pub fn state_dir() -> PathBuf {
    base_dir().join("state")
}

/// Seconds since the epoch as a float — mirrors Python's `time.time()`.
pub fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Sort weight for a state (lower = more urgent). Matches the `ORDER` map in both
/// Python scripts and `StateReader.kt`; unknown states sort last (9).
pub fn order(state: &str) -> i32 {
    match state {
        "needs" => 0,
        "yourturn" => 1,
        "working" => 2,
        "idle" => 3,
        "done" => 4,
        _ => 9,
    }
}

/// Human age of a timestamp: `""` for 0, else `Ns` / `Nm` / `Nh`.
/// Mirrors `age()` in dashboard.py and SessionsToolWindowFactory.kt.
pub fn age(ts: f64) -> String {
    if ts <= 0.0 {
        return String::new();
    }
    let s = (now_secs() - ts) as i64;
    if s < 60 {
        format!("{}s", s)
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else {
        format!("{}h", s / 3600)
    }
}

/// Is `pid` still alive? Mirrors the `os.kill(pid, 0)` probe in dashboard.py:
/// a falsy/non-positive pid is treated as alive, ESRCH means gone, EPERM (exists
/// but not ours) and anything else are treated as alive (fail-safe — never hide a
/// session we're unsure about).
pub fn alive(pid: i64) -> bool {
    if pid <= 0 {
        return true;
    }
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if r == 0 {
        return true;
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(e) if e == libc::ESRCH => false,
        Some(e) if e == libc::EPERM => true,
        _ => true,
    }
}
