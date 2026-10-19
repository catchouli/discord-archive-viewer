//! Discord-ish dark theme colours and fonts.

use std::sync::Arc;

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId};

pub const BG_DARKEST: Color32 = Color32::from_rgb(0x1E, 0x1F, 0x22);
pub const BG_SIDEBAR: Color32 = Color32::from_rgb(0x2B, 0x2D, 0x31);
pub const BG_MAIN: Color32 = Color32::from_rgb(0x31, 0x33, 0x38);
pub const BG_HOVER: Color32 = Color32::from_rgb(0x2E, 0x30, 0x35);
pub const BG_CHANNEL_HOVER: Color32 = Color32::from_rgb(0x35, 0x37, 0x3C);
pub const BG_CHANNEL_SELECTED: Color32 = Color32::from_rgb(0x40, 0x42, 0x49);
pub const BG_EMBED: Color32 = Color32::from_rgb(0x2B, 0x2D, 0x31);
pub const BG_CODE: Color32 = Color32::from_rgb(0x2B, 0x2D, 0x31);
pub const BG_INPUT: Color32 = Color32::from_rgb(0x1E, 0x1F, 0x22);
pub const BORDER: Color32 = Color32::from_rgb(0x1F, 0x20, 0x23);
pub const DIVIDER: Color32 = Color32::from_rgb(0x3F, 0x41, 0x47);

pub const TEXT: Color32 = Color32::from_rgb(0xDB, 0xDE, 0xE1);
pub const TEXT_STRONG: Color32 = Color32::from_rgb(0xF2, 0xF3, 0xF5);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x94, 0x9B, 0xA4);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x6D, 0x6F, 0x78);
pub const LINK: Color32 = Color32::from_rgb(0x00, 0xA8, 0xFC);
pub const BLURPLE: Color32 = Color32::from_rgb(0x58, 0x65, 0xF2);
pub const MENTION_BG: Color32 = Color32::from_rgba_premultiplied(0x2C, 0x32, 0x78, 0x78);
pub const MENTION_FG: Color32 = Color32::from_rgb(0xC9, 0xCD, 0xFB);
pub const SPOILER: Color32 = Color32::from_rgb(0x1E, 0x1F, 0x22);
pub const SPOILER_REVEALED: Color32 = Color32::from_rgb(0x41, 0x43, 0x4A);
pub const HIGHLIGHT: Color32 = Color32::from_rgba_premultiplied(0x3A, 0x33, 0x14, 0x60);
pub const QUOTE_BAR: Color32 = Color32::from_rgb(0x4E, 0x50, 0x58);
pub const PINNED: Color32 = Color32::from_rgb(0xF0, 0xB2, 0x32);

pub const BODY: f32 = 15.0;
pub const LINE_H: f32 = 20.0;

pub fn bold_family() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn body() -> FontId {
    FontId::proportional(BODY)
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, bold_family())
}

fn try_font(path: &str, index: u32) -> Option<FontData> {
    let bytes = std::fs::read(path).ok()?;
    let mut f = FontData::from_owned(bytes);
    f.index = index;
    Some(f)
}

/// Uses system fonts when present (for a cleaner look and wide Unicode coverage),
/// falling back to egui's built-in fonts. Emoji fall through to egui's emoji fonts.
pub fn fonts() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let builtin_prop = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let builtin_mono = defs.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();

    let candidates: &[(&str, &str, u32)] = &[
        ("regular", "/System/Library/Fonts/Supplemental/Arial.ttf", 0),
        ("bold", "/System/Library/Fonts/Supplemental/Arial Bold.ttf", 0),
        ("unicode", "/Library/Fonts/Arial Unicode.ttf", 0),
        ("unicode", "/System/Library/Fonts/Supplemental/Arial Unicode.ttf", 0),
        ("myanmar", "/System/Library/Fonts/NotoSansMyanmar.ttc", 0),
        ("mono", "/System/Library/Fonts/Menlo.ttc", 0),
        // Linux
        ("regular", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 0),
        ("bold", "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", 0),
        ("unicode", "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
        // Windows
        ("regular", "C:\\Windows\\Fonts\\segoeui.ttf", 0),
        ("bold", "C:\\Windows\\Fonts\\segoeuib.ttf", 0),
        ("unicode", "C:\\Windows\\Fonts\\msyh.ttc", 0),
    ];
    let mut loaded = std::collections::HashSet::new();
    for (name, path, index) in candidates {
        if loaded.contains(name) {
            continue;
        }
        if let Some(f) = try_font(path, *index) {
            defs.font_data.insert(name.to_string(), Arc::new(f));
            loaded.insert(*name);
        }
    }
    let has = |n: &str| loaded.contains(n);

    let fallbacks: Vec<String> = ["unicode", "myanmar"].iter().filter(|n| has(n)).map(|s| s.to_string()).collect();
    let mut prop = Vec::new();
    if has("regular") {
        prop.push("regular".to_string());
    }
    prop.extend(builtin_prop.iter().cloned());
    prop.extend(fallbacks.iter().cloned());

    let mut bold = Vec::new();
    if has("bold") {
        bold.push("bold".to_string());
    }
    bold.extend(prop.iter().cloned());

    let mut mono = Vec::new();
    if has("mono") {
        mono.push("mono".to_string());
    }
    mono.extend(builtin_mono);
    mono.extend(fallbacks);

    defs.families.insert(FontFamily::Proportional, prop);
    defs.families.insert(FontFamily::Monospace, mono);
    defs.families.insert(bold_family(), bold);
    defs
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|s| {
        s.visuals.panel_fill = BG_MAIN;
        s.visuals.window_fill = BG_SIDEBAR;
        s.visuals.extreme_bg_color = BG_INPUT;
        s.visuals.override_text_color = None;
        s.visuals.widgets.noninteractive.fg_stroke.color = TEXT;
        s.visuals.hyperlink_color = LINK;
        s.visuals.selection.bg_fill = Color32::from_rgb(0x3D, 0x4B, 0x9E);
        s.spacing.item_spacing = egui::vec2(6.0, 2.0);
        s.spacing.scroll.bar_width = 8.0;
        s.spacing.scroll.floating = false;
        s.text_styles.insert(egui::TextStyle::Body, body());
        s.text_styles.insert(egui::TextStyle::Button, FontId::proportional(14.0));
        s.text_styles.insert(egui::TextStyle::Small, FontId::proportional(12.0));
        s.text_styles.insert(egui::TextStyle::Monospace, FontId::monospace(13.5));
    });
}
