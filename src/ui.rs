// egui window: Presence tab (card, rotation chips) and Settings tab. Closing hides to the tray.
use crate::config::{ALL_PAGES, DEFAULT_IMAGE};
use crate::engine::{Engine, Snapshot};
use crate::shell;
use eframe::egui::{self, vec2, Align, Color32, FontFamily, FontId, Layout, RichText, Sense, Stroke, Ui};
use std::sync::Arc;

const BG: Color32 = Color32::from_rgb(0x1E, 0x1F, 0x22);
const CARD: Color32 = Color32::from_rgb(0x23, 0x24, 0x28);
const PANEL: Color32 = Color32::from_rgb(0x2B, 0x2D, 0x31);
const MUTED: Color32 = Color32::from_rgb(0xB5, 0xBA, 0xC1);
const DIM: Color32 = Color32::from_rgb(0x80, 0x84, 0x8E);
const TEXT: Color32 = Color32::from_rgb(0xF2, 0xF3, 0xF5);
const SOFT: Color32 = Color32::from_rgb(0xDB, 0xDE, 0xE1);
const ACCENT: Color32 = Color32::from_rgb(0x58, 0x65, 0xF2);
const GREEN: Color32 = Color32::from_rgb(0x23, 0xA5, 0x5A);
const YELLOW: Color32 = Color32::from_rgb(0xF0, 0xB2, 0x32);
const RED: Color32 = Color32::from_rgb(0xF2, 0x3F, 0x43);
const LOGO: egui::ImageSource = egui::include_image!("../t3-logo-1024.png");
const TYPES: [(i64, &str); 4] = [(3, "Watching"), (0, "Playing"), (2, "Listening"), (5, "Competing")];

fn white(a: u8) -> Color32 { Color32::from_rgba_unmultiplied(255, 255, 255, a) }
fn page_title(n: &str) -> &'static str {
    match n { "activity" => "Activity", "agents" => "Agents", "tokens" => "Tokens", "value" => "Value", _ => "Plan" }
}
fn semi(t: impl Into<String>, size: f32) -> RichText { RichText::new(t).family(FontFamily::Name("semibold".into())).size(size) }

pub struct App {
    eng: Arc<Engine>,
    settings: bool,
    cover_text: String,
    cover_project: Option<Option<String>>,
}

pub fn setup_fonts(ctx: &egui::Context) {
    let mut f = egui::FontDefinitions::default();
    let dir = std::path::PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join("Fonts");
    if let Ok(b) = std::fs::read(dir.join("segoeui.ttf")) {
        f.font_data.insert("segoe".into(), egui::FontData::from_owned(b));
        f.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "segoe".into());
    }
    let semi = if let Ok(b) = std::fs::read(dir.join("seguisb.ttf")) { f.font_data.insert("segoe-sb".into(), egui::FontData::from_owned(b)); "segoe-sb" } else { "segoe" };
    let mut fam = vec![semi.to_string()];
    fam.extend(f.families[&FontFamily::Proportional].iter().cloned());
    f.families.insert(FontFamily::Name("semibold".into()), fam);
    ctx.set_fonts(f);
    let mut st = (*ctx.style()).clone();
    st.visuals = egui::Visuals::dark();
    st.visuals.panel_fill = BG;
    st.visuals.window_fill = BG;
    st.visuals.override_text_color = Some(TEXT);
    st.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, white(20));
    st.visuals.extreme_bg_color = BG;
    st.visuals.selection.bg_fill = ACCENT;
    st.visuals.widgets.hovered.bg_stroke = Stroke::NONE;
    st.visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    st.text_styles.insert(egui::TextStyle::Body, FontId::proportional(13.0));
    st.text_styles.insert(egui::TextStyle::Button, FontId::proportional(13.0));
    ctx.set_style(st);
}

impl App {
    pub fn new(eng: Arc<Engine>, settings: bool) -> Self {
        App { eng, settings, cover_text: String::new(), cover_project: None }
    }
    fn saved(&self) { self.eng.cfg.lock().unwrap().save(); self.eng.poke(); }

    fn header(&mut self, ui: &mut Ui, snap: &Snapshot) {
        ui.horizontal(|ui| {
            egui::Frame::none().fill(Color32::from_black_alpha(38)).rounding(9.0).inner_margin(2.0).show(ui, |ui| {
                ui.spacing_mut().button_padding = vec2(16.0, 6.0);
                ui.spacing_mut().item_spacing.x = 2.0;
                for (label, s) in [("Presence", false), ("Settings", true)] {
                    let on = self.settings == s;
                    let t = if on { semi(label, 13.0).color(Color32::WHITE) } else { RichText::new(label).size(13.0).color(MUTED) };
                    if ui.add(egui::Button::new(t).fill(if on { white(64) } else { Color32::TRANSPARENT }).rounding(7.0)).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { self.settings = s; }
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (dot, txt) = if snap.paused { (YELLOW, "Paused".to_string()) }
                    else if snap.conn == "connected" { (GREEN, snap.user.as_ref().map_or("Connected".into(), |u| format!("Connected as {u}"))) }
                    else if snap.conn == "connecting" { (YELLOW, "Connecting".into()) }
                    else { (DIM, "Discord not running, retrying".into()) };
                egui::Frame::none().fill(white(38)).rounding(11.0).inner_margin(egui::Margin::symmetric(10.0, 4.0)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // right-to-left here: the label goes in first so the dot lands on its left
                        ui.label(RichText::new(txt).size(12.0).color(SOFT));
                        let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.0, dot);
                    });
                });
            });
        });
    }

    fn cover(&self, ui: &mut Ui, snap: &Snapshot) {
        let (rect, _) = ui.allocate_exact_size(vec2(104.0, 104.0), Sense::hover());
        let img = egui::Rect::from_min_size(rect.min, vec2(96.0, 96.0));
        ui.painter().rect_filled(img, 10.0, Color32::BLACK);
        let url = snap.image.as_deref().filter(|u| *u != DEFAULT_IMAGE && u.to_ascii_lowercase().starts_with("https://"));
        let ok = url.map_or(false, |u| ui.ctx().try_load_texture(u, Default::default(), egui::SizeHint::Size(192, 192)).is_ok());
        let src = match url { Some(u) if ok => egui::ImageSource::Uri(u.to_string().into()), _ => LOGO };
        egui::Image::new(src).rounding(10.0).paint_at(ui, img);
        if let Some(o) = snap.overlay.as_deref() {
            let c = rect.max - vec2(15.0, 15.0);
            ui.painter().circle_filled(c, 15.0, CARD);
            ui.painter().circle_filled(c, 10.0, match o { "limit" => RED, "working" => GREEN, _ => DIM });
        }
    }

    fn presence(&mut self, ui: &mut Ui, snap: &Snapshot) {
        egui::Frame::none().fill(CARD).rounding(14.0).inner_margin(egui::Margin { left: 20.0, right: 20.0, top: 18.0, bottom: 20.0 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(semi(&snap.type_word, 12.0).color(MUTED));
            ui.add_space(12.0);
            ui.horizontal_top(|ui| {
                self.cover(ui, snap);
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.add_space(4.0);
                    ui.label(semi("T3 Code", 17.0).color(Color32::WHITE));
                    let (details, state) = if snap.page_name.is_none() {
                        (Some(snap.none_reason.clone().unwrap_or_else(|| "Nothing to show".into())), None)
                    } else { (snap.details.clone(), snap.state.clone()) };
                    if let Some(d) = details.filter(|x| !x.is_empty()) { ui.add_space(3.0); ui.add(egui::Label::new(RichText::new(d).size(14.0).color(SOFT)).wrap()); }
                    if let Some(s) = state.filter(|x| !x.is_empty()) { ui.add(egui::Label::new(RichText::new(s).size(14.0).color(MUTED)).wrap()); }
                    if let (Some(start), Some(_)) = (snap.start, &snap.page_name) {
                        let t = (chrono::Local::now() - start).num_seconds().max(0);
                        let txt = if t >= 3600 { format!("{}:{:02}:{:02}", t / 3600, t % 3600 / 60, t % 60) } else { format!("{:02}:{:02}", t / 60, t % 60) };
                        ui.add_space(5.0);
                        ui.label(semi(txt, 13.0).color(GREEN));
                    }
                });
            });
            ui.add_space(14.0);
            ui.separator();
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("On image hover").size(11.5).color(DIM));
                ui.add_space(6.0);
                ui.add(egui::Label::new(RichText::new(snap.hover.clone().unwrap_or_default()).size(11.5).color(MUTED)).truncate());
            });
        });
        ui.add_space(16.0);
        ui.label(semi("ROTATION", 11.0).color(DIM));
        ui.add_space(6.0);
        let pages = if snap.pages.is_empty() { self.eng.cfg().pages } else { snap.pages.clone() };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().button_padding = vec2(11.0, 4.0);
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for n in pages {
                let on = snap.page_name.as_deref() == Some(n.as_str());
                let b = egui::Button::new(RichText::new(page_title(&n)).size(12.0).color(if on { Color32::WHITE } else { MUTED })).fill(if on { ACCENT } else { white(38) }).rounding(12.0);
                if ui.add(b).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Show this page now").clicked() { self.eng.jump_to(&n); }
            }
        });
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(&snap.coverage).size(12.0).color(DIM)).wrap());
    }

    fn section(ui: &mut Ui, t: &str) { ui.add_space(18.0); ui.label(semi(t, 11.0).color(DIM)); ui.add_space(8.0); }
    fn panel(ui: &mut Ui, f: impl FnOnce(&mut Ui)) {
        egui::Frame::none().fill(PANEL).rounding(10.0).inner_margin(egui::Margin::symmetric(16.0, 4.0)).show(ui, |ui| { ui.set_width(ui.available_width()); f(ui); });
    }
    fn row(ui: &mut Ui, label: &str, note: Option<&str>, ctrl_w: f32, control: impl FnOnce(&mut Ui)) {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let text_w = (ui.available_width() - ctrl_w - 40.0).max(80.0);
            ui.allocate_ui_with_layout(vec2(text_w, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(text_w);
                ui.label(RichText::new(label).size(13.5));
                if let Some(n) = note { ui.add(egui::Label::new(RichText::new(n).size(11.5).color(DIM)).wrap()); }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), control);
        });
        ui.add_space(10.0);
    }
    fn toggle(ui: &mut Ui, on: &mut bool) -> bool {
        let (rect, resp) = ui.allocate_exact_size(vec2(38.0, 22.0), Sense::click());
        let clicked = resp.clicked();
        if clicked { *on = !*on; }
        let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.14);
        let off = Color32::from_rgb(0x4E, 0x50, 0x58);
        let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
        let bg = Color32::from_rgb(lerp(off.r(), ACCENT.r()), lerp(off.g(), ACCENT.g()), lerp(off.b(), ACCENT.b()));
        ui.painter().rect_filled(rect, 11.0, bg);
        let x = egui::lerp((rect.left() + 11.0)..=(rect.right() - 11.0), t);
        ui.painter().circle_filled(egui::pos2(x, rect.center().y), 8.0, Color32::WHITE);
        resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        clicked
    }
    fn stepper(ui: &mut Ui, v: i64, step: i64, lo: i64, hi: i64) -> Option<i64> {
        let mut out = None;
        ui.spacing_mut().item_spacing.x = 0.0;
        let b = |t: &str| egui::Button::new(RichText::new(t).size(15.0)).fill(white(51)).rounding(7.0).min_size(vec2(28.0, 28.0));
        if ui.add(b("+")).clicked() { out = Some((v + step).clamp(lo, hi)); }
        ui.allocate_ui_with_layout(vec2(40.0, 28.0), Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| ui.label(RichText::new(v.to_string()).size(14.0)));
        if ui.add(b("\u{2212}")).clicked() { out = Some((v - step).clamp(lo, hi)); }
        out
    }

    fn settings(&mut self, ui: &mut Ui) {
        let eng = self.eng.clone();
        let mut c = eng.cfg();
        let mut changed = false;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width() - 12.0);
            Self::section(ui, "DISPLAY");
            Self::panel(ui, |ui| {
                Self::row(ui, "Activity type", None, 150.0, |ui| {
                    let cur = TYPES.iter().find(|t| t.0 == c.activity_type).map_or("Watching", |t| t.1);
                    egui::ComboBox::from_id_salt("type").selected_text(cur).width(150.0).show_ui(ui, |ui| {
                        for (v, n) in TYPES { if ui.selectable_label(c.activity_type == v, n).clicked() && c.activity_type != v { c.activity_type = v; changed = true; } }
                    });
                });
                ui.separator();
                Self::row(ui, "Live line", Some("What the agent is doing right now, by kind only: Editing a file, Running a command. Never paths or commands."), 38.0, |ui| changed |= Self::toggle(ui, &mut c.live_line));
                ui.separator();
                Self::row(ui, "Show project name", None, 38.0, |ui| changed |= Self::toggle(ui, &mut c.show_project));
            });
            Self::section(ui, "PAGES");
            Self::panel(ui, |ui| {
                let notes = [("Activity", "Project and live line"), ("Agents", "Running count, then each model and effort"), ("Tokens", "Input, output, cache read and write today"),
                    ("Token value", "Dollar value at API list prices, not a bill"), ("Plan usage", "5 hour and weekly limits")];
                for (i, name) in ALL_PAGES.iter().enumerate() {
                    if i > 0 { ui.separator(); }
                    let mut on = c.pages.iter().any(|p| p == name);
                    Self::row(ui, notes[i].0, Some(notes[i].1), 38.0, |ui| {
                        if Self::toggle(ui, &mut on) {
                            c.pages = ALL_PAGES.iter().filter(|p| if **p == *name { on } else { c.pages.iter().any(|x| x == *p) }).map(|p| p.to_string()).collect();
                            changed = true;
                        }
                    });
                }
            });
            Self::section(ui, "TIMING");
            Self::panel(ui, |ui| {
                Self::row(ui, "Seconds per page", Some("15 is the floor, Discord's rate limit"), 96.0, |ui| {
                    if let Some(v) = Self::stepper(ui, c.page_seconds, 5, 15, 300) { c.page_seconds = v; changed = true; }
                });
                ui.separator();
                Self::row(ui, "Idle minutes", Some("Clear presence after this long with no agents"), 96.0, |ui| {
                    if let Some(v) = Self::stepper(ui, c.idle_minutes, 5, 5, 240) { c.idle_minutes = v; changed = true; }
                });
            });
            Self::section(ui, "COVER IMAGE");
            Self::panel(ui, |ui| {
                ui.add_space(10.0);
                let proj = self.cover_project.clone().flatten();
                ui.label(RichText::new(proj.as_ref().map_or("Default cover".into(), |p| format!("Cover for {p}"))).size(13.5));
                ui.add_space(6.0);
                let r = egui::Frame::none().fill(BG).rounding(7.0).inner_margin(egui::Margin::symmetric(9.0, 7.0)).show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.cover_text).frame(false).desired_width(f32::INFINITY))
                }).inner;
                if r.lost_focus() {
                    let v = self.cover_text.trim().to_string();
                    match &proj {
                        None => c.large_image_key = if v.is_empty() { DEFAULT_IMAGE.into() } else { v },
                        Some(p) => { if v.is_empty() { c.project_images.remove(p); } else { c.project_images.insert(p.clone(), v); } }
                    }
                    changed = true;
                }
                ui.add_space(6.0);
                ui.add(egui::Label::new(RichText::new("Public https image URL or a Discord art asset name. Empty uses the T3 logo. Applies to the project shown here.").size(11.5).color(DIM)).wrap());
                ui.add_space(8.0);
                ui.spacing_mut().button_padding = vec2(14.0, 7.0);
                if ui.add(egui::Button::new("Open config.json").fill(white(51)).rounding(7.0)).clicked() {
                    let _ = std::process::Command::new("notepad.exe").arg(crate::config::Config::file_path()).spawn();
                }
                ui.add_space(10.0);
            });
            Self::section(ui, "APP");
            Self::panel(ui, |ui| {
                Self::row(ui, "Start with Windows", None, 38.0, |ui| {
                    let mut on = shell::autostart_enabled();
                    if Self::toggle(ui, &mut on) { shell::set_autostart(on); }
                });
                if eng.has_remotes() {
                    ui.separator();
                    Self::row(ui, "All environments", Some("Add linked machines to agents, tokens and value. Off counts this PC only."), 38.0, |ui| {
                        if Self::toggle(ui, &mut c.include_remotes) { changed = true; }
                    });
                }
            });
            ui.add_space(6.0);
        });
        if changed {
            let old = eng.cfg();
            *eng.cfg.lock().unwrap() = c.clone();
            self.saved();
            if old.include_remotes != c.include_remotes { eng.sync_remotes(); }
            if old.page_seconds != c.page_seconds { eng.reschedule(); }
        }
    }

    fn track_cover(&mut self, ctx: &egui::Context, snap: &Snapshot) {
        if self.cover_project.as_ref() == Some(&snap.project) || ctx.memory(|m| m.focused().is_some()) { return; }
        self.cover_project = Some(snap.project.clone());
        let c = self.eng.cfg();
        self.cover_text = match &snap.project {
            Some(p) => c.project_images.get(p).cloned().unwrap_or_default(),
            None => if c.large_image_key == DEFAULT_IMAGE { String::new() } else { c.large_image_key.clone() },
        };
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            shell::hide_main();
        }
        let snap = self.eng.snapshot.lock().unwrap().clone().unwrap_or_else(|| Snapshot { conn: "starting".into(), type_word: "Watching".into(), ..Default::default() });
        self.track_cover(ctx, &snap);
        egui::TopBottomPanel::top("head").frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin { left: 22.0, right: 22.0, top: 14.0, bottom: 0.0 })).show_separator_line(false).show(ctx, |ui| self.header(ui, &snap));
        if !self.settings {
            egui::TopBottomPanel::bottom("foot").frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin { left: 22.0, right: 22.0, top: 8.0, bottom: 18.0 })).show_separator_line(false).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().button_padding = vec2(14.0, 7.0);
                    let p = self.eng.is_paused();
                    if ui.add(egui::Button::new(if p { "Resume presence" } else { "Pause presence" }).fill(white(51)).rounding(7.0)).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { self.eng.set_paused(!p); }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(RichText::new("Closing keeps it running in the tray").size(11.5).color(DIM)));
                });
            });
        }
        egui::CentralPanel::default().frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin { left: 22.0, right: 22.0, top: 16.0, bottom: 4.0 })).show(ctx, |ui| {
            if self.settings { self.settings(ui); } else { self.presence(ui, &snap); }
        });
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }
}
