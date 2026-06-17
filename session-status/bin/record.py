#!/usr/bin/env python3
"""Claude Code session-status recorder.

Invoked by Claude Code hooks. Reads the hook JSON on stdin and writes a tiny
per-session state file to ~/.claude/session-status/state/<session_id>.json.
Notifications are posted by the menu-bar app (Claude Sessions.app), which watches
that state and fires a clickable alert when a session flips INTO "needs me".

Usage (from a hook):  record.py <state>
  state in: start | working | needs | done | end

This script is intentionally fail-safe: any error exits 0 so it can never
break a Claude Code session. Fully reversible — delete the hooks block in
settings.json and this directory to return to stock.
"""
import sys, os, json, time, subprocess, tempfile

BASE = os.path.expanduser("~/.claude/session-status")
STATE_DIR = os.path.join(BASE, "state")


def main():
    try:
        state = sys.argv[1] if len(sys.argv) > 1 else "working"
        raw = sys.stdin.read()
        data = json.loads(raw) if raw.strip() else {}
    except Exception:
        return 0

    sid = data.get("session_id")
    if not sid:
        return 0

    try:
        os.makedirs(STATE_DIR, exist_ok=True)
    except Exception:
        return 0
    path = os.path.join(STATE_DIR, str(sid) + ".json")

    if state == "end":
        try:
            os.remove(path)
        except OSError:
            pass
        return 0

    prev = {}
    try:
        with open(path) as f:
            prev = json.load(f)
    except Exception:
        prev = {}

    reg = registry_for(sid)
    cwd = (reg.get("cwd") if reg else None) or data.get("cwd") or prev.get("cwd") or os.getcwd()
    pid = (reg.get("pid") if reg else None) or os.getppid()
    transcript = data.get("transcript_path")
    topic = prev.get("topic")
    # Recompute the label on meaningful events (name/ai-title/first-prompt can
    # appear or change); reuse the cache on hot-path PostToolUse "working" pings.
    if not topic or state in ("start", "done") or (state == "working" and data.get("prompt")):
        topic = derive_label(reg, cwd, transcript) or topic

    eff = "idle" if state == "start" else state
    msg = sanitize(data.get("message"))

    # The Notification hook fires both for real attention prompts (permission,
    # AskUserQuestion, plan approval) AND for a benign ~60s idle ping
    # ("Claude is waiting for your input"). Suppress the idle ping so a finished
    # session stays "done · safe to close" instead of decaying to needs-me after
    # a minute. Everything else (permission etc.) still escalates to needs-me.
    if eff == "needs" and "waiting for" in msg.lower():
        return 0

    # Stop hook: tell "Claude finished" (✅ done, safe to close) apart from
    # "Claude is waiting on you" (🟡 your turn — greeting, question, offer).
    # Primary signal = the trailing ⏳/✅ marker the model emits per global
    # CLAUDE.md; fallback = the last line ends with a question mark. This is the
    # soft state; the urgent red "needs" only comes from the Notification hook.
    if state == "done":
        verdict, snippet = classify_turn(data.get("transcript_path"))
        if verdict == "yourturn":
            eff = "yourturn"
            if not msg:
                msg = snippet or "your turn"

    rec = {
        "session_id": str(sid),
        "state": eff,
        "topic": topic,
        "cwd": cwd,
        "pid": pid,
        "tty": tty_of(pid),
        "ppid": os.getppid(),
        "updated_at": time.time(),
    }
    if eff in ("needs", "yourturn") and msg:
        rec["message"] = msg

    write_atomic(path, rec)

    # Notifications are posted by the menu-bar app (Claude Sessions.app) so they're
    # clickable (click → jump to the session). record.py just writes state.
    return 0


def derive_label(reg, cwd, transcript_path):
    """A human-meaningful session label: the user's /rename name, else the
    AI-generated title, else the first prompt, else the git branch (AGI-XXXXX in
    worktrees), else the folder name."""
    if reg and (reg.get("name") or "").strip():
        return reg["name"].strip()
    title, first = transcript_titles(transcript_path)
    if title:
        return title
    if first:
        return first[:46]
    br = git_branch(cwd)
    if br:
        return br
    return os.path.basename(cwd.rstrip("/")) or cwd


def registry_for(sid):
    """Claude's native per-session registry entry (~/.claude/sessions/<pid>.json)
    matching this session id, picking the most-recently-updated if a session was
    resumed under the same id. Gives the authoritative claude pid, cwd, and name."""
    import glob
    match = None
    for f in glob.glob(os.path.expanduser("~/.claude/sessions/*.json")):
        try:
            d = json.load(open(f))
        except Exception:
            continue
        if isinstance(d, dict) and d.get("sessionId") == sid:
            if match is None or d.get("updatedAt", 0) > match.get("updatedAt", 0):
                match = d
    return match


def transcript_titles(path):
    """(title, first_user_prompt) from the transcript — title is the latest
    custom/ai title, first_user_prompt is the first real user message."""
    title = ""
    first = ""
    for obj in read_all_entries(path):
        if not isinstance(obj, dict):
            continue
        t = obj.get("type")
        if t == "ai-title" and obj.get("aiTitle"):
            title = obj["aiTitle"].strip()
        elif t == "custom-title" and obj.get("customTitle"):
            title = obj["customTitle"].strip()
        elif not first and t == "user":
            m = obj.get("message") if isinstance(obj.get("message"), dict) else obj
            txt = extract_text(m.get("content")).strip()
            if txt and txt[0] not in "<" and not txt.startswith("Caveat:"):
                first = txt.replace("\n", " ")
    return title, first


def git_branch(cwd):
    try:
        out = subprocess.run(
            ["git", "-C", cwd, "rev-parse", "--abbrev-ref", "HEAD"],
            capture_output=True, text=True, timeout=1.5)
        b = out.stdout.strip()
        if out.returncode == 0 and b and b != "HEAD":
            return b
    except Exception:
        pass
    return ""


def tty_of(pid):
    """Controlling terminal of a pid (e.g. 'ttys004'); '' if none (headless / IDE
    agent), which also lets the surfaces tag those sessions."""
    try:
        out = subprocess.run(["ps", "-o", "tty=", "-p", str(pid)],
                             capture_output=True, text=True, timeout=1.5)
        t = out.stdout.strip()
        return "" if (not t or t in ("??", "?")) else t
    except Exception:
        return ""


def read_all_entries(path):
    if not path:
        return []
    try:
        with open(os.path.expanduser(path), "rb") as f:
            data = f.read()
    except Exception:
        return []
    out = []
    for raw in data.split(b"\n"):
        if not raw.strip():
            continue
        try:
            out.append(json.loads(raw))
        except Exception:
            continue
    return out


def classify_turn(transcript_path):
    """Return (verdict, snippet) for the CURRENT turn's final assistant message.
    verdict is "yourturn" if it leaves the ball in the user's court (⏳ marker, or
    last line ends "?"), else "done" (✅ marker / plain statement). "yourturn" is
    the soft, no-ping state; the urgent red "needs" only comes from Notification.

    Retries briefly: the Stop hook can fire a beat before the final assistant
    message is flushed to the transcript — that race otherwise mis-files fresh
    greetings as done (empty transcript -> fallback)."""
    text = ""
    for _ in range(12):  # up to ~1.2s for the message to land
        text = current_turn_text(transcript_path)
        if text:
            break
        time.sleep(0.1)
    lines = [ln for ln in text.splitlines() if ln.strip()]
    if not lines:
        return ("done", "")
    last = lines[-1].strip()
    if "⏳" in last:   # ⏳ (marker on its own final line)
        snippet = lines[-2].strip() if len(lines) >= 2 else ""
        return ("yourturn", snippet)
    if "✅" in last:   # ✅
        return ("done", "")
    if last.rstrip().endswith("?"):
        return ("yourturn", last)
    return ("done", "")


def current_turn_text(transcript_path):
    """Text of the assistant's final message for the CURRENT turn: the last
    assistant message that appears AFTER the last user message. Returns "" if the
    response isn't in the transcript yet, so the caller can retry. (Tool results
    are recorded as user-role entries, which is fine — the final text still comes
    after them.)"""
    last_user = -1
    a_text = ""
    a_idx = -1
    for i, obj in enumerate(read_tail_entries(transcript_path)):
        if not isinstance(obj, dict):
            continue
        m = obj.get("message") if isinstance(obj.get("message"), dict) else obj
        role = m.get("role") or obj.get("type")
        if role == "user":
            last_user = i
        elif role == "assistant":
            txt = extract_text(m.get("content"))
            if txt.strip():
                a_idx = i
                a_text = txt
    return a_text if a_idx > last_user else ""


def read_tail_entries(path, maxbytes=1048576):
    """Parsed JSONL entries from the tail of the transcript (in file order)."""
    if not path:
        return []
    p = os.path.expanduser(path)
    try:
        size = os.path.getsize(p)
        with open(p, "rb") as f:
            if size > maxbytes:
                f.seek(size - maxbytes)
            chunk = f.read()
    except Exception:
        return []
    lines = chunk.split(b"\n")
    if size > maxbytes and len(lines) > 1:
        lines = lines[1:]  # drop the partial first line
    out = []
    for raw in lines:
        if not raw.strip():
            continue
        try:
            out.append(json.loads(raw))
        except Exception:
            continue
    return out


def extract_text(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = []
        for block in content:
            if isinstance(block, dict) and block.get("type") == "text":
                parts.append(block.get("text", ""))
            elif isinstance(block, str):
                parts.append(block)
        return "\n".join(parts)
    return ""


def sanitize(s):
    if not s:
        return ""
    return str(s).replace('"', "'").replace("\n", " ").replace("\r", " ").strip()


def write_atomic(path, rec):
    d = os.path.dirname(path)
    try:
        fd, tmp = tempfile.mkstemp(dir=d)
    except Exception:
        return
    try:
        with os.fdopen(fd, "w") as f:
            json.dump(rec, f)
        os.replace(tmp, path)
    except Exception:
        try:
            os.remove(tmp)
        except OSError:
            pass


if __name__ == "__main__":
    sys.exit(main())
