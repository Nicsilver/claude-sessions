//! Standalone terminal dashboard — Rust port of bin/dashboard.py.
//!
//! A no-dependency TUI that live-renders every Claude session's state by reading
//! ~/.claude/session-status/state/*.json. Run it in its own terminal window:
//!
//!     dashboard
//!
//! Ctrl-C to quit. Prunes dead sessions (parent process gone).

use serde_json::Value;
use session_status::{age, alive, order, state_dir};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";

static RUNNING: AtomicBool = AtomicBool::new(true);

extern "C" fn on_sigint(_sig: libc::c_int) {
    RUNNING.store(false, Ordering::SeqCst);
}

fn glyph(state: &str) -> &'static str {
    match state {
        "needs" => "\u{1F534}",    // 🔴
        "yourturn" => "\u{1F7E1}", // 🟡
        "working" => "\u{1F7E2}",  // 🟢
        "done" => "✅",
        "idle" => "⚪",
        _ => "·",
    }
}

fn color(state: &str) -> &'static str {
    match state {
        "needs" => "\x1b[91m",
        "yourturn" => "\x1b[93m",
        "working" => "\x1b[92m",
        "done" => "\x1b[90m",
        "idle" => "\x1b[37m",
        _ => "",
    }
}

fn label_for(state: &str) -> String {
    match state {
        "needs" => "NEEDS YOU".to_string(),
        "yourturn" => "your turn".to_string(),
        "working" => "working".to_string(),
        "done" => "done · safe to close".to_string(),
        "idle" => "idle".to_string(),
        other => other.to_string(),
    }
}

fn main() {
    unsafe {
        libc::signal(libc::SIGINT, on_sigint as *const () as usize);
    }
    let mut stdout = std::io::stdout();
    while RUNNING.load(Ordering::SeqCst) {
        let out = render();
        let _ = write!(stdout, "\x1b[2J\x1b[H{}\n", out);
        let _ = stdout.flush();
        // sleep ~1s but wake promptly on Ctrl-C
        for _ in 0..10 {
            if !RUNNING.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let _ = write!(stdout, "\n");
    let _ = stdout.flush();
}

fn load() -> Vec<Value> {
    let dir = state_dir();
    let rd = match std::fs::read_dir(&dir) {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let rec: Value = match std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(v) => v,
            None => continue,
        };
        let pid = rec
            .get("pid")
            .and_then(|v| v.as_i64())
            .filter(|&p| p != 0)
            .or_else(|| rec.get("ppid").and_then(|v| v.as_i64()))
            .unwrap_or(0);
        if !alive(pid) {
            let _ = std::fs::remove_file(&p);
            continue;
        }
        out.push(rec);
    }
    out
}

fn st(rec: &Value) -> &str {
    rec.get("state").and_then(|v| v.as_str()).unwrap_or("")
}

fn updated_at(rec: &Value) -> f64 {
    rec.get("updated_at").and_then(|v| v.as_f64()).unwrap_or(0.0)
}

/// Truncate to `width` chars and left-pad to that width (Python `s[:w].ljust(w)`).
fn pad_trunc(s: &str, width: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() >= width {
        chars[..width].iter().collect()
    } else {
        let mut out: String = chars.iter().collect();
        out.extend(std::iter::repeat(' ').take(width - chars.len()));
        out
    }
}

/// First `n` chars of a string (char-safe).
fn take_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn clock() -> String {
    std::process::Command::new("date")
        .arg("+%H:%M:%S")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn render() -> String {
    let mut recs = load();
    recs.sort_by(|a, b| {
        order(st(a)).cmp(&order(st(b))).then_with(|| {
            // -updated_at => most recent first
            updated_at(b)
                .partial_cmp(&updated_at(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });

    let count = |state: &str| recs.iter().filter(|r| st(r) == state).count();
    let needs = count("needs");
    let yourturn = count("yourturn");
    let working = count("working");
    let done = count("done");

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "{b}Claude sessions{r}   \u{1F534} {n} need you   \u{1F7E1} {y} your turn   \u{1F7E2} {w} working   ✅ {d} done    {dim}{t}{r}",
        b = BOLD, r = RESET, dim = DIM, n = needs, y = yourturn, w = working, d = done, t = clock()
    ));
    lines.push(String::new());

    if recs.is_empty() {
        lines.push(format!(
            "{dim}(no active sessions — start a Claude session to populate){r}",
            dim = DIM,
            r = RESET
        ));
    }

    for rec in &recs {
        let state = st(rec);
        let g = glyph(state);
        let c = color(state);
        let mut label = rec
            .get("topic")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("?")
            .to_string();
        let has_tty = rec
            .get("tty")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if !has_tty {
            label.push_str(" (ide)");
        }
        let topic = pad_trunc(&label, 42);
        let lab = label_for(state);
        let a = age(updated_at(rec));
        let msg = if (state == "needs" || state == "yourturn")
            && rec.get("message").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false)
        {
            let m = take_chars(rec.get("message").and_then(|v| v.as_str()).unwrap_or(""), 54);
            format!("  {dim}{m}{r}", dim = DIM, m = m, r = RESET)
        } else {
            String::new()
        };
        // "{c}{g} {topic}{r} {c}{lab:<22}{r}{dim}{a:>5}{r}{msg}"
        lines.push(format!(
            "{c}{g} {topic}{r} {c}{lab:<22}{r}{dim}{a:>5}{r}{msg}",
            c = c, g = g, topic = topic, r = RESET, lab = lab, dim = DIM, a = a, msg = msg
        ));
    }
    lines.join("\n")
}
