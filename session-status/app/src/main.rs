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
mod selfinstall;
mod styles;
mod terminals;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        // Called by Claude Code hooks — write the session state and exit (never launches the GUI).
        Some("record") => {
            let state = args.get(1).map(String::as_str).unwrap_or("working");
            recorder::record(state);
            std::process::exit(0);
        }
        // CLI subcommands attach the launching console first so their output is visible in
        // release builds (windows subsystem detaches stdio).
        Some("install") => {
            platform::attach_parent_console();
            std::process::exit(install::run(true))
        }
        Some("uninstall") => {
            platform::attach_parent_console();
            std::process::exit(install::run(false))
        }
        // Copy the binary to a permanent location and register launch-at-login there, so
        // autostart survives moving/cleaning the build tree. `uninstall-app` reverses it.
        Some("install-app") => {
            platform::attach_parent_console();
            std::process::exit(selfinstall::install_app())
        }
        Some("uninstall-app") => {
            platform::attach_parent_console();
            std::process::exit(selfinstall::uninstall_app())
        }
        // Diagnostic: jump to a session (id prefix match) exactly like clicking its row.
        Some("focus") => {
            platform::attach_parent_console();
            let id = args.get(1).cloned().unwrap_or_default();
            match model::load().into_iter().find(|s| s.id.starts_with(&id)) {
                Some(s) => {
                    terminals::focus(&s);
                    std::process::exit(0)
                }
                None => {
                    eprintln!("no live session matching '{id}'");
                    std::process::exit(1)
                }
            }
        }
        // Append the optional ●/○ turn-marker instruction to the global CLAUDE.md.
        Some("markers") => {
            platform::attach_parent_console();
            std::process::exit(if install::append_claude_md_markers() {
                0
            } else {
                1
            })
        }
        // Default (double-click): launch the floating dashboard + tray.
        _ => {
            if let Err(e) = gui::run() {
                eprintln!("failed to start: {e}");
                std::process::exit(1);
            }
        }
    }
}
