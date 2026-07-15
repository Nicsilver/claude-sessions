//! Non-destructive Claude Code hook wiring in ~/.claude/settings.json. Appends our own hook
//! group per lifecycle event (pointing at this exe's `record` subcommand) without touching any
//! hooks you already have; `uninstall` removes only ours. Also recognises the old Python
//! record.py hooks so installing the Rust build cleanly migrates off Python.

use crate::paths::*;
use serde_json::{json, Value};
use std::path::Path;

const EVENTS: &[(&str, &str)] = &[
    ("SessionStart", "start"),
    ("UserPromptSubmit", "working"),
    ("PostToolUse", "working"),
    ("Notification", "needs"),
    ("Stop", "done"),
    ("SessionEnd", "end"),
];

/// A hook command is ours when its *executable* is a recorder build: record.py, the old
/// compiled `record` prototype, this exe, or anything named claude-sessions. Matching the
/// executable token — not the whole command string — keeps unrelated hooks that merely live
/// under a "claude-sessions" directory (e.g. a worklog script in the same repo) untouched.
fn is_recorder_cmd(cmd: &str) -> bool {
    if cmd.contains("record.py") {
        return true;
    }
    let t = cmd.trim_start();
    let exe = if let Some(rest) = t.strip_prefix('"') {
        rest.split('"').next().unwrap_or("")
    } else {
        t.split_whitespace().next().unwrap_or("")
    };
    let stem = Path::new(exe)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    stem == "record" || stem == "claude-sessions" || Some(stem) == exe_stem()
}

fn exe_stem() -> Option<String> {
    std::env::current_exe()
        .ok()?
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
}

fn is_ours(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hooks| hooks.iter().any(|h| is_recorder_cmd(&str_of(h, "command"))))
}

fn our_command() -> String {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "claude-sessions".into());
    format!("\"{exe}\" record")
}

/// True if our recorder hooks are already present in settings.json (used by the GUI to decide
/// whether first-run auto-install is needed).
pub fn already_installed() -> bool {
    let root = load_json(&settings_path());
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return false;
    };
    let cmd_needle = our_command();
    hooks.values().any(|groups| {
        groups.as_array().is_some_and(|arr| {
            arr.iter().any(|g| {
                g.get("hooks").and_then(Value::as_array).is_some_and(|hs| {
                    hs.iter()
                        .any(|h| str_of(h, "command").starts_with(&cmd_needle))
                })
            })
        })
    })
}

pub fn run(install: bool) -> i32 {
    let path = settings_path();
    let mut root = load_json(&path);
    if !root.is_object() {
        root = json!({});
    }
    let obj = root.as_object_mut().unwrap();
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let hooks = hooks.as_object_mut().unwrap();

    let mut changed = 0usize;
    if install {
        for (event, state) in EVENTS {
            let groups = hooks.entry(event.to_string()).or_insert_with(|| json!([]));
            let arr = groups.as_array_mut().unwrap();
            arr.retain(|g| !is_ours(g));
            arr.push(json!({
                "hooks": [ { "type": "command", "command": format!("{} {}", our_command(), state) } ]
            }));
            changed += 1;
        }
    } else {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for event in keys {
            if let Some(arr) = hooks.get_mut(&event).and_then(Value::as_array_mut) {
                let before = arr.len();
                arr.retain(|g| !is_ours(g));
                changed += before - arr.len();
                if arr.is_empty() {
                    hooks.remove(&event);
                }
            }
        }
        if hooks.is_empty() {
            obj.remove("hooks");
        }
    }

    backup(&path);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = serde_json::to_string_pretty(&root).unwrap_or_else(|_| "{}".into());
    if std::fs::write(&path, text).is_err() {
        eprintln!("error: could not write {}", path.display());
        return 1;
    }
    if install {
        println!("installed session-status hooks on {changed} events");
        println!("recorder: {}", our_command());
        println!("Start a NEW Claude session (or send a prompt) to populate the widget.");
        if !claude_md_has_markers() {
            println!();
            println!("Optional: `claude-sessions markers` adds a turn-marker instruction (⏳/✅)");
            println!("to ~/.claude/CLAUDE.md so done/your-turn states are accurate.");
            println!("Also available from the tray icon's right-click menu.");
        }
    } else {
        println!("removed {changed} session-status hook group(s); your other hooks are untouched");
    }
    0
}

fn backup(path: &Path) {
    if !path.exists() {
        return;
    }
    let orig = path.file_name().and_then(|s| s.to_str()).unwrap_or("file");
    let dst = path.with_file_name(format!("{orig}.bak-{}", unix_now() as u64));
    if std::fs::copy(path, &dst).is_ok() {
        if let Some(name) = dst.file_name().and_then(|s| s.to_str()) {
            println!("backed up {orig} -> {name}");
        }
    }
}

// ---- optional: the ⏳/✅ turn-marker instruction in the global CLAUDE.md ----

const MARKER_SENTINEL: &str = "claude-sessions: turn markers";
const MARKER_SNIPPET: &str = "\n\n<!-- claude-sessions: turn markers -->\n\
At the end of every response, put a status marker on its own final line:\n\
- `✅` when the task is complete and you need nothing from me.\n\
- `⏳` when you are waiting on me (a question, a decision, or confirmation).\n";

fn claude_md_path() -> std::path::PathBuf {
    home().join(".claude").join("CLAUDE.md")
}

/// True if the global CLAUDE.md already carries a turn-marker instruction (so we don't offer again).
pub fn claude_md_has_markers() -> bool {
    std::fs::read_to_string(claude_md_path())
        .map(|s| s.contains(MARKER_SENTINEL) || (s.contains('⏳') && s.contains('✅')))
        .unwrap_or(false)
}

/// Append the turn-marker instruction to ~/.claude/CLAUDE.md (idempotent; backs up first if the
/// file exists). Returns true on success / already-present.
pub fn append_claude_md_markers() -> bool {
    let path = claude_md_path();
    if claude_md_has_markers() {
        return true;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    backup(&path);
    let mut content = std::fs::read_to_string(&path).unwrap_or_default();
    content.push_str(MARKER_SNIPPET);
    let ok = std::fs::write(&path, content).is_ok();
    if ok {
        println!("added turn-marker instruction to {}", path.display());
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_our_recorder_commands() {
        // The Python original, the old compiled `record`, and the current binary (quoted path).
        assert!(is_recorder_cmd(
            "/opt/homebrew/bin/python3 /x/session-status/bin/record.py working"
        ));
        assert!(is_recorder_cmd("/usr/local/bin/record done"));
        // Quoted path with our binary — forward slashes parse the same on every OS.
        assert!(is_recorder_cmd(
            "\"/Users/me/ClaudeSessions/claude-sessions\" record needs"
        ));
        // On Windows the hook path uses backslashes, which must resolve to our stem too.
        #[cfg(windows)]
        assert!(is_recorder_cmd(
            "\"C:\\Users\\me\\AppData\\Local\\ClaudeSessions\\claude-sessions.exe\" record needs"
        ));
    }

    #[test]
    fn leaves_unrelated_hooks_alone() {
        // The whole point of matching the executable, not the string: never clobber a user's own
        // hooks, e.g. a keep-awake hook that also runs on every turn.
        assert!(!is_recorder_cmd("caffeinate -di"));
        assert!(!is_recorder_cmd("/usr/bin/keep-awake --loop"));
        assert!(!is_recorder_cmd("node /home/me/notify.js"));
        // A script that merely lives under a "claude-sessions" directory is not our recorder.
        assert!(!is_recorder_cmd("python3 /home/claude-sessions/worklog.py"));
    }

    #[test]
    fn is_ours_only_flags_groups_that_run_the_recorder() {
        let ours = json!({ "hooks": [ { "type": "command", "command": "/bin/record working" } ] });
        let awake = json!({ "hooks": [ { "type": "command", "command": "caffeinate -di" } ] });
        let empty = json!({ "hooks": [] });
        assert!(is_ours(&ours));
        assert!(!is_ours(&awake));
        assert!(!is_ours(&empty));
    }
}
