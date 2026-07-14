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
IS_WIN = os.name == "nt"


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
        # utf-8-sig tolerates a BOM if some other Windows tool wrote the file.
        with open(path, encoding="utf-8-sig") as f:
            prev = json.load(f)
    except Exception:
        prev = {}

    reg = registry_for(sid)
    cwd = (reg.get("cwd") if reg else None) or data.get("cwd") or prev.get("cwd") or os.getcwd()
    pid = (reg.get("pid") if reg else None) or os.getppid()
    tty = tty_of(pid)
    transcript = data.get("transcript_path")
    topic = prev.get("topic")
    # Recompute the label on meaningful events (name/title/prompt can appear or change), or
    # whenever a custom rename or IDE tab name exists (so it applies promptly); reuse otherwise.
    if custom_label(sid) or tab_label(tty) or not topic or state in ("start", "done") or (state == "working" and data.get("prompt")):
        topic = derive_label(sid, reg, cwd, transcript, tty) or topic

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
        "tty": tty,
        "ppid": os.getppid(),
        "updated_at": time.time(),
    }
    # Windows has no tty; locate the owning terminal window instead so the widget
    # can focus it (wt / jetbrains / vscode / console) and knows which pid to raise.
    if IS_WIN:
        term, term_pid = win_terminal(pid)
        rec["terminal"] = term
        rec["term_pid"] = term_pid
        rec["tab_title"] = win_tab_title(transcript, topic)   # for WT per-tab focus via UIA
    if eff in ("needs", "yourturn") and msg:
        rec["message"] = msg

    write_atomic(path, rec)

    # Notifications are posted by the menu-bar app (Claude Sessions.app) so they're
    # clickable (click → jump to the session). record.py just writes state.
    return 0


LABELS_PATH = os.path.join(BASE, "labels.json")
TABNAMES_DIR = os.path.join(BASE, "tab-names")   # per-project tty→tab-name maps, published by the IntelliJ plugin
DEFAULT_BRANCHES = {"main", "master", "develop", "trunk"}
# Titles Claude sometimes generates that describe the *conversation* rather than the task —
# treat these as low-value so we fall through to the latest real prompt.
META_TITLES = {
    "dig deeper and follow up", "dig deeper", "follow up", "follow-up", "continue",
    "keep going", "next", "next steps", "more", "help", "untitled", "new chat",
    "wip", "test", "testing", "debugging", "conversation",
}
TRIVIAL_PROMPTS = {
    "yes", "no", "y", "n", "ok", "okay", "sure", "go", "do it", "continue", "proceed",
    "next", "commit", "push", "thanks", "ty", "yep", "yeah", "nope", "stop", "wait",
    "please", "done", "good", "perfect", "nice", "cool", "great", "fix it", "go on",
    "keep going", "carry on",
}


def custom_label(sid):
    """A user-set rename for this session (from the widget's ⌥-click), if any."""
    try:
        with open(LABELS_PATH) as f:
            v = json.load(f).get(sid)
        return v.strip() if isinstance(v, str) and v.strip() else None
    except Exception:
        return None


def is_default_tab(name):
    """IntelliJ's stock terminal tab titles ("Local", "Local (2)", "Local(2)") carry no
    meaning — don't use them as a session label."""
    base = name.replace(" ", "")
    return base == "Local" or (base.startswith("Local(") and base.endswith(")"))


def tab_label(tty):
    """The IntelliJ terminal tab name for this tty (e.g. "TT AGI-18033"), published by the
    Claude Sessions plugin under TABNAMES_DIR. Returns it only when it's a real, non-default
    tab title refreshed in the last 15s (so a closed project's stale entry is ignored)."""
    if not tty:
        return None
    import glob
    best = None
    for f in glob.glob(os.path.join(TABNAMES_DIR, "*.json")):
        try:
            entry = json.load(open(f)).get(tty)
        except Exception:
            continue
        if not isinstance(entry, dict):
            continue
        name = (entry.get("name") or "").strip()
        ts = entry.get("ts", 0)
        if not name or is_default_tab(name) or (time.time() - ts) > 15:
            continue
        if best is None or ts > best[1]:
            best = (name, ts)
    return best[0] if best else None


def is_substantial(txt):
    """A user prompt worth using as a label — not a command, caveat, or filler."""
    if not txt or txt[0] == "<" or txt.startswith("Caveat:"):
        return False
    s = txt.strip().lower().rstrip(".!?")
    return len(s) >= 6 and s not in TRIVIAL_PROMPTS


def short(txt, n=44):
    """Trim to n chars on a word boundary with an ellipsis."""
    txt = txt.strip()
    if len(txt) <= n:
        return txt
    return (txt[:n].rsplit(" ", 1)[0] or txt[:n]) + "…"


def derive_label(sid, reg, cwd, transcript_path, tty=""):
    """A human-meaningful session label, best-first:
    custom rename → IntelliJ tab name (e.g. "TT AGI-18033") → Claude /rename → meaningful
    git branch (AGI-XXXXX in worktrees) → decent AI title → latest substantial prompt → folder."""
    c = custom_label(sid)
    if c:
        return c
    tab = tab_label(tty)
    if tab:
        return tab
    if reg and (reg.get("name") or "").strip():
        return reg["name"].strip()
    title, latest = transcript_titles(transcript_path)
    br = git_branch(cwd)
    if br and br.lower() not in DEFAULT_BRANCHES:   # feature/AGI branch = descriptive
        return short(br)
    if title and title.strip().lower() not in META_TITLES:
        return title
    if latest:
        return short(latest)
    if title:                                       # a meta title still beats the bare folder
        return title
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
    """(title, latest_prompt): title = the latest custom/ai title; latest_prompt = the most
    recent *substantial* user message (reflects current focus better than the first one)."""
    title = ""
    latest = ""
    for obj in read_all_entries(path):
        if not isinstance(obj, dict):
            continue
        t = obj.get("type")
        if t == "ai-title" and obj.get("aiTitle"):
            title = obj["aiTitle"].strip()
        elif t == "custom-title" and obj.get("customTitle"):
            title = obj["customTitle"].strip()
        elif t == "user":
            m = obj.get("message") if isinstance(obj.get("message"), dict) else obj
            txt = extract_text(m.get("content")).strip().replace("\n", " ")
            if is_substantial(txt):
                latest = txt
    return title, latest


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
    agent), which also lets the surfaces tag those sessions. Unix only — Windows
    has no tty; there we locate the owning terminal window instead (win_terminal)."""
    if IS_WIN:
        return ""
    try:
        out = subprocess.run(["ps", "-o", "tty=", "-p", str(pid)],
                             capture_output=True, text=True, timeout=1.5)
        t = out.stdout.strip()
        return "" if (not t or t in ("??", "?")) else t
    except Exception:
        return ""


# JetBrains IDE launcher exe names (their embedded terminal is the tab we jump to).
JETBRAINS_EXES = {
    "idea64.exe", "idea.exe", "pycharm64.exe", "pycharm.exe", "webstorm64.exe",
    "clion64.exe", "goland64.exe", "phpstorm64.exe", "rider64.exe", "rubymine64.exe",
    "datagrip64.exe", "rustrover64.exe", "aqua64.exe", "fleet.exe",
}


def win_process_map():
    """{pid: (parent_pid, exe_name_lower)} for every running process, via the
    Toolhelp snapshot API. ctypes-only so record.py stays dependency-free."""
    import ctypes
    from ctypes import wintypes

    class PROCESSENTRY32(ctypes.Structure):
        _fields_ = [
            ("dwSize", wintypes.DWORD),
            ("cntUsage", wintypes.DWORD),
            ("th32ProcessID", wintypes.DWORD),
            ("th32DefaultHeapID", ctypes.c_size_t),
            ("th32ModuleID", wintypes.DWORD),
            ("cntThreads", wintypes.DWORD),
            ("th32ParentProcessID", wintypes.DWORD),
            ("pcPriClassBase", ctypes.c_long),
            ("dwFlags", wintypes.DWORD),
            ("szExeFile", ctypes.c_char * 260),
        ]

    k = ctypes.windll.kernel32
    TH32CS_SNAPPROCESS = 0x00000002
    INVALID = ctypes.c_void_p(-1).value
    snap = k.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    if snap == INVALID:
        return {}
    out = {}
    try:
        e = PROCESSENTRY32()
        e.dwSize = ctypes.sizeof(PROCESSENTRY32)
        ok = k.Process32First(snap, ctypes.byref(e))
        while ok:
            name = e.szExeFile.decode("latin-1", "ignore").lower()
            out[int(e.th32ProcessID)] = (int(e.th32ParentProcessID), name)
            ok = k.Process32Next(snap, ctypes.byref(e))
    finally:
        k.CloseHandle(snap)
    return out


def win_console_title():
    """The current console/pseudo-console title (what Windows Terminal shows on the tab).
    Claude Code sets this to the session's task title via an OSC escape; a hook subprocess
    shares that ConPTY, so GetConsoleTitle reads the exact tab text. Returns "" if it just
    looks like a shell path (i.e. Claude hasn't set a title)."""
    try:
        import ctypes
        buf = ctypes.create_unicode_buffer(1024)
        ctypes.windll.kernel32.GetConsoleTitleW(buf, 1024)
        t = (buf.value or "").strip()
    except Exception:
        return ""
    low = t.lower()
    if not t or ".exe" in low or low.endswith(("bash", "pwsh", "cmd", "powershell")):
        return ""   # a shell's own title, not Claude's task title
    return t


def win_tab_title(transcript_path, topic):
    """Best guess of the Windows Terminal tab title for this session, so the widget can
    match a session to its tab (WT exposes tab titles but not per-tab pids). Prefer the live
    console title, then the transcript's AI/custom title, then the derived topic."""
    ct = win_console_title()
    if ct:
        return ct
    title, _ = transcript_titles(transcript_path)
    return title or (topic or "")


def win_terminal(start_pid):
    """(terminal, term_pid): identify the terminal that owns this session by walking
    the process tree up from the Claude process. term_pid is the ancestor whose
    top-level window the widget should focus.
      "wt"       -> Windows Terminal (focus the WindowsTerminal.exe window)
      "jetbrains"-> a JetBrains IDE (tab-precise jump handled by the plugin, by pid)
      "vscode"   -> VS Code integrated terminal
      "console"  -> classic conhost / OpenConsole window
      "other"    -> unknown; fall back to the nearest ancestor
    Returns ("", 0) if the tree can't be read."""
    try:
        pmap = win_process_map()
    except Exception:
        return ("", 0)
    chain, pid, seen = [], start_pid, set()
    for _ in range(30):
        if pid in seen or pid not in pmap:
            break
        seen.add(pid)
        ppid, name = pmap[pid]
        chain.append((pid, name))
        pid = ppid
    for pid, name in chain:
        if name == "windowsterminal.exe":
            return ("wt", pid)
    for pid, name in chain:
        if name in JETBRAINS_EXES:
            return ("jetbrains", pid)
    for pid, name in chain:
        if name == "code.exe":
            return ("vscode", pid)
    for pid, name in chain:
        if name in ("conhost.exe", "openconsole.exe"):
            return ("console", pid)
    return ("other", chain[0][0] if chain else start_pid)


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
