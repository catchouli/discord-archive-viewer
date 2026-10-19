//! A virtualised, variable-height message list.
//!
//! Only rows near the viewport are laid out. Each row's height is measured when
//! drawn and estimated otherwise. Scroll position is anchored to the top visible
//! row, so rows above changing height (images loading) don't make the view jump.

use chrono::NaiveDate;
use egui::{Align, Align2, Button, Color32, FontId, Layout, Rect, RichText, ScrollArea, Ui, UiBuilder, pos2, vec2};

use crate::model::{Archive, ChannelKind, MsgKind, local_date};
use crate::render::{self, Env};
use crate::theme::*;

pub enum Row {
    Start,
    End,
    Divider(NaiveDate),
    Msg { idx: u32, grouped: bool },
}

#[derive(Clone, Copy)]
pub enum Target {
    Bottom,
    Row(usize),
}

pub struct ChannelView {
    pub ch: usize,
    rows: Vec<Row>,
    msg_row: Vec<u32>,
    heights: Vec<f32>,
    prefix: Vec<f32>,
    dirty: bool,
    width: f32,
    /// (row, offset into it, row's prefix when recorded)
    anchor: Option<(usize, f32, f32)>,
    at_bottom: bool,
    pub scroll_to: Option<(Target, u8)>,
}

const GROUP_MS: i64 = 7 * 60 * 1000;

impl ChannelView {
    pub fn new(a: &Archive, ch: usize) -> Self {
        let msgs = &a.channels[ch].messages;
        let mut rows = vec![Row::Start];
        let mut msg_row = Vec::with_capacity(msgs.len());
        let mut last_date = None;
        for (i, m) in msgs.iter().enumerate() {
            let date = local_date(m.ts);
            let new_day = last_date != Some(date);
            if new_day {
                rows.push(Row::Divider(date));
                last_date = Some(date);
            }
            let grouped = i > 0 && !new_day && {
                let p = &msgs[i - 1];
                p.author == m.author
                    && m.ts - p.ts < GROUP_MS
                    && matches!(m.kind, MsgKind::Normal)
                    && matches!(p.kind, MsgKind::Normal | MsgKind::Reply)
            };
            msg_row.push(rows.len() as u32);
            rows.push(Row::Msg { idx: i as u32, grouped });
        }
        rows.push(Row::End);
        let mut v = Self {
            ch,
            heights: Vec::new(),
            prefix: Vec::new(),
            rows,
            msg_row,
            dirty: true,
            width: 800.0,
            anchor: None,
            at_bottom: true,
            scroll_to: Some((Target::Bottom, 30)),
        };
        v.heights = (0..v.rows.len()).map(|r| v.estimate(a, r)).collect();
        v
    }

    pub fn jump_to_message(&mut self, msg_idx: usize) {
        if let Some(&r) = self.msg_row.get(msg_idx) {
            self.scroll_to = Some((Target::Row(r as usize), 30));
            self.at_bottom = false;
        }
    }

    pub fn jump_to_bottom(&mut self) {
        self.scroll_to = Some((Target::Bottom, 30));
    }

    fn estimate(&self, a: &Archive, r: usize) -> f32 {
        match &self.rows[r] {
            Row::Start => 170.0,
            Row::End => 24.0,
            Row::Divider(_) => 40.0,
            Row::Msg { idx, grouped } => {
                let m = &a.channels[self.ch].messages[*idx as usize];
                let chars_per_line = ((self.width - 90.0) / 7.5).max(20.0);
                let lines: f32 = m.content.lines().map(|l| (l.chars().count() as f32 / chars_per_line).ceil().max(1.0)).sum();
                let mut h = if *grouped { 4.0 } else { 38.0 } + lines * LINE_H;
                for att in &m.attachments {
                    h += match att.kind {
                        crate::model::MediaKind::Image | crate::model::MediaKind::Video => 300.0,
                        _ => 70.0,
                    };
                }
                for e in &m.embeds {
                    h += if e.video.is_some() || !e.images.is_empty() || e.is_plain_media() { 320.0 } else { 120.0 };
                }
                if !m.reactions.is_empty() {
                    h += 30.0;
                }
                if m.reply_to.is_some() {
                    h += 22.0;
                }
                h
            }
        }
    }

    fn update_prefix(&mut self) {
        if !self.dirty && self.prefix.len() == self.rows.len() + 1 {
            return;
        }
        self.prefix.clear();
        self.prefix.reserve(self.rows.len() + 1);
        let mut acc = 0.0;
        self.prefix.push(0.0);
        for h in &self.heights {
            acc += h;
            self.prefix.push(acc);
        }
        self.dirty = false;
    }

    fn row_at(&self, y: f32) -> usize {
        let n = self.rows.len();
        self.prefix.partition_point(|&p| p <= y).saturating_sub(1).min(n.saturating_sub(1))
    }

    fn total(&self) -> f32 {
        self.prefix.last().copied().unwrap_or(0.0)
    }
}

fn start_header(ui: &mut Ui, a: &Archive, ch: usize) {
    let c = &a.channels[ch];
    ui.add_space(40.0);
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        ui.vertical(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(68.0, 68.0), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), 34.0, BG_CHANNEL_SELECTED);
            let icon = match c.kind {
                ChannelKind::Voice => "🔊",
                ChannelKind::Thread => "🧵",
                ChannelKind::Text => "#",
            };
            ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, FontId::proportional(38.0), Color32::WHITE);
            ui.add_space(8.0);
            ui.label(RichText::new(format!("Welcome to #{}!", c.name)).font(bold(30.0)).color(TEXT_STRONG));
            ui.label(RichText::new(format!("This is the start of the #{} channel.", c.name)).color(TEXT_MUTED));
            if let Some(t) = &c.topic {
                ui.label(RichText::new(t).color(TEXT_MUTED));
            }
            if let (Some(f), Some(l)) = (c.messages.first(), c.messages.last()) {
                ui.label(
                    RichText::new(format!(
                        "{} messages, {} to {}",
                        c.messages.len(),
                        render::fmt_date(local_date(f.ts)),
                        render::fmt_date(local_date(l.ts))
                    ))
                    .size(12.5)
                    .color(TEXT_FAINT),
                );
            }
        });
    });
    ui.add_space(8.0);
}

fn divider(ui: &mut Ui, d: NaiveDate) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), egui::Sense::hover());
    let y = rect.center().y + 6.0;
    let text = render::fmt_date(d);
    let galley = ui.painter().layout_no_wrap(text, bold(12.0), TEXT_MUTED);
    let w = galley.size().x + 16.0;
    let c = rect.center().x;
    let line = |x0: f32, x1: f32| ui.painter().hline(x0..=x1, y, egui::Stroke::new(1.0, DIVIDER));
    line(rect.left() + 16.0, c - w / 2.0);
    line(c + w / 2.0, rect.right() - 16.0);
    ui.painter().galley(pos2(c - galley.size().x / 2.0, y - galley.size().y / 2.0), galley, TEXT_MUTED);
}

/// Draws the message list for `view.ch`. `highlight` is (message id, strength 0..1).
pub fn show(ui: &mut Ui, view: &mut ChannelView, env: &mut Env, highlight: Option<(u64, f32)>) {
    let a = env.archive;
    let width = ui.available_width();
    if (width - view.width).abs() > 0.5 {
        view.width = width;
        // Unmeasured rows keep stale estimates; measured rows re-measure as they're drawn.
    }
    view.update_prefix();
    let total = view.total();
    let vp_h = ui.available_height();
    let max_offset = (total - vp_h).max(0.0);

    let mut forced = None;
    let sticking = view.at_bottom || matches!(view.scroll_to, Some((Target::Bottom, _)));
    if let Some((target, frames)) = view.scroll_to {
        forced = Some(match target {
            Target::Bottom => max_offset,
            Target::Row(r) => (view.prefix[r] - vp_h * 0.3).clamp(0.0, max_offset),
        });
        view.scroll_to = if frames > 1 { Some((target, frames - 1)) } else { None };
    } else if view.at_bottom {
        forced = Some(max_offset);
    } else if let Some((r, intra, old)) = view.anchor
        && r < view.rows.len()
        && (view.prefix[r] - old).abs() > 0.5
    {
        forced = Some((view.prefix[r] + intra).clamp(0.0, max_offset));
    }

    let mut sa = ScrollArea::vertical().id_salt(("channel", view.ch)).auto_shrink([false, false]).animated(false);
    if let Some(off) = forced {
        sa = sa.vertical_scroll_offset(off);
    }
    let list_rect = ui.available_rect_before_wrap();
    let out = sa.show_viewport(ui, |ui, vp| {
        ui.set_min_height(total);
        let origin = ui.max_rect().min;
        let n = view.rows.len();
        if n == 0 {
            return false;
        }
        let first = view.row_at(vp.min.y).saturating_sub(1);
        let last = (view.row_at(vp.max.y) + 1).min(n - 1);
        let mut changed = false;
        for r in first..=last {
            let y = origin.y + view.prefix[r];
            let rect = Rect::from_min_size(pos2(origin.x, y), vec2(width, 100_000.0));
            let mut row_ui = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)).id_salt(("row", r)));
            match &view.rows[r] {
                Row::Start => start_header(&mut row_ui, a, view.ch),
                Row::End => {
                    row_ui.allocate_exact_size(vec2(width, 24.0), egui::Sense::hover());
                }
                Row::Divider(d) => divider(&mut row_ui, *d),
                Row::Msg { idx, grouped } => {
                    let m = &a.channels[view.ch].messages[*idx as usize];
                    let hl = highlight.filter(|h| h.0 == m.id).map(|h| h.1).unwrap_or(0.0);
                    render::message(&mut row_ui, env, *idx as usize, *grouped, hl, view.heights[r]);
                }
            }
            let h = (row_ui.min_rect().bottom() - y).max(1.0);
            if (h - view.heights[r]).abs() > 0.5 {
                view.heights[r] = h;
                changed = true;
            }
        }
        changed
    });
    if out.inner {
        view.dirty = true;
        ui.ctx().request_repaint();
    }

    let offset = out.state.clamped_offset().y;
    // Stay stuck to the bottom while content above grows (images loading), until the user scrolls.
    // The scroll area clamps to last frame's height, so the offset alone can't tell us.
    let user_scrolled = ui.input(|i| i.smooth_scroll_delta.y != 0.0 || i.key_pressed(egui::Key::PageUp))
        || (ui.input(|i| i.pointer.primary_down()) && ui.rect_contains_pointer(list_rect));
    view.at_bottom = match view.scroll_to {
        Some((Target::Row(_), _)) => false,
        _ => (sticking && !user_scrolled) || offset >= max_offset - 4.0,
    };
    let r = view.row_at(offset);
    view.anchor = Some((r, offset - view.prefix[r], view.prefix[r]));

    // "Jump to present" when scrolled well up.
    if max_offset - offset > vp_h * 2.0 && view.scroll_to.is_none() {
        let size = vec2(180.0, 30.0);
        let pos = pos2(list_rect.center().x - size.x / 2.0, list_rect.bottom() - size.y - 16.0);
        let btn = Button::new(RichText::new("Jump to present  ↓").font(bold(13.0)).color(Color32::WHITE))
            .fill(BLURPLE)
            .corner_radius(15);
        if ui.put(Rect::from_min_size(pos, size), btn).clicked() {
            view.jump_to_bottom();
        }
    }
}
