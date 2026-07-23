# Adding support for a terminal

Terminal-specific behaviour lives in one adapter file per terminal under
`session-status/app/src/terminals/`. The widget never branches on terminal type — it calls
`terminals::focus / close / new_session`, which dispatch to the adapter matching the session's
`terminal` field and fall back generically (focus-window-by-pid / kill-process-tree) when no
adapter claims it.

## The recipe

1. **Detection** — teach the recorder to tag sessions with your terminal's id:
   `platform.rs :: win_terminal()` (Windows, walks the process tree) or the unix `annotate()`.
   The id string ("wt", "jetbrains", ...) is the contract between recorder and adapter.
2. **Adapter** — add `terminals/<name>.rs` implementing the `Terminal` trait:
   - `focus(&Sess)` — bring the session's tab/window to front. Return `true` when handled
     (including a deliberate safe no-op); `false` triggers the generic window-focus fallback.
   - `close(&Sess)` — close the tab. `false` falls back to killing the process tree, so
     return `true` if you attempted anything (or safely declined).
   - `new_session(&[String])` — open a new tab/window and run the configured commands.
   - Keep OS-specific code inside the file behind `#[cfg(...)]`; the trait is neutral.
3. **Register** — add the adapter to `registry()` in `terminals/mod.rs` (per-platform).
   Registry order matters: the first entry is the default "open new sessions in" target,
   and the fallback order for spawning.

Useful fields on `Sess`: `pid` (the claude process), `term_pid` (the hosting terminal
process), `tab_title` + `topic` (for title-based tab matching — see `wt.rs` for a careful
fuzzy matcher), `tty` (mac only).

`wt.rs` (UI Automation + synthetic keystrokes), `wezterm.rs` (drives the terminal's own
control CLI; Windows + macOS from one file with small `#[cfg]` islands) and `jetbrains.rs`
(hands the work to the IntelliJ plugin via `focus-request.json`) are the reference
implementations — prefer the `wezterm.rs` shape whenever the terminal exposes a real CLI/API.
