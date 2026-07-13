using System.IO;
using System.Text.Json;

namespace ClaudeSessions;

/// One Claude session as shown in the widget. Mirrors the fields record.py writes to
/// ~/.claude/session-status/state/<id>.json (plus the Windows-only terminal/term_pid).
public sealed record Sess(
    string Id,
    string Topic,
    string State,
    double Updated,
    string Message,
    string Terminal,   // "wt" | "jetbrains" | "vscode" | "console" | "other" | ""
    int TermPid,       // ancestor window to focus (wt/vscode/console) — 0 if unknown
    int Pid,           // the Claude process pid (JetBrains jump matches on this)
    string TabTitle,   // WT tab title, for per-tab focus via UI Automation
    double MuteUntil);

/// Reads shared runtime state from ~/.claude/session-status/. All file access is best-effort:
/// a half-written or missing file is skipped, never thrown — the widget just shows less.
public static class StateReader
{
    public static readonly string Base =
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
                     ".claude", "session-status");
    public static readonly string StateDir = Path.Combine(Base, "state");
    public static readonly string RequestPath = Path.Combine(Base, "focus-request.json");
    public static readonly string MutesPath = Path.Combine(Base, "mutes.json");
    public static readonly string LabelsPath = Path.Combine(Base, "labels.json");

    private static readonly JsonSerializerOptions Opts = new() { PropertyNameCaseInsensitive = true };

    public static List<Sess> Load()
    {
        var mutes = LoadDoubleMap(MutesPath);
        var labels = LoadStringMap(LabelsPath);
        var outp = new List<Sess>();
        if (!Directory.Exists(StateDir)) return outp;

        foreach (var path in Directory.EnumerateFiles(StateDir, "*.json"))
        {
            JsonElement root;
            try
            {
                using var doc = JsonDocument.Parse(File.ReadAllText(path));
                root = doc.RootElement.Clone();
            }
            catch { continue; }   // partial write mid-refresh — skip this tick

            int pid = GetInt(root, "pid");
            if (pid > 0 && !ProcessAlive(pid)) continue;   // session's Claude process is gone

            string terminal = GetStr(root, "terminal");
            int termPid = GetInt(root, "term_pid");
            // Hide sessions with no resolvable terminal (IDE/headless with nothing to focus),
            // matching the macOS surface hiding empty-tty rows.
            if (terminal.Length == 0 && termPid == 0) continue;

            string id = GetStr(root, "session_id");
            string tabTitle = GetStr(root, "tab_title");
            // Display name, best-first: custom rename → the WT tab title Claude generates (much
            // better than the derived topic, which is often just the folder name) → topic.
            string display =
                labels.TryGetValue(id, out var custom) && custom.Length > 0 ? custom
                : tabTitle.Length > 0 ? tabTitle
                : GetStr(root, "topic") is { Length: > 0 } t ? t : "?";

            outp.Add(new Sess(
                Id: id,
                Topic: display,
                State: GetStr(root, "state") is { Length: > 0 } s ? s : "?",
                Updated: GetDouble(root, "updated_at"),
                Message: GetStr(root, "message"),
                Terminal: terminal,
                TermPid: termPid,
                Pid: pid,
                TabTitle: tabTitle,
                MuteUntil: mutes.TryGetValue(id, out var mu) ? mu : 0));
        }
        return outp;
    }

    // ---- request file (jump / close / mute / new), shared with the JetBrains plugin ----

    public static void WriteRequest(string tty, int pid, string terminal, int termPid, string action)
    {
        double ts = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() / 1000.0;
        var payload = new
        {
            tty,
            pid,
            terminal,
            term_pid = termPid,
            ts,
            action,
        };
        try
        {
            Directory.CreateDirectory(Base);
            File.WriteAllText(RequestPath, JsonSerializer.Serialize(payload));
        }
        catch { /* best effort */ }
    }

    public static void ToggleMute(string id)
    {
        if (id.Length == 0) return;
        double now = Now();
        var m = LoadDoubleMap(MutesPath);
        var kept = m.Where(kv => kv.Value > now).ToDictionary(kv => kv.Key, kv => kv.Value);
        if (kept.TryGetValue(id, out var until) && until > now) kept.Remove(id);
        else kept[id] = now + 3600;   // snooze 1h
        Save(MutesPath, kept);
    }

    public static void SetLabel(string id, string name)
    {
        if (id.Length == 0) return;
        var m = LoadStringMap(LabelsPath);
        var n = name.Trim();
        if (n.Length == 0) m.Remove(id); else m[id] = n;
        Save(LabelsPath, m);
    }

    public static Dictionary<string, string> LoadLabels() => LoadStringMap(LabelsPath);

    // ---- helpers ----

    public static double Now() => DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() / 1000.0;

    private static bool ProcessAlive(int pid)
    {
        try { using var p = System.Diagnostics.Process.GetProcessById(pid); return !p.HasExited; }
        catch { return false; }
    }

    private static void Save<T>(string path, Dictionary<string, T> map)
    {
        try
        {
            Directory.CreateDirectory(Base);
            File.WriteAllText(path, JsonSerializer.Serialize(map));
        }
        catch { }
    }

    private static Dictionary<string, double> LoadDoubleMap(string path)
    {
        var outp = new Dictionary<string, double>();
        try
        {
            using var doc = JsonDocument.Parse(File.ReadAllText(path));
            foreach (var p in doc.RootElement.EnumerateObject())
                if (p.Value.ValueKind == JsonValueKind.Number) outp[p.Name] = p.Value.GetDouble();
        }
        catch { }
        return outp;
    }

    private static Dictionary<string, string> LoadStringMap(string path)
    {
        var outp = new Dictionary<string, string>();
        try
        {
            using var doc = JsonDocument.Parse(File.ReadAllText(path));
            foreach (var p in doc.RootElement.EnumerateObject())
                if (p.Value.ValueKind == JsonValueKind.String) outp[p.Name] = p.Value.GetString() ?? "";
        }
        catch { }
        return outp;
    }

    private static string GetStr(JsonElement e, string k) =>
        e.TryGetProperty(k, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() ?? "" : "";
    private static int GetInt(JsonElement e, string k) =>
        e.TryGetProperty(k, out var v) && v.ValueKind == JsonValueKind.Number ? v.GetInt32() : 0;
    private static double GetDouble(JsonElement e, string k) =>
        e.TryGetProperty(k, out var v) && v.ValueKind == JsonValueKind.Number ? v.GetDouble() : 0;
}
