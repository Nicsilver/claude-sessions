using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Markup;
using System.Windows.Media;
using System.Windows.Shapes;
using System.Windows.Interop;

namespace ClaudeSessions;

/// A modern, WPF-native replacement for the WinForms tray menu. WPF's ContextMenu can be fully
/// templated — rounded corners, drop shadow, rounded hover — so it matches the dashboard instead
/// of the flat gray Office-era ToolStripMenu. The tricky part is that a tray icon has no WPF
/// placement target and our process usually isn't foreground, so this hosts the menu on a tiny
/// invisible owner window positioned at the cursor and foregrounds it before opening (otherwise a
/// tray popup silently dismisses itself — Win32 KB135788).
public sealed class WpfTrayMenu : IDisposable
{
    private Window? _owner;
    private ContextMenu? _open;
    private static ResourceDictionary? _theme;

    [DllImport("user32.dll")] private static extern bool SetForegroundWindow(nint hWnd);
    [DllImport("user32.dll")] private static extern bool GetCursorPos(out POINT p);
    [StructLayout(LayoutKind.Sequential)] private struct POINT { public int X, Y; }

    // Styles for ContextMenu / MenuItem / Separator, matching Styles.Bg and the dashboard hover.
    // Built once via XamlReader — far terser than assembling ControlTemplates in C#.
    private static ResourceDictionary Theme => _theme ??= (ResourceDictionary)XamlReader.Parse("""
        <ResourceDictionary xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
                            xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml">
          <Style x:Key="Menu" TargetType="ContextMenu">
            <Setter Property="OverridesDefaultStyle" Value="True"/>
            <Setter Property="SnapsToDevicePixels" Value="True"/>
            <Setter Property="HasDropShadow" Value="True"/>
            <Setter Property="Template">
              <Setter.Value>
                <ControlTemplate TargetType="ContextMenu">
                  <Border Background="#F22B2B2B" BorderBrush="#3A3A3A" BorderThickness="1"
                          CornerRadius="8" Padding="4">
                    <Border.Effect>
                      <DropShadowEffect Color="Black" BlurRadius="18" ShadowDepth="2" Opacity="0.55"/>
                    </Border.Effect>
                    <ItemsPresenter/>
                  </Border>
                </ControlTemplate>
              </Setter.Value>
            </Setter>
          </Style>
          <Style x:Key="Item" TargetType="MenuItem">
            <Setter Property="OverridesDefaultStyle" Value="True"/>
            <Setter Property="FontFamily" Value="Segoe UI"/>
            <Setter Property="FontSize" Value="13"/>
            <Setter Property="Foreground" Value="#ECECEC"/>
            <Setter Property="Template">
              <Setter.Value>
                <ControlTemplate TargetType="MenuItem">
                  <Border x:Name="bg" Background="Transparent" CornerRadius="6" Margin="0,1" Padding="8,6">
                    <Grid>
                      <Grid.ColumnDefinitions>
                        <ColumnDefinition Width="16"/>
                        <ColumnDefinition Width="*"/>
                      </Grid.ColumnDefinitions>
                      <ContentPresenter Grid.Column="0" ContentSource="Icon"
                                        VerticalAlignment="Center" HorizontalAlignment="Center"/>
                      <ContentPresenter x:Name="hdr" Grid.Column="1" ContentSource="Header"
                                        VerticalAlignment="Center" Margin="9,0,4,0"
                                        TextBlock.Foreground="{TemplateBinding Foreground}"/>
                    </Grid>
                  </Border>
                  <ControlTemplate.Triggers>
                    <Trigger Property="IsHighlighted" Value="True">
                      <Setter TargetName="bg" Property="Background" Value="#3D3D3D"/>
                    </Trigger>
                    <Trigger Property="IsEnabled" Value="False">
                      <Setter Property="Foreground" Value="#8E8E93"/>
                    </Trigger>
                  </ControlTemplate.Triggers>
                </ControlTemplate>
              </Setter.Value>
            </Setter>
          </Style>
          <Style x:Key="Sep" TargetType="Separator">
            <Setter Property="Template">
              <Setter.Value>
                <ControlTemplate TargetType="Separator">
                  <Border Height="1" Background="#3A3A3A" Margin="8,4"/>
                </ControlTemplate>
              </Setter.Value>
            </Setter>
          </Style>
        </ResourceDictionary>
        """);

    /// A fresh, themed, empty menu ready for AddItem/AddSeparator.
    public ContextMenu NewMenu() => new() { Style = (Style)Theme["Menu"] };

    /// Add a clickable row. `dot`, if given, draws a small state-coloured disc in the icon column.
    public MenuItem AddItem(ContextMenu menu, string text, Color? dot, Action onClick, bool enabled = true)
    {
        var item = new MenuItem { Header = text, Style = (Style)Theme["Item"], IsEnabled = enabled };
        if (dot is { } c)
            item.Icon = new Ellipse { Width = 10, Height = 10, Fill = new SolidColorBrush(c) };
        if (enabled) item.Click += (_, _) => onClick();
        menu.Items.Add(item);
        return item;
    }

    public void AddSeparator(ContextMenu menu) =>
        menu.Items.Add(new Separator { Style = (Style)Theme["Sep"] });

    /// Foreground a tiny invisible owner at the cursor, then open the menu there.
    public void Show(ContextMenu menu)
    {
        _owner ??= new Window
        {
            Width = 1, Height = 1, Left = -32000, Top = -32000,
            WindowStyle = WindowStyle.None, ShowInTaskbar = false,
            AllowsTransparency = true, Background = Brushes.Transparent, Opacity = 0,
            ResizeMode = ResizeMode.NoResize,
        };
        _owner.Show();

        var hwnd = new WindowInteropHelper(_owner).Handle;
        var src = PresentationSource.FromVisual(_owner);
        if (src?.CompositionTarget is { } ct && GetCursorPos(out var p))
        {
            var diu = ct.TransformFromDevice.Transform(new Point(p.X, p.Y));  // physical px → DIU
            _owner.Left = diu.X;
            _owner.Top = diu.Y;
        }
        SetForegroundWindow(hwnd);

        _open = menu;
        menu.PlacementTarget = _owner;
        menu.Placement = PlacementMode.Bottom;   // WPF flips it upward over a bottom taskbar
        menu.StaysOpen = false;                  // dismiss on outside click
        menu.Closed += (_, _) => _owner?.Hide();
        menu.IsOpen = true;
    }

    public void Dispose()
    {
        if (_open is not null) _open.IsOpen = false;
        _owner?.Close();
        _owner = null;
    }
}
