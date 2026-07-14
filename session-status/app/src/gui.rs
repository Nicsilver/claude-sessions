//! The always-on-top floating dashboard (Tauri v2 port of floatdash.swift / MainWindow.cs).
//! The window chrome (transparent + undecorated + CSS drop shadow) and all drawing live in
//! ui/index.html; this module is the shell: window placement, tray, hook auto-install, the
//! 1.5s heartbeat that pushes sessions to the frontend, and the commands the frontend invokes.

use crate::{install, model, platform, styles};
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
        .invoke_handler(tauri::generate_handler![
            load_sessions,
            jump,
            new_session,
            toggle_mute,
            rename,
            menu_action
        ])
        .setup(|app| {
            setup_tray(app.handle())?;
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
    let x = mon.position().x + mon.size().width as i32 - size.width as i32
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
            if let TrayIconEvent::Click { button, button_state: MouseButtonState::Up, position, .. } = ev
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

/// Jump to the first unmuted session — model::load() already sorts needs > your-turn > working,
/// newest first. With nothing to jump to, fall back to toggling the dashboard.
fn jump_to_top(app: &tauri::AppHandle) {
    let sessions = model::load();
    let now = model::now();
    match sessions.iter().find(|s| !s.muted(now)) {
        Some(s) => platform::jump(&s.terminal, s.term_pid, s.pid),
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
                "mute_until": s.mute_until,
            })
        })
        .collect()
}

#[tauri::command]
fn load_sessions() -> Vec<Value> {
    to_json(&model::load())
}

#[tauri::command]
fn jump(terminal: String, term_pid: i64, pid: i64) {
    platform::jump(&terminal, term_pid, pid);
}

#[tauri::command]
fn new_session() {
    platform::new_session();
}

#[tauri::command]
fn toggle_mute(id: String) {
    model::toggle_mute(&id);
}

#[tauri::command]
fn rename(id: String, name: String) {
    model::set_label(&id, &name);
}
