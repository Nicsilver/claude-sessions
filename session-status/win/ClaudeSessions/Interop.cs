using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

namespace ClaudeSessions;

/// Win32 glue: (1) make the panel a non-activating tool window so clicking a row never
/// steals focus from your editor, and (2) find + raise a terminal window by pid for the jump.
public static partial class Interop
{
    // ---- extended window styles ----
    public const int GWL_EXSTYLE = -20;
    public const int WS_EX_TOOLWINDOW = 0x00000080;  // hide from Alt-Tab
    public const int WS_EX_NOACTIVATE = 0x08000000;  // clicks don't activate the window

    [LibraryImport("user32.dll", SetLastError = true)]
    public static partial int GetWindowLongW(nint hWnd, int nIndex);

    [LibraryImport("user32.dll", SetLastError = true)]
    public static partial int SetWindowLongW(nint hWnd, int nIndex, int dwNewLong);

    /// Apply NOACTIVATE + TOOLWINDOW to a window we own (call after the HWND exists).
    public static void MakeToolWindow(nint hwnd)
    {
        int ex = GetWindowLongW(hwnd, GWL_EXSTYLE);
        SetWindowLongW(hwnd, GWL_EXSTYLE, ex | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE);
    }

    // ---- focusing another process's window (the jump) ----

    private delegate bool EnumWindowsProc(nint hWnd, nint lParam);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, nint lParam);
    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(nint hWnd, out uint pid);
    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(nint hWnd);
    [DllImport("user32.dll")]
    private static extern int GetWindowTextLengthW(nint hWnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetWindowTextW(nint hWnd, StringBuilder text, int count);
    [DllImport("user32.dll")]
    private static extern nint GetWindow(nint hWnd, uint uCmd);

    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(nint hWnd);
    [DllImport("user32.dll")]
    private static extern bool ShowWindow(nint hWnd, int nCmdShow);
    [DllImport("user32.dll")]
    private static extern bool IsIconic(nint hWnd);
    [DllImport("user32.dll")]
    private static extern bool AttachThreadInput(uint idAttach, uint idAttachTo, bool fAttach);
    [DllImport("kernel32.dll")]
    private static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")]
    private static extern nint GetForegroundWindow();

    private const int SW_RESTORE = 9;
    private const uint GW_OWNER = 4;

    /// The main top-level, visible, titled window owned by <paramref name="pid"/> (largest such
    /// window wins if there are several). Returns 0 if the process has no such window.
    public static nint MainWindowForPid(int pid)
    {
        nint best = 0;
        long bestArea = -1;
        EnumWindows((h, _) =>
        {
            GetWindowThreadProcessId(h, out uint wpid);
            if (wpid != (uint)pid) return true;
            if (!IsWindowVisible(h)) return true;
            if (GetWindow(h, GW_OWNER) != 0) return true;          // skip owned/tool popups
            if (GetWindowTextLengthW(h) == 0) return true;          // skip untitled shells
            var r = new RECT();
            GetWindowRect(h, ref r);
            long area = (long)Math.Max(0, r.Right - r.Left) * Math.Max(0, r.Bottom - r.Top);
            if (area > bestArea) { bestArea = area; best = h; }
            return true;
        }, 0);
        return best;
    }

    [DllImport("user32.dll")]
    private static extern bool GetWindowRect(nint hWnd, ref RECT lpRect);
    [StructLayout(LayoutKind.Sequential)]
    private struct RECT { public int Left, Top, Right, Bottom; }

    /// Bring a window owned by another process to the foreground reliably. Windows only lets
    /// the foreground thread call SetForegroundWindow, so we briefly attach our input queue to
    /// the target's foreground thread to gain the right, then detach.
    public static bool FocusWindow(nint hwnd)
    {
        if (hwnd == 0) return false;
        if (IsIconic(hwnd)) ShowWindow(hwnd, SW_RESTORE);   // un-minimize ONLY — never shrink a maximized window
        nint fg = GetForegroundWindow();
        uint fgThread = GetWindowThreadProcessId(fg, out _);
        uint ourThread = GetCurrentThreadId();
        uint targetThread = GetWindowThreadProcessId(hwnd, out _);
        bool a1 = AttachThreadInput(ourThread, fgThread, true);
        bool a2 = AttachThreadInput(targetThread, fgThread, true);
        bool ok = SetForegroundWindow(hwnd);
        if (a2) AttachThreadInput(targetThread, fgThread, false);
        if (a1) AttachThreadInput(ourThread, fgThread, false);
        return ok;
    }

    [DllImport("user32.dll")]
    private static extern void keybd_event(byte vk, byte scan, uint flags, nuint extra);
    [DllImport("user32.dll")]
    private static extern short VkKeyScanW(char ch);
    private const byte VK_CONTROL = 0x11, VK_SHIFT = 0x10, VK_W = 0x57, VK_T = 0x54, VK_RETURN = 0x0D;
    private const uint KEYEVENTF_KEYUP = 0x0002;

    private const string ProgrammingDir = @"C:\Programming";
    // What "clauded" expands to — typed in full so it works in any shell (the alias only exists
    // in shells that load Nic's PowerShell/bash profile).
    private const string ClaudedCmd = "claude --dangerously-skip-permissions";

    /// Close a session's terminal (right-click). For Windows Terminal we focus the exact tab and
    /// send its close-tab chord (Ctrl+Shift+W) — the same as clicking the tab's ×, which also
    /// terminates Claude inside it. For other terminals we kill the Claude process tree.
    public static void CloseSession(Sess s)
    {
        if (s.Terminal == "jetbrains") return;   // handled by the IDE plugin, not us
        if (s.Terminal == "wt")
        {
            int pid = s.TermPid > 0 ? s.TermPid : s.Pid;
            var hwnd = MainWindowForPid(pid);
            if (hwnd == 0) return;
            Task.Run(() =>
            {
                WtTabs.SelectTab(hwnd, s.TabTitle, s.Topic);   // make the session's tab active first
                FocusWindow(hwnd);
                Thread.Sleep(120);
                if (GetForegroundWindow() != hwnd) return;      // never send the chord elsewhere
                SendChord(VK_CONTROL, VK_SHIFT, VK_W);          // WT close-tab
            });
        }
        else if (s.Pid > 0)
        {
            try { Process.Start(new ProcessStartInfo("taskkill", $"/PID {s.Pid} /T /F")
                { CreateNoWindow = true, UseShellExecute = false }); }
            catch { /* best effort */ }
        }
    }

    /// New session (the "+" button): open a new tab in the CURRENT Windows Terminal window,
    /// cd to C:\Programming and start a --dangerously-skip-permissions session. Elevated WT
    /// refuses the `wt -w 0` remote-tab command, so we drive it the way it always works: focus
    /// the window, press its new-tab chord, and type the command into the fresh shell. If we
    /// can't focus the WT window (nothing to attach to, or an elevation wall), open a new window.
    public static void NewClaudeSession() => Task.Run(NewClaudeSessionSync);

    /// Synchronous variant for the --new CLI (the process exits the moment this returns, so it
    /// can't be fire-and-forget there).
    public static void NewClaudeSessionSync()
    {
        var wt = FindWtWindow();
        if (wt != 0) NewTabViaKeys(wt);
        else NewWindowFallback();
    }

    private static void NewTabViaKeys(nint wt)
    {
        FocusWindow(wt);
        Thread.Sleep(180);
        if (GetForegroundWindow() != wt) { NewWindowFallback(); return; }   // couldn't take focus

        SendChord(VK_CONTROL, VK_SHIFT, VK_T);   // WT "new tab" (default profile)
        Thread.Sleep(750);                        // let the shell come up
        if (GetForegroundWindow() != wt) return;  // focus moved — never type into the wrong window

        // Nic's shell aliases (Windows PowerShell profile): `prog` = cd C:\Programming, `c` =
        // claude --dangerously-skip-permissions.
        TypeString("prog");
        TapKey(VK_RETURN);
        Thread.Sleep(150);
        TypeString("c");
        TapKey(VK_RETURN);
    }

    private static void NewWindowFallback()
    {
        try
        {
            var psi = new ProcessStartInfo("wt.exe") { UseShellExecute = true };
            foreach (var a in new[] { "-w", "0", "new-tab", "-d", ProgrammingDir, "cmd", "/k", ClaudedCmd })
                psi.ArgumentList.Add(a);
            Process.Start(psi);
        }
        catch { /* wt not on PATH — nothing sensible to fall back to */ }
    }

    private static nint FindWtWindow()
    {
        foreach (var p in Process.GetProcessesByName("WindowsTerminal"))
        {
            var h = MainWindowForPid(p.Id);
            if (h != 0) return h;
        }
        return 0;
    }

    // ---- synthetic keyboard helpers ----

    private static void SendChord(byte mod1, byte mod2, byte key)
    {
        keybd_event(mod1, 0, 0, 0);
        keybd_event(mod2, 0, 0, 0);
        keybd_event(key, 0, 0, 0);
        keybd_event(key, 0, KEYEVENTF_KEYUP, 0);
        keybd_event(mod2, 0, KEYEVENTF_KEYUP, 0);
        keybd_event(mod1, 0, KEYEVENTF_KEYUP, 0);
    }

    private static void TapKey(byte vk)
    {
        keybd_event(vk, 0, 0, 0);
        keybd_event(vk, 0, KEYEVENTF_KEYUP, 0);
    }

    private static void TypeString(string s)
    {
        foreach (char c in s)
        {
            short vk = VkKeyScanW(c);
            if (vk == -1) continue;
            byte v = (byte)(vk & 0xFF);
            bool shift = (vk & 0x100) != 0;
            if (shift) keybd_event(VK_SHIFT, 0, 0, 0);
            keybd_event(v, 0, 0, 0);
            keybd_event(v, 0, KEYEVENTF_KEYUP, 0);
            if (shift) keybd_event(VK_SHIFT, 0, KEYEVENTF_KEYUP, 0);
            Thread.Sleep(6);   // small gap so fast shells don't drop characters
        }
    }

    /// Focus the terminal window that owns a session. Returns false if there's nothing to focus
    /// (e.g. a JetBrains session, which is handled by the plugin via the request file instead).
    public static bool FocusSession(Sess s)
    {
        if (s.Terminal == "jetbrains") return false;
        int pid = s.TermPid > 0 ? s.TermPid : s.Pid;
        if (pid <= 0) return false;
        var hwnd = MainWindowForPid(pid);
        if (s.Terminal == "wt")
            WtTabs.SelectTab(hwnd, s.TabTitle, s.Topic);   // switch to the session's tab first
        return FocusWindow(hwnd);
    }
}
