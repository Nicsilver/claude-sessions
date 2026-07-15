# Roadmap

Planned improvements, roughly in priority order. Nothing here is committed to a release yet.

## 1. Stable install + reliable launch-at-login  ✅ done

`claude-sessions install-app` copies the running binary to a permanent per-user location
(`%LOCALAPPDATA%\ClaudeSessions` on Windows, `~/Library/Application Support/ClaudeSessions` on
macOS), registers launch-at-login pointing *there* (overwriting the single Run entry, so an install
previously enabled from a build path is corrected), stops any other running instance, and launches
the installed copy — whose first run also re-points the hooks at the installed exe. `uninstall-app`
clears launch-at-login and removes the binary. Verified end-to-end on Windows (see `selfinstall.rs`).

## 2. Auto-update

A background always-on tool should update itself. The release pipeline already emits per-OS
artifacts; add the Tauri **updater plugin** + an `latest.json` update manifest published on the
GitHub Release, plus an update signing keypair (separate from code signing). Check on launch and
maybe a "Check for updates" tray item. Best paired with the SignPath code-signing work so updates
don't trip SmartScreen.

## 3. CI on `main` + tests  ✅ done

`.github/workflows/ci.yml` runs on every push/PR to `main` (Windows + macOS): `cargo fmt --check`,
`cargo clippy --all-targets -D warnings`, `cargo test`, and a release build. Replaced the dead
`rust-app.yml` (which was pinned to the merged `rust-rewrite` branch, so nothing verified `main`).
Unit tests cover the invariant-critical logic:
- the non-destructive hook merge (`is_recorder_cmd`/`is_ours` in `install.rs`) — proves a user's
  unrelated hooks (e.g. "awake") are never matched as ours.
- the fuzzy WT tab matching, extracted into the platform-independent `terminals/tabmatch.rs`
  (token overlap, exact-title win, single generic "Claude Code" fallback, don't-guess-on-a-tie).
- the ⏳/✅ turn-marker parsing (`classify_turn_text` in `recorder.rs`).

## 4. Auto-hide / auto-show (two independent options)  ✅ done

Two independent toggles in the options pane, stored as `auto_hide` (default off) / `auto_show`
(default on) in `config.json` and driven off the 1.5 s heartbeat:
- **Hide when nothing is live** — drops to the tray when the last live session finishes.
- **Show on new or needs-you** — pops the panel back when a session appears or flips to needs-you.

Both are **edge-triggered** (`auto_action` in `gui.rs`): they act only on the heartbeat where the
count crosses zero or a session *newly* asks for you, never on the standing level — otherwise a
hand-hidden panel would be dragged back up every 1.5 s. Auto-hide is also applied once as a level
check at startup, so launching at login with nothing running goes straight to the tray. "Live"
means unmuted and in one of `needs`/`yourturn`/`working`, matching the tray badge.

## 5. Light theme

The UI (`app/ui/index.html`, `menu.html`) is hard-coded dark. Make it theme-aware: follow the OS
via `prefers-color-scheme`, with an explicit override option (Auto / Light / Dark) in the options
pane. Tray badge colours already come from `styles.rs` and are fine on both.

---

## Other ideas considered

- More global hotkeys: show/hide the dashboard, jump-to-Nth session (Ctrl+Alt+1/2/3), cycle
  through needs-you sessions.
- Remember a manually dragged window position instead of always snapping to top-right.
- Distribution via winget / scoop / Homebrew once signing lands.
