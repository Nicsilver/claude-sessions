using System.Windows.Media;

namespace ClaudeSessions;

/// State → visual style, ported from floatdash.swift. Colours approximate the macOS
/// system palette so the two surfaces read the same.
public static class Styles
{
    public static readonly Color Red    = Color.FromRgb(0xFF, 0x45, 0x3A); // needs
    public static readonly Color Yellow = Color.FromRgb(0xFF, 0xD6, 0x0A); // your turn
    public static readonly Color Green  = Color.FromRgb(0x34, 0xC7, 0x59); // working
    public static readonly Color Gray   = Color.FromRgb(0x8E, 0x8E, 0x93); // done
    public static readonly Color Faint  = Color.FromRgb(0x5A, 0x5A, 0x5E); // idle

    public static readonly Color Bg          = Color.FromRgb(26, 26, 26);  // g(0.10)
    public static readonly Color Label        = Color.FromRgb(0xEC, 0xEC, 0xEC);
    public static readonly Color Secondary    = Color.FromRgb(0x98, 0x98, 0x9E);
    public static readonly Color Tertiary     = Color.FromRgb(0x6A, 0x6A, 0x6E);

    public static Color ColorFor(string state) => state switch
    {
        "needs"    => Red,
        "yourturn" => Yellow,
        "working"  => Green,
        "done"     => Gray,
        _          => Faint,
    };

    public static string LabelFor(string state) => state switch
    {
        "needs"    => "Needs you",
        "yourturn" => "Your turn",
        "working"  => "Working",
        "done"     => "Done",
        _          => "Idle",
    };

    /// Sort rank: needs first, then your-turn, working, idle, everything else.
    public static int Order(string state) => state switch
    {
        "needs" => 0, "yourturn" => 1, "working" => 2, "idle" => 3, _ => 4,
    };

    public static bool IsActive(string state) =>
        state is "needs" or "yourturn" or "working";

    public static string AgeStr(double ts)
    {
        if (ts <= 0) return "";
        int s = (int)(StateReader.Now() - ts);
        if (s < 60) return $"{s}s";
        if (s < 3600) return $"{s / 60}m";
        return $"{s / 3600}h";
    }
}
