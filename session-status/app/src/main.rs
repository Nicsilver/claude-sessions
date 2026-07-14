// Hide the console window in release so the recorder (called by hooks) never flashes a window and
// the GUI launches clean. Debug builds keep a console for logging.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Some helpers (label_for, set_label, etc.) are scaffolding for later phases (rename, mute UI).
#![allow(dead_code)]

mod gui;
mod install;
mod model;
mod paths;
mod platform;
mod recorder;
mod styles;
mod tray;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        // Called by Claude Code hooks — write the session state and exit (never launches the GUI).
        Some("record") => {
            let state = args.get(1).map(String::as_str).unwrap_or("working");
            recorder::record(state);
            std::process::exit(0);
        }
        Some("install") => std::process::exit(install::run(true)),
        Some("uninstall") => std::process::exit(install::run(false)),
        // Append the optional ⏳/✅ turn-marker instruction to the global CLAUDE.md.
        Some("markers") => std::process::exit(if install::append_claude_md_markers() { 0 } else { 1 }),
        // Default (double-click): launch the floating dashboard + tray.
        _ => {
            if let Err(e) = gui::run() {
                eprintln!("failed to start: {e}");
                std::process::exit(1);
            }
        }
    }
}
