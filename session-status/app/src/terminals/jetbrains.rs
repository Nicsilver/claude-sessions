//! JetBrains terminals: the widget doesn't touch the IDE itself — it writes
//! ~/.claude/session-status/focus-request.json and the IntelliJ plugin (FocusWatcher.kt)
//! acts on it. Wire format matches the Swift/C# surfaces exactly. Works on every OS the
//! IDE runs on; note the plugin currently matches sessions by tty, so focus/close only
//! resolve on macOS today (Windows sessions have no tty — a pid match in the plugin is
//! the known follow-up).

use super::Terminal;
use crate::model::Sess;
use serde_json::json;

pub struct JetBrains;

fn write_request(tty: &str, pid: i64, term_pid: i64, action: &str) -> bool {
    let path = crate::paths::request_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let payload = json!({
        "tty": tty,
        "pid": pid,
        "terminal": "jetbrains",
        "term_pid": term_pid,
        "ts": crate::paths::unix_now(),
        "action": action,
    });
    std::fs::write(&path, payload.to_string()).is_ok()
}

impl Terminal for JetBrains {
    fn id(&self) -> &'static str {
        "jetbrains"
    }

    fn label(&self) -> &'static str {
        "JetBrains IDE"
    }

    fn focus(&self, s: &Sess) -> bool {
        write_request(&s.tty, s.pid, s.term_pid, "focus")
    }

    fn close(&self, s: &Sess) -> bool {
        write_request(&s.tty, s.pid, s.term_pid, "close")
    }

    fn new_session(&self, _cmds: &[String]) -> bool {
        // The plugin spawns its own claude command in the most-recently-focused project;
        // a "new" request carries no tty.
        write_request("", 0, 0, "new")
    }
}
