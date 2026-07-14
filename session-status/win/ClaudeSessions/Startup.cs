using Microsoft.Win32;

namespace ClaudeSessions;

/// Launch-at-login for Windows, the analog of the macOS SMAppService login item. Registers the
/// running executable under the per-user Run key so it starts (silently, to the tray) at sign-in.
/// Per-user (HKCU) needs no elevation and shows up in Task Manager → Startup for the user to toggle.
public static class Startup
{
    private const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string ValueName = "ClaudeSessions";

    public static void Register()
    {
        var exe = Environment.ProcessPath;
        if (exe is null) return;
        using var key = Registry.CurrentUser.CreateSubKey(RunKey);
        key.SetValue(ValueName, $"\"{exe}\"");
    }

    public static void Unregister()
    {
        using var key = Registry.CurrentUser.OpenSubKey(RunKey, writable: true);
        key?.DeleteValue(ValueName, throwOnMissingValue: false);
    }

    public static bool IsRegistered()
    {
        using var key = Registry.CurrentUser.OpenSubKey(RunKey);
        return key?.GetValue(ValueName) is not null;
    }
}
