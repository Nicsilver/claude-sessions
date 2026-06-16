# session-status

Native macOS surfaces that show, at a glance, which Claude Code sessions are
waiting on you — the data source behind the companion IntelliJ plugin in this repo.

## How it works

Claude Code hooks invoke `bin/record.py` on session lifecycle events
(`start` / `working` / `needs` / `done` / `end`). It writes a tiny per-session
state file to `~/.claude/session-status/state/<session_id>.json` and fires a
macOS notification when a session flips into "needs you".

Two surfaces read that state:

- **`bin/menubar.swift`** — a menu-bar badge (`🔴N` / `🟡N` / `🟢N` / `✅`) with a
  dropdown of every session.
- **`bin/floatdash.swift`** — an always-on-top floating dashboard. Click a row to
  jump to that session's terminal tab/pane (writes
  `~/.claude/session-status/focus-request.json`, which the IntelliJ plugin watches).

Code lives here in the repo; **runtime data** (`state/`, `focus-request.json`)
lives in `~/.claude/session-status/`, shared with the IntelliJ plugin.

## Build & run

```sh
cd bin
swiftc -O floatdash.swift -o floatdash
swiftc -O menubar.swift   -o menubar

./status float      # floating dashboard (top-right, always on top)
./status menubar    # menu-bar badge
./status uninstall  # remove hooks from settings.json + clear state
```

## Wiring the hooks

Point Claude Code's hooks (in `~/.claude/settings.json`) at `bin/record.py`, e.g.:

```json
{ "type": "command",
  "command": "/opt/homebrew/bin/python3 /ABSOLUTE/PATH/TO/session-status/bin/record.py working" }
```

Events: `SessionStart → start`, `UserPromptSubmit`/`PostToolUse → working`,
`Notification → needs`, `Stop → done`, `SessionEnd → end`.
