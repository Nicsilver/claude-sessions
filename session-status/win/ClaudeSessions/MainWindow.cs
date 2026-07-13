using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Effects;
using System.Windows.Shapes;
using System.Windows.Threading;

namespace ClaudeSessions;

/// The always-on-top floating dashboard. A borderless, non-activating, rounded dark panel
/// pinned top-right that lists every Claude session and lets you jump to (left-click),
/// close (right-click), mute (middle-click) or rename (Alt-click) each one. Ported from
/// floatdash.swift.
public sealed class MainWindow : Window
{
    private const double WinW = 320;      // includes the shadow margin
    private const double Pad = 12;        // transparent gutter for the drop shadow
    private const double RowH = 24;

    private readonly StackPanel _list = new();
    private readonly ScrollViewer _scroll = new();
    private readonly StackPanel _chips = new();
    private readonly TextBlock _empty = new();
    private readonly DispatcherTimer _timer = new() { Interval = TimeSpan.FromMilliseconds(1500) };
    private string _sig = "";                          // structural signature; rebuild only on change
    private readonly List<Row> _rows = new();
    private Tray? _tray;

    public MainWindow()
    {
        WindowStyle = WindowStyle.None;
        AllowsTransparency = true;
        Background = Brushes.Transparent;
        ResizeMode = ResizeMode.NoResize;
        ShowInTaskbar = false;
        Topmost = true;
        SizeToContent = SizeToContent.Height;
        Width = WinW;
        Title = "Claude sessions";

        BuildChrome();
        PositionTopRight();
        _tray = new Tray(JumpTo, NewSession, ToggleDashboard, QuitApp);

        Loaded += (_, _) =>
        {
            var hwnd = new System.Windows.Interop.WindowInteropHelper(this).Handle;
            Interop.MakeToolWindow(hwnd);   // non-activating + hidden from Alt-Tab
            Refresh();
        };
        _timer.Tick += (_, _) => Refresh();
        _timer.Start();
    }

    // ---- layout ----

    private void BuildChrome()
    {
        var shell = new Border
        {
            CornerRadius = new CornerRadius(10),
            Background = new SolidColorBrush(Styles.Bg),
            BorderBrush = new SolidColorBrush(Color.FromArgb(0x2A, 0xFF, 0xFF, 0xFF)),  // faint hairline
            BorderThickness = new Thickness(1),
            Margin = new Thickness(Pad),          // leave room for the shadow to fall
            Effect = new DropShadowEffect
            {
                Color = Colors.Black, BlurRadius = 22, ShadowDepth = 3,
                Direction = 270, Opacity = 0.55,
            },
        };

        var grid = new Grid { Margin = new Thickness(0) };
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });   // header
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });   // list
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });   // separator
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });   // footer chips

        grid.Children.Add(BuildHeader());

        // list
        var maxH = SystemParameters.WorkArea.Height - 80;
        _scroll.MaxHeight = maxH;
        _scroll.VerticalScrollBarVisibility = ScrollBarVisibility.Auto;
        _scroll.HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled;
        _scroll.Margin = new Thickness(6, 4, 6, 4);
        _list.Orientation = Orientation.Vertical;
        _scroll.Content = _list;
        Grid.SetRow(_scroll, 1);
        grid.Children.Add(_scroll);

        _empty.Text = "No active sessions";
        _empty.FontSize = 12;
        _empty.Foreground = new SolidColorBrush(Styles.Tertiary);
        _empty.HorizontalAlignment = HorizontalAlignment.Center;
        _empty.Margin = new Thickness(0, 16, 0, 16);
        Grid.SetRow(_empty, 1);
        grid.Children.Add(_empty);

        var sep = new Border
        {
            Height = 1,
            Background = new SolidColorBrush(Color.FromRgb(0x33, 0x33, 0x33)),
            Margin = new Thickness(14, 0, 14, 0),
        };
        Grid.SetRow(sep, 2);
        grid.Children.Add(sep);

        _chips.Orientation = Orientation.Horizontal;
        _chips.HorizontalAlignment = HorizontalAlignment.Center;
        _chips.Margin = new Thickness(0, 7, 0, 9);
        // Footer is a drag handle (like the mac window-background drag): Transparent (not null)
        // so the whole strip — including gaps between chips — is hit-testable.
        var footer = new Border { Background = Brushes.Transparent, Child = _chips };
        footer.MouseLeftButtonDown += (_, e) => { if (e.ButtonState == MouseButtonState.Pressed) DragMove(); };
        Grid.SetRow(footer, 3);
        grid.Children.Add(footer);

        shell.Child = grid;
        Content = shell;
    }

    private UIElement BuildHeader()
    {
        var header = new Grid { Height = 28, Background = Brushes.Transparent };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        // × top-left (hides to tray — the app stays alive there), + top-right (new session).
        var close = DiscButton("×", "Hide to tray", Hide);
        close.Margin = new Thickness(8, 0, 0, 0);
        close.HorizontalAlignment = HorizontalAlignment.Left;
        Grid.SetColumn(close, 0);
        header.Children.Add(close);

        var title = new TextBlock
        {
            Text = "Claude sessions",
            FontSize = 11.5,
            Foreground = new SolidColorBrush(Styles.Secondary),
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
        };
        Grid.SetColumn(title, 1);
        header.Children.Add(title);

        var plus = DiscButton("+", "New Claude session", NewSession);
        plus.Margin = new Thickness(0, 0, 8, 0);
        plus.HorizontalAlignment = HorizontalAlignment.Right;
        Grid.SetColumn(plus, 2);
        header.Children.Add(plus);

        // drag the window by the header background
        header.MouseLeftButtonDown += (_, e) => { if (e.ButtonState == MouseButtonState.Pressed) DragMove(); };
        return header;
    }

    private FrameworkElement DiscButton(string glyph, string tip, Action onClick)
    {
        var tb = new TextBlock
        {
            Text = glyph,
            FontSize = glyph == "+" ? 15 : 14,
            Foreground = new SolidColorBrush(Styles.Secondary),
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
        };
        var b = new Border
        {
            Width = 18, Height = 18, Margin = new Thickness(4, 0, 0, 0),
            CornerRadius = new CornerRadius(9),
            Background = Brushes.Transparent,
            Child = tb,
            Cursor = Cursors.Hand,
            ToolTip = tip,
        };
        // Swallow the press so the header's drag handler doesn't start a window drag (which would
        // eat the button's click and make + / × appear to do nothing).
        b.MouseLeftButtonDown += (_, e) => e.Handled = true;
        b.MouseEnter += (_, _) =>
        {
            tb.Foreground = new SolidColorBrush(Styles.Label);
            b.Background = new SolidColorBrush(Color.FromArgb(0x22, 0xFF, 0xFF, 0xFF));
        };
        b.MouseLeave += (_, _) =>
        {
            tb.Foreground = new SolidColorBrush(Styles.Secondary);
            b.Background = Brushes.Transparent;
        };
        b.MouseLeftButtonUp += (_, e) => { e.Handled = true; onClick(); };
        return b;
    }

    private void PositionTopRight()
    {
        var wa = SystemParameters.WorkArea;
        Left = wa.Right - WinW - 20;
        Top = wa.Top + 20;
    }

    // ---- refresh ----

    private void Refresh()
    {
        double now = StateReader.Now();
        var sessions = StateReader.Load();
        sessions.Sort((a, b) =>
        {
            bool am = a.MuteUntil > now, bm = b.MuteUntil > now;
            if (am != bm) return am ? 1 : -1;                          // muted sink
            int oa = Styles.Order(a.State), ob = Styles.Order(b.State);
            if (oa != ob) return oa.CompareTo(ob);
            return b.Updated.CompareTo(a.Updated);
        });

        _tray?.Update(sessions);
        UpdateChips(sessions);
        _empty.Visibility = sessions.Count == 0 ? Visibility.Visible : Visibility.Collapsed;

        // Rebuild the row list only when its structure changes; otherwise just refresh ages
        // so hovering / clicking isn't interrupted every tick.
        string sig = string.Join("|", sessions.Select(s =>
            $"{s.Id}:{s.State}:{s.Topic}:{(s.MuteUntil > now ? 1 : 0)}"));
        if (sig != _sig)
        {
            _sig = sig;
            _list.Children.Clear();
            _rows.Clear();
            foreach (var s in sessions)
            {
                var row = new Row(s, JumpTo, CloseTab, MuteToggle, Rename);
                _rows.Add(row);
                _list.Children.Add(row);
            }
        }
        else
        {
            foreach (var (row, s) in _rows.Zip(sessions)) row.UpdateLive(s);
        }
    }

    private void UpdateChips(List<Sess> sessions)
    {
        _chips.Children.Clear();
        foreach (var key in new[] { "needs", "yourturn", "working", "done" })
        {
            int n = sessions.Count(s => s.State == key);
            _chips.Children.Add(Chip(key, n));
        }
    }

    /// A count chip: a filled state dot (with a cut-out glyph — "!" for needs, "✓" for done —
    /// mirroring the SF Symbols the mac surface uses), then the count. Dimmed when zero.
    private static FrameworkElement Chip(string key, int n)
    {
        var col = n > 0 ? Styles.ColorFor(key) : Styles.Tertiary;
        var icon = new Grid { Width = 12, Height = 12, VerticalAlignment = VerticalAlignment.Center };
        icon.Children.Add(new Ellipse
        {
            Width = 11, Height = 11, Fill = new SolidColorBrush(col),
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center,
        });
        string glyph = key switch { "needs" => "!", "done" => "✓", _ => "" };
        if (glyph.Length > 0)
        {
            icon.Children.Add(new TextBlock
            {
                Text = glyph, FontSize = key == "done" ? 7.5 : 9, FontWeight = FontWeights.Bold,
                Foreground = new SolidColorBrush(Styles.Bg),   // reads as a hole in the disc
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center,
                Margin = key == "needs" ? new Thickness(0, -1, 0, 0) : new Thickness(0),
            });
        }
        var lbl = new TextBlock
        {
            Text = n.ToString(), FontSize = 11.5, FontWeight = FontWeights.SemiBold,
            Margin = new Thickness(4, 0, 0, 0),
            Foreground = new SolidColorBrush(n > 0 ? Styles.Label : Styles.Tertiary),
            VerticalAlignment = VerticalAlignment.Center,
        };
        var sp = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(7, 0, 7, 0) };
        sp.Children.Add(icon); sp.Children.Add(lbl);
        return sp;
    }

    // ---- row actions ----

    private void JumpTo(Sess s)
    {
        if (s.Terminal == "jetbrains")
            StateReader.WriteRequest(s.Id, s.Pid, s.Terminal, s.TermPid, "focus");  // plugin handles it
        else
            Interop.FocusSession(s);
    }

    private void CloseTab(Sess s)
    {
        if (s.Terminal == "jetbrains")
            StateReader.WriteRequest(s.Id, s.Pid, s.Terminal, s.TermPid, "close");  // plugin acts
        else
            Interop.CloseSession(s);
    }

    private void NewSession() => Interop.NewClaudeSession();

    /// Tray "Show / hide dashboard": toggle the floating window, re-pinning it top-right when shown.
    private void ToggleDashboard()
    {
        if (IsVisible) { Hide(); }
        else { Show(); PositionTopRight(); Refresh(); }
    }

    private void QuitApp()
    {
        _timer.Stop();
        _tray?.Dispose();
        _tray = null;
        Application.Current.Shutdown();
    }

    private void MuteToggle(Sess s) { StateReader.ToggleMute(s.Id); Refresh(); }

    private void Rename(Sess s)
    {
        var current = StateReader.LoadLabels().TryGetValue(s.Id, out var v) ? v : "";
        var name = RenameDialog.Show(this, s.Topic, current);
        if (name is not null) { StateReader.SetLabel(s.Id, name); _sig = ""; Refresh(); }
    }

    protected override void OnClosed(EventArgs e) { _tray?.Dispose(); _timer.Stop(); base.OnClosed(e); }
}
