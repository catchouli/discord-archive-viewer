//! Drawing messages: header, markdown content, attachments, embeds, reactions,
//! and the inline image, video and audio widgets.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::{Datelike, Local, TimeZone};
use egui::text::LayoutJob;
use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Frame, Label, Layout, Margin, Pos2, Rect, Response, RichText,
    Sense, Shape, Stroke, TextFormat, Ui, UiBuilder, Vec2, pos2, vec2,
};

use crate::markdown::{self, Block, Span, Style};
use crate::media::{MediaCache, Tex};
use crate::model::{Archive, Attachment, Embed, EmbedMedia, InlineEmoji, MediaKind, MediaRef, Message, MsgKind, User, is_local};
use crate::player::{AudioOut, Player, State};
use crate::theme::*;

pub enum Action {
    JumpTo(u64),
    OpenChannel(usize),
    Lightbox { ch: usize, msg: usize, path: MediaRef },
    Open(String),
    Reveal(String),
}

/// Players that are on screen. A player that isn't drawn for a frame is stopped.
#[derive(Default)]
pub struct Players {
    map: HashMap<String, (Player, u64)>,
    frame: u64,
}

impl Players {
    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }
    pub fn end_frame(&mut self) {
        let f = self.frame;
        self.map.retain(|_, (_, seen)| *seen >= f);
    }
    pub fn get(&mut self, key: &str) -> Option<&mut Player> {
        let f = self.frame;
        self.map.get_mut(key).map(|(p, seen)| {
            *seen = f;
            p
        })
    }
    pub fn start(&mut self, key: &str, path: &str, autoplay: bool) -> &mut Player {
        let f = self.frame;
        let e = self.map.entry(key.to_string()).or_insert_with(|| (Player::new(Path::new(path), autoplay), f));
        e.1 = f;
        &mut e.0
    }
    pub fn pause_all(&mut self, audio: &mut AudioOut) {
        for (p, _) in self.map.values_mut() {
            if p.state == State::Playing && !p.looping {
                p.toggle(audio);
            }
        }
    }
}

pub struct Env<'a> {
    pub archive: &'a Archive,
    pub media: &'a mut MediaCache,
    pub players: &'a mut Players,
    pub audio: &'a mut AudioOut,
    pub revealed: &'a mut HashSet<u64>,
    pub actions: &'a mut Vec<Action>,
    /// Smaller media and avatars (search results).
    pub compact: bool,
    pub ch: usize,
}

// ---------------------------------------------------------------------------
// Formatting helpers

pub fn fmt_ts(ts: i64) -> String {
    let Some(d) = Local.timestamp_millis_opt(ts).single() else { return String::new() };
    let today = Local::now().date_naive();
    let date = d.date_naive();
    if date == today {
        format!("Today at {}", d.format("%H:%M"))
    } else if today.pred_opt() == Some(date) {
        format!("Yesterday at {}", d.format("%H:%M"))
    } else {
        d.format("%d/%m/%Y %H:%M").to_string()
    }
}

pub fn fmt_full(ts: i64) -> String {
    Local.timestamp_millis_opt(ts).single().map(|d| d.format("%A, %-d %B %Y at %H:%M").to_string()).unwrap_or_default()
}

pub fn fmt_time(ts: i64) -> String {
    Local.timestamp_millis_opt(ts).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default()
}

pub fn fmt_date(d: chrono::NaiveDate) -> String {
    format!("{} {} {}", d.day(), d.format("%B"), d.year())
}

pub fn fmt_size(b: u64) -> String {
    match b {
        b if b >= 1 << 30 => format!("{:.2} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.2} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.2} KB", b as f64 / 1024.0),
        b => format!("{b} bytes"),
    }
}

pub fn fmt_dur(s: f64) -> String {
    let s = s.max(0.0) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

fn fit(natural: Vec2, max: Vec2) -> Vec2 {
    let s = (max.x / natural.x).min(max.y / natural.y).min(1.0);
    (natural * s).max(vec2(1.0, 1.0)).round()
}

/// Scales `n` up or down to fit inside `max`, keeping its aspect ratio.
fn contain(n: Vec2, max: Vec2) -> Vec2 {
    n * (max.x / n.x).min(max.y / n.y)
}

fn default_avatar_color(i: u32) -> Color32 {
    const C: [Color32; 5] = [
        Color32::from_rgb(0x58, 0x65, 0xF2),
        Color32::from_rgb(0x75, 0x7E, 0x8A),
        Color32::from_rgb(0x3B, 0xA5, 0x5C),
        Color32::from_rgb(0xFA, 0xA6, 0x1A),
        Color32::from_rgb(0xED, 0x42, 0x45),
    ];
    C[i as usize % C.len()]
}

// ---------------------------------------------------------------------------
// Small widgets

pub fn avatar(ui: &Ui, env: &mut Env, user_idx: u32, rect: Rect) {
    let u = env.archive.user(user_idx);
    let ppp = ui.ctx().pixels_per_point();
    if let Some(path) = &u.avatar
        && let Tex::Ready { id, .. } = env.media.image(path, rect.width() * ppp)
    {
        egui::Image::from_texture((id, rect.size())).corner_radius(rect.width() / 2.0).paint_at(ui, rect);
        return;
    }
    ui.painter().circle_filled(rect.center(), rect.width() / 2.0, default_avatar_color(user_idx));
    let initial = u.display_name().chars().next().unwrap_or('?').to_uppercase().to_string();
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, initial, FontId::proportional(rect.width() * 0.45), Color32::WHITE);
}

fn user_card(ui: &mut Ui, u: &User) {
    ui.set_max_width(260.0);
    ui.label(RichText::new(u.display_name()).font(bold(16.0)).color(u.color.unwrap_or(TEXT_STRONG)));
    ui.label(RichText::new(&u.name).color(TEXT_MUTED));
    if !u.roles.is_empty() {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            for (name, color) in &u.roles {
                Frame::new().fill(BG_DARKEST).corner_radius(4).inner_margin(Margin::symmetric(6, 2)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 5.0, color.unwrap_or(TEXT_MUTED));
                        ui.label(RichText::new(name).size(12.0).color(TEXT));
                    });
                });
            }
        });
    }
}

/// An image that fits in `max`, clickable to open in the lightbox (or externally).
pub fn image(ui: &mut Ui, env: &mut Env, path: &MediaRef, max: Vec2, natural: Option<[u32; 2]>, msg: Option<usize>) -> Response {
    let click = match msg {
        Some(m) => Action::Lightbox { ch: env.ch, msg: m, path: path.clone() },
        None => Action::Open(path.to_string()),
    };
    image_ex(ui, env, path, max, natural, click)
}

/// Like [`image`], with a custom click action.
fn image_ex(ui: &mut Ui, env: &mut Env, path: &MediaRef, max: Vec2, natural: Option<[u32; 2]>, click: Action) -> Response {
    if !is_local(path) {
        return remote_placeholder(ui, env, path, max);
    }
    let d = env.media.dims(path).or(natural).map(|[w, h]| vec2(w as f32, h as f32));
    let size = match d {
        Some(d) => fit(d, max),
        None => vec2(max.x.min(320.0), max.y.min(220.0)),
    };
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let ppp = ui.ctx().pixels_per_point();
        match env.media.image(path, size.max_elem() * ppp) {
            Tex::Ready { id, size: tex } => {
                // Natural size may have changed after decoding (EXIF rotation).
                if d.is_none_or(|d| ((d.x / d.y) - (tex.x / tex.y)).abs() > 0.02) {
                    ui.ctx().request_repaint();
                }
                egui::Image::from_texture((id, size)).corner_radius(4).paint_at(ui, rect)
            }
            Tex::Loading => {
                ui.painter().rect_filled(rect, 4, BG_DARKEST);
            }
            Tex::Failed => {
                ui.painter().rect_filled(rect, 4, BG_DARKEST);
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, "🖼  can't display", FontId::proportional(13.0), TEXT_MUTED);
            }
        }
    }
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    if resp.clicked() {
        env.actions.push(click);
    }
    resp
}

/// True if an embed's video was actually downloaded as a video (YouTube "videos" are HTML pages).
pub fn playable(media: &mut MediaCache, v: &EmbedMedia) -> bool {
    is_local(&v.url) && media.kind(&v.url, None) == MediaKind::Video
}

/// A thumbnail with a play badge that opens the video's page in the browser.
fn external_video(ui: &mut Ui, env: &mut Env, thumb: &EmbedMedia, url: &str, max: Vec2) {
    let r = image_ex(ui, env, &thumb.url, max, thumb.size, Action::Open(url.to_string())).on_hover_text(url);
    big_play(ui, r.rect);
}

fn remote_placeholder(ui: &mut Ui, env: &mut Env, url: &str, max: Vec2) -> Response {
    let size = vec2(max.x.min(280.0), 56.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    ui.painter().rect_filled(rect, 4, BG_DARKEST);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        "🔗  media wasn't downloaded: open link",
        FontId::proportional(13.0),
        TEXT_MUTED,
    );
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(url);
    if resp.clicked() {
        env.actions.push(Action::Open(url.to_string()));
    }
    resp
}

fn play_icon(painter: &egui::Painter, c: Pos2, r: f32, playing: bool, color: Color32) {
    if playing {
        let w = r * 0.32;
        for dx in [-r * 0.35, r * 0.35] {
            painter.rect_filled(Rect::from_center_size(c + vec2(dx, 0.0), vec2(w, r * 1.4)), 1, color);
        }
    } else {
        let pts = vec![c + vec2(-r * 0.5, -r * 0.75), c + vec2(r * 0.8, 0.0), c + vec2(-r * 0.5, r * 0.75)];
        painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
    }
}

/// Play/pause, time, seek bar and mute, drawn into `bar`.
fn transport(ui: &mut Ui, env: &mut Env, key: &str, path: &str, bar: Rect, fullscreen: Option<Action>) {
    let id = ui.scope_id().with(("transport", key));
    let painter = ui.painter().clone();
    let (playing, pos, dur, muted, started) = match env.players.get(key) {
        Some(p) => (p.is_playing(), p.position(), p.duration(), p.muted, true),
        None => (false, 0.0, 0.0, false, false),
    };

    let mut x = bar.left() + 6.0;
    let pp = Rect::from_center_size(pos2(x + 12.0, bar.center().y), vec2(24.0, 24.0));
    let r = ui.interact(pp, id.with("pp"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    play_icon(&painter, pp.center(), 7.0, playing, if r.hovered() { Color32::WHITE } else { TEXT });
    if r.clicked() {
        match env.players.get(key) {
            Some(p) => p.toggle(env.audio),
            None => {
                env.players.start(key, path, true);
            }
        }
    }
    x += 30.0;

    let time = format!("{} / {}", fmt_dur(pos), fmt_dur(dur));
    let galley = painter.layout_no_wrap(time, FontId::monospace(11.5), TEXT);
    painter.galley(pos2(x, bar.center().y - galley.size().y / 2.0), galley.clone(), TEXT);
    x += galley.size().x + 10.0;

    let right_buttons = if fullscreen.is_some() { 60.0 } else { 32.0 };
    let track = Rect::from_min_max(pos2(x, bar.center().y - 8.0), pos2(bar.right() - right_buttons, bar.center().y + 8.0));
    if track.width() > 20.0 {
        let tr = ui.interact(track, id.with("seek"), Sense::click_and_drag()).on_hover_cursor(CursorIcon::PointingHand);
        let line = Rect::from_center_size(track.center(), vec2(track.width(), 4.0));
        painter.rect_filled(line, 2, Color32::from_white_alpha(60));
        let frac = if dur > 0.0 { (pos / dur).clamp(0.0, 1.0) as f32 } else { 0.0 };
        let hover_frac = tr.interact_pointer_pos().or(tr.hover_pos()).map(|p| ((p.x - line.left()) / line.width()).clamp(0.0, 1.0));
        let shown = if tr.dragged() { hover_frac.unwrap_or(frac) } else { frac };
        let filled = Rect::from_min_max(line.min, pos2(line.left() + line.width() * shown, line.bottom()));
        painter.rect_filled(filled, 2, BLURPLE);
        if tr.hovered() || tr.dragged() {
            painter.circle_filled(pos2(filled.right(), line.center().y), 6.0, Color32::WHITE);
            if let Some(h) = hover_frac
                && dur > 0.0
            {
                tr.clone().on_hover_text_at_pointer(fmt_dur(h as f64 * dur));
            }
        }
        if (tr.drag_stopped() || tr.clicked())
            && let Some(f) = hover_frac
        {
            if env.players.get(key).is_none() {
                env.players.start(key, path, true);
            }
            if let Some(p) = env.players.get(key)
                && p.duration() > 0.0
            {
                p.seek(f as f64 * p.duration(), env.audio);
            }
        }
    }

    let mute = Rect::from_center_size(pos2(bar.right() - right_buttons + 16.0, bar.center().y), vec2(24.0, 24.0));
    let mr = ui.interact(mute, id.with("mute"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    painter.text(mute.center(), Align2::CENTER_CENTER, if muted { "🔇" } else { "🔊" }, FontId::proportional(14.0), TEXT);
    if mr.clicked()
        && let Some(p) = env.players.get(key)
    {
        let v = p.volume;
        p.set_volume(v, !muted);
    }
    if let Some(action) = fullscreen {
        let fs = Rect::from_center_size(pos2(bar.right() - 16.0, bar.center().y), vec2(24.0, 24.0));
        let fr = ui.interact(fs, id.with("fs"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Full screen");
        painter.text(fs.center(), Align2::CENTER_CENTER, "⛶", FontId::proportional(15.0), TEXT);
        if fr.clicked() {
            env.actions.push(action);
        }
    }
    if started {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    }
}

/// A video with a poster frame; plays inline when clicked. `gifv` videos loop silently and autoplay.
#[allow(clippy::too_many_arguments)]
pub fn video(
    ui: &mut Ui,
    env: &mut Env,
    key: &str,
    path: &MediaRef,
    poster: Option<&MediaRef>,
    natural: Option<[u32; 2]>,
    max: Vec2,
    gifv: bool,
    msg: Option<usize>,
) {
    if !is_local(path) {
        match poster.filter(|p| is_local(p)) {
            Some(p) => {
                image(ui, env, p, max, natural, msg);
            }
            None => {
                remote_placeholder(ui, env, path, max);
            }
        }
        return;
    }
    let player_size = env.players.get(key).and_then(|p| p.video_size());
    let poster_dims = poster.filter(|p| is_local(p)).and_then(|p| env.media.dims(p));
    let d = player_size
        .or(natural.map(|[w, h]| vec2(w as f32, h as f32)))
        .or(poster_dims.map(|[w, h]| vec2(w as f32, h as f32)))
        .or(env.media.dims(path).map(|[w, h]| vec2(w as f32, h as f32)))
        .unwrap_or(vec2(16.0, 9.0) * 40.0);
    let size = fit(d, max).max(vec2(160.0, 90.0).min(max));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, 4, Color32::BLACK);

    let ppp = ui.ctx().pixels_per_point();
    let draw_poster = |ui: &Ui, env: &mut Env| {
        let tex = match poster.filter(|p| is_local(p)) {
            Some(p) => env.media.image(p, size.max_elem() * ppp),
            None => env.media.poster(path),
        };
        if let Tex::Ready { id, size: ts } = tex {
            let r = Rect::from_center_size(rect.center(), contain(ts, rect.size()));
            egui::Image::from_texture((id, r.size())).corner_radius(4).paint_at(ui, r);
        }
    };

    if gifv && env.players.get(key).is_none() {
        let p = env.players.start(key, path, true);
        p.looping = true;
        p.set_volume(1.0, true);
    }

    let hovered = ui.rect_contains_pointer(rect);
    if let Some(p) = env.players.get(key) {
        p.update(ui.ctx(), env.audio);
        let frame = p.frame();
        let state = p.state.clone();
        let playing = p.is_playing();
        match frame {
            Some((id, ts)) => {
                let r = Rect::from_center_size(rect.center(), contain(ts, rect.size()));
                egui::Image::from_texture((id, r.size())).corner_radius(4).paint_at(ui, r);
            }
            None => draw_poster(ui, env),
        }
        if let State::Error(e) = &state {
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, format!("⚠ {e}"), FontId::proportional(13.0), TEXT);
        }
        if gifv {
            gif_badge(ui, rect);
            if resp.clicked()
                && let Some(m) = msg
            {
                env.actions.push(Action::Lightbox { ch: env.ch, msg: m, path: path.clone() });
            }
        } else {
            if !playing && !matches!(state, State::Error(_)) {
                big_play(ui, rect);
            }
            if resp.clicked()
                && let Some(p) = env.players.get(key) {
                    p.toggle(env.audio);
                }
            if hovered || !playing {
                let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 34.0), rect.max);
                ui.painter().rect_filled(bar, CornerRadius { nw: 0, ne: 0, sw: 4, se: 4 }, Color32::from_black_alpha(170));
                let fs = msg.map(|m| Action::Lightbox { ch: env.ch, msg: m, path: path.clone() });
                transport(ui, env, key, path, bar, fs);
            }
        }
    } else {
        draw_poster(ui, env);
        if gifv {
            gif_badge(ui, rect);
        } else {
            big_play(ui, rect);
        }
        let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
        if resp.clicked() {
            env.players.start(key, path, true);
        }
    }
}

fn big_play(ui: &Ui, rect: Rect) {
    let c = rect.center();
    ui.painter().circle_filled(c, 24.0, Color32::from_black_alpha(160));
    play_icon(ui.painter(), c + vec2(2.0, 0.0), 11.0, false, Color32::WHITE);
}

fn gif_badge(ui: &Ui, rect: Rect) {
    let r = Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(30.0, 18.0));
    ui.painter().rect_filled(r, 4, Color32::from_black_alpha(150));
    ui.painter().text(r.center(), Align2::CENTER_CENTER, "GIF", bold(11.0), Color32::WHITE);
}

fn audio(ui: &mut Ui, env: &mut Env, key: &str, a: &Attachment) {
    let w = ui.available_width().min(420.0);
    Frame::new().fill(BG_EMBED).stroke(Stroke::new(1.0, BORDER)).corner_radius(8).inner_margin(10).show(ui, |ui| {
        ui.set_width(w - 20.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("🎵").size(24.0));
            ui.vertical(|ui| {
                let r = ui.add(Label::new(RichText::new(&a.file_name).color(LINK)).sense(Sense::click()).truncate());
                if r.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    env.actions.push(Action::Open(a.url.to_string()));
                }
                ui.label(RichText::new(fmt_size(a.size)).size(12.0).color(TEXT_MUTED));
            });
        });
        ui.add_space(4.0);
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
        ui.painter().rect_filled(bar, 15, BG_DARKEST);
        if let Some(p) = env.players.get(key) {
            p.update(ui.ctx(), env.audio);
            if let State::Error(e) = &p.state {
                let e = e.clone();
                ui.label(RichText::new(format!("⚠ {e}")).size(12.0).color(TEXT_MUTED));
            }
        }
        if is_local(&a.url) {
            transport(ui, env, key, &a.url, bar, None);
        }
    });
}

fn file(ui: &mut Ui, env: &mut Env, a: &Attachment) {
    let w = ui.available_width().min(420.0);
    Frame::new().fill(BG_EMBED).stroke(Stroke::new(1.0, BORDER)).corner_radius(8).inner_margin(10).show(ui, |ui| {
        ui.set_width(w - 20.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("📄").size(26.0));
            ui.vertical(|ui| {
                // Files that could run code when opened are only shown in their folder.
                let safe = crate::model::safe_to_open(&a.url);
                let hint = if safe { "Open" } else { "Show in folder (this kind of file isn't opened directly)" };
                let r = ui.add(Label::new(RichText::new(&a.file_name).color(LINK)).sense(Sense::click()).truncate());
                if r.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(hint).clicked() {
                    env.actions.push(if safe { Action::Open(a.url.to_string()) } else { Action::Reveal(a.url.to_string()) });
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(fmt_size(a.size)).size(12.0).color(TEXT_MUTED));
                    if is_local(&a.url) {
                        let r = ui.add(Label::new(RichText::new("Show in folder").size(12.0).color(LINK)).sense(Sense::click()));
                        if r.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            env.actions.push(Action::Reveal(a.url.to_string()));
                        }
                    }
                });
            });
        });
    });
}

// ---------------------------------------------------------------------------
// Markdown content

struct SpanCx {
    msg_id: u64,
    revealed: bool,
    emoji_px: f32,
}

fn text_format(style: Style, size: f32, color: Color32, bold_base: bool, hidden: bool) -> TextFormat {
    let family = if style.bold || bold_base { bold_family() } else { egui::FontFamily::Proportional };
    let mut f = TextFormat { font_id: FontId::new(size, family), color, italics: style.italic, ..Default::default() };
    if style.underline {
        f.underline = Stroke::new(1.0, color);
    }
    if style.strike {
        f.strikethrough = Stroke::new(1.0, color);
    }
    if style.spoiler {
        if hidden {
            f.color = SPOILER;
            f.background = SPOILER;
            f.underline = Stroke::NONE;
            f.strikethrough = Stroke::NONE;
        } else {
            f.background = SPOILER_REVEALED;
        }
    }
    f
}

fn span(ui: &mut Ui, env: &mut Env, s: &Span, cx: &SpanCx, size: f32, color: Color32, bold_base: bool) {
    let style = match s {
        Span::Text { style, .. }
        | Span::Code { style, .. }
        | Span::Link { style, .. }
        | Span::Emoji { style, .. }
        | Span::Mention { style, .. }
        | Span::Channel { style, .. } => *style,
    };
    let hidden = style.spoiler && !cx.revealed;
    let label = |text: &str, fmt: TextFormat| Label::new(LayoutJob::single_section(text.to_string(), fmt)).wrap();
    let resp = match s {
        Span::Text { text, .. } => ui.add(label(text, text_format(style, size, color, bold_base, hidden)).sense(if hidden {
            Sense::click()
        } else {
            Sense::hover()
        })),
        Span::Code { text, .. } => {
            let mut f = text_format(style, size, color, false, hidden);
            f.font_id = FontId::monospace(size * 0.88);
            if !hidden {
                f.background = BG_CODE;
            }
            ui.add(label(text, f))
        }
        Span::Link { text, url, .. } => {
            let r = ui.add(label(text, text_format(style, size, LINK, bold_base, hidden)).sense(Sense::click()));
            let r = r.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(url.as_str());
            if r.clicked() && !hidden {
                env.actions.push(Action::Open(url.clone()));
            }
            r
        }
        Span::Mention { text, .. } => {
            let mut f = text_format(style, size, MENTION_FG, bold_base, hidden);
            if !hidden {
                f.background = MENTION_BG;
            }
            ui.add(label(text, f))
        }
        Span::Channel { name, .. } => {
            let mut f = text_format(style, size, MENTION_FG, bold_base, hidden);
            if !hidden {
                f.background = MENTION_BG;
            }
            let r = ui.add(label(&format!("# {name}"), f).sense(Sense::click())).on_hover_cursor(CursorIcon::PointingHand);
            if r.clicked()
                && let Some(&c) = env.archive.channel_by_name.get(name)
            {
                env.actions.push(Action::OpenChannel(c));
            }
            r
        }
        Span::Emoji { image, fallback, .. } => {
            let px = cx.emoji_px;
            match image.as_ref().filter(|p| is_local(p)) {
                Some(path) if !hidden => {
                    let ppp = ui.ctx().pixels_per_point();
                    let (rect, r) = ui.allocate_exact_size(vec2(px, px), Sense::hover());
                    if let Tex::Ready { id, size: ts } = env.media.image(path, px * ppp) {
                        let sz = fit(ts * 100.0, vec2(px, px));
                        egui::Image::from_texture((id, sz)).paint_at(ui, Rect::from_center_size(rect.center(), sz));
                    }
                    r.on_hover_text(fallback.as_str())
                }
                _ => ui.add(label(fallback, text_format(style, size, color, bold_base, hidden))),
            }
        }
    };
    if hidden && resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
        env.revealed.insert(cx.msg_id);
    }
}

#[allow(clippy::too_many_arguments)]
fn spans_line(ui: &mut Ui, env: &mut Env, spans: &[Span], cx: &SpanCx, size: f32, color: Color32, bold_base: bool, trailing: Option<&str>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.set_row_height(size + 5.0);
        for s in spans {
            span(ui, env, s, cx, size, color, bold_base);
        }
        if let Some(t) = trailing {
            ui.label(RichText::new(t).size(11.0).color(TEXT_FAINT));
        }
    });
}

fn blocks(ui: &mut Ui, env: &mut Env, bs: &[Block], cx: &SpanCx, color: Color32, trailing: Option<&str>) {
    for (i, b) in bs.iter().enumerate() {
        let trailing = if i + 1 == bs.len() { trailing } else { None };
        match b {
            Block::Line(s) => spans_line(ui, env, s, cx, BODY, color, false, trailing),
            Block::Heading(level, s) => {
                ui.add_space(4.0);
                let size = match level {
                    1 => 24.0,
                    2 => 20.0,
                    _ => 17.0,
                };
                spans_line(ui, env, s, cx, size, TEXT_STRONG, true, trailing);
                ui.add_space(2.0);
            }
            Block::Subtext(s) => spans_line(ui, env, s, cx, 12.5, TEXT_MUTED, false, trailing),
            Block::ListItem(s) => {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_sized(vec2(18.0, LINE_H), Label::new(RichText::new("•").color(color)));
                    ui.vertical(|ui| spans_line(ui, env, s, cx, BODY, color, false, trailing));
                });
            }
            Block::Quote(inner) => {
                let top = ui.cursor().top();
                let r = Frame::new().inner_margin(Margin { left: 14, right: 0, top: 0, bottom: 0 }).show(ui, |ui| {
                    blocks(ui, env, inner, cx, color, trailing);
                });
                let rr = r.response.rect;
                let bar = Rect::from_min_max(pos2(rr.left(), top), pos2(rr.left() + 4.0, rr.bottom()));
                ui.painter().rect_filled(bar, 2, QUOTE_BAR);
            }
            Block::Code { text } => {
                ui.add_space(2.0);
                Frame::new().fill(BG_CODE).stroke(Stroke::new(1.0, BORDER)).corner_radius(4).inner_margin(8).show(ui, |ui| {
                    ui.set_width(ui.available_width().min(700.0) - 16.0);
                    ui.add(Label::new(RichText::new(text).font(FontId::monospace(13.0)).color(TEXT)).wrap());
                });
                ui.add_space(2.0);
            }
            Block::Blank => ui.add_space(LINE_H * 0.7),
        }
    }
}

/// Renders message (or embed) text with Discord markdown, emoji and mentions.
pub fn content(ui: &mut Ui, env: &mut Env, msg_id: u64, text: &str, emojis: &[InlineEmoji], mentions: &[u32], edited: bool) {
    if text.trim().is_empty() {
        return;
    }
    let a = env.archive;
    let mut pats: Vec<(&str, Option<MediaRef>)> = emojis.iter().map(|e| (e.pattern.as_str(), e.image.clone())).collect();
    pats.sort_by_key(|p| std::cmp::Reverse(p.0.len()));
    pats.dedup_by(|x, y| x.0 == y.0);
    let names: Vec<&str> = mentions.iter().flat_map(|&u| {
        let u = a.user(u);
        [u.display_name(), u.name.as_str()]
    }).collect();
    let exists = |n: &str| a.channel_by_name.contains_key(n);
    let mcx = markdown::Context { emojis: &pats, mentions: &names, channel_exists: &exists };
    let bs = markdown::parse(text, &mcx);
    let jumbo = markdown::is_emoji_only(&bs);
    let cx = SpanCx { msg_id, revealed: env.revealed.contains(&msg_id), emoji_px: if jumbo { 48.0 } else { 22.0 } };
    blocks(ui, env, &bs, &cx, TEXT, edited.then_some(" (edited)"));
}

// ---------------------------------------------------------------------------
// Message parts

fn media_max(ui: &Ui, env: &Env) -> Vec2 {
    if env.compact { vec2(ui.available_width().min(320.0), 220.0) } else { vec2(ui.available_width().min(550.0), 350.0) }
}

fn attachments(ui: &mut Ui, env: &mut Env, msg_id: u64, msg: usize, atts: &[Attachment], key: &str) {
    if atts.is_empty() {
        return;
    }
    let mut visual = Vec::new();
    let mut rest = Vec::new();
    for (i, a) in atts.iter().enumerate() {
        let kind = if a.kind == MediaKind::Other { env.media.kind(&a.url, Some(&a.file_name)) } else { a.kind };
        match kind {
            MediaKind::Image | MediaKind::Svg => visual.push(a),
            _ => rest.push((i, a, kind)),
        }
    }
    let max = media_max(ui, env);
    ui.add_space(4.0);
    if visual.len() == 1 {
        image(ui, env, &visual[0].url, max, None, Some(msg));
    } else if !visual.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            let each = vec2((max.x / 2.0 - 2.0).max(100.0), if env.compact { 140.0 } else { 260.0 });
            for a in visual {
                image(ui, env, &a.url, each, None, Some(msg));
            }
        });
    }
    for (i, a, kind) in rest {
        ui.add_space(4.0);
        let pkey = format!("{key}{msg_id}:a{i}");
        match kind {
            MediaKind::Video => video(ui, env, &pkey, &a.url, None, None, max, false, Some(msg)),
            MediaKind::Audio => audio(ui, env, &pkey, a),
            _ => file(ui, env, a),
        }
    }
}

fn gallery(ui: &mut Ui, env: &mut Env, imgs: &[EmbedMedia], max: Vec2, msg: usize) {
    if imgs.len() == 1 {
        image(ui, env, &imgs[0].url, max, imgs[0].size, Some(msg));
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            let each = vec2((max.x / 2.0 - 2.0).max(80.0), max.y * 0.6);
            for m in imgs {
                image(ui, env, &m.url, each, m.size, Some(msg));
            }
        });
    }
}

fn embed(ui: &mut Ui, env: &mut Env, e: &Embed, msg_id: u64, msg: usize, key: &str) {
    ui.add_space(4.0);
    let max = media_max(ui, env);
    let playable_video = e.video.as_ref().filter(|v| playable(env.media, v));
    let external = e.video.is_some() && playable_video.is_none();
    if e.is_plain_media() {
        if let (true, Some(t), Some(u)) = (external, &e.thumbnail, &e.url) {
            external_video(ui, env, t, u, max);
        } else if let Some(v) = playable_video {
            let poster = e.thumbnail.as_ref().map(|t| &t.url);
            let natural = v.size.or(e.thumbnail.as_ref().and_then(|t| t.size));
            video(ui, env, key, &v.url, poster, natural, max, e.is_gifv(), Some(msg));
        } else if !e.images.is_empty() {
            gallery(ui, env, &e.images, max, msg);
        } else if let Some(t) = &e.thumbnail {
            image(ui, env, &t.url, max, t.size, Some(msg));
        }
        return;
    }

    let outer_w = ui.available_width().min(if env.compact { 380.0 } else { 520.0 });
    let side_thumb = e.thumbnail.as_ref().filter(|_| e.video.is_none() && e.images.is_empty());
    let big_thumb = e.thumbnail.as_ref().filter(|_| external);
    let r = Frame::new()
        .fill(BG_EMBED)
        .corner_radius(4)
        .inner_margin(Margin { left: 16, right: 16, top: 10, bottom: 12 })
        .show(ui, |ui| {
            let inner_w = outer_w - 32.0;
            ui.set_max_width(inner_w);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_max_width(if side_thumb.is_some() { inner_w - 96.0 } else { inner_w });
                    ui.spacing_mut().item_spacing.y = 4.0;
                    if let Some(name) = &e.author_name {
                        ui.horizontal(|ui| {
                            if let Some(icon) = e.author_icon.as_ref().filter(|i| is_local(i)) {
                                let (rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
                                let ppp = ui.ctx().pixels_per_point();
                                if let Tex::Ready { id, .. } = env.media.image(icon, 24.0 * ppp) {
                                    egui::Image::from_texture((id, rect.size())).corner_radius(12).paint_at(ui, rect);
                                }
                            }
                            let r = ui.add(
                                Label::new(RichText::new(name).font(bold(13.5)).color(TEXT_STRONG)).sense(Sense::click()).truncate(),
                            );
                            if let Some(u) = &e.author_url
                                && r.on_hover_cursor(CursorIcon::PointingHand).clicked()
                            {
                                env.actions.push(Action::Open(u.clone()));
                            }
                        });
                    }
                    if let Some(t) = &e.title {
                        let color = if e.url.is_some() { LINK } else { TEXT_STRONG };
                        let r = ui.add(Label::new(RichText::new(t).font(bold(15.0)).color(color)).wrap().sense(Sense::click()));
                        if let Some(u) = &e.url
                            && r.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(u.as_str()).clicked()
                        {
                            env.actions.push(Action::Open(u.clone()));
                        }
                    }
                    if let Some(d) = &e.description {
                        ui.scope(|ui| content(ui, env, msg_id, d, &[], &[], false));
                    }
                    let mut fields = e.fields.iter().peekable();
                    while let Some(f) = fields.next() {
                        let mut row = vec![f];
                        while f.2 && row.len() < 3 && fields.peek().is_some_and(|n| n.2) {
                            row.push(fields.next().unwrap());
                        }
                        let colw = ui.available_width() / row.len() as f32 - 8.0;
                        ui.horizontal_top(|ui| {
                            for (name, value, _) in row {
                                ui.vertical(|ui| {
                                    ui.set_width(colw);
                                    ui.label(RichText::new(name).font(bold(13.5)).color(TEXT_STRONG));
                                    content(ui, env, msg_id, value, &[], &[], false);
                                });
                            }
                        });
                    }
                });
                if let Some(t) = side_thumb {
                    image(ui, env, &t.url, vec2(80.0, 80.0), t.size, Some(msg));
                }
            });
            let media_max = vec2(inner_w, if env.compact { 200.0 } else { 300.0 });
            if !e.images.is_empty() {
                ui.add_space(6.0);
                gallery(ui, env, &e.images, media_max, msg);
            }
            if let Some(t) = big_thumb {
                ui.add_space(6.0);
                match &e.url {
                    Some(u) => external_video(ui, env, t, u, media_max),
                    None => {
                        image(ui, env, &t.url, media_max, t.size, Some(msg));
                    }
                }
            }
            if let Some(v) = playable_video {
                ui.add_space(6.0);
                let poster = e.thumbnail.as_ref().map(|t| &t.url);
                let natural = v.size.or(e.thumbnail.as_ref().and_then(|t| t.size));
                video(ui, env, key, &v.url, poster, natural, media_max, e.is_gifv(), Some(msg));
            }
            if let Some((text, icon)) = &e.footer {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if let Some(icon) = icon.as_ref().filter(|i| is_local(i)) {
                        let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                        let ppp = ui.ctx().pixels_per_point();
                        if let Tex::Ready { id, .. } = env.media.image(icon, 18.0 * ppp) {
                            egui::Image::from_texture((id, rect.size())).corner_radius(9).paint_at(ui, rect);
                        }
                    }
                    ui.add(Label::new(RichText::new(text).size(12.0).color(TEXT_MUTED)).wrap());
                });
            }
        });
    let rr = r.response.rect;
    let bar = Rect::from_min_max(rr.min, pos2(rr.left() + 4.0, rr.bottom()));
    ui.painter().rect_filled(bar, CornerRadius { nw: 4, sw: 4, ne: 0, se: 0 }, e.color.unwrap_or(DIVIDER));
}

fn reactions(ui: &mut Ui, env: &mut Env, m: &Message) {
    if m.reactions.is_empty() {
        return;
    }
    ui.add_space(4.0);
    let a = env.archive;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for r in &m.reactions {
            let resp = Frame::new()
                .fill(BG_EMBED)
                .stroke(Stroke::new(1.0, BG_EMBED))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(6, 2))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        let ppp = ui.ctx().pixels_per_point();
                        let tex = r.image.as_ref().filter(|p| is_local(p)).map(|p| env.media.image(p, 18.0 * ppp));
                        match tex {
                            Some(Tex::Ready { id, size }) => {
                                ui.add(egui::Image::from_texture((id, fit(size * 100.0, vec2(18.0, 18.0)))));
                            }
                            Some(Tex::Loading) => {
                                ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                            }
                            _ => {
                                ui.label(RichText::new(&r.name).size(14.0));
                            }
                        }
                        ui.label(RichText::new(r.count.to_string()).font(bold(13.0)).color(TEXT_MUTED));
                    });
                })
                .response;
            resp.on_hover_ui(|ui| {
                ui.set_max_width(260.0);
                let names: Vec<&str> = r.users.iter().take(25).map(|&u| a.user(u).display_name()).collect();
                let more = r.count as usize - names.len().min(r.count as usize);
                let mut text = names.join(", ");
                if more > 0 {
                    text.push_str(&format!(" and {more} more"));
                }
                ui.label(RichText::new(format!(":{}:", r.name.trim_matches(':'))).font(bold(13.0)));
                ui.label(text);
            });
        }
    });
}

fn stickers(ui: &mut Ui, env: &mut Env, list: &[crate::model::Sticker], msg: usize) {
    for s in list {
        ui.add_space(4.0);
        if s.lottie {
            ui.label(RichText::new(format!("[sticker: {}]", s.name)).italics().color(TEXT_MUTED));
        } else {
            image(ui, env, &s.url, vec2(160.0, 160.0), None, Some(msg)).on_hover_text(&s.name);
        }
    }
}

fn body(ui: &mut Ui, env: &mut Env, m: &Message, idx: usize) {
    content(ui, env, m.id, &m.content, &m.emojis, &m.mentions, m.edited.is_some());
    if let Some(f) = &m.forwarded {
        ui.add_space(2.0);
        let top = ui.cursor().top();
        let r = Frame::new().inner_margin(Margin { left: 14, right: 0, top: 2, bottom: 2 }).show(ui, |ui| {
            ui.label(RichText::new("↪ Forwarded").italics().size(13.0).color(TEXT_MUTED));
            content(ui, env, m.id, &f.content, &[], &[], false);
            attachments(ui, env, m.id, idx, &f.attachments, "fwd");
            for (i, e) in f.embeds.iter().enumerate() {
                embed(ui, env, e, m.id, idx, &format!("{}:fe{i}", m.id));
            }
            stickers(ui, env, &f.stickers, idx);
            if let Some(ts) = f.timestamp {
                ui.label(RichText::new(fmt_ts(ts)).size(11.5).color(TEXT_FAINT));
            }
        });
        let rr = r.response.rect;
        ui.painter().rect_filled(Rect::from_min_max(pos2(rr.left(), top), pos2(rr.left() + 4.0, rr.bottom())), 2, QUOTE_BAR);
    }
    attachments(ui, env, m.id, idx, &m.attachments, "");
    stickers(ui, env, &m.stickers, idx);
    for (i, e) in m.embeds.iter().enumerate() {
        embed(ui, env, e, m.id, idx, &format!("{}:e{i}", m.id));
    }
    reactions(ui, env, m);
}

fn snippet(m: &Message) -> String {
    let line: String = m.content.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return if !m.attachments.is_empty() || !m.embeds.is_empty() { "Click to see attachment 🖼".into() } else { "…".into() };
    }
    let mut s: String = line.chars().take(120).collect();
    if line.chars().count() > 120 {
        s.push('…');
    }
    s
}

/// Draws the "replying to" line; returns its vertical centre.
fn reply_preview(ui: &mut Ui, env: &mut Env, rid: u64) -> f32 {
    let a = env.archive;
    let r = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        match a.by_id.get(&rid) {
            Some(&(c, i)) => {
                let rm = a.msg(c as usize, i as usize);
                let u = a.user(rm.author);
                let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                avatar(ui, env, rm.author, rect);
                ui.label(RichText::new(format!("@{}", u.display_name())).font(bold(13.0)).color(u.color.unwrap_or(TEXT_STRONG)));
                ui.add(Label::new(RichText::new(snippet(rm)).size(13.0).color(TEXT_MUTED)).truncate().sense(Sense::click()))
            }
            None => ui.label(RichText::new("Original message was deleted").italics().size(13.0).color(TEXT_MUTED)),
        }
    });
    let resp = ui.interact(r.response.rect, ui.scope_id().with(("reply", rid)), Sense::click());
    if a.by_id.contains_key(&rid) && (resp.on_hover_cursor(CursorIcon::PointingHand).clicked() || r.inner.clicked()) {
        env.actions.push(Action::JumpTo(rid));
    }
    r.response.rect.center().y
}

/// One message row: avatar and header (unless grouped with the previous message), then the body.
/// `row_h` is the height measured last frame, used for hover and the context menu.
pub fn message(ui: &mut Ui, env: &mut Env, idx: usize, grouped: bool, highlight: f32, row_h: f32) {
    let a = env.archive;
    let m = a.msg(env.ch, idx);
    let full = ui.max_rect();
    let (left, right) = (full.left(), full.right());
    let top = ui.cursor().top();
    let bg = ui.painter().add(Shape::Noop);

    // Interact first so links and media drawn on top still get their clicks.
    if row_h > 0.0 {
        let row = Rect::from_min_max(pos2(left, top), pos2(right, top + row_h));
        let resp = ui.interact(row, ui.scope_id().with(("msg", m.id)), Sense::click());
        resp.context_menu(|ui| {
            if ui.button("Copy text").clicked() {
                ui.ctx().copy_text(m.content.clone());
                ui.close();
            }
            if ui.button("Copy message ID").clicked() {
                ui.ctx().copy_text(m.id.to_string());
                ui.close();
            }
            if let Some(r) = m.reply_to
                && a.by_id.contains_key(&r)
                && ui.button("Jump to replied message").clicked()
            {
                env.actions.push(Action::JumpTo(r));
                ui.close();
            }
        });
    }

    let gutter = if env.compact { 56.0 } else { 72.0 };
    let av = if env.compact { 32.0 } else { 40.0 };
    let pad_top = if grouped { 1.0 } else { 14.0 };
    let content_rect = Rect::from_min_max(pos2(left + gutter, top + pad_top), pos2(right - 16.0, top + 100_000.0));
    let mut cu = ui.new_child(UiBuilder::new().max_rect(content_rect).layout(Layout::top_down(Align::Min)));
    cu.spacing_mut().item_spacing.y = 2.0;

    let system_icon = match m.kind {
        MsgKind::Pinned => Some("📌"),
        MsgKind::Poll => Some("📊"),
        MsgKind::System => Some("➜"),
        _ => None,
    };
    let mut min_bottom = top;
    if let Some(icon) = system_icon {
        let u = a.user(m.author);
        let r = cu.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(RichText::new(u.display_name()).font(bold(BODY)).color(u.color.unwrap_or(TEXT_STRONG)));
            ui.label(RichText::new(m.content.trim()).color(TEXT_MUTED));
            if let Some(rid) = m.reply_to
                && a.by_id.contains_key(&rid)
            {
                let r = ui.add(Label::new(RichText::new("See message").color(LINK)).sense(Sense::click()));
                if r.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    env.actions.push(Action::JumpTo(rid));
                }
            }
            ui.label(RichText::new(fmt_ts(m.ts)).size(12.0).color(TEXT_MUTED)).on_hover_text(fmt_full(m.ts));
        });
        let color = if m.kind == MsgKind::Pinned { PINNED } else { TEXT_MUTED };
        cu.painter().text(
            pos2(left + gutter / 2.0, r.response.rect.top() + 10.0),
            Align2::CENTER_CENTER,
            icon,
            FontId::proportional(15.0),
            color,
        );
        // Polls and the like can carry embeds.
        for (i, e) in m.embeds.iter().enumerate() {
            embed(&mut cu, env, e, m.id, idx, &format!("{}:e{i}", m.id));
        }
    } else {
        if let Some(rid) = m.reply_to.filter(|_| m.kind == MsgKind::Reply) {
            let y = reply_preview(&mut cu, env, rid);
            // Discord's little "spine" from the avatar up to the reply line.
            let x0 = left + 16.0 + av / 2.0;
            let stroke = Stroke::new(2.0, DIVIDER);
            let pts = vec![pos2(x0, y + 10.0), pos2(x0, y + 2.0), pos2(x0 + 6.0, y - 2.0), pos2(left + gutter - 4.0, y - 2.0)];
            ui.painter().add(Shape::line(pts, stroke));
        }
        let header_top = cu.cursor().top();
        if !grouped {
            let u = a.user(m.author);
            cu.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let r = ui.add(
                    Label::new(RichText::new(u.display_name()).font(bold(BODY + 0.5)).color(u.color.unwrap_or(TEXT_STRONG)))
                        .sense(Sense::hover()),
                );
                r.on_hover_ui(|ui| user_card(ui, u));
                if u.is_bot {
                    Frame::new().fill(BLURPLE).corner_radius(3).inner_margin(Margin::symmetric(4, 0)).show(ui, |ui| {
                        ui.label(RichText::new("BOT").font(bold(10.0)).color(Color32::WHITE));
                    });
                }
                ui.label(RichText::new(fmt_ts(m.ts)).size(12.0).color(TEXT_MUTED)).on_hover_text(fmt_full(m.ts));
                if m.pinned {
                    ui.label(RichText::new("📌").size(12.0).color(PINNED)).on_hover_text("Pinned");
                }
            });
            let rect = Rect::from_min_size(pos2(left + 16.0, header_top + 1.0), vec2(av, av));
            avatar(ui, env, m.author, rect);
            let r = ui.interact(rect, ui.scope_id().with(("avatar", m.id)), Sense::hover());
            r.on_hover_ui(|ui| user_card(ui, u));
            min_bottom = rect.bottom();
        }
        body(&mut cu, env, m, idx);
        if grouped && ui.rect_contains_pointer(Rect::from_min_max(pos2(left, top), pos2(right, top + row_h.max(20.0)))) {
            ui.painter().text(
                pos2(left + gutter - 10.0, header_top + 10.0),
                Align2::RIGHT_CENTER,
                fmt_time(m.ts),
                FontId::proportional(11.0),
                TEXT_FAINT,
            );
        }
    }

    let bottom = cu.min_rect().bottom().max(min_bottom) + 3.0;
    let row = Rect::from_min_max(pos2(left, top), pos2(right, bottom));
    ui.advance_cursor_after_rect(row);
    if highlight > 0.0 {
        ui.painter().set(bg, Shape::rect_filled(row, 0, HIGHLIGHT.gamma_multiply(highlight)));
        ui.painter().add(Shape::rect_filled(
            Rect::from_min_max(row.min, pos2(left + 2.0, bottom)),
            0,
            PINNED.gamma_multiply(highlight),
        ));
    } else if ui.rect_contains_pointer(row) && !env.compact {
        ui.painter().set(bg, Shape::rect_filled(row, 0, BG_HOVER));
    }
}
