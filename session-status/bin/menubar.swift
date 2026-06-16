// Surface B — menu-bar badge + popover. Native, no dependencies.
//
// Build:  swiftc -O menubar.swift -o menubar
// Run:    ~/.claude/session-status/bin/menubar >/dev/null 2>&1 &
//
// A menu-bar item shows a colored "pill" + count for the most-urgent state (✅ when clear).
// LEFT-CLICK the icon (or press the global hotkey ⌃⌥⌘J) to jump to the top session in the
// list; RIGHT-/control-CLICK opens a dark popover that renders the same widget-style list as
// the floating dashboard (accent-bar rows, name + age, per-row glow, hover, footer counts) —
// in the list, LEFT-CLICK a row to jump, RIGHT-CLICK to close it, "+" to spawn a new session.
// Reads ~/.claude/session-status/state.
import Cocoa
import Darwin
import Carbon.HIToolbox   // global hotkey (RegisterEventHotKey)

let STATE_DIR = NSString(string: "~/.claude/session-status/state").expandingTildeInPath
let REQUEST_PATH = (STATE_DIR as NSString).deletingLastPathComponent + "/focus-request.json"

/// Ask the IntelliJ plugin to act via the watched request file:
/// "focus" jumps to the tab, "close" closes it, "new" spawns a fresh `clauded` session.
func writeRequest(_ tty: String, action: String) {
    let ts = Date().timeIntervalSince1970
    let json = "{\"tty\":\"\(tty)\",\"ts\":\(ts),\"action\":\"\(action)\"}"
    try? json.write(toFile: REQUEST_PATH, atomically: true, encoding: .utf8)
}
func writeFocusRequest(_ tty: String) { if !tty.isEmpty { writeRequest(tty, action: "focus") } }

func isAlive(_ pid: Int) -> Bool {
    if pid <= 0 { return true }
    return kill(pid_t(pid), 0) == 0 || errno == EPERM
}

struct Sess { var topic: String; var state: String; var updated: Double; var message: String; var tty: String }

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
            tty: tty))
    }
    return out
}

// MARK: - State → visual style

func order(_ s: String) -> Int { switch s { case "needs": return 0; case "yourturn": return 1; case "working": return 2; case "idle": return 3; default: return 4 } }

struct StateStyle { let symbol: String; let color: NSColor; let label: String }
func style(_ s: String) -> StateStyle {
    switch s {
    case "needs":    return StateStyle(symbol: "exclamationmark.circle.fill", color: .systemRed,          label: "Needs you")
    case "yourturn": return StateStyle(symbol: "circle.fill",                 color: .systemYellow,       label: "Your turn")
    case "working":  return StateStyle(symbol: "circle.fill",                 color: .systemGreen,        label: "Working")
    case "done":     return StateStyle(symbol: "checkmark.circle.fill",       color: .systemGray,         label: "Done")
    default:         return StateStyle(symbol: "circle",                      color: .tertiaryLabelColor, label: "Idle")
    }
}

func dotImage(_ st: StateStyle, size: CGFloat = 13, weight: NSFont.Weight = .semibold) -> NSImage? {
    let cfg = NSImage.SymbolConfiguration(pointSize: size, weight: weight)
    return NSImage(systemSymbolName: st.symbol, accessibilityDescription: st.label)?.withSymbolConfiguration(cfg)
}

/// A small colored capsule for the menu-bar badge — the same "pill" used as the row accent bar.
func pillImage(_ color: NSColor) -> NSImage {
    let w: CGFloat = 5, h: CGFloat = 12
    let img = NSImage(size: NSSize(width: w, height: h))
    img.lockFocus()
    color.setFill()
    NSBezierPath(roundedRect: NSRect(x: 0, y: 0, width: w, height: h), xRadius: w / 2, yRadius: w / 2).fill()
    img.unlockFocus()
    img.isTemplate = false   // keep the state colour (don't tint to the menu-bar fg)
    return img
}

func isActive(_ state: String) -> Bool { state == "needs" || state == "yourturn" || state == "working" }

func ageStr(_ ts: Double) -> String {
    if ts <= 0 { return "" }
    let s = Int(Date().timeIntervalSince1970 - ts)
    if s < 60 { return "\(s)s" }
    if s < 3600 { return "\(s / 60)m" }
    return "\(s / 3600)h"
}

func g(_ w: CGFloat) -> NSColor { NSColor(calibratedWhite: w, alpha: 1) }

// MARK: - Appearance & layout constants (shared look with the floating dashboard)

let ROW_HEIGHT: CGFloat = 22
let NAME_SIZE: CGFloat = 11.5
let META_SIZE: CGFloat = 10
let NAME_TINT = true
let GLOW_ALPHA: CGFloat = 0.22

let BAR_X: CGFloat = 7, BAR_W: CGFloat = 3
let NAME_LEADING: CGFloat = 14
let GLOW_FRAC: CGFloat = 0.30

// MARK: - Row views

/// Non-interactive label that never intercepts clicks, so a click on the row text reaches the cell.
final class PassLabel: NSTextField {
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

final class SessionRow: NSTableCellView {
    private let nameLabel = PassLabel()
    private let metaLabel = PassLabel()

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        for l in [nameLabel, metaLabel] {
            l.isEditable = false; l.isSelectable = false; l.isBezeled = false
            l.isBordered = false; l.drawsBackground = false
        }
        nameLabel.font = .systemFont(ofSize: NAME_SIZE, weight: .medium)
        nameLabel.lineBreakMode = .byTruncatingTail
        nameLabel.cell?.usesSingleLineMode = true
        nameLabel.translatesAutoresizingMaskIntoConstraints = false
        nameLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        nameLabel.setContentHuggingPriority(.defaultLow, for: .horizontal)

        metaLabel.font = .systemFont(ofSize: META_SIZE, weight: .regular)
        metaLabel.textColor = .secondaryLabelColor
        metaLabel.lineBreakMode = .byTruncatingTail
        metaLabel.cell?.usesSingleLineMode = true
        metaLabel.alignment = .right
        metaLabel.translatesAutoresizingMaskIntoConstraints = false
        metaLabel.setContentCompressionResistancePriority(.required, for: .horizontal)
        metaLabel.setContentHuggingPriority(.required, for: .horizontal)

        addSubview(nameLabel)
        addSubview(metaLabel)
        NSLayoutConstraint.activate([
            nameLabel.leadingAnchor.constraint(equalTo: leadingAnchor, constant: NAME_LEADING),
            nameLabel.centerYAnchor.constraint(equalTo: centerYAnchor),
            metaLabel.leadingAnchor.constraint(greaterThanOrEqualTo: nameLabel.trailingAnchor, constant: 8),
            metaLabel.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -11),
            metaLabel.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }
    required init?(coder: NSCoder) { fatalError() }

    func configure(_ s: Sess) {
        nameLabel.stringValue = s.topic
        nameLabel.textColor = (NAME_TINT && isActive(s.state)) ? style(s.state).color : .labelColor
        var parts: [String] = []
        if s.tty.isEmpty { parts.append("IDE") }
        let age = ageStr(s.updated)
        if !age.isEmpty { parts.append(age) }
        metaLabel.stringValue = parts.joined(separator: " · ")
        toolTip = s.message.isEmpty ? nil : s.message
    }
}

final class DecoRowView: NSTableRowView {
    var state: String = "" { didSet { if state != oldValue { needsDisplay = true } } }
    private var hovered = false { didSet { if hovered != oldValue { needsDisplay = true } } }
    private var hoverX: CGFloat = 0

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: .zero,
            options: [.mouseEnteredAndExited, .mouseMoved, .activeAlways, .inVisibleRect],
            owner: self, userInfo: nil))
    }
    override func mouseEntered(with event: NSEvent) { hovered = true; hoverX = convert(event.locationInWindow, from: nil).x; NSCursor.pointingHand.set(); needsDisplay = true }
    override func mouseExited(with event: NSEvent)  { hovered = false; NSCursor.arrow.set() }
    override func mouseMoved(with event: NSEvent)   { hoverX = convert(event.locationInWindow, from: nil).x; if hovered { needsDisplay = true } }

    private func barRect() -> NSRect {
        let bh = max(10, bounds.height - 7)
        return NSRect(x: BAR_X, y: bounds.midY - bh / 2, width: BAR_W, height: bh)
    }

    private func drawGlow(_ col: NSColor, _ active: Bool) {
        let a = GLOW_ALPHA * (active ? 1 : 0.30)
        let p = NSBezierPath(roundedRect: bounds.insetBy(dx: 2, dy: 1), xRadius: 6, yRadius: 6)
        if let grad = NSGradient(colorsAndLocations:
            (col.withAlphaComponent(0),  0.0),
            (col.withAlphaComponent(0),  GLOW_FRAC),
            (col.withAlphaComponent(a),  1.0)) {
            grad.draw(in: p, angle: 0)
        }
    }

    override func drawBackground(in dirtyRect: NSRect) {
        super.drawBackground(in: dirtyRect)
        if hovered {
            NSGraphicsContext.saveGraphicsState()
            NSBezierPath(roundedRect: bounds.insetBy(dx: 4, dy: 1), xRadius: 6, yRadius: 6).addClip()
            let c = NSPoint(x: hoverX, y: bounds.midY)
            if let grad = NSGradient(colors: [NSColor.white.withAlphaComponent(0.13), NSColor.white.withAlphaComponent(0)]) {
                grad.draw(fromCenter: c, radius: 0, toCenter: c, radius: 95, options: [])
            }
            NSGraphicsContext.restoreGraphicsState()
        }
        let col = style(state).color
        drawGlow(col, isActive(state))
        col.setFill()
        NSBezierPath(roundedRect: barRect(), xRadius: 1.5, yRadius: 1.5).fill()
    }
}

/// Right-click a row → immediate action (no context menu). No window-drag (it's a popover).
final class ClickTable: NSTableView {
    var onRightClick: ((Int) -> Void)?
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func rightMouseDown(with event: NSEvent) {
        let r = row(at: convert(event.locationInWindow, from: nil))
        if r >= 0 { onRightClick?(r) } else { super.rightMouseDown(with: event) }
    }
    override func menu(for event: NSEvent) -> NSMenu? { nil }
}

/// Clickable "+" disc: filled circle with a plus cut into it. Brightens + pointing hand on hover.
final class IconButton: NSView {
    var onClick: (() -> Void)?
    private var hovered = false { didSet { needsDisplay = true } }
    override init(frame f: NSRect) { super.init(frame: f) }
    required init?(coder: NSCoder) { fatalError() }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func mouseUp(with e: NSEvent) { if bounds.contains(convert(e.locationInWindow, from: nil)) { onClick?() } }
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
    }
    override func mouseEntered(with e: NSEvent) { hovered = true; NSCursor.pointingHand.set() }
    override func mouseExited(with e: NSEvent)  { hovered = false; NSCursor.arrow.set() }
    override func draw(_ dirtyRect: NSRect) {
        (hovered ? NSColor.labelColor : NSColor.secondaryLabelColor).setFill()
        NSBezierPath(ovalIn: bounds).fill()
        g(0.10).setFill()
        let d = bounds.width, arm = d * 0.46, t = max(1.6, d * 0.14)
        NSBezierPath(rect: NSRect(x: bounds.midX - arm / 2, y: bounds.midY - t / 2, width: arm, height: t)).fill()
        NSBezierPath(rect: NSRect(x: bounds.midX - t / 2, y: bounds.midY - arm / 2, width: t, height: arm)).fill()
    }
}

// MARK: - Popover content (the widget list)

final class PopoverList: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    var onActivate: (() -> Void)?          // host closes the popover after a jump/new
    var onSettings: (() -> Void)?          // host opens the settings window
    private var table: ClickTable!
    private var countStack: NSStackView!
    private var emptyLabel: NSTextField!
    private var sessions: [Sess] = []
    private var timer: Timer?

    override func loadView() {
        let content = NSView(frame: NSRect(x: 0, y: 0, width: 300, height: 340))
        content.wantsLayer = true
        content.layer?.backgroundColor = g(0.10).cgColor

        let cs = NSStackView()
        cs.orientation = .horizontal; cs.alignment = .centerY; cs.spacing = 14
        cs.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(cs); countStack = cs

        let plus = IconButton(frame: NSRect(x: 0, y: 0, width: 13, height: 13))
        plus.toolTip = "New Claude session (clauded)"
        plus.onClick = { [weak self] in writeRequest("", action: "new"); self?.onActivate?() }
        plus.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(plus)

        let gear = NSButton()
        gear.image = NSImage(systemSymbolName: "gearshape", accessibilityDescription: "Settings")?
            .withSymbolConfiguration(NSImage.SymbolConfiguration(pointSize: 12, weight: .regular))
        gear.imagePosition = .imageOnly
        gear.isBordered = false
        gear.contentTintColor = .secondaryLabelColor
        gear.toolTip = "Settings…"
        gear.target = self
        gear.action = #selector(openSettings)
        gear.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(gear)

        let sep = NSBox(); sep.boxType = .separator
        sep.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(sep)

        let scroll = NSScrollView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.hasVerticalScroller = true
        scroll.scrollerStyle = .overlay
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        scroll.automaticallyAdjustsContentInsets = false
        scroll.contentInsets = NSEdgeInsets(top: 3, left: 0, bottom: 4, right: 0)

        let tbl = ClickTable()
        tbl.onRightClick = { [weak self] r in
            guard let self, r < self.sessions.count else { return }
            writeRequest(self.sessions[r].tty, action: "close")
        }
        table = tbl
        table.style = .plain
        table.selectionHighlightStyle = .none
        table.headerView = nil
        table.backgroundColor = .clear
        table.gridStyleMask = []
        table.rowHeight = ROW_HEIGHT
        table.intercellSpacing = NSSize(width: 0, height: 1)
        let colmn = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("c"))
        colmn.resizingMask = .autoresizingMask
        table.addTableColumn(colmn)
        table.columnAutoresizingStyle = .lastColumnOnlyAutoresizingStyle
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.action = #selector(rowClicked)
        scroll.documentView = table
        content.addSubview(scroll)

        emptyLabel = NSTextField(labelWithString: "No active sessions")
        emptyLabel.font = .systemFont(ofSize: 12, weight: .regular)
        emptyLabel.textColor = .tertiaryLabelColor
        emptyLabel.alignment = .center
        emptyLabel.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(emptyLabel)

        NSLayoutConstraint.activate([
            cs.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -9),
            cs.centerXAnchor.constraint(equalTo: content.centerXAnchor),
            plus.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 14),
            plus.centerYAnchor.constraint(equalTo: cs.centerYAnchor),
            plus.widthAnchor.constraint(equalToConstant: 13),
            plus.heightAnchor.constraint(equalToConstant: 13),
            gear.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -12),
            gear.centerYAnchor.constraint(equalTo: cs.centerYAnchor),
            sep.bottomAnchor.constraint(equalTo: cs.topAnchor, constant: -8),
            sep.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 14),
            sep.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -14),

            scroll.topAnchor.constraint(equalTo: content.topAnchor, constant: 8),
            scroll.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 6),
            scroll.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -6),
            scroll.bottomAnchor.constraint(equalTo: sep.topAnchor, constant: -2),

            emptyLabel.centerXAnchor.constraint(equalTo: scroll.centerXAnchor),
            emptyLabel.centerYAnchor.constraint(equalTo: scroll.centerYAnchor),
        ])
        view = content
    }

    func startTimer() {
        refresh()
        timer = Timer.scheduledTimer(withTimeInterval: 1.5, repeats: true) { [weak self] _ in self?.refresh() }
    }
    func stopTimer() { timer?.invalidate(); timer = nil }

    @objc func rowClicked() {
        let r = table.clickedRow
        if r >= 0 && r < sessions.count { writeFocusRequest(sessions[r].tty); onActivate?() }
    }

    @objc func openSettings() { onActivate?(); onSettings?() }

    private func chip(_ stateKey: String, _ count: Int) -> NSView {
        let st = style(stateKey)
        let iv = NSImageView()
        iv.image = dotImage(st, size: 10, weight: .bold)
        iv.contentTintColor = count > 0 ? st.color : .tertiaryLabelColor
        iv.imageScaling = .scaleProportionallyDown
        let lbl = NSTextField(labelWithString: "\(count)")
        lbl.font = .systemFont(ofSize: 11.5, weight: .semibold)
        lbl.textColor = count > 0 ? .labelColor : .tertiaryLabelColor
        let h = NSStackView(views: [iv, lbl])
        h.orientation = .horizontal; h.alignment = .centerY; h.spacing = 4
        return h
    }

    func refresh() {
        sessions = loadSessions().sorted { a, b in
            order(a.state) != order(b.state) ? order(a.state) < order(b.state) : a.updated > b.updated
        }
        let counts = ["needs", "yourturn", "working", "done"].map { k in (k, sessions.filter { $0.state == k }.count) }
        countStack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for (key, n) in counts { countStack.addArrangedSubview(chip(key, n)) }
        emptyLabel.isHidden = !sessions.isEmpty
        table.reloadData()
    }

    func numberOfRows(in tableView: NSTableView) -> Int { sessions.count }

    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        let id = NSUserInterfaceItemIdentifier("deco")
        let rv = tableView.makeView(withIdentifier: id, owner: self) as? DecoRowView ?? {
            let r = DecoRowView(); r.identifier = id; return r
        }()
        rv.state = sessions[row].state
        return rv
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let id = NSUserInterfaceItemIdentifier("row")
        let cell = tableView.makeView(withIdentifier: id, owner: self) as? SessionRow ?? {
            let c = SessionRow(); c.identifier = id; return c
        }()
        cell.configure(sessions[row])
        return cell
    }
}

// MARK: - Settings (rebindable global hotkeys)

struct HotKey { var keyCode: UInt32; var mods: UInt32; var label: String }

/// Carbon modifier flags → ⌃⌥⇧⌘ symbols (display order).
func modSymbols(_ c: UInt32) -> String {
    var s = ""
    if c & UInt32(controlKey) != 0 { s += "⌃" }
    if c & UInt32(optionKey)  != 0 { s += "⌥" }
    if c & UInt32(shiftKey)   != 0 { s += "⇧" }
    if c & UInt32(cmdKey)     != 0 { s += "⌘" }
    return s
}
/// NSEvent modifier flags → Carbon modifier flags (for RegisterEventHotKey).
func carbonMods(_ m: NSEvent.ModifierFlags) -> UInt32 {
    var c: UInt32 = 0
    if m.contains(.control) { c |= UInt32(controlKey) }
    if m.contains(.option)  { c |= UInt32(optionKey) }
    if m.contains(.shift)   { c |= UInt32(shiftKey) }
    if m.contains(.command) { c |= UInt32(cmdKey) }
    return c
}

/// Click to record a shortcut: press a combo (needs ≥1 modifier) and it captures the
/// keyCode + Carbon modifiers, reporting via onChange. Esc cancels.
final class ShortcutField: NSView {
    var hotKey: HotKey { didSet { needsDisplay = true } }
    var onChange: ((HotKey) -> Void)?
    private var recording = false { didSet { needsDisplay = true } }

    init(_ hk: HotKey) { hotKey = hk; super.init(frame: NSRect(x: 0, y: 0, width: 150, height: 26)); wantsLayer = true }
    required init?(coder: NSCoder) { fatalError() }

    override var acceptsFirstResponder: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func mouseDown(with event: NSEvent) { recording = true; window?.makeFirstResponder(self) }
    override func resignFirstResponder() -> Bool { recording = false; return true }

    override func keyDown(with event: NSEvent) {
        guard recording else { super.keyDown(with: event); return }
        if event.keyCode == 53 { recording = false; return }   // Esc cancels
        let mods = event.modifierFlags.intersection([.command, .option, .control, .shift])
        if mods.isEmpty { NSSound.beep(); return }              // require at least one modifier
        let label = (event.charactersIgnoringModifiers ?? "").uppercased()
        hotKey = HotKey(keyCode: UInt32(event.keyCode), mods: carbonMods(mods), label: label.isEmpty ? "·" : label)
        recording = false
        onChange?(hotKey)
    }

    override func draw(_ dirtyRect: NSRect) {
        let r = bounds.insetBy(dx: 1, dy: 1)
        let path = NSBezierPath(roundedRect: r, xRadius: 5, yRadius: 5)
        g(0.18).setFill(); path.fill()
        (recording ? NSColor.controlAccentColor : g(0.32)).setStroke(); path.lineWidth = 1; path.stroke()
        let str = recording ? "Type shortcut…" : (modSymbols(hotKey.mods) + hotKey.label)
        let attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 12, weight: .medium),
            .foregroundColor: recording ? NSColor.secondaryLabelColor : NSColor.labelColor]
        let sz = (str as NSString).size(withAttributes: attrs)
        (str as NSString).draw(at: NSPoint(x: (bounds.width - sz.width) / 2, y: (bounds.height - sz.height) / 2), withAttributes: attrs)
    }
}

// MARK: - Status item + popover

class AppDelegate: NSObject, NSApplicationDelegate, NSPopoverDelegate {
    var statusItem: NSStatusItem!
    let popover = NSPopover()
    var list: PopoverList!
    var timer: Timer?
    var jumpRef: EventHotKeyRef?
    var newRef: EventHotKeyRef?
    var settingsWindow: NSWindow?

    // Defaults (⌃⌥⌘J / ⌃⌥⌘N); overridden by UserDefaults via loadConfig().
    var jumpHK = HotKey(keyCode: UInt32(kVK_ANSI_J), mods: UInt32(controlKey | optionKey | cmdKey), label: "J")
    var newHK  = HotKey(keyCode: UInt32(kVK_ANSI_N), mods: UInt32(controlKey | optionKey | cmdKey), label: "N")

    private func loadConfig() {
        let d = UserDefaults.standard
        if d.object(forKey: "jumpKeyCode") != nil {
            jumpHK = HotKey(keyCode: UInt32(d.integer(forKey: "jumpKeyCode")), mods: UInt32(d.integer(forKey: "jumpMods")), label: d.string(forKey: "jumpLabel") ?? "J")
        }
        if d.object(forKey: "newKeyCode") != nil {
            newHK = HotKey(keyCode: UInt32(d.integer(forKey: "newKeyCode")), mods: UInt32(d.integer(forKey: "newMods")), label: d.string(forKey: "newLabel") ?? "N")
        }
    }
    private func saveConfig() {
        let d = UserDefaults.standard
        d.set(Int(jumpHK.keyCode), forKey: "jumpKeyCode"); d.set(Int(jumpHK.mods), forKey: "jumpMods"); d.set(jumpHK.label, forKey: "jumpLabel")
        d.set(Int(newHK.keyCode),  forKey: "newKeyCode");  d.set(Int(newHK.mods),  forKey: "newMods");  d.set(newHK.label,  forKey: "newLabel")
    }

    /// Install the shared hotkey handler once; it routes by EventHotKeyID.id (1 = jump, 2 = new).
    private func installHotKeyHandler() {
        let target = GetApplicationEventTarget()
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: OSType(kEventHotKeyPressed))
        InstallEventHandler(target, { (_, event, userData) -> OSStatus in
            guard let userData, let event else { return noErr }
            var hkID = EventHotKeyID()
            GetEventParameter(event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID),
                              nil, MemoryLayout<EventHotKeyID>.size, nil, &hkID)
            let me = Unmanaged<AppDelegate>.fromOpaque(userData).takeUnretainedValue()
            let which = hkID.id
            DispatchQueue.main.async { me.hotKeyFired(which) }
            return noErr
        }, 1, &spec, Unmanaged.passUnretained(self).toOpaque(), nil)
    }

    /// (Re)register both global hotkeys from the current config.
    func registerHotKeys() {
        if let r = jumpRef { UnregisterEventHotKey(r); jumpRef = nil }
        if let r = newRef  { UnregisterEventHotKey(r); newRef = nil }
        let t = GetApplicationEventTarget()
        let sig: OSType = 0x434C5353 /* 'CLSS' */
        RegisterEventHotKey(jumpHK.keyCode, jumpHK.mods, EventHotKeyID(signature: sig, id: 1), t, 0, &jumpRef)
        RegisterEventHotKey(newHK.keyCode,  newHK.mods,  EventHotKeyID(signature: sig, id: 2), t, 0, &newRef)
    }

    func hotKeyFired(_ id: UInt32) {
        if id == 1 { jumpToTop() } else if id == 2 { writeRequest("", action: "new") }
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        statusItem.button?.target = self
        statusItem.button?.action = #selector(statusClicked)
        statusItem.button?.sendAction(on: [.leftMouseUp, .rightMouseUp])
        statusItem.button?.imagePosition = .imageLeading

        list = PopoverList()
        list.onActivate = { [weak self] in self?.popover.performClose(nil) }
        list.onSettings = { [weak self] in self?.showSettings() }
        popover.contentViewController = list
        popover.contentSize = NSSize(width: 300, height: 340)
        popover.behavior = .transient
        popover.appearance = NSAppearance(named: .darkAqua)
        popover.delegate = self

        loadConfig()
        installHotKeyHandler()
        registerHotKeys()

        refreshBadge()
        timer = Timer.scheduledTimer(withTimeInterval: 1.5, repeats: true) { [weak self] _ in self?.refreshBadge() }
    }

    private func settingsLabel(_ s: String) -> NSTextField {
        let l = NSTextField(labelWithString: s)
        l.font = .systemFont(ofSize: 12); l.textColor = .labelColor
        return l
    }

    func showSettings() {
        if settingsWindow == nil {
            let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 340, height: 150),
                             styleMask: [.titled, .closable], backing: .buffered, defer: false)
            w.title = "Claude Sessions Settings"
            w.appearance = NSAppearance(named: .darkAqua)
            w.isReleasedWhenClosed = false
            w.center()

            let jumpField = ShortcutField(jumpHK)
            jumpField.onChange = { [weak self] hk in self?.jumpHK = hk; self?.saveConfig(); self?.registerHotKeys() }
            let newField = ShortcutField(newHK)
            newField.onChange = { [weak self] hk in self?.newHK = hk; self?.saveConfig(); self?.registerHotKeys() }

            let grid = NSGridView(views: [
                [settingsLabel("Open top session"), jumpField],
                [settingsLabel("New chat"),         newField],
            ])
            grid.rowSpacing = 14; grid.columnSpacing = 16
            grid.column(at: 0).xPlacement = .trailing
            grid.translatesAutoresizingMaskIntoConstraints = false
            w.contentView?.addSubview(grid)
            NSLayoutConstraint.activate([
                grid.centerXAnchor.constraint(equalTo: w.contentView!.centerXAnchor),
                grid.centerYAnchor.constraint(equalTo: w.contentView!.centerYAnchor),
            ])
            settingsWindow = w
        }
        settingsWindow?.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    /// Left-click jumps to the top (most-urgent) session; right-/control-click opens the list.
    @objc func statusClicked() {
        let e = NSApp.currentEvent
        let rightish = e?.type == .rightMouseUp || (e?.modifierFlags.contains(.control) ?? false)
        if rightish { togglePopover() } else { jumpToTop() }
    }

    /// Sorted like the list (needs → your turn → working → idle → done, newest first); jump to #1.
    func jumpToTop() {
        let top = loadSessions().sorted { a, b in
            order(a.state) != order(b.state) ? order(a.state) < order(b.state) : a.updated > b.updated
        }.first
        if let top { writeFocusRequest(top.tty) }
    }

    @objc func togglePopover() {
        if popover.isShown { popover.performClose(nil); return }
        guard let b = statusItem.button else { return }
        list.startTimer()
        popover.show(relativeTo: b.bounds, of: b, preferredEdge: .minY)
    }

    func popoverDidClose(_ notification: Notification) { list.stopTimer() }

    func refreshBadge() {
        let s = loadSessions()
        let needs = s.filter { $0.state == "needs" }.count
        let yt = s.filter { $0.state == "yourturn" }.count
        let working = s.filter { $0.state == "working" }.count
        if let button = statusItem.button {
            if needs > 0 { button.image = pillImage(.systemRed);    button.title = " \(needs)" }
            else if yt > 0 { button.image = pillImage(.systemYellow); button.title = " \(yt)" }
            else if working > 0 { button.image = pillImage(.systemGreen); button.title = " \(working)" }
            else { button.image = nil; button.title = "✅" }
        }
    }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
