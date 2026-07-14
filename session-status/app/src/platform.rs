//! Platform-specific bits: process inspection and terminal/tty detection for the recorder,
//! plus (Windows) generic window-focus helpers used by the terminal adapters and fallbacks.
//! Terminal-specific focus/close/spawn behaviour lives in src/terminals/.

use serde_json::{json, Map, Value};

// ============================ Windows ============================
#[cfg(windows)]
mod imp {
    use super::*;
    use crate::recorder::transcript_title;
    use std::collections::{HashMap, HashSet};
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, INVALID_HANDLE_VALUE, LPARAM, RECT};
    use windows_sys::Win32::System::Console::{
        AttachConsole, FreeConsole, GetConsoleTitleW, ATTACH_PARENT_PROCESS,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        AttachThreadInput, GetCurrentThreadId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetForegroundWindow, GetWindow, GetWindowRect, GetWindowTextLengthW,
        GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow,
        GW_OWNER, SW_RESTORE,
    };

    const JETBRAINS_EXES: &[&str] = &[
        "idea64.exe", "idea.exe", "pycharm64.exe", "pycharm.exe", "webstorm64.exe",
        "clion64.exe", "goland64.exe", "phpstorm64.exe", "rider64.exe", "rubymine64.exe",
        "datagrip64.exe", "rustrover64.exe", "aqua64.exe", "fleet.exe",
    ];

    /// pid → (parent pid, lowercase exe name), from a Toolhelp snapshot. Shared with the
    /// wt terminal adapter.
    pub fn process_map() -> HashMap<i64, (i64, String)> {
        let mut map = HashMap::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return map;
            }
            let mut e: PROCESSENTRY32 = std::mem::zeroed();
            e.dwSize = std::mem::size_of::<PROCESSENTRY32>() as u32;
            if Process32First(snap, &mut e) != 0 {
                loop {
                    let name: String = e
                        .szExeFile
                        .iter()
                        .take_while(|&&b| b != 0)
                        .map(|&b| b as u8 as char)
                        .collect::<String>()
                        .to_lowercase();
                    map.insert(e.th32ProcessID as i64, (e.th32ParentProcessID as i64, name));
                    if Process32Next(snap, &mut e) == 0 {
                        break;
                    }
                }
            }
            CloseHandle(snap);
        }
        map
    }

    pub fn parent_pid(pid: i64) -> i64 {
        process_map().get(&pid).map(|(p, _)| *p).unwrap_or(0)
    }

    pub fn is_alive(pid: i64) -> bool {
        if pid <= 0 {
            return false;
        }
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32);
            if h.is_null() {
                return false;
            }
            CloseHandle(h);
            true
        }
    }

    fn win_terminal(start: i64) -> (String, i64) {
        let pmap = process_map();
        let mut chain: Vec<(i64, String)> = Vec::new();
        let (mut pid, mut seen) = (start, HashSet::new());
        for _ in 0..30 {
            if seen.contains(&pid) {
                break;
            }
            let Some((ppid, name)) = pmap.get(&pid) else { break };
            seen.insert(pid);
            chain.push((pid, name.clone()));
            pid = *ppid;
        }
        for (p, n) in &chain {
            if n == "windowsterminal.exe" {
                return ("wt".into(), *p);
            }
        }
        for (p, n) in &chain {
            if JETBRAINS_EXES.contains(&n.as_str()) {
                return ("jetbrains".into(), *p);
            }
        }
        for (p, n) in &chain {
            if n == "code.exe" {
                return ("vscode".into(), *p);
            }
        }
        for (p, n) in &chain {
            if n == "conhost.exe" || n == "openconsole.exe" {
                return ("console".into(), *p);
            }
        }
        ("other".into(), chain.first().map(|(p, _)| *p).unwrap_or(start))
    }

    fn console_title() -> String {
        unsafe {
            let attached = AttachConsole(ATTACH_PARENT_PROCESS) != 0;
            let mut buf = [0u16; 1024];
            GetConsoleTitleW(buf.as_mut_ptr(), buf.len() as u32);
            if attached {
                FreeConsole();
            }
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let t = String::from_utf16_lossy(&buf[..end]).trim().to_string();
            let low = t.to_lowercase();
            if t.is_empty()
                || low.contains(".exe")
                || low.ends_with("bash")
                || low.ends_with("pwsh")
                || low.ends_with("cmd")
                || low.ends_with("powershell")
            {
                String::new()
            } else {
                t
            }
        }
    }

    fn tab_title(transcript: &str, topic: &str) -> String {
        let ct = console_title();
        if !ct.is_empty() {
            return ct;
        }
        let title = transcript_title(transcript);
        if title.is_empty() {
            topic.to_string()
        } else {
            title
        }
    }

    pub fn annotate(rec: &mut Map<String, Value>, pid: i64, transcript: &str, topic: &str) {
        rec.insert("tty".into(), json!(""));
        let (term, term_pid) = win_terminal(pid);
        rec.insert("terminal".into(), json!(term));
        rec.insert("term_pid".into(), json!(term_pid));
        rec.insert("tab_title".into(), json!(tab_title(transcript, topic)));
    }

    // ---- window focus (jump) ----

    #[repr(C)]
    struct Find {
        pid: u32,
        best: HWND,
        best_area: i64,
    }

    unsafe extern "system" fn enum_proc(h: HWND, l: LPARAM) -> i32 {
        let ctx = &mut *(l as *mut Find);
        let mut wpid: u32 = 0;
        GetWindowThreadProcessId(h, &mut wpid);
        if wpid != ctx.pid {
            return 1;
        }
        if IsWindowVisible(h) == 0 {
            return 1;
        }
        if !GetWindow(h, GW_OWNER).is_null() {
            return 1; // owned/tool popup
        }
        if GetWindowTextLengthW(h) == 0 {
            return 1; // untitled shell
        }
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetWindowRect(h, &mut r);
        let area = (r.right - r.left).max(0) as i64 * (r.bottom - r.top).max(0) as i64;
        if area > ctx.best_area {
            ctx.best_area = area;
            ctx.best = h;
        }
        1
    }

    pub fn main_window_for_pid(pid: i64) -> HWND {
        let mut ctx = Find { pid: pid as u32, best: std::ptr::null_mut(), best_area: -1 };
        unsafe {
            EnumWindows(Some(enum_proc), &mut ctx as *mut _ as LPARAM);
        }
        ctx.best
    }

    pub fn focus_window(h: HWND) -> bool {
        if h.is_null() {
            return false;
        }
        unsafe {
            if IsIconic(h) != 0 {
                ShowWindow(h, SW_RESTORE);
            }
            let fg = GetForegroundWindow();
            let mut tmp = 0u32;
            let fg_thread = GetWindowThreadProcessId(fg, &mut tmp);
            let our = GetCurrentThreadId();
            let target = GetWindowThreadProcessId(h, &mut tmp);
            AttachThreadInput(our, fg_thread, 1);
            AttachThreadInput(target, fg_thread, 1);
            let ok = SetForegroundWindow(h) != 0;
            AttachThreadInput(target, fg_thread, 0);
            AttachThreadInput(our, fg_thread, 0);
            ok
        }
    }

    /// Attach to the launching console so CLI subcommand output (install/uninstall/markers) is
    /// visible — release builds use the windows subsystem, which detaches stdio.
    pub fn attach_parent_console() {
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }

}

// ============================ Unix (macOS) ============================
#[cfg(unix)]
mod imp {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// App bundle names that mean "a JetBrains IDE hosts this session" — matched against the
    /// lowercased basename of each ancestor's comm. The id "jetbrains" is the contract with
    /// terminals/jetbrains.rs (and the IntelliJ plugin behind it).
    const JETBRAINS_APPS: &[&str] = &[
        "idea", "intellij", "jetbrains", "pycharm", "webstorm", "clion", "goland",
        "phpstorm", "rider", "rubymine", "datagrip", "rustrover", "aqua", "fleet",
    ];

    pub fn parent_pid(_pid: i64) -> i64 {
        unsafe { libc::getppid() as i64 }
    }

    /// pid → (parent pid, comm) for every process, from one `ps` call. comm can be a full
    /// path containing spaces ("…/IntelliJ IDEA.app/…/idea"), so split only twice.
    fn process_map() -> HashMap<i64, (i64, String)> {
        let mut map = HashMap::new();
        let Ok(o) = std::process::Command::new("ps").args(["-axo", "pid=,ppid=,comm="]).output()
        else {
            return map;
        };
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            let t = line.trim_start();
            let Some((pid_s, rest)) = t.split_once(char::is_whitespace) else { continue };
            let rest = rest.trim_start();
            let Some((ppid_s, comm)) = rest.split_once(char::is_whitespace) else { continue };
            let (Ok(pid), Ok(ppid)) = (pid_s.parse::<i64>(), ppid_s.parse::<i64>()) else {
                continue;
            };
            map.insert(pid, (ppid, comm.trim().to_string()));
        }
        map
    }

    /// The unix analogue of win_terminal(): walk the parent chain and identify the hosting
    /// terminal app. Sessions with neither a terminal id nor a term_pid are hidden by the
    /// widget, so "other" still carries a pid to keep unknown hosts visible.
    fn unix_terminal(start: i64) -> (String, i64) {
        let pmap = process_map();
        let mut chain: Vec<(i64, String)> = Vec::new();
        let (mut pid, mut seen) = (start, HashSet::new());
        for _ in 0..30 {
            if pid <= 1 || seen.contains(&pid) {
                break;
            }
            let Some((ppid, comm)) = pmap.get(&pid) else { break };
            seen.insert(pid);
            let base = std::path::Path::new(comm)
                .file_name()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            chain.push((pid, base));
            pid = *ppid;
        }
        for (p, n) in &chain {
            if JETBRAINS_APPS.iter().any(|a| n.contains(a)) {
                return ("jetbrains".into(), *p);
            }
        }
        for (p, n) in &chain {
            if n.contains("iterm") {
                return ("iterm".into(), *p);
            }
        }
        for (p, n) in &chain {
            if n == "terminal" {
                return ("terminal".into(), *p);
            }
        }
        ("other".into(), chain.first().map(|(p, _)| *p).unwrap_or(start))
    }

    pub fn is_alive(pid: i64) -> bool {
        if pid <= 0 {
            return false;
        }
        unsafe {
            if libc::kill(pid as i32, 0) == 0 {
                true
            } else {
                std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
            }
        }
    }

    fn tty_of(pid: i64) -> String {
        if let Ok(o) = std::process::Command::new("ps")
            .args(["-o", "tty=", "-p", &pid.to_string()])
            .output()
        {
            let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if t.is_empty() || t == "??" || t == "?" {
                String::new()
            } else {
                t
            }
        } else {
            String::new()
        }
    }

    /// The IntelliJ terminal tab name for this tty (e.g. "TT AGI-18033"), published under
    /// tab-names/ by the Claude Sessions IntelliJ plugin. Only real titles count: stock
    /// ("Local", "Local (2)") and launcher-default ("claude") names are skipped, as are
    /// entries not refreshed in the last 15s (leftovers from a closed project).
    fn ide_tab_label(tty: &str) -> String {
        let (mut best, mut best_ts) = (String::new(), 0.0f64);
        if tty.is_empty() {
            return best;
        }
        let Ok(entries) = std::fs::read_dir(crate::paths::tab_names_dir()) else { return best };
        let now = crate::paths::unix_now();
        for e in entries.flatten() {
            let v = crate::paths::load_json(&e.path());
            let Some(entry) = v.get(tty) else { continue };
            let name = crate::paths::str_of(entry, "name").trim().to_string();
            let ts = crate::paths::f64_of(entry, "ts");
            if name.is_empty() || is_default_tab(&name) || now - ts > 15.0 || ts <= best_ts {
                continue;
            }
            best = name;
            best_ts = ts;
        }
        best
    }

    fn is_default_tab(name: &str) -> bool {
        let base: String = name.chars().filter(|c| !c.is_whitespace()).collect();
        base == "Local"
            || (base.starts_with("Local(") && base.ends_with(')'))
            || base.eq_ignore_ascii_case("claude")
    }

    /// Mirrors the Windows tab_title(): best display title for the session — the hosting
    /// IDE's tab name, else the transcript's AI title, else the derived topic.
    pub fn annotate(rec: &mut Map<String, Value>, pid: i64, transcript: &str, topic: &str) {
        let tty = tty_of(pid);
        let (term, term_pid) = unix_terminal(pid);
        let mut title =
            if term == "jetbrains" { ide_tab_label(&tty) } else { String::new() };
        if title.is_empty() {
            title = crate::recorder::transcript_title(transcript);
        }
        if title.is_empty() {
            title = topic.to_string();
        }
        rec.insert("tty".into(), json!(tty));
        rec.insert("terminal".into(), json!(term));
        rec.insert("term_pid".into(), json!(term_pid));
        rec.insert("tab_title".into(), json!(title));
    }

    pub fn attach_parent_console() {} // unix CLIs already share the terminal's stdio
}

pub use imp::{annotate, attach_parent_console, is_alive, parent_pid};
#[cfg(windows)]
pub use imp::{focus_window, main_window_for_pid, process_map};
