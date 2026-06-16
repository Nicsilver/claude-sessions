// Surface A — always-on-top floating dashboard panel (clickable). Native, no deps.
//
// Build:  swiftc -O floatdash.swift -o floatdash
// Run:    ~/.claude/session-status/bin/floatdash >/dev/null 2>&1 &
//   Pick a look:    floatdash --variant N
//   Gallery tile:   floatdash --variant N --slot N --galleryOf TOTAL
//
// Pinned above all windows, on every Space, non-activating (clicking it won't steal
// focus from your app). Lists every Claude session; CLICK a row to jump to its
// terminal tab/pane (writes ~/.claude/session-status/focus-request.json, which the
// IntelliJ plugin watches). Reads ~/.claude/session-status/state/*.json.
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
        out.append(Sess(
            topic: (obj["topic"] as? String) ?? "?",
            state: (obj["state"] as? String) ?? "?",
            updated: (obj["updated_at"] as? Double) ?? 0,
            message: (obj["message"] as? String) ?? "",
            tty: (obj["tty"] as? String) ?? ""))
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

func isActive(_ state: String) -> Bool { state == "needs" || state == "yourturn" || state == "working" }

func ageStr(_ ts: Double) -> String {
    if ts <= 0 { return "" }
    let s = Int(Date().timeIntervalSince1970 - ts)
    if s < 60 { return "\(s)s" }
    if s < 3600 { return "\(s / 60)m" }
    return "\(s / 3600)h"
}

func g(_ w: CGFloat) -> NSColor { NSColor(calibratedWhite: w, alpha: 1) }

// MARK: - Theme

enum Wash { case none, flat, fade, sheen }

struct Theme {
    let name: String
    let rowHeight: CGFloat
    let glowRadius: CGFloat       // radial bloom radius (0 = no glow); shapeless, not pill-outlined
    let glowAlpha: CGFloat
    let glowBand: Bool            // soft vertical light band instead of a radial bloom
    let wash: Wash
    let washAlpha: CGFloat
    let nameTint: Bool
    let nameSize: CGFloat
    let metaSize: CGFloat
    let w: CGFloat
    let h: CGFloat
}

private func T(_ name: String, rowHeight: CGFloat = 22, glowRadius: CGFloat = 14, glowAlpha: CGFloat = 0.5,
               glowBand: Bool = false, wash: Wash = .fade, washAlpha: CGFloat = 0.09, nameTint: Bool = true,
               nameSize: CGFloat = 11.5, metaSize: CGFloat = 10, w: CGFloat = 300, h: CGFloat = 270) -> Theme {
    Theme(name: name, rowHeight: rowHeight, glowRadius: glowRadius, glowAlpha: glowAlpha, glowBand: glowBand,
          wash: wash, washAlpha: washAlpha, nameTint: nameTint, nameSize: nameSize, metaSize: metaSize, w: w, h: h)
}

// Base = Glow Soft (fade wash + tinted name), text tight to the bar, and a SHAPELESS
// radial bloom instead of the pill-shaped shadow. The 10 explore the bloom.
let THEMES: [Theme] = [
    T("Halo",   glowAlpha: 0.22, wash: .none, washAlpha: 0),         // 0 — the chosen look (default)
]

// argv: --variant, --slot, --galleryOf
var VARIANT = 0, SLOT = -1, GALLERY = 0
do {
    let a = CommandLine.arguments
    for (i, arg) in a.enumerated() where i + 1 < a.count {
        if arg == "--variant" { VARIANT = Int(a[i + 1]) ?? 0 }
        if arg == "--slot" { SLOT = Int(a[i + 1]) ?? -1 }
        if arg == "--galleryOf" { GALLERY = Int(a[i + 1]) ?? 0 }
    }
}
VARIANT = max(0, min(THEMES.count - 1, VARIANT))
if SLOT < 0 { SLOT = VARIANT }
let THEME = THEMES[VARIANT]

let BAR_X: CGFloat = 7, BAR_W: CGFloat = 3
let NAME_LEADING: CGFloat = 14   // ≈4pt gap off the bar
let GLOW_FRAC: CGFloat = 0.30    // bloom begins ~30% into the row, not at the bar

// MARK: - Row views

final class SessionRow: NSTableCellView {
    private let nameLabel = NSTextField(labelWithString: "")
    private let metaLabel = NSTextField(labelWithString: "")

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        nameLabel.font = .systemFont(ofSize: THEME.nameSize, weight: .medium)
        nameLabel.lineBreakMode = .byTruncatingTail
        nameLabel.cell?.usesSingleLineMode = true
        nameLabel.translatesAutoresizingMaskIntoConstraints = false
        nameLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        nameLabel.setContentHuggingPriority(.defaultLow, for: .horizontal)

        metaLabel.font = .systemFont(ofSize: THEME.metaSize, weight: .regular)
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
        nameLabel.textColor = (THEME.nameTint && isActive(s.state)) ? style(s.state).color : .labelColor
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
    private var hoverX: CGFloat = 0   // cursor x within the row; the glow blooms from here

    // Per-row hover tracking so you can see which row you're about to click.
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
        let a = THEME.glowAlpha * (active ? 1 : 0.30)
        guard a > 0 else { return }
        if THEME.glowBand {
            // Soft vertical light band centred at GLOW_FRAC.
            NSGraphicsContext.saveGraphicsState()
            NSBezierPath(rect: bounds).addClip()
            let cx = bounds.minX + bounds.width * GLOW_FRAC
            let w: CGFloat = 26
            let r = NSRect(x: cx - w / 2, y: bounds.minY, width: w, height: bounds.height)
            if let grad = NSGradient(colors: [col.withAlphaComponent(0), col.withAlphaComponent(a), col.withAlphaComponent(0)]) {
                grad.draw(in: NSBezierPath(rect: r), angle: 0)
            }
            NSGraphicsContext.restoreGraphicsState()
            return
        }
        // Horizontal glow: brightest at the right edge, fading left to nothing by GLOW_FRAC.
        let p = NSBezierPath(roundedRect: bounds.insetBy(dx: 2, dy: 1), xRadius: 6, yRadius: 6)
        if let grad = NSGradient(colorsAndLocations:
            (col.withAlphaComponent(0),  0.0),
            (col.withAlphaComponent(0),  GLOW_FRAC),
            (col.withAlphaComponent(a),  1.0)) {
            grad.draw(in: p, angle: 0)
        }
    }

    private func drawWash(_ col: NSColor, _ active: Bool) {
        guard THEME.wash != .none else { return }
        let a = THEME.washAlpha * (active ? 1 : 0.3)
        let p = NSBezierPath(roundedRect: bounds.insetBy(dx: 4, dy: 1), xRadius: 6, yRadius: 6)
        switch THEME.wash {
        case .flat:
            col.withAlphaComponent(a).setFill(); p.fill()
        case .fade:
            if let grad = NSGradient(colors: [col.withAlphaComponent(a), col.withAlphaComponent(0)]) { grad.draw(in: p, angle: 0) }
        case .sheen:
            if let grad = NSGradient(colors: [col.withAlphaComponent(a), col.withAlphaComponent(a * 0.2)]) { grad.draw(in: p, angle: -90) }
        case .none: break
        }
    }

    override func drawBackground(in dirtyRect: NSRect) {
        super.drawBackground(in: dirtyRect)
        if hovered {
            NSGraphicsContext.saveGraphicsState()
            NSBezierPath(roundedRect: bounds.insetBy(dx: 4, dy: 1), xRadius: 6, yRadius: 6).addClip()
            let c = NSPoint(x: hoverX, y: bounds.midY)   // bloom from the cursor, along the row
            if let grad = NSGradient(colors: [NSColor.white.withAlphaComponent(0.13), NSColor.white.withAlphaComponent(0)]) {
                grad.draw(fromCenter: c, radius: 0, toCenter: c, radius: 95, options: [])
            }
            NSGraphicsContext.restoreGraphicsState()
        }
        let col = style(state).color
        let active = isActive(state)
        drawWash(col, active)
        drawGlow(col, active)
        // Crisp bar on top — no shape-tracing shadow.
        col.setFill()
        NSBezierPath(roundedRect: barRect(), xRadius: 1.5, yRadius: 1.5).fill()
    }
}

// MARK: - App

class AppDelegate: NSObject, NSApplicationDelegate, NSTableViewDataSource, NSTableViewDelegate, NSWindowDelegate {
    var panel: NSPanel!
    var countStack: NSStackView?
    var table: NSTableView!
    var emptyLabel: NSTextField!
    var sessions: [Sess] = []
    var timer: Timer?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let vf = NSScreen.main?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1440, height: 900)

        let rect: NSRect
        if GALLERY > 0 {
            let pw: CGFloat = 268, ph: CGFloat = 250
            let cellW = pw + 10, cellH = ph + 36
            let cols = max(1, Int((vf.width - 12) / cellW))
            let c = SLOT % cols, r = SLOT / cols
            rect = NSRect(x: vf.minX + 10 + CGFloat(c) * cellW,
                          y: vf.maxY - 10 - CGFloat(r) * cellH - ph,
                          width: pw, height: ph)
        } else {
            rect = NSRect(x: vf.maxX - THEME.w - 20, y: vf.maxY - THEME.h - 20, width: THEME.w, height: THEME.h)
        }

        panel = NSPanel(contentRect: rect,
                        styleMask: [.titled, .closable, .resizable, .utilityWindow, .fullSizeContentView],
                        backing: .buffered, defer: false)
        panel.appearance = NSAppearance(named: .darkAqua)
        panel.title = "\(VARIANT). \(THEME.name)"
        panel.titlebarAppearsTransparent = true
        panel.titleVisibility = .visible
        panel.isFloatingPanel = true
        panel.level = .floating
        panel.hidesOnDeactivate = false
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        panel.isMovableByWindowBackground = true
        panel.minSize = NSSize(width: 240, height: 140)
        panel.delegate = self

        let content = NSView()
        content.wantsLayer = true
        content.layer?.backgroundColor = g(0.10).cgColor
        content.translatesAutoresizingMaskIntoConstraints = false
        panel.contentView = content
        panel.isOpaque = true
        panel.backgroundColor = g(0.10)

        let guide = panel.contentLayoutGuide as! NSLayoutGuide

        let cs = NSStackView()
        cs.orientation = .horizontal
        cs.alignment = .centerY
        cs.spacing = 14
        cs.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(cs)
        countStack = cs

        let sep = NSBox(); sep.boxType = .separator
        sep.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(sep)

        let scroll = NSScrollView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.hasVerticalScroller = true
        scroll.scrollerStyle = .overlay        // hidden; fades in only while scrolling
        scroll.autohidesScrollers = true       // and only when there's actually overflow
        scroll.drawsBackground = false
        scroll.automaticallyAdjustsContentInsets = false
        scroll.contentInsets = NSEdgeInsets(top: 3, left: 0, bottom: 4, right: 0)

        table = NSTableView()
        table.style = .plain                  // same origin for bar (row) and text (cell)
        table.selectionHighlightStyle = .none  // no blue selection — click just jumps
        table.headerView = nil
        table.backgroundColor = .clear
        table.gridStyleMask = []
        table.rowHeight = THEME.rowHeight
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
            sep.bottomAnchor.constraint(equalTo: cs.topAnchor, constant: -8),
            sep.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 14),
            sep.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -14),

            scroll.topAnchor.constraint(equalTo: guide.topAnchor, constant: 4),
            scroll.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 6),
            scroll.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -6),
            scroll.bottomAnchor.constraint(equalTo: sep.topAnchor, constant: -2),

            emptyLabel.centerXAnchor.constraint(equalTo: scroll.centerXAnchor),
            emptyLabel.centerYAnchor.constraint(equalTo: scroll.centerYAnchor),
        ])

        panel.makeKeyAndOrderFront(nil)
        refresh()
        timer = Timer.scheduledTimer(withTimeInterval: 1.5, repeats: true) { [weak self] _ in self?.refresh() }
    }

    func windowWillClose(_ notification: Notification) { NSApp.terminate(nil) }

    @objc func rowClicked() {
        let r = table.clickedRow
        if r >= 0 && r < sessions.count { writeFocusRequest(sessions[r].tty) }
    }

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
        h.orientation = .horizontal
        h.alignment = .centerY
        h.spacing = 4
        return h
    }

    func refresh() {
        sessions = loadSessions().sorted { a, b in
            order(a.state) != order(b.state) ? order(a.state) < order(b.state) : a.updated > b.updated
        }
        if let cs = countStack {
            let counts = ["needs", "yourturn", "working", "done"].map { k in (k, sessions.filter { $0.state == k }.count) }
            cs.arrangedSubviews.forEach { $0.removeFromSuperview() }
            for (key, n) in counts { cs.addArrangedSubview(chip(key, n)) }
        }
        emptyLabel.isHidden = !sessions.isEmpty

        let selectedTty = (table.selectedRow >= 0 && table.selectedRow < sessions.count)
            ? sessions[table.selectedRow].tty : nil
        table.reloadData()
        if let t = selectedTty, let i = sessions.firstIndex(where: { $0.tty == t }) {
            table.selectRowIndexes(IndexSet(integer: i), byExtendingSelection: false)
        }
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

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
