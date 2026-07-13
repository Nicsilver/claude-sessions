using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Shapes;

namespace ClaudeSessions;

/// One session row: a colored state bar, a right-to-left state glow, a hover highlight, the
/// session name (tinted on active states) and a right-aligned age/mute meta label. Ported from
/// SessionRow + DecoRowView in floatdash.swift. Left-click jumps, right-click closes,
/// middle-click mutes, Alt-click renames.
public sealed class Row : Grid
{
    private const double GlowAlpha = 0.22;
    private const double GlowFrac = 0.30;

    private readonly Rectangle _glow = new();
    private readonly Border _hover = new();
    private readonly Rectangle _bar = new();
    private readonly TextBlock _name = new();
    private readonly TextBlock _meta = new();
    private Sess _s;

    public Row(Sess s, Action<Sess> jump, Action<Sess> close, Action<Sess> mute, Action<Sess> rename)
    {
        _s = s;
        Height = 24;
        Background = Brushes.Transparent;   // so the whole row is hit-testable
        Cursor = Cursors.Hand;

        ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(14) });   // bar gutter
        ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        // glow spans the whole row, behind everything
        _glow.RadiusX = 6; _glow.RadiusY = 6;
        _glow.Margin = new Thickness(2, 1, 2, 1);
        SetColumnSpan(_glow, 3);
        Children.Add(_glow);

        // hover highlight (WPF evaluates IsMouseOver live, so it survives list rebuilds)
        _hover.CornerRadius = new CornerRadius(6);
        _hover.Margin = new Thickness(4, 1, 4, 1);
        _hover.Background = Brushes.Transparent;
        SetColumnSpan(_hover, 3);
        Children.Add(_hover);

        _bar.Width = 3; _bar.RadiusX = 1.5; _bar.RadiusY = 1.5;
        _bar.HorizontalAlignment = HorizontalAlignment.Left;
        _bar.VerticalAlignment = VerticalAlignment.Stretch;
        _bar.Margin = new Thickness(7, 4, 0, 4);
        SetColumn(_bar, 0);
        Children.Add(_bar);

        _name.FontSize = 11.5;
        _name.VerticalAlignment = VerticalAlignment.Center;
        _name.TextTrimming = TextTrimming.CharacterEllipsis;
        _name.TextWrapping = TextWrapping.NoWrap;
        SetColumn(_name, 1);
        Children.Add(_name);

        _meta.FontSize = 10;
        _meta.VerticalAlignment = VerticalAlignment.Center;
        _meta.Foreground = new SolidColorBrush(Styles.Secondary);
        _meta.Margin = new Thickness(8, 0, 11, 0);
        SetColumn(_meta, 2);
        Children.Add(_meta);

        MouseEnter += (_, _) => _hover.Background = new SolidColorBrush(Color.FromArgb(0x18, 0xFF, 0xFF, 0xFF));
        MouseLeave += (_, _) => _hover.Background = Brushes.Transparent;

        MouseLeftButtonUp += (_, e) =>
        {
            e.Handled = true;
            if (Keyboard.Modifiers.HasFlag(ModifierKeys.Alt)) rename(_s);
            else jump(_s);
        };
        MouseRightButtonUp += (_, e) => { e.Handled = true; close(_s); };
        MouseDown += (_, e) => { if (e.ChangedButton == MouseButton.Middle) { e.Handled = true; mute(_s); } };

        Render();
    }

    /// Cheap per-tick update (age text + mute countdown) without rebuilding the row.
    public void UpdateLive(Sess s) { _s = s; Render(); }

    private void Render()
    {
        double now = StateReader.Now();
        double muteLeft = _s.MuteUntil - now;
        bool muted = muteLeft > 0;
        var col = Styles.ColorFor(_s.State);
        bool active = Styles.IsActive(_s.State);

        ToolTip = string.IsNullOrEmpty(_s.Message) ? null : _s.Message;
        _name.Text = _s.Topic;

        if (muted)
        {
            _name.Foreground = new SolidColorBrush(Styles.Tertiary);
            _meta.Text = $"muted · {Math.Max(1, (int)(muteLeft / 60))}m";
            _bar.Fill = new SolidColorBrush(Styles.Tertiary);
            _glow.Fill = Brushes.Transparent;
            return;
        }

        _name.Foreground = new SolidColorBrush(active ? col : Styles.Label);
        var parts = new List<string>();
        if (_s.Terminal == "jetbrains") parts.Add("IDE");
        var age = Styles.AgeStr(_s.Updated);
        if (age.Length > 0) parts.Add(age);
        _meta.Text = string.Join(" · ", parts);

        _bar.Fill = new SolidColorBrush(col);

        // horizontal glow: clear until GlowFrac, then blooms to the right edge
        double a = GlowAlpha * (active ? 1.0 : 0.30);
        var g = new LinearGradientBrush { StartPoint = new Point(0, 0), EndPoint = new Point(1, 0) };
        g.GradientStops.Add(new GradientStop(Color.FromArgb(0, col.R, col.G, col.B), 0.0));
        g.GradientStops.Add(new GradientStop(Color.FromArgb(0, col.R, col.G, col.B), GlowFrac));
        g.GradientStops.Add(new GradientStop(Color.FromArgb((byte)(a * 255), col.R, col.G, col.B), 1.0));
        _glow.Fill = g;
    }
}
