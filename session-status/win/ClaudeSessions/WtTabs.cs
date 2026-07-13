using System.Text;
using System.Windows.Automation;

namespace ClaudeSessions;

/// Per-tab focus for Windows Terminal via UI Automation. WT exposes no public API to focus a
/// tab by process, but each tab is a UIA TabItem whose Name is the tab title and which supports
/// SelectionItemPattern. We match the session to its tab by title and Select() the winner.
///
/// Matching is fuzzy on purpose: the title Claude writes to the tab and the title record.py
/// captures can drift (a /rename, or the tab shows an older AI-title). So we compare on shared
/// word tokens drawn from BOTH the session's tab_title and its topic, and only switch when
/// there's a confident winner — otherwise we leave the window on its current tab.
public static class WtTabs
{
    private static readonly HashSet<string> Stop = new(StringComparer.OrdinalIgnoreCase)
    { "the", "and", "for", "with", "into", "from", "that", "this", "your", "you", "set", "up",
      "add", "fix", "new", "run", "get", "out", "off", "was", "are", "has" };

    public static bool SelectTab(nint wtHwnd, string tabTitle, string topic)
    {
        if (wtHwnd == 0) return false;
        var want = Tokens(tabTitle).Union(Tokens(topic)).ToHashSet(StringComparer.OrdinalIgnoreCase);
        string exactA = Norm(tabTitle), exactB = Norm(topic);
        if (want.Count == 0 && exactA.Length == 0 && exactB.Length == 0) return false;

        try
        {
            var root = AutomationElement.FromHandle(wtHwnd);
            if (root is null) return false;
            var cond = new PropertyCondition(AutomationElement.ControlTypeProperty, ControlType.TabItem);
            var tabs = root.FindAll(TreeScope.Descendants, cond);
            if (tabs.Count <= 1) return false;   // single tab — nothing to switch to

            AutomationElement? best = null;
            int bestScore = 0, secondScore = 0;
            foreach (AutomationElement t in tabs)
            {
                string name = t.Current.Name ?? "";
                string nn = Norm(name);
                if (nn.Length > 0 && (nn == exactA || nn == exactB))   // exact title still wins outright
                    return Select(t);
                int score = Tokens(name).Count(want.Contains);
                if (score > bestScore) { secondScore = bestScore; bestScore = score; best = t; }
                else if (score > secondScore) { secondScore = score; }
            }
            // Confident match: a clear token winner (>=2 shared, or a single distinctive token that
            // no other tab shares). A tie or nothing shared → don't guess.
            if (best is not null && (bestScore >= 2 || (bestScore == 1 && secondScore == 0)))
                return Select(best);
        }
        catch { /* UIA can throw on transient window state — treat as "couldn't switch" */ }
        return false;
    }

    private static bool Select(AutomationElement tab)
    {
        if (tab.TryGetCurrentPattern(SelectionItemPattern.Pattern, out var p))
        {
            ((SelectionItemPattern)p).Select();
            return true;
        }
        return false;
    }

    /// Lowercased, punctuation/glyph-free, "administrator:" prefix removed.
    private static string Norm(string s)
    {
        if (string.IsNullOrEmpty(s)) return "";
        var sb = new StringBuilder(s.Length);
        foreach (char c in s)
            sb.Append(char.IsLetterOrDigit(c) ? char.ToLowerInvariant(c) : ' ');
        var flat = string.Join(' ', sb.ToString().Split(' ', StringSplitOptions.RemoveEmptyEntries));
        const string admin = "administrator ";
        return flat.StartsWith(admin, StringComparison.Ordinal) ? flat[admin.Length..] : flat;
    }

    /// Meaningful word tokens (len >= 3, not a stopword) for overlap scoring.
    private static IEnumerable<string> Tokens(string s) =>
        Norm(s).Split(' ', StringSplitOptions.RemoveEmptyEntries)
               .Where(w => w.Length >= 3 && !Stop.Contains(w));
}
