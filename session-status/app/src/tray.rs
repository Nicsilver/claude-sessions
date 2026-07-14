//! System-tray icon: a state-coloured badge plus a small menu. The dynamic in-icon count number
//! (as the WinForms build drew) is a follow-up; for now the badge shows the top-priority state
//! colour and the dashboard shows counts.

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub enum TrayAction {
    Toggle,
    Quit,
}

pub struct Tray {
    icon: TrayIcon,
    show_id: MenuId,
    quit_id: MenuId,
    sig: String,
}

impl Tray {
    pub fn new() -> Option<Self> {
        let show = MenuItem::new("Show / hide dashboard", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        menu.append(&show).ok()?;
        menu.append(&PredefinedMenuItem::separator()).ok()?;
        menu.append(&quit).ok()?;

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Claude sessions")
            .with_icon(make_icon("idle", 0))
            .build()
            .ok()?;

        Some(Self {
            icon: tray,
            show_id: show.id().clone(),
            quit_id: quit.id().clone(),
            sig: String::new(),
        })
    }

    pub fn update(&mut self, top: &str, count: i32, tooltip: &str) {
        let sig = format!("{top}:{count}");
        if sig != self.sig {
            self.sig = sig;
            let _ = self.icon.set_icon(Some(make_icon(top, count)));
        }
        let _ = self.icon.set_tooltip(Some(tooltip));
    }

    /// Drain any pending tray/menu event into an action.
    pub fn poll(&self) -> Option<TrayAction> {
        if let Ok(ev) = MenuEvent::receiver().try_recv() {
            if ev.id == self.quit_id {
                return Some(TrayAction::Quit);
            }
            if ev.id == self.show_id {
                return Some(TrayAction::Toggle);
            }
        }
        if let Ok(TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }) =
            TrayIconEvent::receiver().try_recv()
        {
            return Some(TrayAction::Toggle);
        }
        None
    }
}

/// A 32×32 RGBA badge: a filled disc in the state colour (small + dim when nothing is active).
fn make_icon(state: &str, count: i32) -> Icon {
    const W: u32 = 32;
    const H: u32 = 32;
    let (r, g, b) = crate::styles::rgb_for(state);
    let alpha_scale = if count == 0 { 0.6 } else { 1.0 };
    let rad = if count == 0 { 6.0 } else { 13.0 };
    let (cx, cy) = (16.0f32, 16.0f32);
    let mut rgba = vec![0u8; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            let cover = if d <= rad {
                1.0
            } else if d <= rad + 1.0 {
                rad + 1.0 - d
            } else {
                0.0
            };
            let a = (255.0 * cover * alpha_scale) as u8;
            let i = ((y * W + x) * 4) as usize;
            rgba[i] = r;
            rgba[i + 1] = g;
            rgba[i + 2] = b;
            rgba[i + 3] = a;
        }
    }
    Icon::from_rgba(rgba, W, H).expect("valid rgba icon")
}
