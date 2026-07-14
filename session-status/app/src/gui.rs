//! The always-on-top floating dashboard (Tauri v2 port of floatdash.swift / MainWindow.cs).
//! The window chrome (transparent + undecorated + CSS drop shadow) and all drawing live in
//! ui/index.html; this module is the shell: window placement, tray, hook auto-install, the
//! 1.5s heartbeat that pushes sessions to the frontend, and the commands the frontend invokes.

use crate::{install, model, styles, terminals};
use serde_json::{json, Value};
use std::time::Duration;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, WindowEvent};

pub fn run() -> tauri::Result<()> {
    // First-run: wire the Claude Code hooks so simply launching the exe is enough.
    if !install::already_installed() {
        let _ = install::run(true);
    }

    // A static 300px panel doesn't need GPU compositing; software raster renders it identically
    // and drops WebView2's private memory by ~100MB (the GPU process). Overridable via the env.
    if std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").is_err() {
        std::env::set_var(
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
            "--disable-gpu --disable-gpu-compositing",
        );
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            load_sessions,
            jump,
            close_session,
            new_session,
            toggle_mute,
            rename,
            menu_action,
            get_config,
            set_config
        ])
        .setup(|app| {
            // Menu-bar-only app: no Dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            setup_tray(app.handle())?;
            register_hotkeys(app.handle());
            if let Some(win) = app.get_webview_window("main") {
                position_top_right(&win);
                let _ = win.show();
            }
            // Heartbeat: reload sessions, refresh the tray badge, push to the frontend.
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                let sessions = model::load();
                update_tray(&handle, &sessions);
                let _ = handle.emit("sessions", to_json(&sessions));
                std::thread::sleep(Duration::from_millis(1500));
            });
            Ok(())
        })
        // Closing (Alt-F4 etc.) hides to the tray instead of quitting.
        .on_window_event(|win, ev| {
            if let WindowEvent::CloseRequested { api, .. } = ev {
                api.prevent_close();
                let _ = win.hide();
            }
        })
        .run(tauri::generate_context!())?;
    Ok(())
}

/// Pin the panel to the monitor's top-right. The html has a 14px shadow gutter, so a 6px
/// outer margin puts the visible panel 20px from the edges, like the other surfaces.
fn position_top_right(win: &tauri::WebviewWindow) {
    let (Ok(Some(mon)), Ok(size)) = (win.current_monitor(), win.outer_size()) else {
        return;
    };
    let x = mon.position().x + mon.size().width as i32
        - size.width as i32
        - (6.0 * mon.scale_factor()) as i32;
    let y = mon.position().y + (26.0 * mon.scale_factor()) as i32;
    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
}

fn toggle_window(app: &tauri::AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
    } else {
        position_top_right(&win);
        let _ = win.show();
        let _ = win.set_focus();
    }
}

// ---- tray ----

/// Left-click jumps to the topmost (highest-priority) session, like the WPF tray did; the
/// dashboard toggle lives in the menu. Right-click opens our own webview-rendered menu (the
/// native Win32 tray menu can't be styled — same reason the WPF build had WpfTrayMenu.cs).
fn setup_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    TrayIconBuilder::with_id("main")
        .tooltip("Claude sessions")
        .icon(badge_icon("idle", 0))
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click {
                button,
                button_state: MouseButtonState::Up,
                position,
                ..
            } = ev
            {
                match button {
                    MouseButton::Left => jump_to_top(tray.app_handle()),
                    MouseButton::Right => open_tray_menu(tray.app_handle(), position.x, position.y),
                    _ => {}
                }
            }
        })
        .build(app)?;
    Ok(())
}

// ---- global hotkeys (the RegisterEventHotKey pair from menubar.swift, now cross-platform) ----

/// (Re)register both global hotkeys from config. Called at startup and after the options pane
/// saves. Unregisters everything first so a rebind doesn't leave the old combo live. A combo
/// that fails to parse or is already taken by another app is logged and skipped — the widget
/// still runs, that shortcut just stays unbound until it's rebound.
fn register_hotkeys(app: &tauri::AppHandle) {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();

    let jump = hotkey_jump();
    if !jump.is_empty() {
        if let Err(e) = gs.on_shortcut(jump.as_str(), |app, _sc, ev| {
            if ev.state() == ShortcutState::Pressed {
                jump_to_top(app);
            }
        }) {
            eprintln!("claude-sessions: could not bind jump hotkey {jump:?}: {e}");
        }
    }

    let new = hotkey_new();
    if !new.is_empty() {
        if let Err(e) = gs.on_shortcut(new.as_str(), |_app, _sc, ev| {
            if ev.state() == ShortcutState::Pressed {
                spawn_new_session();
            }
        }) {
            eprintln!("claude-sessions: could not bind new-session hotkey {new:?}: {e}");
        }
    }
}

/// Jump to the first unmuted session — model::load() already sorts needs > your-turn > working,
/// newest first. With nothing to jump to, fall back to toggling the dashboard.
fn jump_to_top(app: &tauri::AppHandle) {
    let sessions = model::load();
    let now = model::now();
    match sessions.iter().find(|s| !s.muted(now)) {
        Some(s) => {
            let s = s.clone();
            std::thread::spawn(move || terminals::focus(&s));
        }
        None => toggle_window(app),
    }
}

/// Push the current state to menu.html; it sizes itself, pops up at the cursor and shows.
fn open_tray_menu(app: &tauri::AppHandle, x: f64, y: f64) {
    let monitor = app.monitor_from_point(x, y).ok().flatten().map(|m| {
        json!({
            "x": m.position().x, "y": m.position().y,
            "w": m.size().width, "h": m.size().height,
        })
    });
    let _ = app.emit_to(
        "menu",
        "menu-open",
        json!({
            "x": x,
            "y": y,
            "monitor": monitor,
            "sessions": to_json(&model::load()),
            "markers_missing": !install::claude_md_has_markers(),
        }),
    );
}

#[tauri::command]
fn menu_action(app: tauri::AppHandle, id: String) {
    match id.as_str() {
        "toggle" => toggle_window(&app),
        "markers" => {
            let _ = install::append_claude_md_markers();
        }
        "quit" => app.exit(0),
        _ => {}
    }
}

fn update_tray(app: &tauri::AppHandle, sessions: &[model::Sess]) {
    let Some(tray) = app.tray_by_id("main") else {
        return;
    };
    let now = model::now();
    let active: Vec<&model::Sess> = sessions.iter().filter(|s| s.mute_until <= now).collect();
    let count = |st: &str| active.iter().filter(|s| s.state == st).count();
    let (needs, yt, wk) = (count("needs"), count("yourturn"), count("working"));
    let (top, n) = if needs > 0 {
        ("needs", needs)
    } else if yt > 0 {
        ("yourturn", yt)
    } else if wk > 0 {
        ("working", wk)
    } else {
        ("idle", 0)
    };
    let _ = tray.set_icon(Some(badge_icon(top, n)));

    let mut parts = Vec::new();
    if needs > 0 {
        parts.push(format!("{needs} need you"));
    }
    if yt > 0 {
        parts.push(format!("{yt} your turn"));
    }
    if wk > 0 {
        parts.push(format!("{wk} working"));
    }
    let tip = if parts.is_empty() {
        "Claude sessions — idle".into()
    } else {
        format!("Claude — {}", parts.join(", "))
    };
    let _ = tray.set_tooltip(Some(tip));
}

/// A 32×32 RGBA badge: a filled disc in the state colour (small + dim when nothing is active).
fn badge_icon(state: &str, count: usize) -> tauri::image::Image<'static> {
    const W: u32 = 32;
    let (r, g, b) = styles::rgb_for(state);
    let alpha_scale = if count == 0 { 0.6 } else { 1.0 };
    let rad = if count == 0 { 6.0f32 } else { 13.0 };
    let mut rgba = vec![0u8; (W * W * 4) as usize];
    for y in 0..W {
        for x in 0..W {
            let (dx, dy) = (x as f32 + 0.5 - 16.0, y as f32 + 0.5 - 16.0);
            let d = (dx * dx + dy * dy).sqrt();
            let cover = (rad + 1.0 - d).clamp(0.0, 1.0);
            let i = ((y * W + x) * 4) as usize;
            rgba[i] = r;
            rgba[i + 1] = g;
            rgba[i + 2] = b;
            rgba[i + 3] = (255.0 * cover * alpha_scale) as u8;
        }
    }
    tauri::image::Image::new_owned(rgba, W, W)
}

// ---- commands ----

fn to_json(sessions: &[model::Sess]) -> Vec<Value> {
    sessions
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "topic": s.topic,
                "state": s.state,
                "updated": s.updated,
                "message": s.message,
                "terminal": s.terminal,
                "term_pid": s.term_pid,
                "pid": s.pid,
                "tab_title": s.tab_title,
                "mute_until": s.mute_until,
            })
        })
        .collect()
}

#[tauri::command]
fn load_sessions() -> Vec<Value> {
    to_json(&model::load())
}

// The jump/close/new-session actions can sleep between focus and keystrokes, so they run on
// their own threads rather than blocking a command handler. They take the session id and
// re-load state, so the frontend never carries terminal internals.

fn with_session(id: String, f: impl Fn(&model::Sess) + Send + 'static) {
    std::thread::spawn(move || {
        if let Some(s) = model::load().into_iter().find(|s| s.id == id) {
            f(&s);
        }
    });
}

#[tauri::command]
fn jump(id: String) {
    with_session(id, terminals::focus);
}

#[tauri::command]
fn close_session(id: String) {
    with_session(id, terminals::close);
}

#[tauri::command]
fn new_session() {
    spawn_new_session();
}

/// Open a new Claude session in the configured terminal. Shared by the `+` button command and
/// the new-session global hotkey.
fn spawn_new_session() {
    let (target, cmds) = (new_session_terminal(), new_session_cmds());
    std::thread::spawn(move || terminals::new_session(&target, &cmds));
}

// ---- options (config.json in ~/.claude/session-status/) ----

fn config_str(key: &str, default: &str) -> String {
    crate::paths::load_json(&crate::paths::config_path())
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(default)
        .to_string()
}

fn new_session_cmd_raw() -> String {
    config_str("new_session_cmd", "claude")
}

// Global-hotkey defaults mirror menubar.swift's ⌃⌥⌘J / ⌃⌥⌘N — CmdOrCtrl maps to ⌘ on mac and
// Ctrl on Windows/Linux. Accelerator strings are what tauri-plugin-global-shortcut parses.
const DEFAULT_HOTKEY_JUMP: &str = "CmdOrCtrl+Alt+J";
const DEFAULT_HOTKEY_NEW: &str = "CmdOrCtrl+Alt+N";

fn hotkey_jump() -> String {
    config_str("hotkey_jump", DEFAULT_HOTKEY_JUMP)
}

fn hotkey_new() -> String {
    config_str("hotkey_new", DEFAULT_HOTKEY_NEW)
}

/// Which terminal `+` opens new sessions in; defaults to the platform's first adapter.
fn new_session_terminal() -> String {
    let default = terminals::spawn_targets().first().map_or("", |(id, _)| id);
    config_str("new_session_terminal", default)
}

/// The configured launch command(s), one per line — typed into the new tab in order.
fn new_session_cmds() -> Vec<String> {
    new_session_cmd_raw()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

#[tauri::command]
fn get_config(app: tauri::AppHandle) -> Value {
    use tauri_plugin_autostart::ManagerExt;
    let targets: Vec<Value> = terminals::spawn_targets()
        .into_iter()
        .map(|(id, label)| json!({ "id": id, "label": label }))
        .collect();
    json!({
        "new_session_cmd": new_session_cmd_raw(),
        "new_session_terminal": new_session_terminal(),
        "hotkey_jump": hotkey_jump(),
        "hotkey_new": hotkey_new(),
        // OS state (HKCU Run / login item), not stored in config.json.
        "autostart": app.autolaunch().is_enabled().unwrap_or(false),
        "terminals": targets,
    })
}

#[tauri::command]
fn set_config(
    app: tauri::AppHandle,
    new_session_cmd: String,
    new_session_terminal: String,
    hotkey_jump: String,
    hotkey_new: String,
    autostart: bool,
) {
    use tauri_plugin_autostart::ManagerExt;
    let path = crate::paths::config_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut root = crate::paths::load_json(&path);
    if !root.is_object() {
        root = json!({});
    }
    root["new_session_cmd"] = json!(new_session_cmd.trim());
    root["new_session_terminal"] = json!(new_session_terminal.trim());
    root["hotkey_jump"] = json!(hotkey_jump.trim());
    root["hotkey_new"] = json!(hotkey_new.trim());
    let _ = std::fs::write(
        &path,
        serde_json::to_string_pretty(&root).unwrap_or_default(),
    );
    // Apply the new bindings immediately.
    register_hotkeys(&app);
    // Launch-at-login is OS state — only touch it when the toggle actually changed.
    let mgr = app.autolaunch();
    if autostart != mgr.is_enabled().unwrap_or(false) {
        let _ = if autostart {
            mgr.enable()
        } else {
            mgr.disable()
        };
    }
}

#[tauri::command]
fn toggle_mute(id: String) {
    model::toggle_mute(&id);
}

#[tauri::command]
fn rename(id: String, name: String) {
    model::set_label(&id, &name);
}
