//! `install-app` / `uninstall-app`: copy the binary to a permanent location and point
//! launch-at-login at it, so autostart survives moving or `cargo clean`ing the build tree.
//! A downloaded release binary can install itself the same way.

use crate::paths;
use auto_launch::AutoLaunchBuilder;

/// Registry value name / login-item name. Must match what tauri-plugin-autostart uses (the
/// product name) so the GUI's "Start at login" toggle and these subcommands share one entry.
const APP_NAME: &str = "claude-sessions";

/// An AutoLaunch bound to `path`. macOS uses a LaunchAgent to match the plugin's
/// `MacosLauncher::LaunchAgent`; on Windows the mechanism is the HKCU Run key either way.
fn autolaunch(path: &std::path::Path) -> Option<auto_launch::AutoLaunch> {
    let p = path.to_string_lossy().to_string();
    AutoLaunchBuilder::new()
        .set_app_name(APP_NAME)
        .set_app_path(&p)
        .set_use_launch_agent(cfg!(target_os = "macos"))
        .build()
        .ok()
}

/// Copy this binary into the permanent install dir, enable launch-at-login pointing there,
/// replace any already-running instance, and launch the installed copy.
pub fn install_app() -> i32 {
    let cur = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("install-app: can't resolve own path: {e}");
            return 1;
        }
    };
    let dest = paths::installed_exe();
    let already = same_file(&cur, &dest);

    if !already {
        if let Some(dir) = dest.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("install-app: can't create {}: {e}", dir.display());
                return 1;
            }
        }
        stop_other_instances(); // free the file lock + the single global-hotkey registration
        if let Err(e) = std::fs::copy(&cur, &dest) {
            eprintln!("install-app: copy to {} failed: {e}", dest.display());
            eprintln!("  (quit any running installed copy, then try again)");
            return 1;
        }
    }

    // Point launch-at-login at the installed path. This overwrites the single Run entry, so an
    // install that was previously enabled from a build path is corrected here too.
    match autolaunch(&dest).map(|al| al.enable()) {
        Some(Ok(())) => {}
        _ => eprintln!("install-app: warning — couldn't register launch-at-login"),
    }

    if !already {
        let _ = std::process::Command::new(&dest).spawn();
    }
    println!("Installed to {}", dest.display());
    println!("Launch-at-login is on (turn it off any time under the widget's ⚙ options).");
    0
}

/// Disable launch-at-login and remove the installed binary (unless we're running from it).
pub fn uninstall_app() -> i32 {
    let dest = paths::installed_exe();
    match autolaunch(&dest).map(|al| al.disable()) {
        Some(Ok(())) => {}
        _ => eprintln!("uninstall-app: warning — couldn't clear launch-at-login"),
    }
    let running_from_install = std::env::current_exe()
        .map(|c| same_file(&c, &dest))
        .unwrap_or(false);
    if running_from_install {
        println!(
            "Launch-at-login removed. Delete {} yourself once this exits.",
            dest.display()
        );
        return 0;
    }
    if dest.exists() {
        let _ = std::fs::remove_file(&dest);
    }
    if let Some(dir) = dest.parent() {
        let _ = std::fs::remove_dir(dir); // only succeeds if now empty — leave a non-empty dir be
    }
    println!(
        "Uninstalled (launch-at-login cleared, {} removed).",
        dest.display()
    );
    0
}

/// Best-effort compare of two paths, canonicalising when both exist (handles `.`/casing).
fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Terminate other running copies so the freshly installed one is the sole widget (and owns the
/// global hotkeys). Excludes this process. Best-effort — failures are ignored.
fn stop_other_instances() {
    #[cfg(windows)]
    {
        let me = std::process::id();
        let _ = std::process::Command::new("taskkill")
            .args([
                "/F",
                "/IM",
                "claude-sessions.exe",
                "/FI",
                &format!("PID ne {me}"),
            ])
            .output();
    }
    #[cfg(unix)]
    {
        // pkill matches by name and would include ourselves; --older isn't reliable, so leave a
        // second instance to no-op its hotkey registration rather than risk killing this process.
    }
}
