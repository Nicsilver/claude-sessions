# session-status — Windows

A native Windows port of the macOS floating dashboard + menu-bar badge, built as a single
**WPF** app (`ClaudeSessions/`). It reads the same per-session state that `bin/record.py` writes
to `~/.claude/session-status/state/*.json` and gives you, at a glance, which Claude Code sessions
are waiting on you — with click-to-jump straight to the right terminal tab.

## Two surfaces, one process

- **Floating dashboard** — an always-on-top, borderless, non-activating panel pinned top-right.
  Per-row state bar + glow, count chips, live 1.5s refresh, drop shadow. Left-click a row to jump
  to its terminal, right-click to close it, middle-click to mute 1h, Alt-click to rename. Drag it
  by the title bar or the footer. `×` hides it to the tray; `+` starts a new session.
- **Tray badge** — the menu-bar-badge analog: a slim state-colored accent pill + the count of the
  most-urgent live state. Left-click jumps to the top session, right-click lists them, and a
  session flipping into "needs you" raises a clickable balloon.

## How the data gets there

`bin/record.py` is cross-platform. On Windows it has no tty, so it walks the process tree to tag
each session's owning terminal (`terminal`: `wt` / `jetbrains` / `vscode` / `console`) plus the
`term_pid` to focus and a `tab_title` (Claude's own tab name) used for display and tab matching.

## Jump

- **Windows Terminal** — resolve `term_pid` → the WT window, focus it, and switch to the session's
  tab via **UI Automation** (WT exposes each tab as a `TabItem` with `SelectionItemPattern`; there
  is no CLI/plugin API for per-tab focus). Tab matching is fuzzy on the title so it survives renames.
- **"+" new session** — opens a new tab in the *current* WT window (Ctrl+Shift+T, then types your
  shell command) rather than a new window; elevated WT rejects the `wt -w 0` remote-tab command.

## Build & run

Requires the .NET SDK (`winget install Microsoft.DotNet.SDK.9`).

```powershell
./status.ps1 build      # compile the widget (Release)
./status.ps1 float      # start the floating widget + tray (idempotent)
./status.ps1 float-stop # stop it
./status.ps1 install    # wire record.py into ~/.claude/settings.json hooks (non-destructive, backs up)
./status.ps1 uninstall  # remove ONLY our hooks + clear state (your other hooks are left alone)
```

The hook installer (`install.py`) appends its own hook group per event and never touches hooks you
already have; `uninstall` removes only that group.
