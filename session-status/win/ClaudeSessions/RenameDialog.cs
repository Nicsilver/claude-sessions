using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace ClaudeSessions;

/// Tiny modal for Alt-click rename. Returns the new name, "" to clear, or null if cancelled.
public static class RenameDialog
{
    public static string? Show(Window owner, string topic, string current)
    {
        var win = new Window
        {
            Title = "Rename session",
            Width = 300, Height = 130,
            WindowStyle = WindowStyle.ToolWindow,
            ResizeMode = ResizeMode.NoResize,
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            Owner = owner,
            Background = new SolidColorBrush(Styles.Bg),
            ShowInTaskbar = false,
        };
        var panel = new StackPanel { Margin = new Thickness(14) };
        panel.Children.Add(new TextBlock
        {
            Text = $"Custom name for “{topic}” (blank to clear):",
            Foreground = new SolidColorBrush(Styles.Secondary),
            FontSize = 11, Margin = new Thickness(0, 0, 0, 8), TextWrapping = TextWrapping.Wrap,
        });
        var box = new TextBox { Text = current, FontSize = 12 };
        panel.Children.Add(box);

        var buttons = new StackPanel { Orientation = Orientation.Horizontal,
                                       HorizontalAlignment = HorizontalAlignment.Right,
                                       Margin = new Thickness(0, 12, 0, 0) };
        string? result = null;
        var save = new Button { Content = "Save", Width = 64, IsDefault = true, Margin = new Thickness(0, 0, 8, 0) };
        var cancel = new Button { Content = "Cancel", Width = 64, IsCancel = true };
        save.Click += (_, _) => { result = box.Text; win.Close(); };
        cancel.Click += (_, _) => { result = null; win.Close(); };
        buttons.Children.Add(save); buttons.Children.Add(cancel);
        panel.Children.Add(buttons);

        win.Content = panel;
        box.Focus(); box.SelectAll();
        win.ShowDialog();
        return result;
    }
}
