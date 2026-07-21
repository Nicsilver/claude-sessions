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
            {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                prevent_app_nap();
            }
            setup_tray(app.handle())?;
            register_hotkeys(app.handle());
            // Auto-hide is a level check exactly once, at startup: launching at login with
            // nothing running should go straight to the tray rather than flash the panel.
            let startup = Pulse::of(&model::load());
            if let Some(win) = app.get_webview_window("main") {
                position_top_right(&win);
                if !(auto_hide() && startup.live == 0) {
                    let _ = win.show();
                }
            }
            // Heartbeat: reload sessions, refresh the tray badge, push to the frontend.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut prev = startup;
                loop {
                    let sessions = model::load();
                    update_tray(&handle, &sessions);
                    let _ = handle.emit("sessions", to_json(&sessions));
                    let pulse = Pulse::of(&sessions);
                    apply_auto_visibility(&handle, &prev, &pulse);
                    prev = pulse;
                    std::thread::sleep(Duration::from_millis(1500));
                }
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

/// Opt this process out of macOS App Nap. Without it, once another app takes focus the OS throttles
/// our run loop and compositor, so the dashboard's hover glow and live updates crawl while it sits
/// in the background — staying responsive there is the whole point of the widget. The
/// AllowingIdleSystemSleep variant exempts only this process; the Mac still sleeps normally when
/// idle. The activity token must outlive the process, so it's leaked deliberately.
/// Opt this process out of macOS App Nap. Without it, once another app takes focus the OS throttles
/// our run loop and compositor, so the dashboard's hover glow and live updates crawl while it sits
/// in the background — staying responsive there is the whole point of the widget. The
/// AllowingIdleSystemSleep variant exempts only this process; the Mac still sleeps normally when
/// idle. The activity token must outlive the process, so it's leaked deliberately.
#[cfg(target_os = "macos")]
fn prevent_app_nap() {
    use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
    let reason = NSString::from_str("Live Claude session status");
    let token = NSProcessInfo::processInfo().beginActivityWithOptions_reason(
        NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
        &reason,
    );
    std::mem::forget(token);
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

// ---- auto-hide / auto-show ----

/// What counts as a "live" session for the tray badge and the auto-hide/auto-show toggles: a
/// session still in play. A finished one (idle/done) shouldn't hold the dashboard open or pop
/// it back up.
const LIVE_STATES: [&str; 3] = ["needs", "yourturn", "working"];

/// The slice of a heartbeat that auto-hide/auto-show react to.
struct Pulse {
    live: usize,
    /// Ids of the sessions currently asking for you, so a *new* one can be told from a standing one.
    needs: std::collections::HashSet<String>,
}

impl Pulse {
    fn of(sessions: &[model::Sess]) -> Self {
        let now = model::now();
        let live: Vec<&model::Sess> = sessions
            .iter()
            .filter(|s| !s.muted(now) && LIVE_STATES.contains(&s.state.as_str()))
            .collect();
        Self {
            live: live.len(),
            needs: live
                .iter()
                .filter(|s| s.state == "needs")
                .map(|s| s.id.clone())
                .collect(),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Auto {
    Show,
    Hide,
    Nothing,
}

/// Both toggles are edge-triggered — they act on the heartbeat where something *changed*, never
/// on the standing level. That's what keeps them from fighting the user: hide the panel by hand
/// with sessions running and it stays hidden, because no transition has happened yet.
fn auto_action(prev: &Pulse, now: &Pulse, hide: bool, show: bool) -> Auto {
    // Show wins over hide: on a heartbeat where the last session goes quiet as another one
    // starts asking for you, surfacing it is the useful answer.
    if show {
        let appeared = prev.live == 0 && now.live > 0;
        let newly_needs = now.needs.difference(&prev.needs).next().is_some();
        if appeared || newly_needs {
            return Auto::Show;
        }
    }
    if hide && prev.live > 0 && now.live == 0 {
        return Auto::Hide;
    }
    Auto::Nothing
}

fn apply_auto_visibility(app: &tauri::AppHandle, prev: &Pulse, now: &Pulse) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    match auto_action(prev, now, auto_hide(), auto_show()) {
        Auto::Show => {
            position_top_right(&win);
            // show() without set_focus: this is a passive panel reacting to a background event,
            // and stealing focus mid-keystroke would be worse than not showing at all.
            let _ = win.show();
        }
        Auto::Hide => {
            let _ = win.hide();
        }
        Auto::Nothing => {}
    }
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
            "theme": theme(),
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

fn config_bool(key: &str, default: bool) -> bool {
    crate::paths::load_json(&crate::paths::config_path())
        .get(key)
        .and_then(Value::as_bool)
        .unwrap_or(default)
}

// Independent by design (either, both or neither). Auto-hide defaults off — a dashboard that
// disappears on its own is a surprise you have to learn. Auto-show defaults on — surfacing a
// session that wants you is the widget's whole job.
fn auto_hide() -> bool {
    config_bool("auto_hide", false)
}

fn auto_show() -> bool {
    config_bool("auto_show", true)
}

fn new_session_cmd_raw() -> String {
    config_str("new_session_cmd", "claude")
}

/// Dashboard + tray-menu theme: "system" (follow the OS), "dark" or "light".
fn theme() -> String {
    config_str("theme", "system")
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
        "theme": theme(),
        "hotkey_jump": hotkey_jump(),
        "hotkey_new": hotkey_new(),
        "auto_hide": auto_hide(),
        "auto_show": auto_show(),
        // OS state (HKCU Run / login item), not stored in config.json.
        "autostart": app.autolaunch().is_enabled().unwrap_or(false),
        "terminals": targets,
    })
}

/// What the options pane saves. One struct rather than a parameter per field, so adding an
/// option stays a one-line change here and in the pane.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    new_session_cmd: String,
    new_session_terminal: String,
    theme: String,
    hotkey_jump: String,
    hotkey_new: String,
    auto_hide: bool,
    auto_show: bool,
    autostart: bool,
}

#[tauri::command]
fn set_config(app: tauri::AppHandle, settings: Settings) {
    use tauri_plugin_autostart::ManagerExt;
    let path = crate::paths::config_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut root = crate::paths::load_json(&path);
    if !root.is_object() {
        root = json!({});
    }
    root["new_session_cmd"] = json!(settings.new_session_cmd.trim());
    root["new_session_terminal"] = json!(settings.new_session_terminal.trim());
    root["theme"] = json!(settings.theme.trim());
    root["hotkey_jump"] = json!(settings.hotkey_jump.trim());
    root["hotkey_new"] = json!(settings.hotkey_new.trim());
    root["auto_hide"] = json!(settings.auto_hide);
    root["auto_show"] = json!(settings.auto_show);
    let _ = std::fs::write(
        &path,
        serde_json::to_string_pretty(&root).unwrap_or_default(),
    );
    // Apply the new bindings immediately.
    register_hotkeys(&app);
    // Launch-at-login is OS state — only touch it when the toggle actually changed.
    let mgr = app.autolaunch();
    if settings.autostart != mgr.is_enabled().unwrap_or(false) {
        let _ = if settings.autostart {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A pulse with `live` sessions, of which `needs` are asking for you.
    fn pulse(live: usize, needs: &[&str]) -> Pulse {
        Pulse {
            live,
            needs: needs.iter().map(|s| s.to_string()).collect(),
        }
    }

    const BOTH: (bool, bool) = (true, true);

    #[test]
    fn shows_when_the_first_session_appears() {
        let a = auto_action(&pulse(0, &[]), &pulse(1, &[]), BOTH.0, BOTH.1);
        assert_eq!(a, Auto::Show);
    }

    #[test]
    fn shows_when_a_session_newly_needs_you() {
        let a = auto_action(&pulse(2, &[]), &pulse(2, &["s1"]), BOTH.0, BOTH.1);
        assert_eq!(a, Auto::Show);
    }

    /// The edge-trigger contract: a session that was already asking for you last tick must not
    /// re-show the panel every 1.5s after you hide it by hand.
    #[test]
    fn standing_needs_does_not_re_show() {
        let a = auto_action(&pulse(2, &["s1"]), &pulse(2, &["s1"]), BOTH.0, BOTH.1);
        assert_eq!(a, Auto::Nothing);
    }

    /// Same contract for a steady stream of live sessions: no transition, no interference.
    #[test]
    fn steady_live_sessions_do_not_re_show() {
        let a = auto_action(&pulse(3, &[]), &pulse(2, &[]), BOTH.0, BOTH.1);
        assert_eq!(a, Auto::Nothing);
    }

    #[test]
    fn hides_when_the_last_session_goes_quiet() {
        let a = auto_action(&pulse(1, &[]), &pulse(0, &[]), BOTH.0, BOTH.1);
        assert_eq!(a, Auto::Hide);
    }

    #[test]
    fn stays_hidden_while_nothing_is_live() {
        let a = auto_action(&pulse(0, &[]), &pulse(0, &[]), BOTH.0, BOTH.1);
        assert_eq!(a, Auto::Nothing);
    }

    /// A different session asking for you is a new edge even though the count is unchanged.
    #[test]
    fn a_different_session_needing_you_is_a_new_edge() {
        let a = auto_action(&pulse(2, &["s1"]), &pulse(2, &["s2"]), BOTH.0, BOTH.1);
        assert_eq!(a, Auto::Show);
    }

    #[test]
    fn show_wins_when_one_session_ends_as_another_asks() {
        let a = auto_action(&pulse(1, &[]), &pulse(1, &["s2"]), BOTH.0, BOTH.1);
        assert_eq!(a, Auto::Show);
    }

    // The two toggles are independent: each must be inert when off and still fire when it's the
    // only one on.

    #[test]
    fn hide_off_leaves_the_panel_up() {
        let a = auto_action(&pulse(1, &[]), &pulse(0, &[]), false, true);
        assert_eq!(a, Auto::Nothing);
    }

    #[test]
    fn show_off_leaves_the_panel_hidden() {
        let a = auto_action(&pulse(0, &[]), &pulse(1, &["s1"]), true, false);
        assert_eq!(a, Auto::Nothing);
    }

    #[test]
    fn hide_alone_still_hides() {
        let a = auto_action(&pulse(1, &[]), &pulse(0, &[]), true, false);
        assert_eq!(a, Auto::Hide);
    }

    #[test]
    fn show_alone_still_shows() {
        let a = auto_action(&pulse(0, &[]), &pulse(1, &[]), false, true);
        assert_eq!(a, Auto::Show);
    }

    #[test]
    fn both_off_never_acts() {
        assert_eq!(
            auto_action(&pulse(1, &[]), &pulse(0, &[]), false, false),
            Auto::Nothing
        );
        assert_eq!(
            auto_action(&pulse(0, &[]), &pulse(1, &["s1"]), false, false),
            Auto::Nothing
        );
    }

    /// Muted and finished sessions must not count as live, or a muted session would pop the
    /// panel up — the opposite of what muting it meant.
    #[test]
    fn pulse_counts_only_unmuted_live_sessions() {
        let now = model::now();
        let sess = |id: &str, state: &str, mute_until: f64| model::Sess {
            id: id.into(),
            topic: id.into(),
            state: state.into(),
            updated: now,
            message: String::new(),
            terminal: "wt".into(),
            term_pid: 1,
            pid: 1,
            tab_title: String::new(),
            tty: String::new(),
            mute_until,
        };
        let p = Pulse::of(&[
            sess("a", "needs", 0.0),
            sess("b", "working", 0.0),
            sess("c", "needs", now + 3600.0), // muted
            sess("d", "idle", 0.0),           // finished
            sess("e", "done", 0.0),           // finished
        ]);
        assert_eq!(p.live, 2);
        assert_eq!(p.needs, ["a".to_string()].into_iter().collect());
    }
}
