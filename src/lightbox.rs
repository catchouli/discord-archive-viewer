//! Full-screen media viewer: zoom and pan images, play videos, and step through
//! every image and video in the channel with the arrow keys.

use egui::{Align2, Color32, CursorIcon, FontId, Id, Key, Order, Rect, RichText, Sense, Vec2, pos2, vec2};

use crate::media::{MediaCache, Tex};
use crate::model::{Archive, MediaKind, MediaRef, is_local};
use crate::render::{self, Action, Env};
use crate::theme::*;

pub struct Item {
    pub path: MediaRef,
    pub video: bool,
    pub msg: usize,
    pub name: String,
    pub poster: Option<MediaRef>,
}

pub struct Lightbox {
    pub ch: usize,
    pub items: Vec<Item>,
    pub idx: usize,
    zoom: f32,
    pan: Vec2,
}

fn file_name(p: &str) -> String {
    p.rsplit(['/', '\\']).next().unwrap_or(p).to_string()
}

/// Every viewable image and video in a channel, in message order.
pub fn collect(a: &Archive, media: &mut MediaCache, ch: usize) -> Vec<Item> {
    let mut items = Vec::new();
    for (mi, m) in a.channels[ch].messages.iter().enumerate() {
        let atts = m.attachments.iter().chain(m.forwarded.iter().flat_map(|f| f.attachments.iter()));
        for att in atts {
            let kind = if att.kind == MediaKind::Other { media.kind(&att.url, Some(&att.file_name)) } else { att.kind };
            if matches!(kind, MediaKind::Image | MediaKind::Svg | MediaKind::Video) && is_local(&att.url) {
                items.push(Item {
                    path: att.url.clone(),
                    video: kind == MediaKind::Video,
                    msg: mi,
                    name: att.file_name.clone(),
                    poster: None,
                });
            }
        }
        let embeds = m.embeds.iter().chain(m.forwarded.iter().flat_map(|f| f.embeds.iter()));
        for e in embeds {
            let poster = e.thumbnail.as_ref().map(|t| t.url.clone()).filter(|p| is_local(p));
            if let Some(v) = e.video.as_ref().filter(|v| crate::render::playable(media, v)) {
                items.push(Item { path: v.url.clone(), video: true, msg: mi, name: file_name(&v.url), poster });
                continue;
            }
            for img in e.images.iter().filter(|i| is_local(&i.url)) {
                items.push(Item { path: img.url.clone(), video: false, msg: mi, name: file_name(&img.url), poster: None });
            }
            if e.images.is_empty()
                && let Some(t) = poster
            {
                items.push(Item { name: file_name(&t), path: t, video: false, msg: mi, poster: None });
            }
        }
        for s in m.stickers.iter().filter(|s| !s.lottie && is_local(&s.url)) {
            items.push(Item { path: s.url.clone(), video: false, msg: mi, name: s.name.clone(), poster: None });
        }
    }
    items
}

impl Lightbox {
    pub fn open(a: &Archive, media: &mut MediaCache, ch: usize, msg: usize, path: &MediaRef) -> Option<Self> {
        let items = collect(a, media, ch);
        let idx = items
            .iter()
            .position(|i| i.msg == msg && (i.path == *path || i.poster.as_ref() == Some(path)))
            .or_else(|| items.iter().position(|i| i.msg == msg))?;
        Some(Self { ch, items, idx, zoom: 1.0, pan: Vec2::ZERO })
    }

    /// Opens at the n-th image or video (or only videos) in the channel; negative n counts from the end.
    pub fn open_nth(a: &Archive, media: &mut MediaCache, ch: usize, n: isize, video_only: bool) -> Option<Self> {
        let items = collect(a, media, ch);
        let candidates: Vec<usize> = (0..items.len()).filter(|&i| !video_only || items[i].video).collect();
        let k = if n < 0 { candidates.len() as isize + n } else { n };
        let idx = *candidates.get(usize::try_from(k).ok()?)?;
        Some(Self { ch, items, idx, zoom: 1.0, pan: Vec2::ZERO })
    }

    pub fn next(&mut self) {
        self.go(1);
    }

    fn go(&mut self, delta: isize) {
        let n = self.items.len() as isize;
        let next = (self.idx as isize + delta).clamp(0, n - 1) as usize;
        if next != self.idx {
            self.idx = next;
            self.zoom = 1.0;
            self.pan = Vec2::ZERO;
        }
    }

    /// Returns false when the viewer should close.
    pub fn show(&mut self, ctx: &egui::Context, env: &mut Env) -> bool {
        let mut keep = true;
        let (left, right, esc) = ctx.input(|i| (i.key_pressed(Key::ArrowLeft), i.key_pressed(Key::ArrowRight), i.key_pressed(Key::Escape)));
        if esc {
            return false;
        }
        if left {
            self.go(-1);
        }
        if right {
            self.go(1);
        }
        let screen = ctx.content_rect();
        let a = env.archive;
        egui::Area::new(Id::unique("lightbox")).order(Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            ui.set_min_size(screen.size());
            let painter = ui.painter().clone();
            painter.rect_filled(screen, 0, Color32::from_black_alpha(250));
            let bg = ui.interact(screen, Id::unique("lightbox-bg"), Sense::click_and_drag());

            let item = &self.items[self.idx];
            let m = a.msg(self.ch, item.msg);
            let author = a.user(m.author);

            // Top bar: who/when/what, and buttons.
            let bar = Rect::from_min_size(screen.min, vec2(screen.width(), 52.0));
            let mut close = false;
            ui.scope_builder(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(16.0, 8.0))), |ui| {
                ui.horizontal_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                    render::avatar(ui, env, m.author, r);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(author.display_name()).font(bold(14.0)).color(author.color.unwrap_or(TEXT_STRONG)));
                        ui.label(
                            RichText::new(format!("{}  •  {}", render::fmt_full(m.ts), item.name)).size(12.0).color(TEXT_MUTED),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new("✕").size(16.0)).on_hover_text("Close (Esc)").clicked() {
                            close = true;
                        }
                        if ui.button("Show in folder").clicked() {
                            env.actions.push(Action::Reveal(item.path.to_string()));
                        }
                        if ui.button("Open externally").clicked() {
                            env.actions.push(Action::Open(item.path.to_string()));
                        }
                        if ui.button("Jump to message").clicked() {
                            env.actions.push(Action::JumpTo(m.id));
                            close = true;
                        }
                        ui.label(RichText::new(format!("{} / {}", self.idx + 1, self.items.len())).color(TEXT_MUTED));
                    });
                });
            });

            let area = Rect::from_min_max(pos2(screen.left() + 64.0, bar.bottom() + 8.0), pos2(screen.right() - 64.0, screen.bottom() - 24.0));

            // Prev/next arrows.
            for (dir, x, glyph) in [(-1isize, screen.left() + 32.0, "‹"), (1, screen.right() - 32.0, "›")] {
                let enabled = if dir < 0 { self.idx > 0 } else { self.idx + 1 < self.items.len() };
                if !enabled {
                    continue;
                }
                let r = Rect::from_center_size(pos2(x, area.center().y), vec2(44.0, 44.0));
                let resp = ui.interact(r, Id::unique(("lb-nav", dir)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
                painter.circle_filled(r.center(), 22.0, if resp.hovered() { BG_CHANNEL_SELECTED } else { BG_SIDEBAR });
                painter.text(r.center() - vec2(0.0, 3.0), Align2::CENTER_CENTER, glyph, FontId::proportional(32.0), TEXT_STRONG);
                if resp.clicked() {
                    self.go(dir);
                }
            }

            let item = &self.items[self.idx];
            if item.video {
                let key = format!("lightbox:{}", item.path);
                let path = item.path.clone();
                let poster = item.poster.clone();
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Center)));
                // Keep the frame vertically centred.
                let natural = env.players.get(&key).and_then(|p| p.video_size()).map(|s| [s.x as u32, s.y as u32]);
                let est = natural.map(|[w, h]| vec2(w as f32, h as f32)).unwrap_or(vec2(16.0, 9.0));
                let s = (area.width() / est.x).min(area.height() / est.y);
                child.add_space(((area.height() - est.y * s) / 2.0).max(0.0));
                if env.players.get(&key).is_none() {
                    env.players.start(&key, &path, true);
                }
                render::video(&mut child, env, &key, &path, poster.as_ref(), natural, area.size(), false, None);
            } else {
                let ppp = ctx.pixels_per_point();
                let natural = env.media.dims(&item.path).map(|[w, h]| vec2(w as f32, h as f32)).unwrap_or(vec2(800.0, 600.0));
                // Fit to the screen, but don't blow small images up more than 2x.
                let base = (area.width() / natural.x).min(area.height() / natural.y).min(2.0);
                let size = natural * base * self.zoom;

                if bg.hovered() || ui.rect_contains_pointer(area) {
                    let (scroll, zoom_delta, pointer) = ctx.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta(), i.pointer.hover_pos()));
                    let factor = zoom_delta * (scroll * 0.003).exp();
                    if (factor - 1.0).abs() > 1e-4 {
                        let new_zoom = (self.zoom * factor).clamp(1.0, 20.0);
                        let real = new_zoom / self.zoom;
                        if let Some(p) = pointer {
                            let c = area.center() + self.pan;
                            self.pan += (p - c) * (1.0 - real);
                        }
                        self.zoom = new_zoom;
                        if self.zoom <= 1.0 {
                            self.pan = Vec2::ZERO;
                        }
                    }
                }
                if bg.dragged() && self.zoom > 1.0 {
                    self.pan += bg.drag_delta();
                }
                if bg.double_clicked() {
                    if self.zoom > 1.0 {
                        self.zoom = 1.0;
                        self.pan = Vec2::ZERO;
                    } else {
                        self.zoom = 2.5;
                    }
                }
                let rect = Rect::from_center_size(area.center() + self.pan, size);
                match env.media.image(&item.path, (size.max_elem() * ppp).min(4096.0)) {
                    Tex::Ready { id, .. } => {
                        let clip = ui.clip_rect();
                        ui.set_clip_rect(area.expand(24.0));
                        egui::Image::from_texture((id, size)).paint_at(ui, rect);
                        ui.set_clip_rect(clip);
                    }
                    Tex::Loading => {
                        painter.text(area.center(), Align2::CENTER_CENTER, "Loading…", FontId::proportional(16.0), TEXT_MUTED);
                    }
                    Tex::Failed => {
                        painter.text(area.center(), Align2::CENTER_CENTER, "Can't display this file", FontId::proportional(16.0), TEXT_MUTED);
                    }
                }
                if bg.clicked() && !rect.contains(bg.interact_pointer_pos().unwrap_or_default()) && !bg.double_clicked() {
                    close = true;
                }
                if self.zoom > 1.0 {
                    ui.ctx().set_cursor_icon(if bg.dragged() { CursorIcon::Grabbing } else { CursorIcon::Grab });
                }
            }
            if close {
                keep = false;
            }
        });
        keep
    }
}
