//! Apple Terminal (Terminal.app): tabs expose their controlling tty to AppleScript, so focus
//! matches the session's `tty` field directly. Tab enumeration is try-wrapped — windows can
//! exist without scriptable tabs (e.g. mid-close) and would otherwise abort the whole script.

use super::Terminal;
use crate::model::Sess;

pub struct MacTerminal;

fn osascript(script: &str) -> Option<String> {
    let o = std::process::Command::new("osascript").arg("-e").arg(script).output().ok()?;
    if !o.status.success() {
        eprintln!("osascript failed: {}", String::from_utf8_lossy(&o.stderr).trim());
        return None;
    }
    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
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
        let Some(dev) = dev_tty(&s.tty) else { return false };
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
        let Some(dev) = dev_tty(&s.tty) else { return false };
        let win = window_of(&dev);
        let Ok(o) = std::process::Command::new("ps").args(["-t", &s.tty, "-o", "pid="]).output()
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
                osascript(&format!(r#"tell application "Terminal" to close window id {wid}"#));
            }
            return true;
        }
        any
    }

    fn new_session(&self, cmds: &[String]) -> bool {
        let mut script = String::from("tell application \"Terminal\"\nactivate\n");
        if cmds.is_empty() {
            script.push_str("do script \"\"\n");
        } else {
            for (i, cmd) in cmds.iter().enumerate() {
                let esc = cmd.replace('\\', "\\\\").replace('"', "\\\"");
                if i == 0 {
                    script.push_str(&format!("set newTab to do script \"{esc}\"\n"));
                } else {
                    script.push_str(&format!("do script \"{esc}\" in newTab\n"));
                }
            }
        }
        script.push_str("end tell");
        osascript(&script).is_some()
    }
}
