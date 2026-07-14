//! State → colour, ported from Styles.cs / floatdash.swift so all surfaces read the same.
//! The widget's own colours live in ui/index.html; this maps states for the tray badge.

pub fn rgb_for(state: &str) -> (u8, u8, u8) {
    match state {
        "needs" => (0xFF, 0x45, 0x3A),    // systemRed
        "yourturn" => (0xFF, 0xD6, 0x0A), // systemYellow
        "working" => (0x34, 0xC7, 0x59),  // systemGreen
        "done" => (0x8E, 0x8E, 0x93),     // systemGray
        _ => (0x5A, 0x5A, 0x5E),          // idle / faint
    }
}

pub fn label_for(state: &str) -> &'static str {
    match state {
        "needs" => "Needs you",
        "yourturn" => "Your turn",
        "working" => "Working",
        "done" => "Done",
        _ => "Idle",
    }
}
