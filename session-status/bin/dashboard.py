#!/usr/bin/env python3
"""Surface A — standalone dashboard window (spike).

A no-dependency TUI that live-renders every Claude session's state by reading
~/.claude/session-status/state/*.json. Run it in its own terminal window:

    /opt/homebrew/bin/python3 ~/.claude/session-status/bin/dashboard.py

Ctrl-C to quit. Prunes dead sessions (parent process gone).
"""
import os, json, time, sys

STATE_DIR = os.path.expanduser("~/.claude/session-status/state")

GLYPH = {"needs": "\U0001F534", "yourturn": "\U0001F7E1", "working": "\U0001F7E2", "done": "✅", "idle": "⚪"}
ORDER = {"needs": 0, "yourturn": 1, "working": 2, "idle": 3, "done": 4}
COLOR = {"needs": "\033[91m", "yourturn": "\033[93m", "working": "\033[92m", "done": "\033[90m", "idle": "\033[37m"}
LABEL = {"needs": "NEEDS YOU", "yourturn": "your turn", "working": "working", "done": "done · safe to close", "idle": "idle"}
RESET = "\033[0m"; DIM = "\033[2m"; BOLD = "\033[1m"


def alive(pid):
    if not pid:
        return True
    try:
        os.kill(int(pid), 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    except Exception:
        return True


def load():
    out = []
    try:
        files = os.listdir(STATE_DIR)
    except FileNotFoundError:
        return out
    for fn in files:
        if not fn.endswith(".json"):
            continue
        p = os.path.join(STATE_DIR, fn)
        try:
            with open(p) as f:
                rec = json.load(f)
        except Exception:
            continue
        if not alive(rec.get("pid") or rec.get("ppid")):
            try:
                os.remove(p)
            except OSError:
                pass
            continue
        out.append(rec)
    return out


def age(ts):
    if not ts:
        return ""
    s = int(time.time() - ts)
    if s < 60:
        return "{}s".format(s)
    if s < 3600:
        return "{}m".format(s // 60)
    return "{}h".format(s // 3600)


def render():
    recs = load()
    recs.sort(key=lambda r: (ORDER.get(r.get("state"), 9), -(r.get("updated_at") or 0)))
    needs = sum(1 for r in recs if r.get("state") == "needs")
    yourturn = sum(1 for r in recs if r.get("state") == "yourturn")
    working = sum(1 for r in recs if r.get("state") == "working")
    done = sum(1 for r in recs if r.get("state") == "done")

    lines = []
    lines.append("{b}Claude sessions{r}   \U0001F534 {n} need you   \U0001F7E1 {y} your turn   \U0001F7E2 {w} working   ✅ {d} done    {dim}{t}{r}".format(
        b=BOLD, r=RESET, dim=DIM, n=needs, y=yourturn, w=working, d=done, t=time.strftime("%H:%M:%S")))
    lines.append("")
    if not recs:
        lines.append("{dim}(no active sessions — start a Claude session to populate){r}".format(dim=DIM, r=RESET))
    for rec in recs:
        st = rec.get("state", "?")
        g = GLYPH.get(st, "·")
        c = COLOR.get(st, "")
        label = rec.get("topic") or "?"
        if not rec.get("tty"):
            label += " (ide)"
        topic = label[:42].ljust(42)
        lab = LABEL.get(st, st)
        a = age(rec.get("updated_at"))
        msg = ""
        if st in ("needs", "yourturn") and rec.get("message"):
            msg = "  {dim}{m}{r}".format(dim=DIM, m=rec["message"][:54], r=RESET)
        lines.append("{c}{g} {topic}{r} {c}{lab:<22}{r}{dim}{a:>5}{r}{msg}".format(
            c=c, g=g, topic=topic, r=RESET, lab=lab, dim=DIM, a=a, msg=msg))
    return "\n".join(lines)


def main():
    try:
        while True:
            out = render()
            sys.stdout.write("\033[2J\033[H")
            sys.stdout.write(out + "\n")
            sys.stdout.flush()
            time.sleep(1.0)
    except KeyboardInterrupt:
        sys.stdout.write("\n")


if __name__ == "__main__":
    main()
