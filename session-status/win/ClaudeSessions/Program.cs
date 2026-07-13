using System.Threading;
using System.Windows;

namespace ClaudeSessions;

public static class Program
{
    [STAThread]
    public static void Main(string[] args)
    {
        // Headless jump: `ClaudeSessions.exe --focus <sessionId>` runs the real focus path
        // (window + WT tab) and exits. Handy for diagnostics and future global hotkeys.
        if (args.Length >= 2 && args[0] == "--focus")
        {
            var s = StateReader.Load().FirstOrDefault(x => x.Id == args[1]);
            if (s is not null) Interop.FocusSession(s);
            return;
        }
        if (args.Length >= 1 && args[0] == "--new")   // open a new Claude terminal
        {
            Interop.NewClaudeSessionSync();
            return;
        }

        // Single instance: a second launch just exits, so `status float` is idempotent.
        using var mutex = new Mutex(true, "ClaudeSessionsFloatingWidget", out bool isNew);
        if (!isNew) return;

        // Explicit shutdown only: hiding the dashboard to the tray must not exit the app.
        var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
        var win = new MainWindow();
        app.Run(win);
    }
}
