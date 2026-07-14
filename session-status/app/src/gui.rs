//! The always-on-top floating dashboard (egui/eframe port of floatdash.swift / MainWindow.cs).
//! Lists live Claude sessions; left-click a row to jump to its terminal. Hides to the tray on
//! close; Quit lives in the tray menu.

use crate::{install, model::{self, Sess}, platform, styles, tray::{Tray, TrayAction}};
use eframe::egui;
use egui::{Align2, Color32, FontId, Pos2, Rect, Rounding, Sense, Stroke, Vec2, ViewportCommand};
use std::time::{Duration, Instant};

const W: f32 = 300.0;
const H: f32 = 440.0;

pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Claude sessions")
            .with_inner_size([W, H])
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(false),
        ..Default::default()
    };
    eframe::run_native(
        "Claude sessions",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

struct App {
    sessions: Vec<Sess>,
    last_refresh: Instant,
    tray: Option<Tray>,
    visible: bool,
    quitting: bool,
    positioned: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // First-run: wire the Claude Code hooks so simply launching the exe is enough.
        if !install::already_installed() {
            let _ = install::run(true);
        }
        // Heartbeat thread: wake the UI ~2×/sec so the tray badge and dead-session cleanup keep
        // ticking even while the window is hidden to the tray (a hidden window gets no repaints).
        {
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_millis(400));
                ctx.request_repaint();
            });
        }
        let mut app = App {
            sessions: Vec::new(),
            last_refresh: Instant::now() - Duration::from_secs(10),
            tray: Tray::new(),
            visible: true,
            quitting: false,
            positioned: false,
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        self.sessions = model::load();
        self.last_refresh = Instant::now();
        if let Some(t) = &mut self.tray {
            let (top, count) = top_state(&self.sessions);
            t.update(top, count, &tooltip(&self.sessions));
        }
    }

    fn toggle(&mut self, ctx: &egui::Context) {
        self.visible = !self.visible;
        ctx.send_viewport_cmd(ViewportCommand::Visible(self.visible));
        if self.visible {
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
    }

    fn header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, full: f32) {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(full, 30.0), Sense::click_and_drag());
        if resp.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        let x_center = Pos2::new(rect.right() - 16.0, rect.center().y);
        let x_rect = Rect::from_center_size(x_center, Vec2::splat(20.0));
        let x_resp = ui.interact(x_rect, ui.id().with("hide-to-tray"), Sense::click());
        if x_resp.clicked() {
            self.visible = false;
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
        let p = ui.painter();
        p.text(rect.center(), Align2::CENTER_CENTER, "Claude sessions",
               FontId::proportional(11.5), styles::SECONDARY);
        let x_col = if x_resp.hovered() { styles::LABEL } else { styles::SECONDARY };
        p.text(x_center, Align2::CENTER_CENTER, "×", FontId::proportional(16.0), x_col);
    }
}

impl eframe::App for App {
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0] // transparent window; the rounded panel paints the background
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Pin to the top-right on the first frame, using the monitor size.
        if !self.positioned {
            if let Some(msize) = ctx.input(|i| i.viewport().monitor_size) {
                let x = (msize.x - W - 20.0).max(0.0);
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(Pos2::new(x, 40.0)));
            }
            self.positioned = true;
        }

        if let Some(t) = &self.tray {
            match t.poll() {
                Some(TrayAction::Quit) => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                Some(TrayAction::Toggle) => self.toggle(ctx),
                None => {}
            }
        }

        // Closing (× / Alt-F4) hides to the tray instead of quitting the app.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.visible = false;
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }

        if self.last_refresh.elapsed() >= Duration::from_millis(1500) {
            self.refresh();
        }

        let frame = egui::Frame::none()
            .fill(styles::BG)
            .rounding(Rounding::same(10.0))
            .stroke(Stroke::new(1.0, Color32::from_rgba_premultiplied(0xFF, 0xFF, 0xFF, 0x2A)))
            .inner_margin(egui::Margin::same(0.0));

        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let full = ui.available_width();
            self.header(ui, ctx, full);
            ui.add_space(2.0);

            egui::ScrollArea::vertical()
                .max_height(H - 96.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if self.sessions.is_empty() {
                        ui.add_space(18.0);
                        ui.vertical_centered(|ui| {
                            ui.label(egui::RichText::new("No active sessions")
                                .size(12.0).color(styles::TERTIARY));
                        });
                        ui.add_space(18.0);
                    } else {
                        for s in self.sessions.clone() {
                            row(ui, &s, full);
                        }
                    }
                });

            ui.add_space(4.0);
            separator(ui, full);
            ui.add_space(6.0);
            footer(ui, &self.sessions, full);
            ui.add_space(8.0);
        });

        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

fn row(ui: &mut egui::Ui, s: &Sess, full: f32) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(full, 26.0), Sense::click());
    if resp.clicked() {
        platform::jump(&s.terminal, s.term_pid, s.pid);
    }
    let dim = s.muted(model::now());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect.shrink2(Vec2::new(6.0, 1.0)), Rounding::same(6.0), styles::HOVER);
    }
    let dot = styles::color_for(&s.state);
    p.circle_filled(Pos2::new(rect.left() + 16.0, rect.center().y), 5.0, dot);
    let name_col = if dim { styles::TERTIARY } else { styles::LABEL };
    p.text(Pos2::new(rect.left() + 30.0, rect.center().y), Align2::LEFT_CENTER,
           truncate(&s.topic, 30), FontId::proportional(13.0), name_col);
    let age = model::age_str(s.updated);
    if !age.is_empty() {
        p.text(Pos2::new(rect.right() - 12.0, rect.center().y), Align2::RIGHT_CENTER,
               age, FontId::proportional(11.0), styles::TERTIARY);
    }
}

fn separator(ui: &mut egui::Ui, full: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(full, 1.0), Sense::hover());
    let line = Rect::from_min_max(
        Pos2::new(rect.left() + 14.0, rect.center().y),
        Pos2::new(rect.right() - 14.0, rect.center().y + 1.0),
    );
    ui.painter().rect_filled(line, Rounding::ZERO, styles::HAIRLINE);
}

fn footer(ui: &mut egui::Ui, sessions: &[Sess], full: f32) {
    const KEYS: [&str; 4] = ["needs", "yourturn", "working", "done"];
    const CHIP_W: f32 = 52.0;
    let total = CHIP_W * KEYS.len() as f32;
    ui.horizontal(|ui| {
        ui.add_space(((full - total) / 2.0).max(0.0));
        for key in KEYS {
            let n = sessions.iter().filter(|s| s.state == key).count();
            chip(ui, key, n);
        }
    });
}

fn chip(ui: &mut egui::Ui, key: &str, n: usize) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(52.0, 18.0), Sense::hover());
    let active = n > 0;
    let col = if active { styles::color_for(key) } else { styles::TERTIARY };
    let p = ui.painter();
    p.circle_filled(Pos2::new(rect.left() + 8.0, rect.center().y), 5.5, col);
    let tc = if active { styles::LABEL } else { styles::TERTIARY };
    p.text(Pos2::new(rect.left() + 19.0, rect.center().y), Align2::LEFT_CENTER,
           n.to_string(), FontId::proportional(11.5), tc);
}

// ---- helpers ----

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let cut: String = s.chars().take(n - 1).collect();
    format!("{}…", cut.trim_end())
}

fn top_state(sessions: &[Sess]) -> (&'static str, i32) {
    let now = model::now();
    let active: Vec<&Sess> = sessions.iter().filter(|s| s.mute_until <= now).collect();
    let count = |st: &str| active.iter().filter(|s| s.state == st).count() as i32;
    let (needs, yt, wk) = (count("needs"), count("yourturn"), count("working"));
    if needs > 0 {
        ("needs", needs)
    } else if yt > 0 {
        ("yourturn", yt)
    } else if wk > 0 {
        ("working", wk)
    } else {
        ("idle", 0)
    }
}

fn tooltip(sessions: &[Sess]) -> String {
    let count = |st: &str| sessions.iter().filter(|s| s.state == st).count();
    let mut parts = Vec::new();
    let (needs, yt, wk) = (count("needs"), count("yourturn"), count("working"));
    if needs > 0 {
        parts.push(format!("{needs} need you"));
    }
    if yt > 0 {
        parts.push(format!("{yt} your turn"));
    }
    if wk > 0 {
        parts.push(format!("{wk} working"));
    }
    if parts.is_empty() {
        "Claude sessions — idle".into()
    } else {
        format!("Claude — {}", parts.join(", "))
    }
}
