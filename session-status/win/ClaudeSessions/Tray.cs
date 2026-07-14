using System.Runtime.InteropServices;
using WF = System.Windows.Forms;
using D = System.Drawing;

namespace ClaudeSessions;

/// System-tray badge — the Windows analog of the macOS menu-bar app. A state-colored icon
/// showing the live session count, a right-click menu of sessions, left-click to jump to the
/// most-urgent one, and a clickable balloon when a session flips into "needs you".
public sealed class Tray : IDisposable
{
    private readonly WF.NotifyIcon _icon = new() { Visible = true };
    private readonly Action<Sess> _jump;
    private readonly Action _newSession;
    private readonly Action _toggleDashboard;
    private readonly Action _quit;

    private List<Sess> _sessions = new();
    private string _iconSig = "";
    private nint _lastHIcon;
    private HashSet<string> _prevNeeds = new();
    private Sess? _balloonTarget;
    private readonly WpfTrayMenu _menu = new();

    [DllImport("user32.dll")] private static extern bool DestroyIcon(nint handle);

    public Tray(Action<Sess> jump, Action newSession, Action toggleDashboard, Action quit)
    {
        _jump = jump; _newSession = newSession; _toggleDashboard = toggleDashboard; _quit = quit;

        _icon.Text = "Claude sessions";
        _icon.MouseClick += (_, e) =>
        {
            if (e.Button == WF.MouseButtons.Left)
            {
                var top = _sessions.FirstOrDefault();
                if (top is not null) _jump(top);       // left-click → jump to the top session
            }
            else if (e.Button == WF.MouseButtons.Right)
            {
                ShowMenu();                             // right-click → show the session menu
            }
        };
        _icon.BalloonTipClicked += (_, _) => { if (_balloonTarget is not null) _jump(_balloonTarget); };
        SetIcon("done", 0);   // placeholder until the first update
    }

    /// Called from MainWindow.Refresh with the already-sorted session list.
    public void Update(List<Sess> sessions)
    {
        _sessions = sessions;
        double now = StateReader.Now();
        var active = sessions.Where(s => s.MuteUntil <= now).ToList();

        // Badge shows the count of the single top-priority live state (needs → your-turn →
        // working), colored by it — matching menubar.swift. NOT the total active count.
        int needs = active.Count(s => s.State == "needs");
        int yt = active.Count(s => s.State == "yourturn");
        int working = active.Count(s => s.State == "working");
        string top; int count;
        if (needs > 0) { top = "needs"; count = needs; }
        else if (yt > 0) { top = "yourturn"; count = yt; }
        else if (working > 0) { top = "working"; count = working; }
        else { top = "clear"; count = 0; }

        string sig = $"{top}:{count}";
        if (sig != _iconSig) { _iconSig = sig; SetIcon(top, count); }

        _icon.Text = Tooltip(sessions);
        // Note: the menu is NOT rebuilt here. The refresh runs every ~1.5s, and disposing/rebuilding
        // the ContextMenuStrip while it's open would close it out from under the user. Instead the
        // menu is built fresh on right-click (ShowMenu), so it always reflects the current sessions.
        NotifyNeeds(sessions, now);
    }

    // ---- balloon on transition into "needs" ----

    private void NotifyNeeds(List<Sess> sessions, double now)
    {
        var needs = sessions.Where(s => s.State == "needs" && s.MuteUntil <= now).ToList();
        var ids = needs.Select(s => s.Id).ToHashSet();
        var fresh = needs.FirstOrDefault(s => !_prevNeeds.Contains(s.Id));   // newly flipped
        if (fresh is not null)
        {
            _balloonTarget = fresh;
            var body = string.IsNullOrEmpty(fresh.Message) ? "Waiting on you" : fresh.Message;
            _icon.ShowBalloonTip(5000, $"{fresh.Topic} needs you", body, WF.ToolTipIcon.Warning);
        }
        _prevNeeds = ids;
    }

    private static string Tooltip(List<Sess> s)
    {
        int Needs() => s.Count(x => x.State == "needs");
        int Yt() => s.Count(x => x.State == "yourturn");
        int Wk() => s.Count(x => x.State == "working");
        var parts = new List<string>();
        if (Needs() > 0) parts.Add($"{Needs()} need you");
        if (Yt() > 0) parts.Add($"{Yt()} your turn");
        if (Wk() > 0) parts.Add($"{Wk()} working");
        return parts.Count == 0 ? "Claude sessions — idle" : "Claude — " + string.Join(", ", parts);
    }

    // ---- right-click menu ----

    /// Build a fresh WPF ContextMenu of the current sessions and show it at the cursor. Built on
    /// demand (not on the 1.5s refresh) so a refresh tick can't dispose it out from under the user,
    /// and rendered with WPF — real rounded corners, shadow and hover — instead of the flat gray
    /// WinForms ToolStripMenu.
    private void ShowMenu()
    {
        var menu = _menu.NewMenu();
        if (_sessions.Count == 0)
        {
            _menu.AddItem(menu, "No active sessions", null, () => { }, enabled: false);
        }
        else
        {
            foreach (var s in _sessions)
            {
                var age = Styles.AgeStr(s.Updated);
                var name = Trunc(s.Topic, 30);   // keep long topics from stretching the menu
                var text = age.Length > 0 ? $"{name}   ·  {age}" : name;
                var captured = s;
                _menu.AddItem(menu, text, Styles.ColorFor(s.State), () => _jump(captured));
            }
        }
        _menu.AddSeparator(menu);
        _menu.AddItem(menu, "Show / hide dashboard", null, _toggleDashboard);
        _menu.AddItem(menu, "New Claude session", null, _newSession);
        _menu.AddSeparator(menu);
        _menu.AddItem(menu, "Quit", null, _quit);

        _menu.Show(menu);
    }

    /// Clip an over-long topic to `max` chars with an ellipsis so the menu stays a sane width.
    private static string Trunc(string s, int max) =>
        s.Length <= max ? s : s[..(max - 1)].TrimEnd() + "…";

    private static void RoundRect(D.Drawing2D.GraphicsPath p, float x, float y, float w, float h, float r)
    {
        float d = 2 * r;
        p.AddArc(x, y, d, d, 180, 90);
        p.AddArc(x + w - d, y, d, d, 270, 90);
        p.AddArc(x + w - d, y + h - d, d, d, 0, 90);
        p.AddArc(x, y + h - d, d, d, 90, 90);
        p.CloseFigure();
    }

    // ---- dynamic badge icon ----

    private void SetIcon(string state, int count)
    {
        var c = Styles.ColorFor(state);
        var col = D.Color.FromArgb(c.A, c.R, c.G, c.B);
        using var bmp = new D.Bitmap(32, 32);
        using (var g = D.Graphics.FromImage(bmp))
        {
            g.SmoothingMode = D.Drawing2D.SmoothingMode.AntiAlias;
            g.Clear(D.Color.Transparent);
            if (count == 0)
            {
                // all clear: a single small state-colored dot, nothing loud
                using var b = new D.SolidBrush(D.Color.FromArgb(150, col));
                g.FillEllipse(b, 12, 12, 8, 8);
            }
            else
            {
                // mac look: a slim colored accent pill on the left…
                using (var pill = new D.Drawing2D.GraphicsPath())
                {
                    RoundRect(pill, 3, 6, 5, 20, 2.5f);
                    using var pb = new D.SolidBrush(col);
                    g.FillPath(pb, pill);
                }
                // …and the count as a clean white number to its right (thin dark halo so it
                // stays legible on a light taskbar too).
                string t = count > 99 ? "99+" : count.ToString();
                float em = t.Length >= 3 ? 15f : t.Length == 2 ? 19f : 23f;
                using var ff = new D.FontFamily("Segoe UI");
                using var path = new D.Drawing2D.GraphicsPath();
                var fmt = new D.StringFormat { Alignment = D.StringAlignment.Center, LineAlignment = D.StringAlignment.Center };
                path.AddString(t, ff, (int)D.FontStyle.Bold, em, new D.RectangleF(9, -1, 23, 33), fmt);
                using var halo = new D.Pen(D.Color.FromArgb(140, 0, 0, 0), 2f)
                    { LineJoin = D.Drawing2D.LineJoin.Round };
                g.DrawPath(halo, path);
                using var fill = new D.SolidBrush(D.Color.White);
                g.FillPath(fill, path);
            }
        }
        nint h = bmp.GetHicon();
        _icon.Icon = D.Icon.FromHandle(h);
        if (_lastHIcon != 0) DestroyIcon(_lastHIcon);   // free the previous handle
        _lastHIcon = h;
    }

    public void Dispose()
    {
        _icon.Visible = false;
        _menu.Dispose();
        _icon.Dispose();
        if (_lastHIcon != 0) DestroyIcon(_lastHIcon);
    }
}
