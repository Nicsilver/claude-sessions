//! The Claude Code hook recorder: reads hook JSON on stdin, writes the per-session state file.
//! Ported 1:1 from record.py and verified against it. Fail-safe: callers exit 0 regardless.

use crate::paths::*;
use crate::platform;
use serde_json::{json, Map, Value};
use std::io::Read;
use std::path::Path;

pub fn record(state: &str) {
    let mut raw = String::new();
    let _ = std::io::stdin().read_to_string(&mut raw);
    let data: Value = if raw.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))
    };

    let sid = str_of(&data, "session_id");
    if sid.is_empty() {
        return;
    }
    if std::fs::create_dir_all(state_dir()).is_err() {
        return;
    }
    let path = state_dir().join(format!("{sid}.json"));

    if state == "end" {
        let _ = std::fs::remove_file(&path);
        return;
    }

    let prev = load_json(&path);
    let reg = registry_for(&sid);

    let cwd = first_nonempty(&[
        str_of(&reg, "cwd"),
        str_of(&data, "cwd"),
        str_of(&prev, "cwd"),
        std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
    ]);
    let self_ppid = platform::parent_pid(std::process::id() as i64);
    let mut pid = i64_of(&reg, "pid");
    if pid <= 0 {
        pid = self_ppid;
    }
    let transcript = str_of(&data, "transcript_path");

    let mut topic = str_of(&prev, "topic");
    let has_prompt = !str_of(&data, "prompt").is_empty();
    if custom_label(&sid).is_some()
        || topic.is_empty()
        || matches!(state, "start" | "done")
        || (state == "working" && has_prompt)
    {
        let d = derive_label(&sid, &reg, &cwd, &transcript);
        if !d.is_empty() {
            topic = d;
        }
    }

    let mut eff = if state == "start" { "idle".to_string() } else { state.to_string() };
    let mut msg = sanitize(&str_of(&data, "message"));

    if eff == "needs" && msg.to_lowercase().contains("waiting for") {
        return;
    }

    if state == "done" {
        let (verdict, snippet) = classify_turn(&transcript);
        if verdict == "yourturn" {
            eff = "yourturn".to_string();
            if msg.is_empty() {
                msg = if snippet.is_empty() { "your turn".into() } else { snippet };
            }
        }
    }

    let mut rec = Map::new();
    rec.insert("session_id".into(), json!(sid));
    rec.insert("state".into(), json!(eff));
    rec.insert("topic".into(), json!(topic));
    rec.insert("cwd".into(), json!(cwd));
    rec.insert("pid".into(), json!(pid));
    rec.insert("ppid".into(), json!(self_ppid));
    rec.insert("updated_at".into(), json!(unix_now()));
    platform::annotate(&mut rec, pid, &transcript, &topic);
    if (eff == "needs" || eff == "yourturn") && !msg.is_empty() {
        rec.insert("message".into(), json!(msg));
    }

    write_atomic(&path, &Value::Object(rec));
}

// ---- label derivation ----

const DEFAULT_BRANCHES: &[&str] = &["main", "master", "develop", "trunk"];
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

fn custom_label(sid: &str) -> Option<String> {
    let m = load_json(&labels_path());
    let v = m.get(sid)?.as_str()?.trim().to_string();
    if v.is_empty() { None } else { Some(v) }
}

fn is_substantial(txt: &str) -> bool {
    if txt.is_empty() || txt.starts_with('<') || txt.starts_with("Caveat:") {
        return false;
    }
    let s = txt.trim().to_lowercase();
    let s = s.trim_end_matches(['.', '!', '?']);
    s.chars().count() >= 6 && !TRIVIAL_PROMPTS.contains(&s)
}

pub fn short(txt: &str, n: usize) -> String {
    let txt = txt.trim();
    if txt.chars().count() <= n {
        return txt.to_string();
    }
    let cut: String = txt.chars().take(n).collect();
    let head = match cut.rsplit_once(' ') {
        Some((h, _)) if !h.is_empty() => h.to_string(),
        _ => cut,
    };
    format!("{head}…")
}

fn derive_label(sid: &str, reg: &Value, cwd: &str, transcript: &str) -> String {
    if let Some(c) = custom_label(sid) {
        return c;
    }
    // Claude Code ≥2.1 auto-names every session ("fullstack-a4", nameSource "derived") —
    // worthless as a label. Only honour registry names the user set (e.g. /rename).
    let reg_name = str_of(reg, "name");
    if !reg_name.trim().is_empty() && str_of(reg, "nameSource") != "derived" {
        return reg_name.trim().to_string();
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
    let base = Path::new(cwd.trim_end_matches(['/', '\\']))
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if base.is_empty() { cwd.to_string() } else { base }
}

fn registry_for(sid: &str) -> Value {
    let mut best: Option<Value> = None;
    let mut best_updated = f64::MIN;
    let Ok(entries) = std::fs::read_dir(sessions_dir()) else {
        return Value::Null;
    };
    for e in entries.flatten() {
        if e.path().extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        if !v.is_object() || str_of(&v, "sessionId") != sid {
            continue;
        }
        let up = f64_of(&v, "updatedAt");
        if best.is_none() || up > best_updated {
            best_updated = up;
            best = Some(v);
        }
    }
    best.unwrap_or(Value::Null)
}

fn transcript_titles(path: &str) -> (String, String) {
    let (mut title, mut latest) = (String::new(), String::new());
    for obj in read_all_entries(path) {
        if !obj.is_object() {
            continue;
        }
        match str_of(&obj, "type").as_str() {
            "ai-title" if !str_of(&obj, "aiTitle").is_empty() => {
                title = str_of(&obj, "aiTitle").trim().to_string()
            }
            "custom-title" if !str_of(&obj, "customTitle").is_empty() => {
                title = str_of(&obj, "customTitle").trim().to_string()
            }
            "user" => {
                let m = if obj.get("message").map(Value::is_object).unwrap_or(false) {
                    obj.get("message").unwrap()
                } else {
                    &obj
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
    if let Ok(o) = std::process::Command::new("git")
        .args(["-C", cwd, "rev-parse", "--abbrev-ref", "HEAD"])
        .output()
    {
        if o.status.success() {
            let b = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !b.is_empty() && b != "HEAD" {
                return b;
            }
        }
    }
    String::new()
}

pub fn transcript_title(path: &str) -> String {
    transcript_titles(path).0
}

// ---- turn classification ----

fn classify_turn(transcript: &str) -> (String, String) {
    let mut text = String::new();
    for _ in 0..12 {
        text = current_turn_text(transcript);
        if !text.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let lines: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    let Some(last) = lines.last() else {
        return ("done".into(), String::new());
    };
    let last = last.trim();
    if last.contains('⏳') {
        let snippet = if lines.len() >= 2 {
            lines[lines.len() - 2].trim().to_string()
        } else {
            String::new()
        };
        return ("yourturn".into(), snippet);
    }
    if last.contains('✅') {
        return ("done".into(), String::new());
    }
    if last.trim_end().ends_with('?') {
        return ("yourturn".into(), last.to_string());
    }
    ("done".into(), String::new())
}

fn current_turn_text(transcript: &str) -> String {
    let entries = read_tail_entries(transcript, 1_048_576);
    let (mut last_user, mut a_idx) = (-1i64, -1i64);
    let mut a_text = String::new();
    for (i, obj) in entries.iter().enumerate() {
        if !obj.is_object() {
            continue;
        }
        let m = if obj.get("message").map(Value::is_object).unwrap_or(false) {
            obj.get("message").unwrap()
        } else {
            obj
        };
        let role = {
            let r = str_of(m, "role");
            if r.is_empty() { str_of(obj, "type") } else { r }
        };
        if role == "user" {
            last_user = i as i64;
        } else if role == "assistant" {
            let txt = extract_text(m.get("content"));
            if !txt.trim().is_empty() {
                a_idx = i as i64;
                a_text = txt;
            }
        }
    }
    if a_idx > last_user { a_text } else { String::new() }
}

fn read_all_entries(path: &str) -> Vec<Value> {
    if path.is_empty() {
        return vec![];
    }
    let Ok(bytes) = std::fs::read(expand_user(path)) else {
        return vec![];
    };
    parse_jsonl(&bytes)
}

fn read_tail_entries(path: &str, max_bytes: u64) -> Vec<Value> {
    if path.is_empty() {
        return vec![];
    }
    let p = expand_user(path);
    let Ok(meta) = std::fs::metadata(&p) else {
        return vec![];
    };
    let size = meta.len();
    let Ok(mut f) = std::fs::File::open(&p) else {
        return vec![];
    };
    use std::io::{Seek, SeekFrom};
    if size > max_bytes {
        let _ = f.seek(SeekFrom::Start(size - max_bytes));
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return vec![];
    }
    if size > max_bytes {
        if let Some(nl) = buf.iter().position(|&b| b == b'\n') {
            if nl + 1 < buf.len() {
                buf = buf[nl + 1..].to_vec();
            }
        }
    }
    parse_jsonl(&buf)
}

fn parse_jsonl(bytes: &[u8]) -> Vec<Value> {
    String::from_utf8_lossy(bytes)
        .split('\n')
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

fn extract_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => {
            let mut parts = Vec::new();
            for block in arr {
                if block.is_object() && str_of(block, "type") == "text" {
                    parts.push(str_of(block, "text"));
                } else if let Value::String(s) = block {
                    parts.push(s.clone());
                }
            }
            parts.join("\n")
        }
        _ => String::new(),
    }
}

// ---- small io helpers ----

fn write_atomic(path: &Path, v: &Value) {
    let Some(dir) = path.parent() else { return };
    let tmp = dir.join(format!(".tmp-{}-{}", std::process::id(), unix_now() as u64));
    if std::fs::write(&tmp, v.to_string()).is_ok() {
        if std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn sanitize(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    s.replace('"', "'").replace('\n', " ").replace('\r', " ").trim().to_string()
}

fn expand_user(p: &str) -> std::path::PathBuf {
    if let Some(rest) = p.strip_prefix('~') {
        home().join(rest.trim_start_matches(['/', '\\']))
    } else {
        std::path::PathBuf::from(p)
    }
}

fn first_nonempty(xs: &[String]) -> String {
    xs.iter().find(|s| !s.is_empty()).cloned().unwrap_or_default()
}
