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
            println!("Optional: `claude-sessions markers` adds a turn-marker instruction (●/○/◐)");
            println!("to ~/.claude/CLAUDE.md so done/your-turn/working states are accurate.");
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

// ---- optional: the ●/○/◐ turn-marker instruction in the global CLAUDE.md ----
//
// The block is versioned via the sentinel comment ("… turn markers v2"): bump MARKER_VERSION
// whenever MARKER_SNIPPET's content changes, and marker_status() reports existing installs as
// Outdated so both the GUI (startup dialog + tray item) and `claude-sessions markers` offer an
// in-place upgrade. A sentinel-less hand-pasted copy is matched by its heading instead and
// judged current by whether it already teaches every glyph.

const MARKER_SENTINEL: &str = "claude-sessions: turn markers";
const MARKER_VERSION: u32 = 2;
/// Every version of the block ends with this exact line — it bounds the in-place replacement.
const MARKER_END: &str = "Use exactly one, every turn, as the very last line.";
const MARKER_SNIPPET: &str = r#"

<!-- claude-sessions: turn markers v2 -->
## Session-status marker (end every response with one)

A session-status dashboard reads the transcript to tell, at a glance, which sessions are waiting on me. To make that reliable, **end every response with a status marker on its own final line — nothing after it:**

- `○` — the ball is in my court: you asked a question, greeted me, offered options/choices, or need a decision, approval, or any reply from me before the conversation moves on. **When in doubt, use this.**
- `◐` — the ball is in nobody's court yet: you ended your turn only because you're waiting on background work you started (a build or test run, a background task, a subagent, a monitor/watcher, a scheduled wake-up) that will report back to you; nothing is needed from me.
- `●` — you completed the work I asked for and nothing is pending from me; the session is safe to close.

Rules of thumb:
- A greeting or "what would you like to work on?" is `○`, **never** `●` — you're waiting on me.
- An open question or an offer ("want me to…?") is `○`.
- Ending the turn while a background task, agent, or watcher is still running and will wake you is `◐` — but if you also need an answer from me, `○` wins.
- `●` is only for a turn that actually *finished a task* with nothing left for me to answer or decide, and nothing still running in the background.

Use exactly one, every turn, as the very last line.
"#;

fn claude_md_path() -> std::path::PathBuf {
    home().join(".claude").join("CLAUDE.md")
}

/// True if the global CLAUDE.md already carries a turn-marker instruction (so we don't offer again).
pub fn claude_md_has_markers() -> bool {
    std::fs::read_to_string(claude_md_path())
        .map(|s| s.contains(MARKER_SENTINEL) || (s.contains('○') && s.contains('●')))
        .unwrap_or(false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerStatus {
    Missing,
    Current,
    Outdated,
}

/// Where the global CLAUDE.md stands relative to this build's marker convention.
pub fn marker_status() -> MarkerStatus {
    marker_status_of(&std::fs::read_to_string(claude_md_path()).unwrap_or_default())
}

fn marker_status_of(content: &str) -> MarkerStatus {
    if let Some(v) = marker_version(content) {
        return if v >= MARKER_VERSION {
            MarkerStatus::Current
        } else {
            MarkerStatus::Outdated
        };
    }
    if content.contains('○') && content.contains('●') {
        // Hand-pasted copy without our sentinel: current once it teaches every glyph.
        return if content.contains('◐') {
            MarkerStatus::Current
        } else {
            MarkerStatus::Outdated
        };
    }
    MarkerStatus::Missing
}

/// Version carried by the sentinel comment; a sentinel predating versioning counts as 1.
fn marker_version(content: &str) -> Option<u32> {
    let pos = content.find(MARKER_SENTINEL)?;
    let rest = content[pos + MARKER_SENTINEL.len()..].trim_start();
    Some(
        rest.strip_prefix('v')
            .and_then(|r| {
                let digits: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
                digits.parse().ok()
            })
            .unwrap_or(1),
    )
}

/// Add the marker block if it's missing, or swap an outdated one for the current version.
/// The single entry point for the CLI `markers` subcommand and both GUI surfaces.
pub fn sync_claude_md_markers() -> bool {
    match marker_status() {
        MarkerStatus::Missing => append_claude_md_markers(),
        MarkerStatus::Outdated => upgrade_claude_md_markers(),
        MarkerStatus::Current => {
            println!("turn-marker instruction is already up to date");
            true
        }
    }
}

/// Replace an outdated marker block in place (backing the file up first). Handles both a
/// sentinel-tagged block and a hand-pasted copy matched by its heading; a heavily customized
/// block missing the closing line is left untouched.
fn upgrade_claude_md_markers() -> bool {
    let path = claude_md_path();
    let Ok(content) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Some(updated) = replace_marker_block(&content) else {
        eprintln!(
            "markers: couldn't locate the old block in {} — update it manually",
            path.display()
        );
        return false;
    };
    backup(&path);
    let ok = std::fs::write(&path, updated).is_ok();
    if ok {
        println!("updated turn-marker instruction in {}", path.display());
    }
    ok
}

fn replace_marker_block(content: &str) -> Option<String> {
    let start = content
        .find(&format!("<!-- {MARKER_SENTINEL}"))
        .or_else(|| content.find("## Session-status marker"))?;
    let end = start + content[start..].find(MARKER_END)? + MARKER_END.len();
    Some(format!(
        "{}{}{}",
        &content[..start],
        MARKER_SNIPPET.trim(),
        &content[end..]
    ))
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
    fn marker_status_detects_missing_current_and_outdated() {
        assert_eq!(marker_status_of(""), MarkerStatus::Missing);
        assert_eq!(marker_status_of("# my rules\nbe nice"), MarkerStatus::Missing);
        // A sentinel that predates versioning is v1 → outdated.
        assert_eq!(
            marker_status_of("<!-- claude-sessions: turn markers -->\n## Session-status marker"),
            MarkerStatus::Outdated
        );
        assert_eq!(marker_status_of(MARKER_SNIPPET), MarkerStatus::Current);
        // A future version must never be "upgraded" backwards.
        assert_eq!(
            marker_status_of("<!-- claude-sessions: turn markers v9 -->"),
            MarkerStatus::Current
        );
        // Hand-pasted copies (no sentinel) are judged by the glyphs they teach.
        assert_eq!(
            marker_status_of("## Session-status marker\n- `○` …\n- `●` …"),
            MarkerStatus::Outdated
        );
        assert_eq!(
            marker_status_of("## Session-status marker\n- `○` …\n- `◐` …\n- `●` …"),
            MarkerStatus::Current
        );
    }

    #[test]
    fn replace_marker_block_swaps_only_the_block() {
        let old = format!(
            "# Global instructions\n\n<!-- {MARKER_SENTINEL} -->\n## Session-status marker \
             (end every response with one)\nold body\n{MARKER_END}\n\n## Other rules\nkeep me\n"
        );
        let new = replace_marker_block(&old).unwrap();
        assert!(new.starts_with("# Global instructions\n\n<!-- claude-sessions: turn markers v2 -->"));
        assert!(new.contains('◐'));
        assert!(!new.contains("old body"));
        assert!(new.ends_with("\n\n## Other rules\nkeep me\n"));

        // A sentinel-less hand-pasted block is matched by its heading and gains the sentinel.
        let hand = format!(
            "## Session-status marker (end every response with one)\nbody\n{MARKER_END}\ntail"
        );
        let up = replace_marker_block(&hand).unwrap();
        assert!(up.starts_with("<!-- claude-sessions: turn markers v2 -->"));
        assert!(up.ends_with("\ntail"));

        // A heavily customized block without the closing line is left alone.
        assert_eq!(replace_marker_block("## Session-status marker\ncustom stuff"), None);
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
