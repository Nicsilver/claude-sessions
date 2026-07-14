//! Reads the shared runtime state (~/.claude/session-status/) into the session list the widget
//! draws. Mirrors StateReader.cs + the sort in MainWindow.Refresh.

use crate::paths::*;
use crate::platform;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Sess {
    pub id: String,
    pub topic: String, // display name (custom rename > tab_title > topic)
    pub state: String, // needs | yourturn | working | idle | done | ...
    pub updated: f64,
    pub message: String,
    pub terminal: String,
    pub term_pid: i64,
    pub pid: i64,
    pub tab_title: String,
    pub mute_until: f64,
}

impl Sess {
    pub fn muted(&self, now: f64) -> bool {
        self.mute_until > now
    }
}

pub fn now() -> f64 {
    unix_now()
}

/// Load all live sessions, sorted the way the widget shows them.
pub fn load() -> Vec<Sess> {
    let mutes = load_f64_map(&mutes_path());
    let labels = load_str_map(&labels_path());
    let mut out = Vec::new();

    let Ok(entries) = std::fs::read_dir(state_dir()) else {
        return out;
    };
    for e in entries.flatten() {
        if e.path().extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        let Ok(root) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else {
            continue; // partial write mid-refresh
        };

        let pid = i64_of(&root, "pid");
        if pid > 0 && !platform::is_alive(pid) {
            continue; // the session's Claude process is gone
        }
        let terminal = str_of(&root, "terminal");
        let term_pid = i64_of(&root, "term_pid");
        if terminal.is_empty() && term_pid == 0 {
            continue; // nothing focusable (headless / IDE) — hidden, matching the mac surface
        }

        let id = str_of(&root, "session_id");
        let tab_title = str_of(&root, "tab_title");
        let topic = str_of(&root, "topic");
        let display = labels
            .get(&id)
            .filter(|s| !s.is_empty())
            .cloned()
            .or_else(|| if tab_title.is_empty() { None } else { Some(tab_title.clone()) })
            .unwrap_or_else(|| if topic.is_empty() { "?".into() } else { topic.clone() });

        out.push(Sess {
            id: id.clone(),
            topic: display,
            state: {
                let s = str_of(&root, "state");
                if s.is_empty() { "?".into() } else { s }
            },
            updated: f64_of(&root, "updated_at"),
            message: str_of(&root, "message"),
            terminal,
            term_pid,
            pid,
            tab_title,
            mute_until: mutes.get(&id).copied().unwrap_or(0.0),
        });
    }

    let n = now();
    out.sort_by(|a, b| {
        let (am, bm) = (a.muted(n), b.muted(n));
        if am != bm {
            return if am { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Less };
        }
        let (oa, ob) = (state_order(&a.state), state_order(&b.state));
        if oa != ob {
            return oa.cmp(&ob);
        }
        b.updated.partial_cmp(&a.updated).unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

pub fn state_order(state: &str) -> i32 {
    match state {
        "needs" => 0,
        "yourturn" => 1,
        "working" => 2,
        "idle" => 3,
        _ => 4,
    }
}

/// Age string like the mac/Windows surfaces: "12s", "3m", "2h". Empty if no timestamp.
pub fn age_str(ts: f64) -> String {
    if ts <= 0.0 {
        return String::new();
    }
    let s = (now() - ts) as i64;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else {
        format!("{}h", s / 3600)
    }
}

// ---- mutations (mute / rename), used by the widget ----

pub fn toggle_mute(id: &str) {
    if id.is_empty() {
        return;
    }
    let n = now();
    let mut m = load_f64_map(&mutes_path());
    m.retain(|_, &mut v| v > n); // drop expired
    match m.get(id) {
        Some(&until) if until > n => {
            m.remove(id);
        }
        _ => {
            m.insert(id.to_string(), n + 3600.0); // snooze 1h
        }
    }
    save_map(&mutes_path(), &m);
}

pub fn set_label(id: &str, name: &str) {
    if id.is_empty() {
        return;
    }
    let mut m = load_str_map(&labels_path());
    let n = name.trim();
    if n.is_empty() {
        m.remove(id);
    } else {
        m.insert(id.to_string(), n.to_string());
    }
    save_map(&labels_path(), &m);
}

// ---- small map io ----

fn load_f64_map(path: &std::path::Path) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    if let Value::Object(m) = load_json(path) {
        for (k, v) in m {
            if let Some(n) = v.as_f64() {
                out.insert(k, n);
            }
        }
    }
    out
}

fn load_str_map(path: &std::path::Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Value::Object(m) = load_json(path) {
        for (k, v) in m {
            if let Some(s) = v.as_str() {
                out.insert(k, s.to_string());
            }
        }
    }
    out
}

fn save_map<T: serde::Serialize>(path: &std::path::Path, m: &HashMap<String, T>) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(m) {
        let _ = std::fs::write(path, text);
    }
}
