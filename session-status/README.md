# session-status

The Claude Code session tracker: a small **always-on-top dashboard + tray badge** driven by
Claude Code lifecycle hooks. See the [top-level README](../README.md) for what it does, the
screenshots, and how to install it.

## Layout

- **`app/`** — the product: one cross-platform Rust + [Tauri](https://tauri.app) binary that is
  the GUI widget, the tray icon, the hook **recorder** (`claude-sessions record <state>`) and the
  hook **installer** (`claude-sessions install` / `uninstall`) all in one. All visuals are plain
  HTML/CSS in `app/ui/`.
- **`bin/`**, **`win/`**, **`recorder/`** — the earlier native implementations (the macOS Swift
  menu-bar + floating dashboard, the WPF Windows widget, and a standalone Rust recorder), kept for
  reference only. The `app/` binary supersedes them all.

## How it works

Claude Code hooks (`SessionStart`, `UserPromptSubmit`, `PostToolUse`, `Notification`, `Stop`,
`SessionEnd`) call `claude-sessions record <state>`, which writes one small JSON file per session
under `~/.claude/session-status/state/`. Every surface just reads that directory — no daemon, no
IPC; dead sessions are pruned by liveness-checking their PIDs.

State → colour: `needs` (red — a prompt/permission), `yourturn` (yellow — waiting on you),
`working` (green), `done` (grey). The `Notification` hook drives *needs*; the `Stop` hook decides
*your turn* vs *done* from a `⏳`/`✅` marker on the last line of Claude's reply (run
`claude-sessions markers`, or see the turn-marker note in the root README).
