//! WezTerm. Unlike WT it has a real control CLI (`wezterm cli`), so focus/close/spawn go
//! through `cli list / activate-pane / kill-pane / spawn / send-text` — no UI Automation and
//! no synthetic keystrokes. Panes are matched to sessions by tty on unix (exact: the recorder
//! stores the session's tty and the mux reports each pane's tty_name), falling back to title
//! via tabmatch — the only option on Windows, same policy as WT tabs. The CLI plumbing is
//! shared between Windows and macOS; only executable paths, process names and window
//! activation differ (HWND focus vs AppleScript activate).
//!
//! Gotcha (wezterm/wezterm#4456): the GUI publishes only the socket *filename*, and AF_UNIX
//! connect on Windows resolves relative paths against the client's cwd — so `wezterm cli`
//! from outside WezTerm can't connect on its own. We bypass discovery entirely by setting
//! WEZTERM_UNIX_SOCKET to the absolute `gui-sock-<pid>` path built from the terminal pid we
//! already track; this also makes stale socket files from dead instances a non-issue. macOS
//! uses the same `~/.local/share/wezterm` path, and pinning the socket there too keeps one
//! code path and picks the right instance when several are running.

use super::tabmatch::{self, Target};
use super::Terminal;
use crate::model::Sess;
use crate::paths::home;
#[cfg(windows)]
use crate::platform::{focus_window, main_window_for_pid};
use crate::platform::{is_alive, process_map};
use serde_json::Value;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct WezTerm;

impl Terminal for WezTerm {
    fn id(&self) -> &'static str {
        "wezterm"
    }

    fn label(&self) -> &'static str {
        "WezTerm"
    }

    fn focus(&self, s: &Sess) -> bool {
        let p = if s.term_pid > 0 { s.term_pid } else { s.pid };
        if p <= 0 || !is_alive(p) {
            return false;
        }
        let _ = activate_session_pane(s); // best-effort; window focus still helps
        focus_gui(p)
    }

    /// Kill the session's pane — precise (no keystroke chord), so no foreground guard needed.
    /// If the pane can't be identified confidently, do nothing but report handled: never close
    /// the wrong pane, and don't let the caller kill the process tree.
    fn close(&self, s: &Sess) -> bool {
        if s.term_pid <= 0 {
            return false;
        }
        if !is_alive(s.term_pid) {
            return false; // terminal gone — generic close can deal with the leftovers
        }
        if let Some(pane) = find_session_pane(s) {
            let _ = cli(s.term_pid, &["kill-pane", "--pane-id", &pane.to_string()]);
        }
        true
    }

    fn new_session(&self, cmds: &[String]) -> bool {
        let (pid, pane) = match live_instance() {
            // Running instance: spawn a new tab next to the session's siblings.
            Some(pid) => {
                let Some(win) = first_window_id(pid) else {
                    return false;
                };
                let out = cli(pid, &["spawn", "--window-id", &win.to_string()]);
                let Some(pane) = out.as_deref().and_then(|s| s.trim().parse::<i64>().ok()) else {
                    return false;
                };
                (pid, pane)
            }
            // No WezTerm open — start one and use its first pane.
            None => {
                let Ok(child) = Command::new(gui_exe()).spawn() else {
                    return false;
                };
                let pid = child.id() as i64;
                // The sock appears once the mux is listening; only then is the CLI usable.
                let sock = sock_path(pid);
                for _ in 0..40 {
                    std::thread::sleep(Duration::from_millis(250));
                    if sock.exists() {
                        break;
                    }
                }
                match first_pane_id(pid) {
                    Some(pane) => (pid, pane),
                    None => return false,
                }
            }
        };
        std::thread::sleep(Duration::from_millis(600)); // let the shell come up
        for cmd in cmds {
            // The pty (ConPTY on Windows) queues input the shell hasn't read yet, so this is
            // lossless even if the profile is still printing. "\r" runs the command, exactly
            // like pressing Enter.
            let _ = cli_stdin(
                pid,
                &["send-text", "--no-paste", "--pane-id", &pane.to_string()],
                format!("{cmd}\r"),
            );
            std::thread::sleep(Duration::from_millis(150));
        }
        focus_gui(pid);
        true
    }
}

// ---- platform islands ----

#[cfg(windows)]
fn exe() -> PathBuf {
    let p = PathBuf::from(r"C:\Program Files\WezTerm\wezterm.exe");
    if p.exists() {
        p
    } else {
        PathBuf::from("wezterm.exe") // fall back to PATH
    }
}

#[cfg(windows)]
fn gui_exe() -> PathBuf {
    let p = PathBuf::from(r"C:\Program Files\WezTerm\wezterm-gui.exe");
    if p.exists() {
        p
    } else {
        PathBuf::from("wezterm-gui.exe")
    }
}

#[cfg(target_os = "macos")]
fn exe() -> PathBuf {
    for p in [
        "/opt/homebrew/bin/wezterm",
        "/usr/local/bin/wezterm",
        "/Applications/WezTerm.app/Contents/MacOS/wezterm",
    ] {
        if PathBuf::from(p).exists() {
            return PathBuf::from(p);
        }
    }
    PathBuf::from("wezterm") // fall back to PATH
}

#[cfg(target_os = "macos")]
fn gui_exe() -> PathBuf {
    let p = PathBuf::from("/Applications/WezTerm.app/Contents/MacOS/wezterm-gui");
    if p.exists() {
        p
    } else {
        PathBuf::from("wezterm-gui")
    }
}

/// process_map() names: lowercase exe basename on Windows, full comm path on unix.
#[cfg(windows)]
fn is_gui_process(name: &str) -> bool {
    name == "wezterm-gui.exe"
}

#[cfg(target_os = "macos")]
fn is_gui_process(name: &str) -> bool {
    std::path::Path::new(name)
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("wezterm-gui"))
}

#[cfg(windows)]
fn focus_gui(pid: i64) -> bool {
    let h = main_window_for_pid(pid);
    if h.is_null() {
        return false;
    }
    focus_window(h);
    true
}

/// No per-window handles on macOS: activate the app and let the already-sent activate-pane
/// select the right tab. Plain `activate` needs only the one-time Automation consent.
#[cfg(target_os = "macos")]
fn focus_gui(_pid: i64) -> bool {
    Command::new("osascript")
        .args(["-e", r#"tell application "WezTerm" to activate"#])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ---- wezterm cli plumbing (shared) ----

fn sock_path(term_pid: i64) -> PathBuf {
    home()
        .join(".local")
        .join("share")
        .join("wezterm")
        .join(format!("gui-sock-{term_pid}"))
}

fn cli_command(term_pid: i64, args: &[&str]) -> Command {
    let mut c = Command::new(exe());
    c.env("WEZTERM_UNIX_SOCKET", sock_path(term_pid))
        .arg("cli")
        .args(args);
    #[cfg(windows)]
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

/// Run `wezterm cli <args>` against the instance owning `term_pid`; stdout on success.
fn cli(term_pid: i64, args: &[&str]) -> Option<String> {
    let out = cli_command(term_pid, args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Like `cli`, but feeding `text` to the child's stdin (send-text reads it there when no
/// literal argument is given, which sidesteps any argv quoting of user commands).
fn cli_stdin(term_pid: i64, args: &[&str], text: String) -> Option<()> {
    use std::io::Write;
    let mut child = cli_command(term_pid, args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(text.as_bytes()).ok()?;
    child.wait().ok().filter(|s| s.success()).map(|_| ())
}

fn live_instance() -> Option<i64> {
    process_map()
        .iter()
        .find(|(_, (_, name))| is_gui_process(name))
        .map(|(p, _)| *p)
}

struct Pane {
    id: i64,
    window: i64,
    name: String,
    tty: String,
}

/// One entry per pane, in mux order. Name prefers the explicit tab title over the OSC title
/// the shell/Claude sets, mirroring what the tab bar displays. tty is the pane's pty device
/// ("/dev/ttys007"); absent on Windows, where it stays empty.
fn panes(term_pid: i64) -> Vec<Pane> {
    let Some(json) = cli(term_pid, &["list", "--format", "json"]) else {
        return Vec::new();
    };
    let Ok(list) = serde_json::from_str::<Vec<Value>>(&json) else {
        return Vec::new();
    };
    list.iter()
        .map(|p| {
            let tab = p.get("tab_title").and_then(Value::as_str).unwrap_or("");
            let title = p.get("title").and_then(Value::as_str).unwrap_or("");
            Pane {
                id: p.get("pane_id").and_then(Value::as_i64).unwrap_or(-1),
                window: p.get("window_id").and_then(Value::as_i64).unwrap_or(0),
                name: if tab.is_empty() { title } else { tab }.to_string(),
                tty: p
                    .get("tty_name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            }
        })
        .filter(|p| p.id >= 0)
        .collect()
}

fn first_window_id(term_pid: i64) -> Option<i64> {
    panes(term_pid).first().map(|p| p.window)
}

fn first_pane_id(term_pid: i64) -> Option<i64> {
    panes(term_pid).first().map(|p| p.id)
}

/// The session's pane: exact tty match where available (unix), else the shared fuzzy title
/// policy — None means "not confident".
fn find_session_pane(s: &Sess) -> Option<i64> {
    if s.term_pid <= 0 {
        return None; // no instance pid → no socket to talk to
    }
    let panes = panes(s.term_pid);
    if !s.tty.is_empty() {
        let dev = format!("/dev/{}", s.tty);
        if let Some(p) = panes.iter().find(|p| p.tty == dev) {
            return Some(p.id);
        }
    }
    let target = Target::new(&s.tab_title, &s.topic);
    if target.is_empty() {
        return None;
    }
    let names: Vec<String> = panes.iter().map(|p| p.name.clone()).collect();
    tabmatch::choose(&names, &target).map(|i| panes[i].id)
}

fn activate_session_pane(s: &Sess) -> bool {
    match find_session_pane(s) {
        Some(pane) => cli(
            s.term_pid,
            &["activate-pane", "--pane-id", &pane.to_string()],
        )
        .is_some(),
        None => false,
    }
}
