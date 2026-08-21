//! Apple Terminal (Terminal.app): tabs expose their controlling tty to AppleScript, so focus
//! matches the session's `tty` field directly. Tab enumeration is try-wrapped — windows can
//! exist without scriptable tabs (e.g. mid-close) and would otherwise abort the whole script.

use super::Terminal;
use crate::model::Sess;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct MacTerminal;

/// Raised when a tab was refused for want of Accessibility permission, so the GUI can explain
/// once why `+` opened a window instead. Read-and-clear via `take_accessibility_denied`.
static ACCESSIBILITY_DENIED: AtomicBool = AtomicBool::new(false);

/// True once per denial: the GUI polls this after spawning a session.
pub fn take_accessibility_denied() -> bool {
    ACCESSIBILITY_DENIED.swap(false, Ordering::Relaxed)
}

fn osascript_result(script: &str) -> Result<String, String> {
    let o = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| e.to_string())?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn osascript(script: &str) -> Option<String> {
    match osascript_result(script) {
        Ok(out) => Some(out),
        Err(e) => {
            eprintln!("osascript failed: {e}");
            None
        }
    }
}

/// "ttys003" → "/dev/ttys003", sanitized to the charset ps emits so it can be embedded in an
/// AppleScript string literal safely.
fn dev_tty(tty: &str) -> Option<String> {
    if tty.is_empty() || !tty.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(format!("/dev/{tty}"))
}

/// The window hosting the tty's tab, as (window id, tab count); None when no tab matches.
fn window_of(dev: &str) -> Option<(i64, i64)> {
    let out = osascript(&format!(
        r#"tell application "Terminal"
            repeat with w in windows
                try
                    repeat with t in tabs of w
                        if tty of t is "{dev}" then return ((id of w) as text) & "," & ((count of tabs of w) as text)
                    end repeat
                end try
            end repeat
        end tell
        return """#
    ))?;
    let (wid, tabs) = out.split_once(',')?;
    Some((wid.trim().parse().ok()?, tabs.trim().parse().ok()?))
}

impl Terminal for MacTerminal {
    fn id(&self) -> &'static str {
        "terminal"
    }

    fn label(&self) -> &'static str {
        "Terminal.app"
    }

    fn focus(&self, s: &Sess) -> bool {
        let Some(dev) = dev_tty(&s.tty) else {
            return false;
        };
        osascript(&format!(
            r#"tell application "Terminal"
                repeat with w in windows
                    try
                        repeat with t in tabs of w
                            if tty of t is "{dev}" then
                                set selected of t to true
                                set index of w to 1
                                activate
                                return
                            end if
                        end repeat
                    end try
                end repeat
            end tell"#
        ))
        .is_some()
    }

    fn close(&self, s: &Sess) -> bool {
        // No scriptable per-tab close — end the tab's session instead: SIGHUP everything on
        // the tty, like dropping the connection. A single-tab window is then closed outright
        // (the shell is already dead, so Terminal doesn't prompt); a dead tab in a multi-tab
        // window is left showing [Process completed].
        let Some(dev) = dev_tty(&s.tty) else {
            return false;
        };
        let win = window_of(&dev);
        let Ok(o) = std::process::Command::new("ps")
            .args(["-t", &s.tty, "-o", "pid="])
            .output()
        else {
            return false;
        };
        let mut any = false;
        for pid in String::from_utf8_lossy(&o.stdout).split_whitespace() {
            if let Ok(p) = pid.parse::<i32>() {
                unsafe {
                    any |= libc::kill(p, libc::SIGHUP) == 0;
                }
            }
        }
        if let Some((wid, tabs)) = win {
            if tabs == 1 {
                std::thread::sleep(std::time::Duration::from_millis(400));
                osascript(&format!(
                    r#"tell application "Terminal" to close window id {wid}"#
                ));
            }
            return true;
        }
        any
    }

    fn new_session(&self, cmds: &[String]) -> bool {
        // A bare `do script` always opens a new window, so a tab has to be arranged first;
        // when that cannot be done we still open a window, so `+` always does something.
        let target = if open_tab() { " in front window" } else { "" };

        let mut script = String::from("tell application \"Terminal\"\nactivate\n");
        if cmds.is_empty() {
            script.push_str(&format!("do script \"\"{target}\n"));
        } else {
            for (i, cmd) in cmds.iter().enumerate() {
                let esc = cmd.replace('\\', "\\\\").replace('"', "\\\"");
                if i == 0 {
                    script.push_str(&format!("set newTab to do script \"{esc}\"{target}\n"));
                } else {
                    script.push_str(&format!("do script \"{esc}\" in newTab\n"));
                }
            }
        }
        script.push_str("end tell");
        osascript(&script).is_some()
    }
}

/// How many tabs Terminal has open, across every window. Terminal's AppleScript `windows`
/// collection has one entry per *tab*, not per window — the two only agree when every window
/// holds a single tab, which is why this is the count worth watching.
fn tab_count() -> Option<i64> {
    osascript(r#"tell application "Terminal" to count windows"#)?
        .parse()
        .ok()
}

fn is_running() -> bool {
    osascript(r#"tell application "System Events" to (name of processes) contains "Terminal""#)
        .is_some_and(|s| s == "true")
}

/// Get a fresh tab for the caller to run in, reporting whether the front window now holds one.
///
/// A cold Terminal is the easy case: launching it produces a window we can simply use. Otherwise
/// the tab has to come from the Shell menu — Terminal scripts no "make new tab", and a synthetic
/// ⌘T is not dependable (a keyboard remapper can swallow it before Terminal sees it), so we click
/// the menu item itself. Either needs Accessibility, and a refusal is reported so the GUI can say
/// so once.
fn open_tab() -> bool {
    if !is_running() {
        // The window Terminal opens on launch is ours to use; `do script` would add a second one.
        return osascript(r#"tell application "Terminal" to activate"#).is_some()
            && wait_for(|| tab_count().unwrap_or(0) >= 1);
    }
    let Some(before) = tab_count() else {
        return false;
    };
    if !click_new_tab() {
        return false;
    }
    // Only claim the tab once it exists: `do script … in front window` targets whatever tab is
    // in front, so acting on an unconfirmed click would run the command on top of a live shell.
    wait_for(|| tab_count().is_some_and(|n| n > before))
}

/// Click Terminal's "New Tab" item, found by its ⌘T shortcut rather than by name so a localised
/// menu bar still works.
fn click_new_tab() -> bool {
    if osascript(r#"tell application "Terminal" to activate"#).is_none() {
        return false;
    }
    match osascript_result(
        r#"tell application "System Events" to tell process "Terminal"
            repeat with m in menus of menu bar 1
                repeat with mi in menu items of m
                    try
                        repeat with sub in menu items of menu 1 of mi
                            if (value of attribute "AXMenuItemCmdChar" of sub) is "T" and (value of attribute "AXMenuItemCmdModifiers" of sub) is 0 then
                                click sub
                                return "ok"
                            end if
                        end repeat
                    end try
                end repeat
            end repeat
            return "none"
        end tell"#,
    ) {
        Ok(out) => out == "ok",
        Err(e) => {
            // -1002 / "not allowed assistive access": Accessibility is not granted to this app.
            if e.contains("1002") || e.contains("not allowed") {
                ACCESSIBILITY_DENIED.store(true, Ordering::Relaxed);
            }
            eprintln!("new tab failed, falling back to a new window: {e}");
            false
        }
    }
}

/// Poll `cond` for up to two seconds — a tab exists a beat after the click, not on return.
fn wait_for(cond: impl Fn() -> bool) -> bool {
    for _ in 0..20 {
        if cond() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}
