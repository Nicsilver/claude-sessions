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
        std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
    ]);
    let self_ppid = platform::parent_pid(std::process::id() as i64);
    let mut pid = i64_of(&reg, "pid");
    if pid <= 0 {
        pid = self_ppid;
    }
    let transcript = str_of(&data, "transcript_path");

    let mut topic = str_of(&prev, "topic");
    let prompt = str_of(&data, "prompt");
    let has_prompt = !prompt.is_empty();
    let mut cmd_label = str_of(&prev, "cmd_label");
    if let Some((cmd, args)) = parse_command(&prompt) {
        // A later bare command (no args) must not steal the label from an earlier
        // argument-carrying one (e.g. /run-tests after /implement PROJ-x).
        if !args.is_empty() || cmd_label.is_empty() {
            cmd_label = command_topic(&cmd, &args);
        }
    }
    if custom_label(&sid).is_some()
        || topic.is_empty()
        || matches!(state, "start" | "done")
        || (state == "working" && has_prompt)
    {
        let d = derive_label(&sid, &reg, &cwd, &transcript, &cmd_label);
        if !d.is_empty() {
            topic = d;
        }
    }

    let mut eff = if state == "start" {
        "idle".to_string()
    } else {
        state.to_string()
    };
    let mut msg = sanitize(&str_of(&data, "message"));

    if eff == "needs" && msg.to_lowercase().contains("waiting for") {
        return;
    }

    if state == "done" {
        let (verdict, snippet) = classify_turn(&transcript);
        if verdict == "yourturn" {
            eff = "yourturn".to_string();
            if msg.is_empty() {
                msg = if snippet.is_empty() {
                    "your turn".into()
                } else {
                    snippet
                };
            }
        }
    }

    let mut rec = Map::new();
    rec.insert("session_id".into(), json!(sid));
    rec.insert("state".into(), json!(eff));
    rec.insert("topic".into(), json!(topic));
    rec.insert("cmd_label".into(), json!(cmd_label));
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
    "dig deeper and follow up",
    "dig deeper",
    "follow up",
    "follow-up",
    "continue",
    "keep going",
    "next",
    "next steps",
    "more",
    "help",
    "untitled",
    "new chat",
    "wip",
    "test",
    "testing",
    "debugging",
    "conversation",
];
const TRIVIAL_PROMPTS: &[&str] = &[
    "yes",
    "no",
    "y",
    "n",
    "ok",
    "okay",
    "sure",
    "go",
    "do it",
    "continue",
    "proceed",
    "next",
    "commit",
    "push",
    "thanks",
    "ty",
    "yep",
    "yeah",
    "nope",
    "stop",
    "wait",
    "please",
    "done",
    "good",
    "perfect",
    "nice",
    "cool",
    "great",
    "fix it",
    "go on",
    "keep going",
    "carry on",
];

fn custom_label(sid: &str) -> Option<String> {
    let m = load_json(&labels_path());
    let v = m.get(sid)?.as_str()?.trim().to_string();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
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

fn derive_label(sid: &str, reg: &Value, cwd: &str, transcript: &str, cmd_label: &str) -> String {
    if let Some(c) = custom_label(sid) {
        return c;
    }
    // Claude Code ≥2.1 auto-names every session ("fullstack-a4", nameSource "derived") —
    // worthless as a label. Only honour registry names the user set (e.g. /rename).
    let reg_name = str_of(reg, "name");
    if !reg_name.trim().is_empty() && str_of(reg, "nameSource") != "derived" {
        return reg_name.trim().to_string();
    }
    if !cmd_label.is_empty() {
        return cmd_label.to_string();
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
    if base.is_empty() {
        cwd.to_string()
    } else {
        base
    }
}

// ---- slash-command labels ----

/// The tab-name budget TabNamer applies (word-boundary cut at 20). Command topics are
/// composed to fit it so the IDE tab shows them whole.
const TAB_LABEL_MAX: usize = 20;

/// Built-in CLI commands that would make meaningless labels.
const BUILTIN_COMMANDS: &[&str] = &[
    "clear", "compact", "config", "cost", "doctor", "exit", "fast", "help", "init", "login",
    "logout", "mcp", "memory", "model", "quit", "rename", "resume", "status",
];

/// Parse a slash-command prompt into (command, args). Handles both the raw form
/// ("/implement PROJ-18546") and the transcript XML form the hook may deliver
/// ("<command-name>/implement</command-name>…<command-args>PROJ-18546</command-args>").
fn parse_command(prompt: &str) -> Option<(String, String)> {
    let p = prompt.trim();
    if let Some(name) = between(p, "<command-name>", "</command-name>") {
        let args = between(p, "<command-args>", "</command-args>").unwrap_or_default();
        return normalize_command(&name, &args);
    }
    let rest = p.strip_prefix('/')?;
    let mut it = rest.splitn(2, char::is_whitespace);
    let name = it.next().unwrap_or("").to_string();
    let args = it.next().unwrap_or("").to_string();
    normalize_command(&name, &args)
}

fn between(s: &str, open: &str, close: &str) -> Option<String> {
    let start = s.find(open)? + open.len();
    let end = start + s[start..].find(close)?;
    Some(s[start..end].trim().to_string())
}

fn normalize_command(name: &str, args: &str) -> Option<(String, String)> {
    let name = name.trim().trim_start_matches('/');
    // Plugin-scoped names ("acme-tools:release-prep") label by the skill part.
    let name = name.rsplit(':').next().unwrap_or(name).trim();
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        || BUILTIN_COMMANDS.contains(&name.to_lowercase().as_str())
    {
        return None;
    }
    Some((
        name.to_string(),
        args.split_whitespace().collect::<Vec<_>>().join(" "),
    ))
}

/// Compose "cmd args" into a label that fits [TAB_LABEL_MAX]. Args are compressed first
/// (PR URLs → "PR #n", flags dropped). If the natural form still overflows, identifiers
/// come first and the command name is trimmed to whatever fits ("run-team-test
/// PROJ-18033" → "PROJ-18033 team-test") — never a blind cut through an identifier.
fn command_topic(cmd: &str, args: &str) -> String {
    let toks = compress_args(args);
    let natural = if toks.is_empty() {
        cmd.to_string()
    } else {
        format!("{cmd} {}", toks.join(" "))
    };
    if natural.chars().count() <= TAB_LABEL_MAX {
        return natural;
    }
    let ids: Vec<&String> = toks.iter().filter(|t| is_identifier(t)).collect();
    if ids.is_empty() {
        return short(&natural, TAB_LABEL_MAX);
    }
    let mut base = String::new();
    for id in ids {
        let grown = if base.is_empty() {
            id.clone()
        } else {
            format!("{base} {id}")
        };
        if grown.chars().count() > TAB_LABEL_MAX {
            break;
        }
        base = grown;
    }
    if base.is_empty() {
        return short(&natural, TAB_LABEL_MAX);
    }
    let budget = TAB_LABEL_MAX.saturating_sub(base.chars().count() + 1);
    let action = fit_command(cmd, budget);
    if action.is_empty() {
        base
    } else {
        format!("{base} {action}")
    }
}

/// Longest tail of the hyphen-split command words that fits [budget]
/// ("prepare-release-notes", 13 → "release-notes"), falling back to initials ("PRN"), else "".
fn fit_command(cmd: &str, budget: usize) -> String {
    let words: Vec<&str> = cmd.split(['-', '_']).filter(|w| !w.is_empty()).collect();
    for i in 0..words.len() {
        let cand = words[i..].join("-");
        if cand.chars().count() <= budget {
            return cand;
        }
    }
    let initials: String = words
        .iter()
        .filter_map(|w| w.chars().next())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if !initials.is_empty() && initials.chars().count() <= budget {
        return initials;
    }
    String::new()
}

/// Tokenize args for a label: GitHub PR/issue URLs and "pr #n" pairs become "PR #n",
/// ticket ids are uppercased, --flags and other URLs are dropped.
fn compress_args(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut toks = args.split_whitespace().peekable();
    while let Some(t) = toks.next() {
        if t.starts_with('-') && t.len() > 1 && !t[1..].chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Some(n) = url_number(t) {
            out.push(format!("PR #{n}"));
            continue;
        }
        if t.contains("://") {
            continue;
        }
        let low = t.to_lowercase();
        if low == "pr" || low == "pull" {
            if let Some(next) = toks.peek() {
                if let Some(n) = hash_number(next) {
                    toks.next();
                    out.push(format!("PR #{n}"));
                    continue;
                }
            }
        }
        if hash_number(t).is_some() || is_ticket(t) {
            out.push(t.to_uppercase());
            continue;
        }
        out.push(t.to_string());
    }
    out
}

fn is_identifier(t: &str) -> bool {
    t.starts_with("PR #") || hash_number(t).is_some() || is_ticket(t)
}

/// "PROJ-18546"-shaped ticket id: 2+ letters, a dash, digits.
fn is_ticket(t: &str) -> bool {
    let Some((alpha, num)) = t.split_once('-') else {
        return false;
    };
    alpha.chars().count() >= 2
        && alpha.chars().all(|c| c.is_ascii_alphabetic())
        && !num.is_empty()
        && num.chars().all(|c| c.is_ascii_digit())
}

/// "#123" → Some("123").
fn hash_number(t: &str) -> Option<&str> {
    let n = t.strip_prefix('#')?;
    if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) {
        Some(n)
    } else {
        None
    }
}

/// Trailing number of a ".../pull/123" or ".../issues/123" URL.
fn url_number(t: &str) -> Option<String> {
    if !t.contains("://") {
        return None;
    }
    for marker in ["/pull/", "/issues/"] {
        if let Some(pos) = t.find(marker) {
            let digits: String = t[pos + marker.len()..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if !digits.is_empty() {
                return Some(digits);
            }
        }
    }
    None
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
        let Ok(text) = std::fs::read_to_string(e.path()) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
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
    classify_turn_text(&text)
}

/// Decide Done vs Your-turn from the assistant's final message text, plus a short snippet to
/// show for your-turn. Looks at the last non-empty line: `○` (hollow) → your turn (snippet = the
/// line above it), `●` (filled) → done, otherwise a trailing `?` is a weak your-turn fallback,
/// else done.
fn classify_turn_text(text: &str) -> (String, String) {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let Some(last) = lines.last() else {
        return ("done".into(), String::new());
    };
    let last = last.trim();
    if last.contains('○') {
        let snippet = if lines.len() >= 2 {
            lines[lines.len() - 2].trim().to_string()
        } else {
            String::new()
        };
        return ("yourturn".into(), snippet);
    }
    if last.contains('●') {
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
            if r.is_empty() {
                str_of(obj, "type")
            } else {
                r
            }
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
    if a_idx > last_user {
        a_text
    } else {
        String::new()
    }
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
    s.replace('"', "'")
        .replace(['\n', '\r'], " ")
        .trim()
        .to_string()
}

fn expand_user(p: &str) -> std::path::PathBuf {
    if let Some(rest) = p.strip_prefix('~') {
        home().join(rest.trim_start_matches(['/', '\\']))
    } else {
        std::path::PathBuf::from(p)
    }
}

fn first_nonempty(xs: &[String]) -> String {
    xs.iter()
        .find(|s| !s.is_empty())
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_done_when_last_line_has_filled_circle() {
        assert_eq!(
            classify_turn_text("did the thing\n\n●"),
            ("done".into(), String::new())
        );
    }

    #[test]
    fn marker_yourturn_when_last_line_has_hollow_circle() {
        let (verdict, snippet) = classify_turn_text("Which database should I use?\n○");
        assert_eq!(verdict, "yourturn");
        assert_eq!(snippet, "Which database should I use?"); // snippet is the line above ○
    }

    #[test]
    fn hollow_circle_alone_gives_empty_snippet() {
        assert_eq!(classify_turn_text("○"), ("yourturn".into(), String::new()));
    }

    #[test]
    fn trailing_question_is_a_weak_yourturn() {
        let (verdict, snippet) = classify_turn_text("some reasoning\nShould I proceed?");
        assert_eq!(verdict, "yourturn");
        assert_eq!(snippet, "Should I proceed?");
    }

    #[test]
    fn plain_text_and_empty_are_done() {
        assert_eq!(classify_turn_text("all set, nothing needed").0, "done");
        assert_eq!(classify_turn_text("").0, "done");
        assert_eq!(classify_turn_text("   \n  \n").0, "done");
    }

    #[test]
    fn filled_circle_beats_a_dangling_earlier_question() {
        // A `?` earlier in the turn must not override a final ● line.
        assert_eq!(
            classify_turn_text("Do you want X?\nOkay, done.\n●").0,
            "done"
        );
    }

    #[test]
    fn is_substantial_filters_trivial_and_short_prompts() {
        assert!(is_substantial("please refactor the auth module"));
        assert!(!is_substantial("ok")); // trivial + too short
        assert!(!is_substantial("yes")); // in TRIVIAL_PROMPTS
        assert!(!is_substantial("<command-name>")); // tool/meta noise
        assert!(!is_substantial(""));
    }

    #[test]
    fn parse_command_reads_raw_and_xml_forms() {
        assert_eq!(
            parse_command("/implement PROJ-18546"),
            Some(("implement".into(), "PROJ-18546".into()))
        );
        assert_eq!(
            parse_command(
                "<command-name>/acme-tools:release-prep</command-name>\
                 <command-message>release-prep</command-message>\
                 <command-args>PROJ-18546</command-args>"
            ),
            Some(("release-prep".into(), "PROJ-18546".into()))
        );
        assert_eq!(parse_command("address comments for pr #1234"), None); // not a command
        assert_eq!(parse_command("/model"), None); // built-in
        assert_eq!(parse_command("/ divided we fall"), None); // not a command name
    }

    #[test]
    fn command_topic_keeps_the_natural_form_when_it_fits() {
        assert_eq!(command_topic("implement", "PROJ-18546"), "implement PROJ-18546"); // exactly 20
        assert_eq!(command_topic("standup-notes", ""), "standup-notes");
        assert_eq!(command_topic("review-prs", "5"), "review-prs 5");
    }

    #[test]
    fn command_topic_drops_flags_and_compresses_urls() {
        // dropping --direct brings it back under the cap
        assert_eq!(
            command_topic("implement", "PROJ-18546 --direct"),
            "implement PROJ-18546"
        );
        assert_eq!(
            command_topic("review-pr", "https://github.com/octocat/hello-world/pull/1234"),
            "review-pr PR #1234"
        );
    }

    #[test]
    fn command_topic_puts_identifiers_first_when_over_budget() {
        assert_eq!(
            command_topic("run-team-test", "PROJ-18033"),
            "PROJ-18033 team-test"
        );
        // no command tail fits the leftover budget → initials
        assert_eq!(
            command_topic("prepare-release-notes", "PROJ-1234567890"),
            "PROJ-1234567890 PRN"
        );
    }

    #[test]
    fn compress_args_normalizes_pr_references() {
        assert_eq!(compress_args("pr #1234"), vec!["PR #1234"]);
        assert_eq!(compress_args("proj-18546"), vec!["PROJ-18546"]);
        assert_eq!(
            compress_args("https://example.com/some/page"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn short_truncates_on_a_word_boundary_with_ellipsis() {
        assert_eq!(short("keep me", 20), "keep me"); // under the limit, untouched
        assert_eq!(short("one two three four five", 12), "one two…");
    }

    #[test]
    fn sanitize_flattens_quotes_and_newlines() {
        assert_eq!(
            sanitize("he said \"hi\"\nthen left"),
            "he said 'hi' then left"
        );
        assert_eq!(sanitize(""), "");
    }
}
