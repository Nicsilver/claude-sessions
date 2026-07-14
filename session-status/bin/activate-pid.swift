// Brings the application that owns <pid> to the foreground, fast.
//
// A drop-in replacement for the IntelliJ plugin's old activation call
//   osascript -e 'tell application "System Events" to set frontmost of
//                 (first process whose unix id is <pid>) to true'
// which routes through System Events + a full process-table scan and can spike to
// 1–2s. NSRunningApplication talks straight to the WindowServer: ~10ms and consistent.
//
// Build:  swiftc -O activate-pid.swift -o activate-pid
// Usage:  activate-pid <pid>
import AppKit

let pid = CommandLine.arguments.count > 1 ? (Int32(CommandLine.arguments[1]) ?? -1) : -1
if pid > 0, let app = NSRunningApplication(processIdentifier: pid) {
    app.activate(options: [.activateAllWindows])
}
