//! The application: sidebar, header, message list, search/pins panel and lightbox.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use egui::{
    Align, Align2, CentralPanel, Color32, CursorIcon, FontId, Frame, Key, Label, Layout, Margin, Panel, Rect, RichText,
    ScrollArea, Sense, Stroke, TextEdit, Ui, pos2, vec2,
};

use crate::lightbox::Lightbox;
use crate::media::MediaCache;
use crate::model::{Archive, ChannelKind};
use crate::player::AudioOut;
use crate::render::{self, Action, Env, Players};
use crate::search;
use crate::theme::*;
use crate::view::ChannelView;

const PAGE: usize = 25;

#[derive(PartialEq)]
enum PanelMode {
    Search,
    Pins,
}

struct Results {
    mode: PanelMode,
    hits: Vec<(u32, u32)>,
    page: usize,
    oldest_first: bool,
    errors: Vec<String>,
    took_ms: f64,
}

pub struct App {
    loader: Option<Receiver<Result<Archive, String>>>,
    status: Arc<Mutex<String>>,
    error: Option<String>,
    archive: Option<Arc<Archive>>,
    media: MediaCache,
    players: Players,
    audio: AudioOut,
    current: Option<usize>,
    views: HashMap<usize, ChannelView>,
    revealed: HashSet<u64>,
    collapsed: HashSet<u64>,
    search_text: String,
    search_ran: String,
    search_edited: Option<Instant>,
    focus_search: bool,
    results: Option<Results>,
    lightbox: Option<Lightbox>,
    highlight: Option<(u64, f64)>,
    script: Option<crate::script::Script>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, data_dir: PathBuf, media_dir: Option<PathBuf>) -> Self {
        apply(&cc.egui_ctx);
        let status = Arc::new(Mutex::new(format!("Loading {}…", data_dir.display())));
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = cc.egui_ctx.clone();
        let st = status.clone();
        std::thread::spawn(move || {
            let progress = |s: String| {
                *st.lock().unwrap() = s;
                ctx.request_repaint();
            };
            let started = Instant::now();
            let res = crate::model::load(&data_dir, media_dir.as_deref(), &progress);
            if let Ok(a) = &res {
                eprintln!(
                    "Loaded {} messages in {} channels from {} in {:.1}s",
                    a.total_messages,
                    a.channels.len(),
                    a.data_dir.display(),
                    started.elapsed().as_secs_f32()
                );
            }
            let _ = tx.send(res);
            ctx.request_repaint();
        });
        Self {
            loader: Some(rx),
            status,
            error: None,
            archive: None,
            media: MediaCache::new(cc.egui_ctx.clone()),
            players: Players::default(),
            audio: AudioOut::default(),
            current: None,
            views: HashMap::new(),
            revealed: HashSet::new(),
            collapsed: HashSet::new(),
            search_text: String::new(),
            search_ran: String::new(),
            search_edited: None,
            focus_search: false,
            results: None,
            lightbox: None,
            highlight: None,
            script: crate::script::Script::from_env(),
        }
    }

    fn open_channel(&mut self, a: &Archive, ch: usize) {
        if self.current == Some(ch) {
            return;
        }
        self.current = Some(ch);
        self.views.entry(ch).or_insert_with(|| ChannelView::new(a, ch));
        if self.results.as_ref().is_some_and(|r| r.mode == PanelMode::Pins) {
            self.show_pins(a);
        }
    }

    fn show_pins(&mut self, a: &Archive) {
        let Some(ch) = self.current else { return };
        let hits = a.channels[ch]
            .messages
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, m)| m.pinned)
            .map(|(i, _)| (ch as u32, i as u32))
            .collect();
        self.results =
            Some(Results { mode: PanelMode::Pins, hits, page: 0, oldest_first: false, errors: Vec::new(), took_ms: 0.0 });
    }

    fn run_search(&mut self, a: &Archive) {
        self.search_ran = self.search_text.clone();
        self.search_edited = None;
        let q = search::parse(&self.search_text);
        if q.is_empty() && q.errors.is_empty() {
            if self.results.as_ref().is_some_and(|r| r.mode == PanelMode::Search) {
                self.results = None;
            }
            return;
        }
        let t = Instant::now();
        let hits = search::run(a, &q);
        let oldest_first = self.results.as_ref().is_some_and(|r| r.mode == PanelMode::Search && r.oldest_first);
        let mut r = Results {
            mode: PanelMode::Search,
            hits,
            page: 0,
            oldest_first,
            errors: q.errors,
            took_ms: t.elapsed().as_secs_f64() * 1000.0,
        };
        if oldest_first {
            r.hits.reverse();
        }
        self.results = Some(r);
    }

    fn jump_to(&mut self, a: &Archive, id: u64, now: f64) {
        let Some(&(c, i)) = a.by_id.get(&id) else { return };
        self.open_channel(a, c as usize);
        if let Some(v) = self.views.get_mut(&(c as usize)) {
            v.jump_to_message(i as usize);
        }
        self.highlight = Some((id, now));
    }

    fn handle(&mut self, a: &Archive, actions: Vec<Action>, now: f64) {
        for act in actions {
            match act {
                Action::JumpTo(id) => self.jump_to(a, id, now),
                Action::OpenChannel(c) => self.open_channel(a, c),
                Action::Lightbox { ch, msg, path } => {
                    self.players.pause_all(&mut self.audio);
                    self.lightbox = Lightbox::open(a, &mut self.media, ch, msg, &path);
                    if self.lightbox.is_none() {
                        open_safely(&path);
                    }
                }
                Action::Open(s) => open_safely(&s),
                Action::Reveal(s) => reveal(&s),
            }
        }
    }

    fn run_script(&mut self, ctx: &egui::Context, a: &Archive, actions: &mut Vec<Action>) {
        use crate::script::Cmd;
        let Some(script) = &mut self.script else { return };
        let Some(cmd) = script.next(ctx) else { return };
        match cmd {
            Cmd::Channel(name) => match a.channels.iter().position(|c| c.name == name) {
                Some(c) => actions.push(Action::OpenChannel(c)),
                None => eprintln!("script: no channel {name}"),
            },
            Cmd::At(f) => {
                if let Some(ch) = self.current
                    && let Some(v) = self.views.get_mut(&ch)
                {
                    let n = a.channels[ch].messages.len();
                    v.jump_to_message(((n as f32 * f) as usize).min(n.saturating_sub(1)));
                }
            }
            Cmd::Jump(id) => actions.push(Action::JumpTo(id)),
            Cmd::Search(q) => {
                self.search_text = q;
                self.run_search(a);
            }
            Cmd::Pins => self.show_pins(a),
            Cmd::Lightbox { n, video } => {
                if let Some(ch) = self.current {
                    self.lightbox = Lightbox::open_nth(a, &mut self.media, ch, n, video);
                }
            }
            Cmd::LightboxNext => {
                if let Some(lb) = &mut self.lightbox {
                    lb.next();
                }
            }
            Cmd::CloseLightbox => self.lightbox = None,
            Cmd::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    fn loading_screen(&mut self, ui: &mut Ui) {
        CentralPanel::default().frame(Frame::new().fill(BG_MAIN)).show(ui, |ui| {
            ui.centered_and_justified(|ui| {
                if let Some(e) = &self.error {
                    ui.label(RichText::new(format!("Couldn't load the export.\n\n{e}")).color(TEXT).size(16.0));
                } else {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() / 2.0 - 40.0);
                        ui.add(egui::Spinner::new().size(32.0).color(BLURPLE));
                        ui.add_space(12.0);
                        ui.label(RichText::new(self.status.lock().unwrap().clone()).color(TEXT_MUTED));
                    });
                }
            });
        });
    }

    fn sidebar(&mut self, ui: &mut Ui, a: &Archive, actions: &mut Vec<Action>) {
        // Server name
        let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::hover());
        ui.painter().hline(head.x_range(), head.bottom(), Stroke::new(1.0, BORDER));
        let mut x = head.left() + 16.0;
        if let Some(icon) = &a.guild_icon {
            let r = Rect::from_center_size(pos2(x + 12.0, head.center().y), vec2(24.0, 24.0));
            let ppp = ui.ctx().pixels_per_point();
            if let crate::media::Tex::Ready { id, .. } = self.media.image(icon, 24.0 * ppp) {
                egui::Image::from_texture((id, r.size())).corner_radius(12).paint_at(ui, r);
            }
            x += 32.0;
        }
        ui.painter().with_clip_rect(head.shrink(4.0)).text(
            pos2(x, head.center().y),
            Align2::LEFT_CENTER,
            &a.guild_name,
            bold(15.0),
            TEXT_STRONG,
        );

        let footer_h = 40.0;
        let list_h = ui.available_height() - footer_h;
        ScrollArea::vertical().id_salt("channels").max_height(list_h).auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(8.0);
            let mut archive_started = false;
            for cat in &a.sidebar {
                if cat.archive && !archive_started {
                    archive_started = true;
                    ui.add_space(10.0);
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
                    ui.painter().hline(r.x_range().shrink(12.0), r.center().y, Stroke::new(1.0, DIVIDER));
                }
                let collapsed = self.collapsed.contains(&cat.id);
                if !cat.name.is_empty() {
                    ui.add_space(12.0);
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                    let color = if resp.hovered() { TEXT } else { TEXT_MUTED };
                    let arrow = if collapsed { "›" } else { "⌄" };
                    ui.painter().text(pos2(r.left() + 10.0, r.center().y), Align2::CENTER_CENTER, arrow, FontId::proportional(12.0), color);
                    ui.painter().with_clip_rect(r).text(
                        pos2(r.left() + 18.0, r.center().y),
                        Align2::LEFT_CENTER,
                        cat.name.to_uppercase(),
                        bold(11.5),
                        color,
                    );
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        if collapsed {
                            self.collapsed.remove(&cat.id);
                        } else {
                            self.collapsed.insert(cat.id);
                        }
                    }
                }
                for sc in &cat.channels {
                    let selected = self.current == Some(sc.channel);
                    let thread_selected = sc.threads.iter().any(|t| self.current == Some(*t));
                    if collapsed && !selected && !thread_selected {
                        continue;
                    }
                    if channel_row(ui, a, sc.channel, selected, 0.0).clicked() {
                        actions.push(Action::OpenChannel(sc.channel));
                    }
                    for &t in &sc.threads {
                        let sel = self.current == Some(t);
                        if collapsed && !sel {
                            continue;
                        }
                        if channel_row(ui, a, t, sel, 20.0).clicked() {
                            actions.push(Action::OpenChannel(t));
                        }
                    }
                }
            }
            ui.add_space(12.0);
        });

        let (foot, _) = ui.allocate_exact_size(vec2(ui.available_width(), footer_h), Sense::hover());
        ui.painter().rect_filled(foot, 0, Color32::from_rgb(0x23, 0x24, 0x28));
        ui.painter().text(
            pos2(foot.left() + 12.0, foot.center().y),
            Align2::LEFT_CENTER,
            format!("{} messages · {} channels", a.total_messages, a.channels.len()),
            FontId::proportional(12.0),
            TEXT_MUTED,
        );
    }

    fn header(&mut self, ui: &mut Ui, a: &Archive) {
        ui.horizontal_centered(|ui| {
            ui.add_space(16.0);
            if let Some(ch) = self.current {
                let c = &a.channels[ch];
                if let Some(p) = c.parent {
                    let r = ui.add(Label::new(RichText::new(format!("# {}", a.channels[p].name)).font(bold(15.0)).color(TEXT_MUTED)).sense(Sense::click()));
                    if r.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        let a2 = a;
                        self.open_channel(a2, p);
                    }
                    ui.label(RichText::new("›").color(TEXT_FAINT));
                }
                let icon = match c.kind {
                    ChannelKind::Voice => "🔊",
                    ChannelKind::Thread => "🧵",
                    ChannelKind::Text => "#",
                };
                ui.label(RichText::new(icon).size(20.0).color(TEXT_FAINT));
                ui.label(RichText::new(&c.name).font(bold(16.0)).color(TEXT_STRONG));
                if let Some(t) = &c.topic {
                    let (r, _) = ui.allocate_exact_size(vec2(13.0, 24.0), Sense::hover());
                    ui.painter().vline(r.center().x, r.y_range(), Stroke::new(1.0, DIVIDER));
                    let w = (ui.available_width() - 340.0).max(0.0);
                    if w > 40.0 {
                        ui.allocate_ui_with_layout(vec2(w, 20.0), Layout::left_to_right(Align::Center), |ui| {
                            ui.add(Label::new(RichText::new(t.replace('\n', " ")).size(13.5).color(TEXT_MUTED)).truncate())
                                .on_hover_text(t);
                        });
                    }
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                // Search box
                let id = egui::Id::unique("search-box");
                let focused = ui.memory(|m| m.has_focus(id));
                let w = if focused || !self.search_text.is_empty() { 280.0 } else { 200.0 };
                let resp = ui.add(
                    TextEdit::singleline(&mut self.search_text)
                        .id(id)
                        .hint_text("Search")
                        .desired_width(w)
                        .margin(Margin::symmetric(8, 5))
                        .background_color(BG_INPUT),
                );
                if self.focus_search {
                    resp.request_focus();
                    self.focus_search = false;
                }
                if resp.changed() {
                    self.search_edited = Some(Instant::now());
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    self.run_search(a);
                    resp.request_focus();
                }
                ui.add_space(8.0);
                if self.current.is_some() {
                    let pins_open = self.results.as_ref().is_some_and(|r| r.mode == PanelMode::Pins);
                    let pin_color = if pins_open { TEXT_STRONG } else { TEXT_MUTED };
                    let r = ui.add(Label::new(RichText::new("📌").size(18.0).color(pin_color)).sense(Sense::click()));
                    if r.on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Pinned messages").clicked() {
                        if pins_open {
                            self.results = None;
                        } else {
                            self.show_pins(a);
                        }
                    }
                }
            });
        });
    }
}

fn results_panel(ui: &mut Ui, env: &mut Env, results: &mut Option<Results>, search_ran: &str) {
    let a = env.archive;
    let Some(res) = results.as_mut() else { return };
    let mut close = false;
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let title = match res.mode {
            PanelMode::Search => format!("{} Results", res.hits.len()),
            PanelMode::Pins => format!("Pinned Messages ({})", res.hits.len()),
        };
        ui.label(RichText::new(title).font(bold(15.0)).color(TEXT_STRONG));
        if res.mode == PanelMode::Search && res.took_ms > 0.0 {
            ui.label(RichText::new(format!("{:.0} ms", res.took_ms)).size(11.0).color(TEXT_FAINT));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(10.0);
            if ui.button("✕").clicked() {
                close = true;
            }
            let label = if res.oldest_first { "Oldest first" } else { "Newest first" };
            if ui.button(label).on_hover_text("Change sort order").clicked() {
                res.oldest_first = !res.oldest_first;
                res.hits.reverse();
                res.page = 0;
            }
        });
    });
    for e in &res.errors {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new(format!("⚠ {e}")).size(12.0).color(Color32::from_rgb(0xF2, 0x3F, 0x43)));
        });
    }
    ui.add_space(6.0);
    let pages = res.hits.len().div_ceil(PAGE).max(1);
    res.page = res.page.min(pages - 1);
    let nav_h = if pages > 1 { 40.0 } else { 0.0 };
    let page = res.page;
    let hits: Vec<(u32, u32)> = res.hits.iter().skip(page * PAGE).take(PAGE).copied().collect();
    let empty_mode = (res.hits.is_empty(), res.mode == PanelMode::Search);

    ScrollArea::vertical()
        .id_salt(("results", page, search_ran))
        .max_height(ui.available_height() - nav_h)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if empty_mode.0 {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(if empty_mode.1 { "No results." } else { "This channel has no pinned messages." }).color(TEXT_MUTED));
                });
                if empty_mode.1 {
                    ui.add_space(20.0);
                    search_help(ui);
                }
            }
            for (c, i) in hits {
                let (c, i) = (c as usize, i as usize);
                let m = a.msg(c, i);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let r = ui.add(Label::new(RichText::new(format!("# {}", a.channels[c].name)).font(bold(12.5)).color(TEXT_MUTED)).sense(Sense::click()));
                    if r.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        env.actions.push(Action::OpenChannel(c));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(12.0);
                        if ui.small_button("Jump").clicked() {
                            env.actions.push(Action::JumpTo(m.id));
                        }
                    });
                });
                Frame::new().fill(BG_MAIN).corner_radius(8).inner_margin(Margin { left: 0, right: 0, top: 0, bottom: 8 }).outer_margin(Margin::symmetric(8, 0)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let saved = (env.ch, env.compact);
                    env.ch = c;
                    env.compact = true;
                    render::message(ui, env, i, false, 0.0, 0.0);
                    (env.ch, env.compact) = saved;
                });
            }
            ui.add_space(8.0);
        });
    if pages > 1 {
        ui.horizontal_centered(|ui| {
            ui.add_space(12.0);
            if ui.add_enabled(res.page > 0, egui::Button::new("‹ Previous")).clicked() {
                res.page -= 1;
            }
            ui.label(RichText::new(format!("Page {} of {}", res.page + 1, pages)).color(TEXT_MUTED));
            if ui.add_enabled(res.page + 1 < pages, egui::Button::new("Next ›")).clicked() {
                res.page += 1;
            }
        });
    }
    if close {
        *results = None;
    }
}

fn search_help(ui: &mut Ui) {
    Frame::new().inner_margin(Margin::symmetric(16, 0)).show(ui, |ui| {
        ui.label(RichText::new("SEARCH OPTIONS").font(bold(11.5)).color(TEXT_MUTED));
        ui.add_space(4.0);
        for (k, v) in [
            ("from:", "user (name or nickname)"),
            ("in:", "channel"),
            ("has:", "image, video, audio, file, link, embed, sticker, reaction"),
            ("before: / after:", "2024-05-01, 2024-05 or 2024"),
            ("during:", "a day, month or year"),
            ("pinned:", "true"),
            ("\"…\"", "exact phrase"),
        ] {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(k).font(bold(13.0)).color(TEXT_STRONG));
                ui.label(RichText::new(v).size(13.0).color(TEXT_MUTED));
            });
        }
    });
}

fn channel_row(ui: &mut Ui, a: &Archive, ch: usize, selected: bool, indent: f32) -> egui::Response {
    let c = &a.channels[ch];
    let (outer, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    let rect = Rect::from_min_max(pos2(outer.left() + 8.0 + indent, outer.top() + 1.0), pos2(outer.right() - 8.0, outer.bottom() - 1.0));
    let hovered = resp.hovered();
    if selected || hovered {
        ui.painter().rect_filled(rect, 4, if selected { BG_CHANNEL_SELECTED } else { BG_CHANNEL_HOVER });
    }
    let (icon, size) = match c.kind {
        ChannelKind::Voice => ("🔊", 15.0),
        ChannelKind::Thread => ("↳", 15.0),
        ChannelKind::Text => ("#", 20.0),
    };
    ui.painter().text(pos2(rect.left() + 14.0, rect.center().y), Align2::CENTER_CENTER, icon, FontId::proportional(size), TEXT_FAINT);
    let color = if selected { Color32::WHITE } else if hovered { TEXT } else { TEXT_MUTED };
    let font = if selected { bold(14.5) } else { FontId::proportional(14.5) };
    ui.painter().with_clip_rect(rect.shrink(2.0)).text(pos2(rect.left() + 30.0, rect.center().y), Align2::LEFT_CENTER, &c.name, font, color);
    resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_ui(|ui| {
        ui.label(RichText::new(&c.name).font(bold(13.0)));
        if let Some(t) = &c.topic {
            ui.label(t);
        }
        ui.label(RichText::new(format!("{} messages", c.messages.len())).color(TEXT_MUTED));
    })
}

/// Opens a link or file from a chat, falling back to showing the file in its folder
/// when opening it could run code (see `model::safe_to_open`).
fn open_safely(s: &str) {
    if crate::model::safe_to_open(s) {
        if let Err(e) = open::that_detached(s) {
            eprintln!("open {s}: {e}");
        }
    } else if crate::model::is_local(s) && !s.contains("://") {
        reveal(s);
    } else {
        eprintln!("not opening {s}: only http and https links are opened");
    }
}

fn reveal(path: &str) {
    // Exported media paths are absolute; anything else could be read as a command option.
    if !std::path::Path::new(path).is_absolute() {
        eprintln!("not revealing {path}: not an absolute path");
        return;
    }
    let res = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg("-R").arg(path).spawn().map(|_| ())
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("explorer").arg(format!("/select,{path}")).spawn().map(|_| ())
    } else {
        let dir = std::path::Path::new(path).parent().map(|p| p.to_path_buf()).unwrap_or_default();
        open::that_detached(dir)
    };
    if let Err(e) = res {
        eprintln!("reveal {path}: {e}");
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        if let Some(rx) = &self.loader
            && let Ok(res) = rx.try_recv()
        {
            self.loader = None;
            match res {
                Ok(a) => {
                    for cat in a.sidebar.iter().filter(|c| c.archive) {
                        self.collapsed.insert(cat.id);
                    }
                    let first = a.sidebar.iter().find(|c| !c.archive).and_then(|c| c.channels.first()).map(|c| c.channel);
                    let a = Arc::new(a);
                    if let Some(f) = first {
                        self.open_channel(&a, f);
                    }
                    self.archive = Some(a);
                }
                Err(e) => self.error = Some(e),
            }
        }
        let Some(a) = self.archive.clone() else {
            self.loading_screen(ui);
            return;
        };
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.media.begin_frame();
        self.players.begin_frame();

        // Keyboard shortcuts
        let (find, esc) = ctx.input(|i| (i.modifiers.command && i.key_pressed(Key::F), i.key_pressed(Key::Escape)));
        if find {
            self.focus_search = true;
        }
        if esc && self.lightbox.is_none() && self.results.is_some() {
            self.results = None;
        }
        if let Some(t) = self.search_edited
            && t.elapsed().as_millis() >= 300
            && self.search_text != self.search_ran
        {
            self.run_search(&a);
        } else if self.search_edited.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        let mut actions = Vec::new();
        self.run_script(&ctx, &a, &mut actions);

        Panel::left("sidebar")
            .exact_size(240.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(BG_SIDEBAR))
            .show(ui, |ui| self.sidebar(ui, &a, &mut actions));

        Panel::top("header")
            .exact_size(48.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(BG_MAIN))
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0, BORDER));
                self.header(ui, &a);
            });

        let highlight = self.highlight.and_then(|(id, t)| {
            let s = 1.0 - ((now - t - 1.0) / 1.5).clamp(0.0, 1.0) as f32;
            if s > 0.0 {
                ctx.request_repaint();
                Some((id, s))
            } else {
                None
            }
        });

        // Split self into the parts the renderer needs.
        let App { media, players, audio, revealed, views, current, results, lightbox, search_ran, .. } = self;
        let mut env = Env {
            archive: &a,
            media,
            players,
            audio,
            revealed,
            actions: &mut actions,
            compact: false,
            ch: current.unwrap_or(0),
        };

        if results.is_some() {
            Panel::right("results")
                .default_size(460.0)
                .size_range(340.0..=800.0)
                .resizable(true)
                .frame(Frame::new().fill(BG_SIDEBAR))
                .show(ui, |ui| results_panel(ui, &mut env, results, search_ran));
        }

        CentralPanel::default().frame(Frame::new().fill(BG_MAIN)).show(ui, |ui| {
            if let Some(ch) = *current
                && let Some(view) = views.get_mut(&ch)
            {
                env.ch = ch;
                crate::view::show(ui, view, &mut env, highlight);
            } else {
                ui.centered_and_justified(|ui| ui.label(RichText::new("Pick a channel").color(TEXT_MUTED)));
            }
        });

        if let Some(lb) = lightbox {
            env.ch = lb.ch;
            if !lb.show(&ctx, &mut env) {
                *lightbox = None;
            }
        }

        self.handle(&a, actions, now);
        self.players.end_frame();
        self.media.end_frame();
    }
}
