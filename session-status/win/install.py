#!/usr/bin/env python3
"""Non-destructive installer for the Claude session-status hooks on Windows.

Adds (or removes) hook commands that call record.py on session lifecycle events, WITHOUT
touching any hooks you already have — each event's hook list gets our group appended, and
`uninstall` removes only our group. A timestamped backup of settings.json is written first.

Usage:
  python install.py install
  python install.py uninstall
"""
import sys, os, json, shutil, time

SETTINGS = os.path.expanduser("~/.claude/settings.json")
RECORD = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bin", "record.py")
RECORD = os.path.abspath(RECORD)
PYTHON = sys.executable  # the interpreter running this installer — a known-good absolute path

# event -> record.py state argument
EVENTS = {
    "SessionStart":     "start",
    "UserPromptSubmit": "working",
    "PostToolUse":      "working",
    "Notification":     "needs",
    "Stop":             "done",
    "SessionEnd":       "end",
}

MARKER = "record.py"  # how we recognise our own hook groups on re-install / uninstall


def our_command(state):
    return f'"{PYTHON}" "{RECORD}" {state}'


def is_ours(group):
    try:
        return any(MARKER in h.get("command", "") for h in group.get("hooks", []))
    except Exception:
        return False


def load():
    try:
        with open(SETTINGS, encoding="utf-8") as f:
            return json.load(f)
    except FileNotFoundError:
        return {}


def backup():
    if os.path.exists(SETTINGS):
        dst = SETTINGS + ".bak-" + time.strftime("%Y%m%d-%H%M%S")
        shutil.copy2(SETTINGS, dst)
        print("backed up settings.json ->", os.path.basename(dst))


def save(data):
    with open(SETTINGS, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=2)


def install():
    data = load()
    hooks = data.setdefault("hooks", {})
    added = 0
    for event, state in EVENTS.items():
        groups = hooks.setdefault(event, [])
        # remove any stale group of ours, then append the current one (idempotent)
        groups[:] = [g for g in groups if not is_ours(g)]
        groups.append({"hooks": [{"type": "command", "command": our_command(state)}]})
        added += 1
    backup()
    save(data)
    print(f"installed session-status hooks on {added} events")
    print("record.py:", RECORD)
    print("python   :", PYTHON)


def uninstall():
    data = load()
    hooks = data.get("hooks", {})
    removed = 0
    for event in list(hooks.keys()):
        before = len(hooks[event])
        hooks[event][:] = [g for g in hooks[event] if not is_ours(g)]
        removed += before - len(hooks[event])
        if not hooks[event]:
            del hooks[event]        # leave the event clean if it's now empty
    if not hooks:
        data.pop("hooks", None)
    backup()
    save(data)
    print(f"removed {removed} session-status hook group(s); your other hooks are untouched")


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "install"
    if cmd == "install":
        install()
    elif cmd == "uninstall":
        uninstall()
    else:
        print("usage: install.py {install|uninstall}")
        sys.exit(1)
