//! Claude Code session-status recorder — Rust port of bin/record.py.
//!
//! Invoked by Claude Code hooks. Reads the hook JSON on stdin and writes a tiny
//! per-session state file to ~/.claude/session-status/state/<session_id>.json.
//! Notifications are posted by the menu-bar app, which watches that state.
//!
//! Usage (from a hook):  record <state>
//!   state in: start | working | needs | done | end
//!
//! Like the Python original this is intentionally fail-safe: any error (or panic)
//! ends with exit 0 so it can never break a Claude Code session. Fully reversible —
//! delete the hooks block in settings.json and this directory to return to stock.

use serde_json::{json, Value};
use session_status::{now_secs, state_dir};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// Titles Claude sometimes generates that describe the *conversation* rather than the
// task — treated as low-value so we fall through to the latest real prompt.
const META_TITLES: &[&str] = &[
    "dig deeper and follow up", "dig deeper", "follow up", "follow-up", "continue",
    "keep going", "next", "next steps", "more", "help", "untitled", "new chat",
    "wip", "test", "testing", "debugging", "conversation",
];
const TRIVIAL_PROMPTS: &[&str] = &[
    "yes", "no", "y", "n", "ok", "okay", "sure", "go", "do it", "continue", "proceed",
    "next", "commit", "push", "thanks", "ty", "yep", "yeah", "nope", "stop", "wait",
    "please", "done", "good", "perfect", "nice", "cool", "great", "fix it", "go on",
    "keep going", "carry on",
];
const DEFAULT_BRANCHES: &[&str] = &["main", "master", "develop", "trunk"];

fn main() {
    // Silence panic output (a hook's stderr is noise) and guarantee exit 0 even if
    // something unexpected panics — matches the Python "any error exits 0" contract.
    std::panic::set_hook(Box::new(|_| {}));
    let _ = std::panic::catch_unwind(run);
    std::process::exit(0);
}

fn run() {
    let state = std::env::args().nth(1).unwrap_or_else(|| "working".to_string());

    let mut raw = String::new();
    if std::io::stdin().read_to_string(&mut raw).is_err() {
        return;
    }

    // Hand control straight back to Claude: the payload is already in memory, so fork
    // and let the PARENT exit now (~ the process-spawn floor). All the real work —
    // registry scan, git/ps, the ~1.2s transcript-flush wait on `done` — runs in the
    // detached child, OFF the hook's critical path. RECORD_SYNC=1 stays synchronous
    // (used by the test/bench harness).
    if std::env::var_os("RECORD_SYNC").is_none() {
        detach();
    }

    let data: Value = if raw.trim().is_empty() {
        json!({})
    } else {
        match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => return,
        }
    };

    let sid = match data.get("session_id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return,
    };

    let sdir = state_dir();
    if std::fs::create_dir_all(&sdir).is_err() {
        return;
    }
    let path = sdir.join(format!("{}.json", sid));

    if state == "end" {
        let _ = std::fs::remove_file(&path);
        return;
    }

    let prev: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));

    let reg = registry_for(&sid);
    let ppid = unsafe { libc::getppid() as i64 };

    // cwd: registry.cwd -> data.cwd -> prev.cwd -> getcwd()
    let cwd = first_nonempty(&[
        reg.as_ref().and_then(|r| r.get("cwd")).and_then(|v| v.as_str()),
        data.get("cwd").and_then(|v| v.as_str()),
        prev.get("cwd").and_then(|v| v.as_str()),
    ])
    .map(|s| s.to_string())
    .unwrap_or_else(getcwd);

    // pid: registry.pid (if non-zero) -> parent pid
    let pid = reg
        .as_ref()
        .and_then(|r| r.get("pid"))
        .and_then(|v| v.as_i64())
        .filter(|&p| p != 0)
        .unwrap_or(ppid);

    let transcript = data.get("transcript_path").and_then(|v| v.as_str());

    // Controlling terminal — reuse the cached value when the pid is unchanged (skips a `ps`
    // spawn on the hot path). Computed up here because the label can come from the IDE
    // terminal tab name, which is keyed by tty.
    let tty = match (
        prev.get("pid").and_then(|v| v.as_i64()),
        prev.get("tty").and_then(|v| v.as_str()).filter(|s| !s.is_empty()),
    ) {
        (Some(prev_pid), Some(prev_tty)) if prev_pid == pid => prev_tty.to_string(),
        _ => tty_of(pid),
    };

    // topic: reuse the cached label unless a meaningful event (a custom rename, or an IDE
    // tab name) means we should recompute it.
    let mut topic: Option<String> = prev
        .get("topic")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let prompt_present = data
        .get("prompt")
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if custom_label(&sid).is_some()
        || tab_label(&tty).is_some()
        || topic.is_none()
        || state == "start"
        || state == "done"
        || (state == "working" && prompt_present)
    {
        let d = derive_label(&sid, reg.as_ref(), &cwd, transcript, &tty);
        if !d.is_empty() {
            topic = Some(d);
        }
    }

    let mut eff = if state == "start" { "idle".to_string() } else { state.clone() };
    let mut msg = sanitize(data.get("message").and_then(|v| v.as_str()));

    // Suppress the benign ~60s idle ping so a finished session stays "done · safe to
    // close" instead of decaying to needs-me. Everything else still escalates.
    if eff == "needs" && msg.to_lowercase().contains("waiting for") {
        return;
    }

    // Stop hook: tell "Claude finished" (done) apart from "Claude is waiting on you"
    // (yourturn — greeting, question, offer) via the trailing ⏳/✅ marker.
    if state == "done" {
        let (verdict, snippet) = classify_turn(transcript);
        if verdict == "yourturn" {
            eff = "yourturn".to_string();
            if msg.is_empty() {
                msg = if !snippet.is_empty() { snippet } else { "your turn".to_string() };
            }
        }
    }

    let mut rec = serde_json::Map::new();
    rec.insert("session_id".into(), Value::String(sid.clone()));
    rec.insert("state".into(), Value::String(eff.clone()));
    rec.insert(
        "topic".into(),
        match &topic {
            Some(s) => Value::String(s.clone()),
            None => Value::Null,
        },
    );
    rec.insert("cwd".into(), Value::String(cwd));
    rec.insert("pid".into(), json!(pid));
    rec.insert("tty".into(), Value::String(tty));
    rec.insert("ppid".into(), json!(ppid));
    rec.insert("updated_at".into(), json!(now_secs()));
    if (eff == "needs" || eff == "yourturn") && !msg.is_empty() {
        rec.insert("message".into(), Value::String(msg));
    }

    write_atomic(&path, &Value::Object(rec));
}

/// First non-empty option in the list (mirrors Python's `a or b or c` truthiness).
fn first_nonempty<'a>(opts: &[Option<&'a str>]) -> Option<&'a str> {
    for o in opts {
        if let Some(s) = o {
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

fn getcwd() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_default()
}

/// A user-set rename for this session (from the widget's ⌥-click), if any.
fn custom_label(sid: &str) -> Option<String> {
    let p = session_status::base_dir().join("labels.json");
    let data = std::fs::read_to_string(&p).ok()?;
    let v: Value = serde_json::from_str(&data).ok()?;
    let s = v.get(sid)?.as_str()?;
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// IntelliJ's stock terminal tab titles ("Local", "Local (2)", "Local(2)") carry no
/// meaning — don't use them as a session label.
fn is_default_tab(name: &str) -> bool {
    let base: String = name.chars().filter(|c| !c.is_whitespace()).collect();
    base == "Local" || (base.starts_with("Local(") && base.ends_with(')'))
}

/// The IntelliJ terminal tab name for this tty (e.g. "TT AGI-18033"), published by the
/// Claude Sessions plugin under ~/.claude/session-status/tab-names/<project>.json. Returns
/// it only when it's a real, non-default tab title refreshed in the last 15s (so a closed
/// project's stale entry is ignored). Newest fresh entry wins across projects.
fn tab_label(tty: &str) -> Option<String> {
    if tty.is_empty() {
        return None;
    }
    let dir = session_status::base_dir().join("tab-names");
    let now = now_secs();
    let mut best: Option<(String, f64)> = None;
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let data = match std::fs::read_to_string(entry.path()) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let v: Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let e = match v.get(tty) {
            Some(e) => e,
            None => continue,
        };
        let name = e.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
        let ts = e.get("ts").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if name.is_empty() || is_default_tab(name) || (now - ts) > 15.0 {
            continue;
        }
        if best.as_ref().map_or(true, |(_, bts)| ts > *bts) {
            best = Some((name.to_string(), ts));
        }
    }
    best.map(|(n, _)| n)
}

/// A user prompt worth using as a label — not a command, caveat, or filler.
fn is_substantial(txt: &str) -> bool {
    if txt.is_empty() || txt.starts_with('<') || txt.starts_with("Caveat:") {
        return false;
    }
    let lowered = txt.trim().to_lowercase();
    let s = lowered.trim_end_matches(['.', '!', '?']);
    s.chars().count() >= 6 && !TRIVIAL_PROMPTS.contains(&s)
}

/// Trim to `n` chars on a word boundary with an ellipsis (Python `short`).
fn short(txt: &str, n: usize) -> String {
    let txt = txt.trim();
    let chars: Vec<char> = txt.chars().collect();
    if chars.len() <= n {
        return txt.to_string();
    }
    let head: String = chars[..n].iter().collect();
    let base = match head.rsplit_once(' ') {
        Some((before, _)) if !before.is_empty() => before.to_string(),
        _ => head,
    };
    format!("{}…", base)
}

/// A human-meaningful session label, best-first.
fn derive_label(sid: &str, reg: Option<&Value>, cwd: &str, transcript: Option<&str>, tty: &str) -> String {
    if let Some(c) = custom_label(sid) {
        return c;
    }
    if let Some(t) = tab_label(tty) {
        return t;
    }
    if let Some(reg) = reg {
        if let Some(name) = reg.get("name").and_then(|v| v.as_str()) {
            if !name.trim().is_empty() {
                return name.trim().to_string();
            }
        }
    }
    let (title, latest) = transcript_titles(transcript);
    let br = git_branch(cwd);
    if !br.is_empty() && !DEFAULT_BRANCHES.contains(&br.to_lowercase().as_str()) {
        return short(&br, 44);
    }
    if !title.is_empty() && !META_TITLES.contains(&title.trim().to_lowercase().as_str()) {
        return title;
    }
    if !latest.is_empty() {
        return short(&latest, 44);
    }
    if !title.is_empty() {
        return title;
    }
    let base = basename(cwd);
    if !base.is_empty() {
        base
    } else {
        cwd.to_string()
    }
}

fn basename(cwd: &str) -> String {
    let trimmed = cwd.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some((_, b)) => b.to_string(),
        None => trimmed.to_string(),
    }
}

/// Claude's native per-session registry entry (~/.claude/sessions/<pid>.json) matching
/// this session id, picking the most-recently-updated if a session was resumed.
fn registry_for(sid: &str) -> Option<Value> {
    let dir = session_status::home().join(".claude").join("sessions");
    let rd = std::fs::read_dir(&dir).ok()?;
    let mut best: Option<Value> = None;
    let mut best_updated = 0.0_f64;
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let v: Value = match std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(v) => v,
            None => continue,
        };
        if !v.is_object() {
            continue;
        }
        if v.get("sessionId").and_then(|x| x.as_str()) != Some(sid) {
            continue;
        }
        let upd = v.get("updatedAt").and_then(|x| x.as_f64()).unwrap_or(0.0);
        if best.is_none() || upd > best_updated {
            best_updated = upd;
            best = Some(v);
        }
    }
    best
}

/// (title, latest_prompt): title = latest custom/ai title; latest_prompt = most recent
/// *substantial* user message.
fn transcript_titles(path: Option<&str>) -> (String, String) {
    let mut title = String::new();
    let mut latest = String::new();
    for obj in read_all_entries(path) {
        let o = match obj.as_object() {
            Some(o) => o,
            None => continue,
        };
        match o.get("type").and_then(|x| x.as_str()) {
            Some("ai-title") => {
                if let Some(raw) = o.get("aiTitle").and_then(|x| x.as_str()) {
                    if !raw.is_empty() {
                        title = raw.trim().to_string();
                    }
                }
            }
            Some("custom-title") => {
                if let Some(raw) = o.get("customTitle").and_then(|x| x.as_str()) {
                    if !raw.is_empty() {
                        title = raw.trim().to_string();
                    }
                }
            }
            Some("user") => {
                let m = match o.get("message") {
                    Some(Value::Object(mm)) => mm,
                    _ => o,
                };
                let txt = extract_text(m.get("content")).trim().replace('\n', " ");
                if is_substantial(&txt) {
                    latest = txt;
                }
            }
            _ => {}
        }
    }
    (title, latest)
}

fn git_branch(cwd: &str) -> String {
    if let Some((code, out)) = run_capture(
        &["git", "-C", cwd, "rev-parse", "--abbrev-ref", "HEAD"],
        Duration::from_millis(1500),
    ) {
        let b = out.trim();
        if code == 0 && !b.is_empty() && b != "HEAD" {
            return b.to_string();
        }
    }
    String::new()
}

/// Controlling terminal of a pid (e.g. 'ttys004'); '' if none.
fn tty_of(pid: i64) -> String {
    if let Some((_code, out)) = run_capture(
        &["ps", "-o", "tty=", "-p", &pid.to_string()],
        Duration::from_millis(1500),
    ) {
        let t = out.trim();
        if t.is_empty() || t == "??" || t == "?" {
            String::new()
        } else {
            t.to_string()
        }
    } else {
        String::new()
    }
}

/// Run a command, capturing stdout, with a wall-clock timeout (the outputs here are
/// tiny, so reading the pipe after exit can't deadlock). Returns (exit_code, stdout).
fn run_capture(args: &[&str], timeout: Duration) -> Option<(i32, String)> {
    let mut child = Command::new(args[0])
        .args(&args[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                if let Some(mut so) = child.stdout.take() {
                    let _ = so.read_to_string(&mut out);
                }
                return Some((status.code().unwrap_or(-1), out));
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                // 1ms poll granularity: git/ps finish in single-digit ms, so a coarse
                // poll would add far more latency than the work itself.
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(_) => return None,
        }
    }
}

fn read_all_entries(path: Option<&str>) -> Vec<Value> {
    let p = match path {
        Some(p) if !p.is_empty() => expand_user(p),
        _ => return vec![],
    };
    let data = match std::fs::read(&p) {
        Ok(d) => d,
        Err(_) => return vec![],
    };
    parse_jsonl(&data)
}

/// Parsed JSONL entries from the tail of the transcript (in file order).
fn read_tail_entries(path: Option<&str>) -> Vec<Value> {
    const MAXBYTES: u64 = 1_048_576;
    let p = match path {
        Some(p) if !p.is_empty() => expand_user(p),
        _ => return vec![],
    };
    let size = match std::fs::metadata(&p) {
        Ok(m) => m.len(),
        Err(_) => return vec![],
    };
    let mut f = match std::fs::File::open(&p) {
        Ok(f) => f,
        Err(_) => return vec![],
    };
    let mut chunk = Vec::new();
    if size > MAXBYTES {
        use std::io::{Seek, SeekFrom};
        if f.seek(SeekFrom::Start(size - MAXBYTES)).is_err() {
            return vec![];
        }
    }
    if f.read_to_end(&mut chunk).is_err() {
        return vec![];
    }
    let mut lines: Vec<&[u8]> = chunk.split(|&b| b == b'\n').collect();
    if size > MAXBYTES && lines.len() > 1 {
        lines.remove(0); // drop the partial first line
    }
    let mut out = Vec::new();
    for raw in lines {
        if raw.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        if let Ok(v) = serde_json::from_slice::<Value>(raw) {
            out.push(v);
        }
    }
    out
}

fn parse_jsonl(data: &[u8]) -> Vec<Value> {
    let mut out = Vec::new();
    for raw in data.split(|&b| b == b'\n') {
        if raw.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        if let Ok(v) = serde_json::from_slice::<Value>(raw) {
            out.push(v);
        }
    }
    out
}

/// Return (verdict, snippet) for the current turn's final assistant message.
fn classify_turn(transcript: Option<&str>) -> (String, String) {
    let mut text = String::new();
    for _ in 0..12 {
        // up to ~1.2s for the message to land
        text = current_turn_text(transcript);
        if !text.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let lines: Vec<&str> = text.lines().filter(|ln| !ln.trim().is_empty()).collect();
    if lines.is_empty() {
        return ("done".to_string(), String::new());
    }
    let last = lines[lines.len() - 1].trim();
    if last.contains('⏳') {
        let snippet = if lines.len() >= 2 {
            lines[lines.len() - 2].trim().to_string()
        } else {
            String::new()
        };
        return ("yourturn".to_string(), snippet);
    }
    if last.contains('✅') {
        return ("done".to_string(), String::new());
    }
    if last.trim_end().ends_with('?') {
        return ("yourturn".to_string(), last.to_string());
    }
    ("done".to_string(), String::new())
}

/// Text of the assistant's final message for the CURRENT turn: the last assistant
/// message that appears AFTER the last user message. Empty if not flushed yet.
fn current_turn_text(transcript: Option<&str>) -> String {
    let mut last_user: i64 = -1;
    let mut a_text = String::new();
    let mut a_idx: i64 = -1;
    for (i, obj) in read_tail_entries(transcript).into_iter().enumerate() {
        let o = match obj.as_object() {
            Some(o) => o,
            None => continue,
        };
        let m = match o.get("message") {
            Some(Value::Object(mm)) => mm,
            _ => o,
        };
        let role = m
            .get("role")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| o.get("type").and_then(|x| x.as_str()));
        match role {
            Some("user") => last_user = i as i64,
            Some("assistant") => {
                let txt = extract_text(m.get("content"));
                if !txt.trim().is_empty() {
                    a_idx = i as i64;
                    a_text = txt;
                }
            }
            _ => {}
        }
    }
    if a_idx > last_user {
        a_text
    } else {
        String::new()
    }
}

fn extract_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => {
            let mut parts: Vec<String> = Vec::new();
            for block in arr {
                match block {
                    Value::Object(o) => {
                        if o.get("type").and_then(|t| t.as_str()) == Some("text") {
                            parts.push(
                                o.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string(),
                            );
                        }
                    }
                    Value::String(s) => parts.push(s.clone()),
                    _ => {}
                }
            }
            parts.join("\n")
        }
        _ => String::new(),
    }
}

fn sanitize(s: Option<&str>) -> String {
    match s {
        None => String::new(),
        Some(s) if s.is_empty() => String::new(),
        Some(s) => s
            .replace('"', "'")
            .replace('\n', " ")
            .replace('\r', " ")
            .trim()
            .to_string(),
    }
}

fn expand_user(p: &str) -> std::path::PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        session_status::home().join(rest)
    } else if p == "~" {
        session_status::home()
    } else {
        std::path::PathBuf::from(p)
    }
}

fn write_atomic(path: &Path, rec: &Value) {
    let dir = match path.parent() {
        Some(d) => d,
        None => return,
    };
    let bytes = match serde_json::to_vec(rec) {
        Ok(b) => b,
        Err(_) => return,
    };
    let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("state");
    let tmp = dir.join(format!(".{}.{}.tmp", fname, std::process::id()));
    if std::fs::write(&tmp, &bytes).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    if std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Fire-and-forget: fork, let the parent exit so the hook returns to Claude in ~1ms,
/// and continue the real work in the detached child. No-op on failure (we just keep
/// running synchronously). The hook payload is already read into memory before this,
/// so dropping the inherited stdio in the child is safe.
fn detach() {
    unsafe {
        let pid = libc::fork();
        if pid < 0 {
            return; // fork failed — fall back to synchronous work
        }
        if pid > 0 {
            libc::_exit(0); // parent: hand control straight back to Claude
        }
        // child: detach from the controlling terminal/session and drop inherited stdio
        // so the hook's stdout hits EOF (otherwise the launcher could block reading it).
        libc::setsid();
        let devnull = libc::open(b"/dev/null\0".as_ptr() as *const libc::c_char, libc::O_RDWR);
        if devnull >= 0 {
            libc::dup2(devnull, 0);
            libc::dup2(devnull, 1);
            libc::dup2(devnull, 2);
            if devnull > 2 {
                libc::close(devnull);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tab_titles_are_ignored() {
        assert!(is_default_tab("Local"));
        assert!(is_default_tab("Local (2)"));
        assert!(is_default_tab("Local(2)"));
        assert!(!is_default_tab("TT AGI-18033"));
    }

    #[test]
    fn tab_label_reads_a_fresh_non_default_entry() {
        let dir = session_status::base_dir().join("tab-names");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("zzz_record_unit_test.json");
        let tty = "ttysUNITTEST";
        let now = now_secs();
        let write = |body: String| std::fs::write(&f, body).unwrap();

        // fresh + real name → used
        write(format!("{{\"{tty}\":{{\"name\":\"TT AGI-18033\",\"ts\":{now}}}}}"));
        assert_eq!(tab_label(tty), Some("TT AGI-18033".to_string()));

        // stale (>15s) → ignored
        write(format!("{{\"{tty}\":{{\"name\":\"TT AGI-18033\",\"ts\":{}}}}}", now - 100.0));
        assert_eq!(tab_label(tty), None);

        // default tab title → ignored
        write(format!("{{\"{tty}\":{{\"name\":\"Local (2)\",\"ts\":{now}}}}}"));
        assert_eq!(tab_label(tty), None);

        std::fs::remove_file(&f).ok();
        assert_eq!(tab_label(""), None);
        assert_eq!(tab_label("ttysNOSUCH"), None);
    }
}
