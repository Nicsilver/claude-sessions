#!/usr/bin/env python3
"""
worklog_capture.py - Claude Code SessionEnd hook.

Parses the just-ended session transcript and upserts a terse, standup-oriented
entry into a running per-day worklog at ~/.claude/worklog/YYYY-MM-DD.md.

Design constraints:
  * Zero-LLM and fast: pure parsing, no network, no `claude` subprocess.
    (Spawning `claude -p` here would fire a nested session that re-triggers the
     session-status record.py hooks => phantom rows + recursion. Don't.)
  * Never blocks or breaks session exit: every failure path just exits 0.
  * Coexists with the session-status SessionEnd hook (added as a second entry).
  * Idempotent per session: re-running (e.g. on resume) replaces that session's
    block rather than duplicating it. Concurrent ends are flock-serialised.

The unit of work is the AGI ticket, NOT the cwd: ~98% of sessions launch from the
non-git Fullstack hub and the real code lives in per-ticket worktrees, so signal
comes from AGI numbers, PR/deploy/log_work tool calls, branch names and aiTitle.

AGI numbers are ranked: "primary" (from aiTitle, prompts, branches, PR titles,
time-logs, YT updates, skill args) vs incidental (a stray match deep in some tool
input, e.g. an edit to MEMORY.md). Only primary tickets are shown.

Output dir override (for testing): WORKLOG_DIR_OVERRIDE=/tmp/wl-test
"""

import sys
import os
import re
import json

try:
    import fcntl  # POSIX; present on macOS/Linux
except Exception:
    fcntl = None

from datetime import datetime, timezone

WORKLOG_DIR = os.environ.get(
    "WORKLOG_DIR_OVERRIDE", os.path.expanduser("~/.claude/worklog")
)

# --- tunables ---------------------------------------------------------------
MAX_PROMPTS_SHOWN = 2      # how many "Asked:" prompts to surface
MAX_AGENTS_SHOWN = 4
PROMPT_TRIM = 100          # chars per surfaced prompt
INPUT_SCAN_CHARS = 3000    # cap when scanning a tool input blob for AGI/branch
# ---------------------------------------------------------------------------

AGI_RE = re.compile(r"\bAGI-\d{3,6}\b", re.IGNORECASE)
BRANCH_RE = re.compile(r"\bAGI-\d{3,6}-[A-Za-z0-9._/-]+")
GH_PR_RE = re.compile(r"\bgh\s+pr\s+([a-z-]+)([^\n|;&]*)", re.IGNORECASE)
CMD_NAME_RE = re.compile(r"<command-name>(.*?)</command-name>", re.S)
CMD_ARGS_RE = re.compile(r"<command-args>(.*?)</command-args>", re.S)
PR_NUM_VERBS = ("view", "ready", "merge", "edit", "checks",
                "comment", "close", "reopen", "diff", "review")
RAN_NOISE = ("/model", "/effort", "/clear", "/config", "/compact",
             "/exit", "/help", "/fast", "/status", "/resume")


def agis_in(s):
    return {m.upper() for m in AGI_RE.findall(s or "")}


def dedup(seq):
    return list(dict.fromkeys(seq))


def trunc(s, n):
    s = (s or "").replace("\n", " ").strip()
    return s if len(s) <= n else s[: n - 1].rstrip() + "…"


def iso_to_local(ts):
    if not ts:
        return None
    try:
        dt = datetime.fromisoformat(ts.replace("Z", "+00:00"))
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        return dt.astimezone()
    except Exception:
        return None


def fmt_range(first_ts, last_ts):
    s = iso_to_local(first_ts)
    e = iso_to_local(last_ts)
    if not s and not e:
        return "??:??"
    if not s:
        return e.strftime("%H:%M")
    if not e:
        return s.strftime("%H:%M")
    if s.date() == e.date():
        return f"{s:%H:%M}–{e:%H:%M}"
    return f"{s:%b%d %H:%M}→{e:%b%d %H:%M}"  # crosses midnight: show both dates


def add_agis(d, text, primary=False):
    found = agis_in(text)
    if found:
        d["agis_all"] |= found
        if primary:
            d["agis_primary"] |= found
    return found


def parse_transcript(path):
    """Stream the JSONL transcript and pull standup-relevant facts."""
    d = {
        "ai_title": "",
        "user_prompts": [],
        "slash_cmds": [],      # "/code-story AGI-18546"
        "agis_primary": set(),
        "agis_all": set(),
        "branches": set(),
        "pr_actions": [],
        "deploy_actions": [],
        "time_logs": [],       # (issueId, minutes, workType)
        "state_updates": set(),
        "skills_ran": [],
        "agents_ran": [],      # (subagent_type, description)
        "files_edited": set(),
        "first_ts": None,
        "last_ts": None,
    }

    with open(path, "r", errors="replace") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                o = json.loads(line)
            except Exception:
                continue

            ts = o.get("timestamp")
            if ts:
                if d["first_ts"] is None:
                    d["first_ts"] = ts
                d["last_ts"] = ts

            t = o.get("type")

            if t == "ai-title":
                at = o.get("aiTitle")
                if at:
                    d["ai_title"] = at  # keep the latest
                    add_agis(d, at, primary=True)
                continue

            if t == "user":
                msg = o.get("message") or {}
                c = msg.get("content")
                if isinstance(c, str):
                    if c.startswith("<command-name>"):
                        m = CMD_NAME_RE.search(c)
                        a = CMD_ARGS_RE.search(c)
                        if m:
                            entry = (m.group(1).strip() + " "
                                     + (a.group(1).strip() if a else "")).strip()
                            d["slash_cmds"].append(entry)
                            add_agis(d, entry, primary=True)
                    elif c.startswith("<"):
                        pass  # injected wrapper / system-reminder noise
                    else:
                        p = c.strip()
                        if p:
                            d["user_prompts"].append(p)
                            add_agis(d, p, primary=True)
                continue

            if t == "assistant":
                msg = o.get("message") or {}
                content = msg.get("content")
                if not isinstance(content, list):
                    continue
                for b in content:
                    if not isinstance(b, dict) or b.get("type") != "tool_use":
                        continue
                    name = b.get("name") or ""
                    inp = b.get("input") or {}
                    ln = name.lower()

                    # catch-all: AGI numbers (incidental) + branch names (primary)
                    try:
                        blob = json.dumps(inp, default=str)[:INPUT_SCAN_CHARS]
                    except Exception:
                        blob = ""
                    add_agis(d, blob, primary=False)
                    for bm in BRANCH_RE.findall(blob):
                        d["branches"].add(bm)
                        add_agis(d, bm, primary=True)

                    if ln.endswith("log_work") or "quick_add_time" in ln:
                        iid = (inp.get("issueId") or inp.get("issue")
                               or inp.get("issueIdOrKey") or "")
                        mins_raw = (inp.get("durationMinutes") or inp.get("minutes")
                                    or inp.get("duration") or "")
                        mins = re.sub(r"\D", "", str(mins_raw))  # "10m" -> "10"
                        wt = str(inp.get("workType") or inp.get("type") or "").strip()
                        d["time_logs"].append((str(iid), mins, wt))
                        add_agis(d, str(iid), primary=True)
                    elif "update_issue" in ln or "change_issue" in ln:
                        d["state_updates"] |= add_agis(d, blob, primary=True)
                    elif "create_pull_request" in ln:
                        title = inp.get("title") or ""
                        repo = (inp.get("repo") or inp.get("repository")
                                or inp.get("owner") or "")
                        s = "opened PR"
                        if repo:
                            s += f" ({repo})"
                        if title:
                            s += f": {trunc(title, 70)}"
                        d["pr_actions"].append(s)
                        add_agis(d, title, primary=True)
                    elif "trigger_build" in ln:
                        bt = (inp.get("buildTypeId") or inp.get("build_type_id")
                              or inp.get("buildType") or "")
                        br = inp.get("branchName") or inp.get("branch") or ""
                        s = "TeamCity"
                        if bt:
                            s += f" {bt}"
                        if br:
                            s += f" @ {br}"
                        d["deploy_actions"].append(s)
                    elif name == "Bash":
                        cmd = inp.get("command") or ""
                        for mm in GH_PR_RE.finditer(cmd):
                            verb = mm.group(1).lower()
                            rest = AGI_RE.sub("", mm.group(2))       # AGI-#### isn't a PR no.
                            rest = re.sub(r"\d*>{1,2}&?\d*", " ", rest)  # drop 2>&1, 2>/dev/null
                            num = ""
                            if verb in PR_NUM_VERBS:
                                nm = re.match(r"\s*#?(\d{2,6})\b", rest)  # PR no. is the 1st arg
                                if nm:
                                    num = " #" + nm.group(1)
                            d["pr_actions"].append(f"gh pr {verb}{num}")
                    elif name == "Skill":
                        sk = inp.get("skill") or ""
                        ar = inp.get("args") or ""
                        if sk:
                            d["skills_ran"].append((f"/{sk} {trunc(ar, 45)}").strip())
                            add_agis(d, str(ar), primary=True)
                    elif name in ("Agent", "Task"):
                        st = inp.get("subagent_type") or ""
                        desc = inp.get("description") or ""
                        if st or desc:
                            d["agents_ran"].append((st, desc))
                            add_agis(d, desc, primary=True)
                    elif name in ("Edit", "Write", "NotebookEdit"):
                        fp = inp.get("file_path") or inp.get("notebook_path") or ""
                        if fp:
                            d["files_edited"].add(fp)
                continue
    return d


def build_block(d, session_id):
    tickets = sorted(d["agis_primary"]) or sorted(d["agis_all"])
    title = trunc(d["ai_title"]
                  or (d["user_prompts"][0] if d["user_prompts"] else ""), 90)
    if not title or title.lower() in ("clear", "resume", "exit", "session") \
            or title.startswith("/"):
        title = ("Work on " + ", ".join(tickets[:2])) if tickets else "session"

    lines = [f"### {fmt_range(d['first_ts'], d['last_ts'])} · {title}"]

    if tickets:
        lines.append(f"- **Tickets:** {', '.join(tickets)}")
    if d["branches"]:
        lines.append(f"- **Branches:** {', '.join(sorted(d['branches']))}")

    prs = dedup(d["pr_actions"])
    if any(p.startswith("opened PR") for p in prs):
        prs = [p for p in prs if not p.startswith("gh pr create")]
    if prs:
        lines.append(f"- **PRs:** {'; '.join(prs[:6])}")
    if d["deploy_actions"]:
        lines.append(f"- **Deploys:** {'; '.join(dedup(d['deploy_actions']))}")
    if d["state_updates"]:
        lines.append(f"- **Updated in YT:** {', '.join(sorted(d['state_updates']))}")
    if d["time_logs"]:
        def _tl(i, m, w):
            s = i or "?"
            if m:
                s += f" +{m}m"
            if w:
                s += f" {w}"
            return s
        tl = "; ".join(_tl(i, m, w) for i, m, w in dedup(d["time_logs"]))
        lines.append(f"- **Time logged:** {tl}")

    ran = list(d["skills_ran"])
    ran += [c if c.startswith("/") else "/" + c for c in d["slash_cmds"]]
    ran = [trunc(r, 55) for r in dedup(ran) if not r.startswith(RAN_NOISE)]
    if ran:
        lines.append(f"- **Ran:** {', '.join(dedup(ran)[:8])}")

    if d["agents_ran"]:
        ag = "; ".join(
            f"{st or 'agent'}: {trunc(desc, 45)}"
            for st, desc in d["agents_ran"][:MAX_AGENTS_SHOWN]
        )
        lines.append(f"- **Agents:** {ag}")
    if d["files_edited"]:
        fe = sorted(d["files_edited"])
        samp = ", ".join(os.path.basename(p) for p in fe[:3])
        more = f" +{len(fe) - 3} more" if len(fe) > 3 else ""
        lines.append(f"- **Edited:** {len(fe)} files ({samp}{more})")
    if d["user_prompts"]:
        asks = " | ".join('"' + trunc(p, PROMPT_TRIM) + '"'
                          for p in d["user_prompts"][:MAX_PROMPTS_SHOWN])
        lines.append(f"- **Asked:** {asks}")

    marker = (f"<!-- worklog session={session_id} start={d['first_ts'] or ''} "
              f"agi={','.join(tickets)} -->")
    return marker + "\n" + "\n".join(lines) + "\n"


def has_signal(d):
    work = any([
        d["agis_all"], d["time_logs"], d["pr_actions"], d["deploy_actions"],
        d["state_updates"], d["skills_ran"], d["agents_ran"], d["files_edited"],
    ])
    soft = bool(d["ai_title"]) and len(d["user_prompts"]) >= 2
    return work or soft


def upsert(day_md_path, day, weekday, session_id, start_ts, block):
    """Read-modify-write the day file under flock; replace this session's block
    if present, keep blocks sorted by start time. Atomic via os.replace."""
    header = f"# Worklog — {day} ({weekday})\n"
    lock_dir = os.path.join(os.path.dirname(day_md_path), ".locks")
    os.makedirs(lock_dir, exist_ok=True)
    lock_path = os.path.join(lock_dir, os.path.basename(day_md_path) + ".lock")

    lf = open(lock_path, "w")
    try:
        if fcntl:
            fcntl.flock(lf, fcntl.LOCK_EX)

        existing = ""
        if os.path.exists(day_md_path):
            with open(day_md_path, "r", errors="replace") as rf:
                existing = rf.read()

        blocks = {}  # session_id -> (start_ts, text)
        if existing:
            parts = re.split(r"(?m)^(<!-- worklog session=.*?-->)$", existing)
            i = 1
            while i < len(parts):
                mk = parts[i]
                body = parts[i + 1] if i + 1 < len(parts) else ""
                sidm = re.search(r"session=(\S+)", mk)
                stm = re.search(r"start=(\S*)", mk)
                key = sidm.group(1) if sidm else mk
                blocks[key] = (stm.group(1) if stm else "",
                               (mk + body).strip() + "\n")
                i += 2

        blocks[session_id] = (start_ts or "", block.strip() + "\n")
        ordered = sorted(blocks.values(), key=lambda x: x[0] or "")
        out = header + "\n" + "\n\n".join(b[1].strip() for b in ordered) + "\n"

        tmp = day_md_path + ".tmp"
        with open(tmp, "w") as wf:
            wf.write(out)
        os.replace(tmp, day_md_path)
    finally:
        if fcntl:
            try:
                fcntl.flock(lf, fcntl.LOCK_UN)
            except Exception:
                pass
        lf.close()


def main():
    raw = sys.stdin.read() if not sys.stdin.isatty() else ""
    try:
        ev = json.loads(raw) if raw.strip() else {}
    except Exception:
        ev = {}

    tpath = ev.get("transcript_path")
    session_id = ev.get("session_id") or "unknown"
    if not tpath or not os.path.exists(tpath):
        return
    # Skip subagent transcripts: they fire SubagentStop, not SessionEnd. This also
    # stops a backfill glob from turning agent-*.jsonl sidechains into "sessions".
    if session_id.startswith("agent-") or os.path.basename(tpath).startswith("agent-"):
        return

    d = parse_transcript(tpath)
    if not has_signal(d):
        return

    # Bucket by the day work STARTED — more accurate than end-time for sessions
    # left open across midnight.
    day_dt = (iso_to_local(d["first_ts"]) or iso_to_local(d["last_ts"])
              or datetime.now().astimezone())
    day = day_dt.strftime("%Y-%m-%d")

    os.makedirs(WORKLOG_DIR, exist_ok=True)
    day_md_path = os.path.join(WORKLOG_DIR, f"{day}.md")
    block = build_block(d, session_id)
    upsert(day_md_path, day, day_dt.strftime("%A"), session_id, d["first_ts"], block)


if __name__ == "__main__":
    try:
        main()
    except Exception:
        pass  # A worklog hook must never disrupt session exit.
    sys.exit(0)
