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

## 3. CI on `main` + tests

- `rust-app.yml` is pinned to the dead `rust-rewrite` branch, so **nothing verifies that `main`
  builds** — `release.yml` only runs on tags. Point a CI workflow at push/PR to `main` that runs
  `cargo build`, `cargo fmt --check`, and `cargo clippy -D warnings` on Windows + macOS.
- Add unit tests for the invariant-critical logic (currently zero tests):
  - the non-destructive hook merge in `install.rs` — **must never** clobber unrelated hooks
    (e.g. Nic's "awake" hooks); `is_recorder_cmd` only matches recorder executables.
  - the fuzzy WT tab matching in `terminals/wt.rs` (token overlap, exact-title win, the single
    generic "Claude Code" fallback, and "don't guess on a tie").
  - the ⏳/✅ turn-marker parsing in `recorder.rs` (marker → Done/Your-turn, `?`-fallback).

## 4. Auto-hide / auto-show (two independent options)

Two **separate** toggles in the options pane:
- **Auto-hide** — hide the dashboard to the tray when there are no live sessions.
- **Auto-show** — pop the dashboard back up when a session appears (or flips to needs-you).

They must be independent (either on its own, both, or neither). Driven off the 1.5 s heartbeat's
session count. Store as `auto_hide` / `auto_show` in `config.json`.

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
