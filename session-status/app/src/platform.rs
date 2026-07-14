//! Platform-specific bits: terminal/tty detection for the recorder, and window focus for the
//! jump. Public surface is the same on both OSes: `parent_pid`, `annotate`, `jump`.

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

    fn process_map() -> HashMap<i64, (i64, String)> {
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

    fn main_window_for_pid(pid: i64) -> HWND {
        let mut ctx = Find { pid: pid as u32, best: std::ptr::null_mut(), best_area: -1 };
        unsafe {
            EnumWindows(Some(enum_proc), &mut ctx as *mut _ as LPARAM);
        }
        ctx.best
    }

    fn focus_window(h: HWND) -> bool {
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

    pub fn jump(terminal: &str, term_pid: i64, pid: i64, tab_title: &str, topic: &str) {
        if terminal == "jetbrains" {
            return; // handled by the IDE plugin via the request file (future)
        }
        let p = if term_pid > 0 { term_pid } else { pid };
        if p <= 0 {
            return;
        }
        let h = main_window_for_pid(p);
        if h.is_null() {
            return;
        }
        if terminal == "wt" {
            let _ = wt_select_tab(h, tab_title, topic); // best-effort; window focus still helps
        }
        focus_window(h);
    }

    /// Close a session (right-click): for WT select its tab and send Ctrl+Shift+W (guarded on
    /// the terminal actually being foreground so the chord can't hit another app); otherwise
    /// kill the session's process tree. Ported from Interop.CloseSession.
    pub fn close_session(terminal: &str, term_pid: i64, pid: i64, tab_title: &str, topic: &str) {
        if terminal == "jetbrains" {
            return; // IDE plugin territory
        }
        if terminal == "wt" && term_pid > 0 {
            let h = main_window_for_pid(term_pid);
            if !h.is_null() && wt_select_tab(h, tab_title, topic) {
                focus_window(h);
                std::thread::sleep(std::time::Duration::from_millis(200));
                if unsafe { GetForegroundWindow() } == h {
                    send_chord(&[VK_CONTROL, VK_SHIFT, 0x57]); // Ctrl+Shift+W
                    return;
                }
            }
            return; // couldn't identify the tab confidently — don't close the wrong one
        }
        if pid > 0 {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .spawn();
        }
    }

    /// Open a new Claude session: focus a Windows Terminal window, Ctrl+Shift+T for a fresh tab,
    /// then TYPE the configured commands (an elevated WT refuses `wt -w 0` remoting, and typing
    /// runs in the user's own shell so profile aliases work). Ported from Interop.NewClaudeSession.
    pub fn new_session(cmds: &[String]) {
        use std::time::Duration;
        let wt_pid = process_map()
            .iter()
            .find(|(_, (_, name))| name == "windowsterminal.exe")
            .map(|(p, _)| *p);
        let mut h = wt_pid.map(main_window_for_pid).unwrap_or(std::ptr::null_mut());
        if h.is_null() {
            // No terminal open — start one, then find its window.
            let _ = std::process::Command::new("wt").spawn();
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(250));
                if let Some((p, _)) = process_map()
                    .iter()
                    .find(|(_, (_, name))| name == "windowsterminal.exe")
                    .map(|(p, n)| (*p, n.clone()))
                {
                    h = main_window_for_pid(p);
                    if !h.is_null() {
                        break;
                    }
                }
            }
            if h.is_null() {
                return;
            }
            focus_window(h);
            std::thread::sleep(Duration::from_millis(800)); // let the first shell come up
        } else {
            focus_window(h);
            std::thread::sleep(Duration::from_millis(200));
            if unsafe { GetForegroundWindow() } != h {
                return; // focus failed — don't type into whatever is active
            }
            send_chord(&[VK_CONTROL, VK_SHIFT, 0x54]); // Ctrl+Shift+T: new tab
            std::thread::sleep(Duration::from_millis(700)); // let the shell start
        }
        for cmd in cmds {
            if unsafe { GetForegroundWindow() } != h {
                return; // user switched away mid-typing — stop
            }
            type_text(cmd);
            send_chord(&[VK_RETURN]);
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    // ---- synthetic keyboard input ----

    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
        VK_CONTROL, VK_RETURN, VK_SHIFT,
    };

    fn key_input(vk: u16, scan: u16, flags: u32) -> INPUT {
        let mut input: INPUT = unsafe { std::mem::zeroed() };
        input.r#type = INPUT_KEYBOARD;
        input.Anonymous.ki = KEYBDINPUT {
            wVk: vk,
            wScan: scan,
            dwFlags: flags,
            time: 0,
            dwExtraInfo: 0,
        };
        input
    }

    fn send_inputs(inputs: &[INPUT]) {
        unsafe {
            SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
        }
    }

    /// Press the keys in order, release in reverse (e.g. Ctrl+Shift+T).
    fn send_chord(vks: &[u16]) {
        let mut seq: Vec<INPUT> = vks.iter().map(|&vk| key_input(vk, 0, 0)).collect();
        seq.extend(vks.iter().rev().map(|&vk| key_input(vk, 0, KEYEVENTF_KEYUP)));
        send_inputs(&seq);
    }

    /// Type arbitrary text into the focused window via KEYEVENTF_UNICODE (layout-independent).
    fn type_text(s: &str) {
        let mut seq = Vec::new();
        for u in s.encode_utf16() {
            seq.push(key_input(0, u, KEYEVENTF_UNICODE));
            seq.push(key_input(0, u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
        send_inputs(&seq);
    }

    // ---- Windows Terminal per-tab focus via UI Automation (port of WtTabs.cs) ----
    //
    // WT has no API to focus a tab by process, but each tab is a UIA TabItem whose Name is the
    // tab title and which supports SelectionItemPattern. Matching is fuzzy on purpose: the title
    // Claude writes and the one the recorder captured can drift, so we score shared word tokens
    // from BOTH tab_title and topic and only switch on a confident winner.

    const STOP_WORDS: &[&str] = &[
        "the", "and", "for", "with", "into", "from", "that", "this", "your", "you", "set", "up",
        "add", "fix", "new", "run", "get", "out", "off", "was", "are", "has",
    ];

    /// Lowercased, punctuation/glyph-free, "administrator:" prefix removed.
    fn norm(s: &str) -> String {
        let cleaned: String = s
            .chars()
            .flat_map(|c| {
                if c.is_alphanumeric() {
                    c.to_lowercase().collect::<Vec<_>>()
                } else {
                    vec![' ']
                }
            })
            .collect();
        let flat = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
        flat.strip_prefix("administrator ").unwrap_or(&flat).to_string()
    }

    /// Meaningful word tokens (len >= 3, not a stopword) for overlap scoring.
    fn tokens(s: &str) -> Vec<String> {
        norm(s)
            .split_whitespace()
            .filter(|w| w.len() >= 3 && !STOP_WORDS.contains(w))
            .map(str::to_string)
            .collect()
    }

    fn wt_select_tab(hwnd: HWND, tab_title: &str, topic: &str) -> bool {
        let mut want: HashSet<String> = tokens(tab_title).into_iter().collect();
        want.extend(tokens(topic));
        let (exact_a, exact_b) = (norm(tab_title), norm(topic));
        if want.is_empty() && exact_a.is_empty() && exact_b.is_empty() {
            return false;
        }
        // UIA can throw on transient window state — treat any failure as "couldn't switch".
        unsafe { uia_select_tab(hwnd, &want, &exact_a, &exact_b).unwrap_or(false) }
    }

    unsafe fn uia_select_tab(
        hwnd: HWND,
        want: &HashSet<String>,
        exact_a: &str,
        exact_b: &str,
    ) -> windows::core::Result<bool> {
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
        };
        use windows::Win32::UI::Accessibility::{
            CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationSelectionItemPattern,
            TreeScope_Descendants, UIA_ControlTypePropertyId, UIA_SelectionItemPatternId,
            UIA_TabItemControlTypeId,
        };

        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); // fine if already initialised
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        let root = uia.ElementFromHandle(windows::Win32::Foundation::HWND(hwnd as _))?;
        // Raw VT_I4 VARIANT holding the TabItem control-type id (the 0.61 bindings expose the
        // bare C struct, no From impls).
        let mut var: windows::Win32::System::Variant::VARIANT = std::mem::zeroed();
        (*var.Anonymous.Anonymous).vt = windows::Win32::System::Variant::VT_I4;
        (*var.Anonymous.Anonymous).Anonymous.lVal = UIA_TabItemControlTypeId.0;
        let cond = uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &var)?;
        let tabs = root.FindAll(TreeScope_Descendants, &cond)?;
        let n = tabs.Length()?;
        let debug = std::env::var("CS_DEBUG").is_ok();
        if debug {
            eprintln!("uia: {n} tab items; want={want:?} exact_a='{exact_a}' exact_b='{exact_b}'");
        }
        if n <= 1 {
            return Ok(false); // single tab — nothing to switch to
        }

        let select = |el: &IUIAutomationElement| -> bool {
            el.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
                .map(|p| p.Select().is_ok())
                .unwrap_or(false)
        };

        let mut best: Option<IUIAutomationElement> = None;
        let (mut best_score, mut second_score) = (0usize, 0usize);
        let mut generic_claude: Vec<IUIAutomationElement> = Vec::new();
        for i in 0..n {
            let el = tabs.GetElement(i)?;
            let name = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
            let nn = norm(&name);
            if debug {
                eprintln!("uia tab {i}: '{name}' (norm '{nn}')");
            }
            if !nn.is_empty() && (nn == exact_a || nn == exact_b) {
                let ok = select(&el);
                if debug {
                    eprintln!("uia: exact match -> select = {ok}");
                }
                return Ok(ok); // exact title wins outright
            }
            if nn == "claude code" {
                generic_claude.push(el.clone()); // a tab still showing the default title
            }
            let score = tokens(&name).iter().filter(|t| want.contains(*t)).count();
            if score > best_score {
                second_score = best_score;
                best_score = score;
                best = Some(el);
            } else if score > second_score {
                second_score = score;
            }
        }
        // Confident match only: a clear token winner (>=2 shared, or a single distinctive token
        // no other tab shares). A tie → don't guess.
        if let Some(el) = best {
            if best_score >= 2 || (best_score == 1 && second_score == 0) {
                if debug {
                    eprintln!("uia: token match ({best_score}/{second_score}) -> select");
                }
                return Ok(select(&el));
            }
        }
        // Nothing matched: a session whose recorded title matches no tab usually sits in a tab
        // still titled "Claude Code" (the session never set one). If there's exactly one such
        // tab, it's unambiguous; with several (or none), stay put.
        if generic_claude.len() == 1 {
            if debug {
                eprintln!("uia: unique generic 'Claude Code' tab -> select");
            }
            return Ok(select(&generic_claude[0]));
        }
        Ok(false)
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

    pub fn parent_pid(_pid: i64) -> i64 {
        unsafe { libc::getppid() as i64 }
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

    pub fn annotate(rec: &mut Map<String, Value>, pid: i64, _transcript: &str, _topic: &str) {
        rec.insert("tty".into(), json!(tty_of(pid)));
    }

    pub fn jump(_terminal: &str, _term_pid: i64, _pid: i64, _tab_title: &str, _topic: &str) {
        // TODO(mac): focus the owning terminal via the AX API / tty. Not yet ported.
    }

    pub fn close_session(_terminal: &str, _term_pid: i64, _pid: i64, _tab_title: &str, _topic: &str) {
        // TODO(mac): close the owning tab. Not yet ported.
    }

    pub fn new_session(_cmds: &[String]) {
        // TODO(mac): open a new terminal tab and type the commands. Not yet ported.
    }

    pub fn attach_parent_console() {} // unix CLIs already share the terminal's stdio
}

pub use imp::{
    annotate, attach_parent_console, close_session, is_alive, jump, new_session, parent_pid,
};
