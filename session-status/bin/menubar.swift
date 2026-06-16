// Surface B — menu-bar badge (spike). Native, no dependencies.
//
// Build:  swiftc -O menubar.swift -o menubar
// Run:    ~/.claude/session-status/bin/menubar >/dev/null 2>&1 &
//
// Shows a menu-bar item: "🔴N" when N sessions need you, "🟢N" when only
// working, "✅" when all idle/done. The dropdown lists every session;
// clicking one activates IntelliJ IDEA (best-effort jump). Reads the same
// ~/.claude/session-status/state/*.json the dashboard does.
import Cocoa
import Darwin

let STATE_DIR = NSString(string: "~/.claude/session-status/state").expandingTildeInPath
let REQUEST_PATH = (STATE_DIR as NSString).deletingLastPathComponent + "/focus-request.json"

/// Ask the IntelliJ plugin to jump to the tab on this tty (it watches this file).
func writeFocusRequest(_ tty: String) {
    if tty.isEmpty { return }
    let ts = Date().timeIntervalSince1970
    let json = "{\"tty\":\"\(tty)\",\"ts\":\(ts)}"
    try? json.write(toFile: REQUEST_PATH, atomically: true, encoding: .utf8)
}

func isAlive(_ pid: Int) -> Bool {
    if pid <= 0 { return true }
    return kill(pid_t(pid), 0) == 0 || errno == EPERM
}

struct Sess {
    var topic: String
    var state: String
    var updated: Double
    var message: String
    var tty: String
}

func loadSessions() -> [Sess] {
    let fm = FileManager.default
    guard let files = try? fm.contentsOfDirectory(atPath: STATE_DIR) else { return [] }
    var out: [Sess] = []
    for f in files where f.hasSuffix(".json") {
        let p = STATE_DIR + "/" + f
        guard let data = fm.contents(atPath: p),
              let obj = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { continue }
        let pid = (obj["pid"] as? Int) ?? (obj["ppid"] as? Int) ?? 0
        if pid > 0, !isAlive(pid) { continue }
        let tty = (obj["tty"] as? String) ?? ""
        if tty.isEmpty { continue }   // hide IDE/ACP-spawned sessions (no terminal tab)
        out.append(Sess(
            topic: (obj["topic"] as? String) ?? "?",
            state: (obj["state"] as? String) ?? "?",
            updated: (obj["updated_at"] as? Double) ?? 0,
            message: (obj["message"] as? String) ?? "",
            tty: tty
        ))
    }
    return out
}

func order(_ s: String) -> Int {
    switch s { case "needs": return 0; case "yourturn": return 1; case "working": return 2; case "idle": return 3; default: return 4 }
}
func glyph(_ s: String) -> String {
    switch s { case "needs": return "🔴"; case "yourturn": return "🟡"; case "working": return "🟢"; case "done": return "✅"; default: return "⚪" }
}
func stateLabel(_ s: String) -> String {
    switch s { case "needs": return "needs you"; case "yourturn": return "your turn"; case "working": return "working"; case "done": return "done"; default: return "idle" }
}
func ageStr(_ ts: Double) -> String {
    if ts <= 0 { return "" }
    let s = Int(Date().timeIntervalSince1970 - ts)
    if s < 60 { return "\(s)s" }
    if s < 3600 { return "\(s / 60)m" }
    return "\(s / 3600)h"
}

class AppDelegate: NSObject, NSApplicationDelegate {
    var statusItem: NSStatusItem!
    var timer: Timer?

    func applicationDidFinishLaunching(_ notification: Notification) {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        let menu = NSMenu()
        menu.autoenablesItems = false
        statusItem.menu = menu
        refresh()
        timer = Timer.scheduledTimer(withTimeInterval: 1.5, repeats: true) { [weak self] _ in self?.refresh() }
    }

    @objc func activate(_ sender: NSMenuItem) {
        // Only write the request; the plugin (in whichever IDE owns the tab) raises
        // the correct window. Activating "IntelliJ IDEA" here would wrongly raise the
        // main IDE instead of the one holding the tab.
        (sender.representedObject as? String).map { writeFocusRequest($0) }
    }

    @objc func quit(_ sender: Any?) { NSApp.terminate(nil) }

    func refresh() {
        var sessions = loadSessions()
        sessions.sort { a, b in
            order(a.state) != order(b.state) ? order(a.state) < order(b.state) : a.updated > b.updated
        }
        let needs = sessions.filter { $0.state == "needs" }.count
        let yourturn = sessions.filter { $0.state == "yourturn" }.count
        let working = sessions.filter { $0.state == "working" }.count

        if let button = statusItem.button {
            if needs > 0 { button.title = "🔴\(needs)" }
            else if yourturn > 0 { button.title = "🟡\(yourturn)" }
            else if working > 0 { button.title = "🟢\(working)" }
            else { button.title = "✅" }
        }

        guard let menu = statusItem.menu else { return }
        menu.removeAllItems()

        let header = NSMenuItem(title: "🔴 \(needs) need you   ·   🟡 \(yourturn) your turn   ·   🟢 \(working) working", action: nil, keyEquivalent: "")
        header.isEnabled = false
        menu.addItem(header)
        menu.addItem(NSMenuItem.separator())

        if sessions.isEmpty {
            let none = NSMenuItem(title: "No active sessions", action: nil, keyEquivalent: "")
            none.isEnabled = false
            menu.addItem(none)
        }
        for s in sessions {
            let age = ageStr(s.updated)
            let suffix = age.isEmpty ? "" : "   ·   \(stateLabel(s.state)) \(age)"
            let name = s.topic + (s.tty.isEmpty ? "  (ide)" : "")
            let item = NSMenuItem(title: "\(glyph(s.state))  \(name)\(suffix)", action: #selector(activate(_:)), keyEquivalent: "")
            item.target = self
            item.isEnabled = true
            item.representedObject = s.tty
            if (s.state == "needs" || s.state == "yourturn") && !s.message.isEmpty { item.toolTip = s.message }
            menu.addItem(item)
        }
        menu.addItem(NSMenuItem.separator())
        let q = NSMenuItem(title: "Quit session-status", action: #selector(quit(_:)), keyEquivalent: "q")
        q.target = self
        menu.addItem(q)
    }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
