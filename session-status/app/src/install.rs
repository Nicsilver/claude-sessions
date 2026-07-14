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

fn markers() -> Vec<String> {
    let mut m = vec!["claude-sessions".to_string(), "record.py".to_string()];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(stem) = exe.file_stem().and_then(|s| s.to_str()) {
            m.push(stem.to_string());
        }
    }
    m
}

fn is_ours(group: &Value, markers: &[String]) -> bool {
    group.get("hooks").and_then(Value::as_array).is_some_and(|hooks| {
        hooks.iter().any(|h| {
            let cmd = str_of(h, "command");
            markers.iter().any(|m| cmd.contains(m.as_str()))
        })
    })
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
                    hs.iter().any(|h| str_of(h, "command").starts_with(&cmd_needle))
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
    let markers = markers();
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
            arr.retain(|g| !is_ours(g, &markers));
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
                arr.retain(|g| !is_ours(g, &markers));
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
    } else {
        println!("removed {changed} session-status hook group(s); your other hooks are untouched");
    }
    0
}

fn backup(path: &Path) {
    if !path.exists() {
        return;
    }
    let dst = path.with_file_name(format!(
        "{}.bak-{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("settings.json"),
        unix_now() as u64
    ));
    if std::fs::copy(path, &dst).is_ok() {
        if let Some(name) = dst.file_name().and_then(|s| s.to_str()) {
            println!("backed up settings.json -> {name}");
        }
    }
}
