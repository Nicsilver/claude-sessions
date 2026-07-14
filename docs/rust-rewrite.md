# Rust rewrite (branch `rust-rewrite`)

Goal: replace the WPF (Windows) + Swift (macOS) + Python (recorder) stack with **one
cross-platform Rust binary** — `claude-sessions` — that is the GUI, the hook recorder, and the
installer. Lives in `session-status/app/`. The old native apps on `main` are untouched.

Payoff already realised: the Windows binary is **~3.4 MB** (vs 74 MB self-contained .NET) with **no
runtime dependency**, and there is **no Python**.

## The single binary

`claude-sessions[.exe]` with subcommands:
- (no args) → launch the floating dashboard + tray (double-click entry point).
- `record <start|working|needs|done|end>` → the Claude Code hook recorder (writes state JSON).
- `install` / `uninstall` → wire/unwire the hooks in `~/.claude/settings.json`.

On first GUI launch it auto-runs `install` (backs up settings.json first, non-destructive), so
**just running the exe is enough** — no manual steps, no Python.

## Status

**Done & verified (Windows):**
- Recorder — full port of `record.py`, verified against it (working/done/end, topic derivation,
  Toolhelp process-tree terminal detection, ConPTY tab-title, turn classification).
- Installer — non-destructive settings.json merge; preserves your other hooks; recognises the old
  `record.py` hooks so it migrates off Python cleanly. Verified.
- Dashboard (egui) — dark rounded top-right always-on-top panel: header + × (hide-to-tray),
  session rows (state dot · name · age), footer count chips, empty state. Rendered & screenshot-verified.
- Tray icon — state-coloured badge + menu (Show/Hide dashboard, Quit); left-click toggles.
- Jump — left-click a row focuses the owning terminal window by pid (SetForegroundWindow + thread-attach).
- 1.5s refresh; heartbeat thread keeps the badge live while hidden; `windows_subsystem=windows` so
  hooks never flash a console.

**Not yet ported (parity gaps — tracked for later phases):**
- Windows Terminal **per-tab** focus via UI Automation (`WtTabs.cs`) — jump focuses the window, not the tab.
- **New session** (+) and **close session** (right-click Ctrl+Shift+W) keystroke actions.
- **Rename** (Alt-click) / **mute** (middle-click) UI — the model writers exist, no GUI wiring yet.
- Tray in-icon **count number** (badge is colour-only) and a tray **session list**.
- **Notifications** on "needs you".
- **Non-activating** tool window (clicking the panel may steal focus); **auto-height** panel (fixed 440px).
- **macOS**: recorder + install are cross-platform and build; the GUI compiles (egui + tray-icon gives a
  menu-bar item) but window focus/jump, notifications, and login-item are TODO.
- **Global hotkeys.**

## Testing it (Windows)

```
cd session-status/app
cargo build --release        # needs Rust >= 1.85 (deps use edition 2024)
./target/release/claude-sessions.exe
```
First launch wires the hooks (backs up `~/.claude/settings.json`). Start a NEW Claude session and it
should populate; left-click a row to jump. Revert anytime with:
```
./target/release/claude-sessions.exe uninstall
```
Nothing here touches `main` — the WPF widget + Python recorder still work there.

CI: `.github/workflows/rust-app.yml` builds the binary for Windows + macOS on this branch and uploads
both as run artifacts (grab the macOS one to test there).

## Roadmap

1. ✅ Recorder + installer (Phase 1) · ✅ egui dashboard + tray + jump (Phase 3 MVP).
2. WT UIA tab-select, new/close keystrokes, rename/mute UI, tray count number + session menu.
3. Notifications, non-activating window, auto-height.
4. macOS focus + notifications + login item; global hotkeys.
5. Retire WPF/Swift, point `release.yml` at the single Rust binary, drop Python everywhere.
