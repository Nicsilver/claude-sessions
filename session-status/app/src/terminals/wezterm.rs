//! WezTerm. Unlike WT it has a real control CLI (`wezterm cli`), so focus/close/spawn go
//! through `cli list / activate-pane / kill-pane / spawn / send-text` — no UI Automation and
//! no synthetic keystrokes. Panes are matched to sessions by title via tabmatch, same policy
//! as WT tabs.
//!
//! Gotcha (wezterm/wezterm#4456): the GUI publishes only the socket *filename*, and AF_UNIX
//! connect on Windows resolves relative paths against the client's cwd — so `wezterm cli`
//! from outside WezTerm can't connect on its own. We bypass discovery entirely by setting
//! WEZTERM_UNIX_SOCKET to the absolute `gui-sock-<pid>` path built from the terminal pid we
//! already track; this also makes stale socket files from dead instances a non-issue.

use super::tabmatch::{self, Target};
use super::Terminal;
use crate::model::Sess;
use crate::paths::home;
use crate::platform::{focus_window, is_alive, main_window_for_pid, process_map};
use serde_json::Value;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

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
        let h = main_window_for_pid(p);
        if h.is_null() {
            return false;
        }
        let _ = activate_session_pane(s); // best-effort; window focus still helps
        focus_window(h);
        true
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
            // ConPTY queues input the shell hasn't read yet, so this is lossless even if the
            // profile is still printing. "\r" runs the command, exactly like pressing Enter.
            let _ = cli_stdin(
                pid,
                &["send-text", "--no-paste", "--pane-id", &pane.to_string()],
                format!("{cmd}\r"),
            );
            std::thread::sleep(Duration::from_millis(150));
        }
        let h = main_window_for_pid(pid);
        if !h.is_null() {
            focus_window(h);
        }
        true
    }
}

// ---- wezterm cli plumbing ----

fn exe() -> PathBuf {
    let p = PathBuf::from(r"C:\Program Files\WezTerm\wezterm.exe");
    if p.exists() {
        p
    } else {
        PathBuf::from("wezterm.exe") // fall back to PATH
    }
}

fn gui_exe() -> PathBuf {
    let p = PathBuf::from(r"C:\Program Files\WezTerm\wezterm-gui.exe");
    if p.exists() {
        p
    } else {
        PathBuf::from("wezterm-gui.exe")
    }
}

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
        .args(args)
        .creation_flags(CREATE_NO_WINDOW);
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
        .find(|(_, (_, name))| name == "wezterm-gui.exe")
        .map(|(p, _)| *p)
}

/// (pane_id, name) per pane, in mux order. Name prefers the explicit tab title over the
/// OSC title the shell/Claude sets, mirroring what the tab bar displays.
fn panes(term_pid: i64) -> Vec<(i64, i64, String)> {
    let Some(json) = cli(term_pid, &["list", "--format", "json"]) else {
        return Vec::new();
    };
    let Ok(list) = serde_json::from_str::<Vec<Value>>(&json) else {
        return Vec::new();
    };
    list.iter()
        .map(|p| {
            let id = p.get("pane_id").and_then(Value::as_i64).unwrap_or(-1);
            let win = p.get("window_id").and_then(Value::as_i64).unwrap_or(0);
            let tab = p.get("tab_title").and_then(Value::as_str).unwrap_or("");
            let title = p.get("title").and_then(Value::as_str).unwrap_or("");
            let name = if tab.is_empty() { title } else { tab };
            (id, win, name.to_string())
        })
        .filter(|(id, _, _)| *id >= 0)
        .collect()
}

fn first_window_id(term_pid: i64) -> Option<i64> {
    panes(term_pid).first().map(|(_, win, _)| *win)
}

fn first_pane_id(term_pid: i64) -> Option<i64> {
    panes(term_pid).first().map(|(id, _, _)| *id)
}

/// The session's pane, via the shared fuzzy title policy — None means "not confident".
fn find_session_pane(s: &Sess) -> Option<i64> {
    if s.term_pid <= 0 {
        return None; // no instance pid → no socket to talk to
    }
    let target = Target::new(&s.tab_title, &s.topic);
    if target.is_empty() {
        return None;
    }
    let panes = panes(s.term_pid);
    let names: Vec<String> = panes.iter().map(|(_, _, n)| n.clone()).collect();
    tabmatch::choose(&names, &target).map(|i| panes[i].0)
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
