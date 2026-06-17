// Generates AppIcon.png (1024²) for Claude Sessions: a dark squircle with three stacked
// status pills (red / yellow / green). Run:  swiftc gen-icon.swift -o /tmp/genicon && /tmp/genicon
// then convert to .icns with sips/iconutil (see the build steps in README).
import Cocoa

let size: CGFloat = 1024
let img = NSImage(size: NSSize(width: size, height: size))
img.lockFocus()

// Dark rounded-square background.
NSColor(calibratedWhite: 0.11, alpha: 1).setFill()
NSBezierPath(roundedRect: NSRect(x: 0, y: 0, width: size, height: size), xRadius: 228, yRadius: 228).fill()

// Three horizontal status pills, vertically centered.
let pillW: CGFloat = 470, pillH: CGFloat = 104, vgap: CGFloat = 78
let totalH = pillH * 3 + vgap * 2
let x = (size - pillW) / 2
var y = (size - totalH) / 2 + (pillH + vgap) * 2     // top pill (bottom-left origin)
for c in [NSColor.systemRed, .systemYellow, .systemGreen] {
    c.setFill()
    NSBezierPath(roundedRect: NSRect(x: x, y: y, width: pillW, height: pillH), xRadius: pillH / 2, yRadius: pillH / 2).fill()
    y -= (pillH + vgap)
}

img.unlockFocus()
if let tiff = img.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff),
   let png = rep.representation(using: .png, properties: [:]) {
    try? png.write(to: URL(fileURLWithPath: "AppIcon.png"))
    print("wrote AppIcon.png")
}
