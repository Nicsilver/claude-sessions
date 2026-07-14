//! State → colour, ported from Styles.cs / floatdash.swift so all surfaces read the same.

use egui::Color32;

pub const RED: Color32 = Color32::from_rgb(0xFF, 0x45, 0x3A); // needs
pub const YELLOW: Color32 = Color32::from_rgb(0xFF, 0xD6, 0x0A); // your turn
pub const GREEN: Color32 = Color32::from_rgb(0x34, 0xC7, 0x59); // working
pub const GRAY: Color32 = Color32::from_rgb(0x8E, 0x8E, 0x93); // done
pub const FAINT: Color32 = Color32::from_rgb(0x5A, 0x5A, 0x5E); // idle

pub const BG: Color32 = Color32::from_rgb(26, 26, 26);
pub const LABEL: Color32 = Color32::from_rgb(0xEC, 0xEC, 0xEC);
pub const SECONDARY: Color32 = Color32::from_rgb(0x98, 0x98, 0x9E);
pub const TERTIARY: Color32 = Color32::from_rgb(0x6A, 0x6A, 0x6E);
pub const HAIRLINE: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
pub const HOVER: Color32 = Color32::from_rgba_premultiplied(0xFF, 0xFF, 0xFF, 0x14);

pub fn color_for(state: &str) -> Color32 {
    match state {
        "needs" => RED,
        "yourturn" => YELLOW,
        "working" => GREEN,
        "done" => GRAY,
        _ => FAINT,
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

/// (r, g, b) for the tray badge, used to build the RGBA icon.
pub fn rgb_for(state: &str) -> (u8, u8, u8) {
    let c = color_for(state);
    (c.r(), c.g(), c.b())
}
