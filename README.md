# Claude Sessions

**See every live Claude Code session at a glance — and jump to the one that needs you.**

A tiny always-on-top dashboard and tray badge, driven by Claude Code lifecycle hooks. Run several sessions across terminals and IDEs, and stop alt-tabbing around to check which one is waiting on you.

<p align="center">
  <img src="docs/img/dashboard.png" width="374" alt="The floating dashboard">
</p>

Each row is a live session, colour-coded by state:

| | State | Meaning |
|---|---|---|
| 🔴 | **Needs you** | Claude asked something (a permission prompt, a question) |
| 🟡 | **Your turn** | Claude finished and is waiting on your reply |
| 🟢 | **Working** | Claude is busy |
| ⚪ | **Done** | Task complete |

The tray/menu-bar icon shows the top-priority state so you know something needs attention even with the dashboard hidden.

## Install

One binary — GUI, hook recorder and installer in one (~5 MB, Rust + [Tauri](https://tauri.app), using the OS webview).

1. Grab `ClaudeSessions-win-x64.zip` or `ClaudeSessions-macos-arm64.zip` from the [latest release](https://github.com/Nicsilver/claude-sessions/releases), **or** build it yourself:
   ```
   cd session-status/app && cargo build --release
   ```
2. Run it. First launch wires the Claude Code hooks into `~/.claude/settings.json` automatically (non-destructive — your existing hooks are untouched; `claude-sessions uninstall` removes only ours).
3. Start a **new** Claude Code session and watch it appear.

To make it permanent, run `claude-sessions install-app` once: it copies the binary to a stable per-user location (`%LOCALAPPDATA%\ClaudeSessions` / `~/Library/Application Support/ClaudeSessions`), turns on launch-at-login, and relaunches from there. `claude-sessions uninstall-app` reverses it. (Or just tick **Start at login** in the ⚙ options — but running from the download/build folder means moving that folder breaks startup.)

Optionally, `claude-sessions markers` (or the tray menu) adds a small instruction to your global `CLAUDE.md` that makes Claude end each reply with ✅/⏳, which sharpens the *done* vs *your turn* distinction.

## Using it

| Action | Result |
|---|---|
| Click a session | Jump to its terminal / IDE tab |
| Middle-click | Mute it for an hour (sinks to the bottom) |
| <kbd>Alt</kbd>-click | Rename it inline |
| `+` / `×` | New Claude session / hide to tray |
| ⚙ | Options (terminal, shortcuts, startup) |
| Tray left-click | Jump to the top session |
| Tray right-click | Menu — with the live session list |

<p align="center">
  <img src="docs/img/tray-menu.png" width="242" alt="The tray menu">
</p>

## Shortcuts & options

Two **global hotkeys** — they work from anywhere, even with the dashboard hidden — rebindable in the options pane:

| Shortcut | Action |
|---|---|
| <kbd>Ctrl/⌘</kbd> <kbd>Alt</kbd> <kbd>J</kbd> | Jump to the top session |
| <kbd>Ctrl/⌘</kbd> <kbd>Alt</kbd> <kbd>N</kbd> | Start a new Claude session |

Click the **⚙ gear** for options: which terminal new sessions open in and the command to launch them, rebind the two hotkeys, and toggle **Start at login**.

<p align="center">
  <img src="docs/img/settings.png" width="326" alt="The options pane">
</p>

## What's in the repo

- **`session-status/app`** — the cross-platform Rust app (widget + tray + hook recorder + installer). All visuals are plain HTML/CSS in `ui/`.
- **`src/`** (repo root, Gradle) — the **IntelliJ plugin**: a tool window with the same session list, plus focus/close handling for sessions running in JetBrains terminals.

## How it works

Claude Code hooks (`SessionStart`, `UserPromptSubmit`, `PostToolUse`, `Notification`, `Stop`, `SessionEnd`) call `claude-sessions record <state>`, which writes one small JSON file per session under `~/.claude/session-status/state/`. Every surface just reads that directory — no daemon, no IPC, dead sessions are pruned by liveness-checking their PIDs.

## License

AGPL-3.0
