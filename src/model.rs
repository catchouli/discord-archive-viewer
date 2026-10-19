//! Loading a DiscordChatExporter JSON export into memory.
//!
//! The export directory looks like `<category>/<channel> [<id>].json`, plus a
//! `server/` folder written by `server_assets.py` (guild info, channel order).
//! Media paths in the JSON are absolute paths into the shared `data/media/`
//! folder; if the data has been moved they are remapped onto `media_dir`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Local, NaiveDate, TimeZone};
use egui::Color32;
use rayon::prelude::*;
use serde::Deserialize;

/// A media file reference: a local path (usually) or a remote URL that wasn't downloaded.
pub type MediaRef = Arc<str>;

pub fn is_local(m: &str) -> bool {
    !(m.starts_with("http://") || m.starts_with("https://"))
}

// ---------------------------------------------------------------------------
// Raw JSON shapes (only the fields we use)

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawExport {
    channel: RawChannel,
    #[serde(default)]
    messages: Vec<RawMessage>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawChannel {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    category_id: Option<String>,
    category: Option<String>,
    name: String,
    topic: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawMessage {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    timestamp: String,
    timestamp_edited: Option<String>,
    is_pinned: bool,
    content: String,
    author: RawUser,
    attachments: Vec<RawAttachment>,
    embeds: Vec<RawEmbed>,
    stickers: Vec<RawSticker>,
    reactions: Vec<RawReaction>,
    mentions: Vec<RawUser>,
    inline_emojis: Vec<RawEmoji>,
    reference: Option<RawRef>,
    forwarded_message: Option<RawForward>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
struct RawUser {
    id: String,
    name: String,
    nickname: Option<String>,
    color: Option<String>,
    is_bot: bool,
    avatar_url: Option<String>,
    roles: Vec<RawRole>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
struct RawRole {
    name: String,
    color: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawAttachment {
    url: String,
    file_name: String,
    file_size_bytes: u64,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawEmbed {
    title: Option<String>,
    url: Option<String>,
    description: Option<String>,
    color: Option<String>,
    author: Option<RawEmbedAuthor>,
    thumbnail: Option<RawEmbedMedia>,
    image: Option<RawEmbedMedia>,
    video: Option<RawEmbedMedia>,
    images: Vec<RawEmbedMedia>,
    fields: Vec<RawField>,
    footer: Option<RawFooter>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawEmbedAuthor {
    name: Option<String>,
    url: Option<String>,
    icon_url: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawEmbedMedia {
    url: String,
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawField {
    name: String,
    value: String,
    is_inline: bool,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawFooter {
    text: String,
    icon_url: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawSticker {
    name: String,
    format: String,
    source_url: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawReaction {
    emoji: RawEmoji,
    count: u32,
    users: Vec<RawUser>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawEmoji {
    id: Option<String>,
    name: String,
    image_url: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawRef {
    message_id: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct RawForward {
    timestamp: Option<String>,
    content: String,
    attachments: Vec<RawAttachment>,
    embeds: Vec<RawEmbed>,
    stickers: Vec<RawSticker>,
}

/// One entry of `server/channels.json` (straight from the Discord API).
#[derive(Deserialize, Default)]
#[serde(default)]
struct ApiChannel {
    id: String,
    #[serde(rename = "type")]
    kind: i64,
    name: String,
    position: i64,
    parent_id: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ApiGuild {
    name: String,
}

// ---------------------------------------------------------------------------
// In-memory model

pub struct User {
    pub name: String,
    pub nickname: Option<String>,
    pub color: Option<Color32>,
    pub is_bot: bool,
    pub avatar: Option<MediaRef>,
    pub roles: Vec<(String, Option<Color32>)>,
}

impl User {
    pub fn display_name(&self) -> &str {
        self.nickname.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Svg,
    Other,
}

pub struct Attachment {
    pub url: MediaRef,
    pub file_name: String,
    pub size: u64,
    /// Guessed from the file name; `Other` may still turn out to be media once sniffed.
    pub kind: MediaKind,
}

#[derive(Clone)]
pub struct EmbedMedia {
    pub url: MediaRef,
    pub size: Option<[u32; 2]>,
}

pub struct Embed {
    pub title: Option<String>,
    pub url: Option<String>,
    pub description: Option<String>,
    pub color: Option<Color32>,
    pub author_name: Option<String>,
    pub author_url: Option<String>,
    pub author_icon: Option<MediaRef>,
    pub thumbnail: Option<EmbedMedia>,
    pub images: Vec<EmbedMedia>,
    pub video: Option<EmbedMedia>,
    pub fields: Vec<(String, String, bool)>,
    pub footer: Option<(String, Option<MediaRef>)>,
}

impl Embed {
    /// Tenor/Giphy/imgur style "gifv" embeds: a looping, muted, autoplaying video.
    pub fn is_gifv(&self) -> bool {
        self.video.is_some()
            && self.url.as_deref().is_some_and(|u| {
                u.contains("tenor.com") || u.contains("giphy.com") || u.contains("imgur.com") || u.ends_with(".gif")
            })
    }

    /// A bare image link: Discord shows these as just the image.
    pub fn is_plain_media(&self) -> bool {
        self.title.is_none()
            && self.description.is_none()
            && self.author_name.is_none()
            && self.fields.is_empty()
            && self.footer.is_none()
            && (self.thumbnail.is_some() || self.video.is_some() || !self.images.is_empty())
    }
}

pub struct Sticker {
    pub name: String,
    pub url: MediaRef,
    pub lottie: bool,
}

pub struct Reaction {
    pub name: String,
    pub image: Option<MediaRef>,
    pub count: u32,
    pub users: Vec<u32>,
}

pub struct InlineEmoji {
    /// Text that appears in the message content: the emoji itself, or `:name:` for custom emoji.
    pub pattern: String,
    pub image: Option<MediaRef>,
}

pub struct Forwarded {
    pub timestamp: Option<i64>,
    pub content: String,
    pub attachments: Vec<Attachment>,
    pub embeds: Vec<Embed>,
    pub stickers: Vec<Sticker>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MsgKind {
    Normal,
    Reply,
    Pinned,
    Poll,
    System,
}

pub mod flags {
    pub const IMAGE: u8 = 1;
    pub const VIDEO: u8 = 2;
    pub const AUDIO: u8 = 4;
    pub const FILE: u8 = 8;
    pub const LINK: u8 = 16;
    pub const EMBED: u8 = 32;
    pub const STICKER: u8 = 64;
    pub const REACTION: u8 = 128;
}

pub struct Message {
    pub id: u64,
    pub kind: MsgKind,
    /// Unix milliseconds, UTC.
    pub ts: i64,
    pub edited: Option<i64>,
    pub pinned: bool,
    pub content: String,
    pub author: u32,
    pub attachments: Vec<Attachment>,
    pub embeds: Vec<Embed>,
    pub stickers: Vec<Sticker>,
    pub reactions: Vec<Reaction>,
    pub mentions: Vec<u32>,
    pub emojis: Vec<InlineEmoji>,
    pub reply_to: Option<u64>,
    pub forwarded: Option<Box<Forwarded>>,
    /// Lowercased text used by search: content, embed text, file names.
    pub haystack: Box<str>,
    pub flags: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Text,
    Voice,
    Thread,
}

pub struct Channel {
    pub id: u64,
    pub name: String,
    pub kind: ChannelKind,
    pub topic: Option<String>,
    pub messages: Vec<Message>,
    /// For threads: the index of the parent channel, if it was exported.
    pub parent: Option<usize>,
}

pub struct SidebarChannel {
    pub channel: usize,
    pub threads: Vec<usize>,
}

pub struct SidebarCategory {
    pub id: u64,
    pub name: String,
    pub channels: Vec<SidebarChannel>,
    pub archive: bool,
}

pub struct Archive {
    pub guild_name: String,
    pub guild_icon: Option<MediaRef>,
    pub channels: Vec<Channel>,
    pub users: Vec<User>,
    pub sidebar: Vec<SidebarCategory>,
    /// message id -> (channel index, message index)
    pub by_id: HashMap<u64, (u32, u32)>,
    pub channel_by_name: HashMap<String, usize>,
    pub total_messages: usize,
    pub data_dir: PathBuf,
}

impl Archive {
    pub fn msg(&self, ch: usize, idx: usize) -> &Message {
        &self.channels[ch].messages[idx]
    }
    pub fn user(&self, idx: u32) -> &User {
        &self.users[idx as usize]
    }
}

// ---------------------------------------------------------------------------
// Conversion helpers

fn parse_color(s: &Option<String>) -> Option<Color32> {
    let s = s.as_deref()?.strip_prefix('#')?;
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

pub fn parse_ts(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis())
}

pub fn local_date(ts: i64) -> NaiveDate {
    Local.timestamp_millis_opt(ts).single().map(|d| d.date_naive()).unwrap_or_default()
}

pub fn ext_kind(name: &str) -> MediaKind {
    let name = name.split(['?', '#']).next().unwrap_or(name);
    let Some((_, ext)) = name.rsplit_once('.') else { return MediaKind::Other };
    match ext.to_ascii_lowercase().as_str() {
        "png" | "jpg" | "jpeg" | "jfif" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "ico" | "avif" => MediaKind::Image,
        "mp4" | "webm" | "mov" | "mkv" | "m4v" | "avi" | "gifv" => MediaKind::Video,
        "mp3" | "m4a" | "ogg" | "oga" | "opus" | "wav" | "flac" | "aac" => MediaKind::Audio,
        "svg" => MediaKind::Svg,
        _ => MediaKind::Other,
    }
}

/// Rewrites media paths whose folder no longer exists onto `media_dir`.
struct PathFixer {
    media_dir: Option<PathBuf>,
}

impl PathFixer {
    fn fix(&self, s: &str) -> MediaRef {
        if is_local(s)
            && let Some(dir) = &self.media_dir
            && let Some(name) = Path::new(s).file_name()
        {
            return Arc::from(dir.join(name).to_string_lossy().as_ref());
        }
        Arc::from(s)
    }
    fn opt(&self, s: &Option<String>) -> Option<MediaRef> {
        s.as_deref().filter(|s| !s.is_empty()).map(|s| self.fix(s))
    }
}

/// A channel parsed on a worker thread, before users are interned.
struct Parsed {
    raw: RawChannel,
    messages: Vec<(RawMessage, Box<str>)>,
}

fn haystack(m: &RawMessage) -> Box<str> {
    let mut s = m.content.to_lowercase();
    let mut push = |t: &str| {
        s.push('\n');
        s.push_str(&t.to_lowercase());
    };
    for a in &m.attachments {
        push(&a.file_name);
    }
    let embeds = m.embeds.iter().chain(m.forwarded_message.iter().flat_map(|f| f.embeds.iter()));
    for e in embeds {
        for t in [&e.title, &e.description, &e.url].into_iter().flatten() {
            push(t);
        }
        if let Some(a) = &e.author
            && let Some(n) = &a.name
        {
            push(n);
        }
        for f in &e.fields {
            push(&f.name);
            push(&f.value);
        }
        if let Some(f) = &e.footer {
            push(&f.text);
        }
    }
    if let Some(f) = &m.forwarded_message {
        push(&f.content);
        for a in &f.attachments {
            push(&a.file_name);
        }
    }
    s.into_boxed_str()
}

struct Converter<'a> {
    fixer: &'a PathFixer,
    users: Vec<User>,
    user_idx: HashMap<String, u32>,
}

impl Converter<'_> {
    fn user(&mut self, u: &RawUser) -> u32 {
        if let Some(&i) = self.user_idx.get(&u.id) {
            return i;
        }
        let i = self.users.len() as u32;
        self.users.push(User {
            name: u.name.clone(),
            nickname: u.nickname.clone().filter(|n| !n.is_empty()),
            color: parse_color(&u.color),
            is_bot: u.is_bot,
            avatar: self.fixer.opt(&u.avatar_url),
            roles: u.roles.iter().map(|r| (r.name.clone(), parse_color(&r.color))).collect(),
        });
        self.user_idx.insert(u.id.clone(), i);
        i
    }

    fn attachment(&self, a: &RawAttachment) -> Attachment {
        let kind = match ext_kind(&a.file_name) {
            MediaKind::Other => ext_kind(&a.url),
            k => k,
        };
        Attachment { url: self.fixer.fix(&a.url), file_name: a.file_name.clone(), size: a.file_size_bytes, kind }
    }

    fn media(&self, m: &RawEmbedMedia) -> EmbedMedia {
        let size = match (m.width, m.height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => Some([w, h]),
            _ => None,
        };
        EmbedMedia { url: self.fixer.fix(&m.url), size }
    }

    fn embed(&self, e: &RawEmbed) -> Embed {
        let nonempty = |s: &Option<String>| s.clone().filter(|s| !s.trim().is_empty());
        let mut images: Vec<EmbedMedia> = Vec::new();
        for m in e.image.iter().chain(e.images.iter()) {
            if !m.url.is_empty() && !images.iter().any(|x| *x.url == *self.fixer.fix(&m.url)) {
                images.push(self.media(m));
            }
        }
        Embed {
            title: nonempty(&e.title),
            url: nonempty(&e.url),
            description: nonempty(&e.description),
            color: parse_color(&e.color),
            author_name: e.author.as_ref().and_then(|a| nonempty(&a.name)),
            author_url: e.author.as_ref().and_then(|a| nonempty(&a.url)),
            author_icon: e.author.as_ref().and_then(|a| self.fixer.opt(&a.icon_url)),
            thumbnail: e.thumbnail.as_ref().filter(|m| !m.url.is_empty()).map(|m| self.media(m)),
            images,
            video: e.video.as_ref().filter(|m| !m.url.is_empty()).map(|m| self.media(m)),
            fields: e.fields.iter().map(|f| (f.name.clone(), f.value.clone(), f.is_inline)).collect(),
            footer: e.footer.as_ref().filter(|f| !f.text.is_empty()).map(|f| (f.text.clone(), self.fixer.opt(&f.icon_url))),
        }
    }

    fn sticker(&self, s: &RawSticker) -> Sticker {
        Sticker {
            name: s.name.clone(),
            url: self.fixer.fix(&s.source_url),
            lottie: s.format.eq_ignore_ascii_case("lottie") || s.source_url.ends_with(".json"),
        }
    }

    fn message(&mut self, m: RawMessage, haystack: Box<str>) -> Message {
        let kind = match m.kind.as_str() {
            "Default" | "20" | "23" => MsgKind::Normal,
            "Reply" => MsgKind::Reply,
            "ChannelPinnedMessage" => MsgKind::Pinned,
            "PollResult" => MsgKind::Poll,
            _ => MsgKind::System,
        };
        let attachments: Vec<Attachment> = m.attachments.iter().map(|a| self.attachment(a)).collect();
        let embeds: Vec<Embed> = m.embeds.iter().map(|e| self.embed(e)).collect();
        let stickers: Vec<Sticker> = m.stickers.iter().map(|s| self.sticker(s)).collect();
        let reactions = m
            .reactions
            .iter()
            .map(|r| Reaction {
                name: r.emoji.name.clone(),
                image: self.fixer.opt(&r.emoji.image_url),
                count: r.count,
                users: r.users.iter().map(|u| self.user(u)).collect(),
            })
            .collect::<Vec<_>>();
        let emojis = m
            .inline_emojis
            .iter()
            .map(|e| InlineEmoji {
                pattern: if e.id.as_deref().unwrap_or("").is_empty() { e.name.clone() } else { format!(":{}:", e.name) },
                image: self.fixer.opt(&e.image_url),
            })
            .collect();
        let forwarded = m.forwarded_message.as_ref().map(|f| {
            Box::new(Forwarded {
                timestamp: f.timestamp.as_deref().and_then(parse_ts),
                content: f.content.clone(),
                attachments: f.attachments.iter().map(|a| self.attachment(a)).collect(),
                embeds: f.embeds.iter().map(|e| self.embed(e)).collect(),
                stickers: f.stickers.iter().map(|s| self.sticker(s)).collect(),
            })
        });

        let mut fl = 0u8;
        let all_atts = attachments.iter().chain(forwarded.iter().flat_map(|f| f.attachments.iter()));
        for a in all_atts {
            fl |= match a.kind {
                MediaKind::Image | MediaKind::Svg => flags::IMAGE,
                MediaKind::Video => flags::VIDEO,
                MediaKind::Audio => flags::AUDIO,
                MediaKind::Other => flags::FILE,
            };
        }
        for e in embeds.iter().chain(forwarded.iter().flat_map(|f| f.embeds.iter())) {
            fl |= flags::EMBED;
            if e.video.is_some() {
                fl |= flags::VIDEO;
            } else if e.thumbnail.is_some() || !e.images.is_empty() {
                fl |= flags::IMAGE;
            }
        }
        if !stickers.is_empty() {
            fl |= flags::STICKER;
        }
        if !reactions.is_empty() {
            fl |= flags::REACTION;
        }
        if haystack.contains("http://") || haystack.contains("https://") {
            fl |= flags::LINK;
        }

        Message {
            id: m.id.parse().unwrap_or(0),
            kind,
            ts: parse_ts(&m.timestamp).unwrap_or(0),
            edited: m.timestamp_edited.as_deref().and_then(parse_ts),
            pinned: m.is_pinned,
            author: self.user(&m.author),
            mentions: m.mentions.iter().map(|u| self.user(u)).collect(),
            reply_to: m.reference.as_ref().and_then(|r| r.message_id.as_deref()).and_then(|s| s.parse().ok()),
            content: m.content,
            attachments,
            embeds,
            stickers,
            reactions,
            emojis,
            forwarded,
            haystack,
            flags: fl,
        }
    }
}

// ---------------------------------------------------------------------------
// Loading

/// Finds where the media folder is, if the absolute paths in the export no longer resolve.
fn find_media_remap(data_dir: &Path, sample: Option<&str>, media_override: Option<&Path>) -> Option<PathBuf> {
    if let Some(m) = media_override {
        return Some(m.to_path_buf());
    }
    let sample = sample?;
    if Path::new(sample).parent().is_some_and(|p| p.is_dir()) {
        return None;
    }
    // data/exports/<stamp>/ -> data/media/
    let real = data_dir.canonicalize().ok()?;
    let candidate = real.parent()?.parent()?.join("media");
    candidate.is_dir().then_some(candidate)
}

pub fn load(data_dir: &Path, media_override: Option<&Path>, progress: &(dyn Fn(String) + Sync)) -> Result<Archive, String> {
    // Gather channel files: <category dir>/<channel>.json, skipping server/.
    let mut files = Vec::new();
    let entries = std::fs::read_dir(data_dir).map_err(|e| format!("Can't read {}: {e}", data_dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || entry.file_name() == "server" {
            continue;
        }
        for f in std::fs::read_dir(&path).into_iter().flatten().flatten() {
            if f.path().extension().is_some_and(|e| e == "json") {
                files.push(f.path());
            }
        }
    }
    if files.is_empty() {
        return Err(format!("No channel .json files found under {}", data_dir.display()));
    }
    progress(format!("Reading {} channels…", files.len()));

    let done = std::sync::atomic::AtomicUsize::new(0);
    let parsed: Vec<Parsed> = files
        .par_iter()
        .filter_map(|path| {
            let bytes = std::fs::read(path).ok()?;
            let raw: RawExport = match serde_json::from_slice(&bytes) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Skipping {}: {e}", path.display());
                    return None;
                }
            };
            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            progress(format!("Read {n} of {} channels…", files.len()));
            let messages = raw
                .messages
                .into_iter()
                .map(|m| {
                    let h = haystack(&m);
                    (m, h)
                })
                .collect();
            Some(Parsed { raw: raw.channel, messages })
        })
        .collect();
    progress("Indexing…".into());

    let sample = parsed
        .iter()
        .flat_map(|p| p.messages.iter())
        .find_map(|(m, _)| m.author.avatar_url.clone().filter(|u| is_local(u)));
    let fixer = PathFixer { media_dir: find_media_remap(data_dir, sample.as_deref(), media_override) };

    let mut conv = Converter { fixer: &fixer, users: Vec::new(), user_idx: HashMap::new() };
    let mut channels = Vec::with_capacity(parsed.len());
    let mut meta = Vec::with_capacity(parsed.len());
    for p in parsed {
        let kind = match p.raw.kind.as_str() {
            k if k.contains("Thread") => ChannelKind::Thread,
            k if k.contains("Voice") || k.contains("Stage") => ChannelKind::Voice,
            _ => ChannelKind::Text,
        };
        let mut messages: Vec<Message> = p.messages.into_iter().map(|(m, h)| conv.message(m, h)).collect();
        messages.sort_by_key(|m| (m.ts, m.id));
        channels.push(Channel {
            id: p.raw.id.parse().unwrap_or(0),
            name: p.raw.name.clone(),
            kind,
            topic: p.raw.topic.clone().filter(|t| !t.trim().is_empty()),
            messages,
            parent: None,
        });
        meta.push((p.raw.category_id.and_then(|c| c.parse::<u64>().ok()), p.raw.category.unwrap_or_default()));
    }

    // Threads: their "category" is the parent channel.
    let idx_by_id: HashMap<u64, usize> = channels.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
    for i in 0..channels.len() {
        if channels[i].kind == ChannelKind::Thread
            && let Some(parent) = meta[i].0.and_then(|p| idx_by_id.get(&p))
        {
            channels[i].parent = Some(*parent);
        }
    }

    let server = data_dir.join("server");
    let api_channels: Vec<ApiChannel> = std::fs::read(server.join("channels.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let guild: ApiGuild =
        std::fs::read(server.join("guild.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let guild_icon = ["icon.png", "icon.gif"]
        .iter()
        .map(|n| server.join(n))
        .find(|p| p.is_file())
        .map(|p| Arc::from(p.to_string_lossy().as_ref()));

    let sidebar = build_sidebar(&channels, &meta, &api_channels);

    let mut by_id = HashMap::new();
    let mut total = 0;
    for (ci, c) in channels.iter().enumerate() {
        total += c.messages.len();
        for (mi, m) in c.messages.iter().enumerate() {
            by_id.insert(m.id, (ci as u32, mi as u32));
        }
    }
    let channel_by_name = channels
        .iter()
        .enumerate()
        .filter(|(_, c)| c.kind != ChannelKind::Thread)
        .map(|(i, c)| (c.name.clone(), i))
        .collect();

    Ok(Archive {
        guild_name: if guild.name.is_empty() { "Discord export".into() } else { guild.name },
        guild_icon,
        channels,
        users: conv.users,
        sidebar,
        by_id,
        channel_by_name,
        total_messages: total,
        data_dir: data_dir.to_path_buf(),
    })
}

/// Orders categories and channels the way Discord does, using `server/channels.json`
/// when available, and puts the archive category in its own section at the bottom.
fn build_sidebar(channels: &[Channel], meta: &[(Option<u64>, String)], api: &[ApiChannel]) -> Vec<SidebarCategory> {
    let api_pos: HashMap<u64, (i64, i64, Option<u64>)> = api
        .iter()
        .filter_map(|c| Some((c.id.parse().ok()?, (c.position, c.kind, c.parent_id.as_deref().and_then(|p| p.parse().ok())))))
        .collect();
    let api_name: HashMap<u64, &str> = api.iter().filter_map(|c| Some((c.id.parse().ok()?, c.name.as_str()))).collect();

    let mut cats: Vec<SidebarCategory> = Vec::new();
    let mut cat_index: HashMap<u64, usize> = HashMap::new();
    for (i, c) in channels.iter().enumerate() {
        if c.kind == ChannelKind::Thread && c.parent.is_some() {
            continue;
        }
        let (cat_id, cat_name) = match api_pos.get(&c.id).and_then(|p| p.2) {
            Some(parent) => (parent, api_name.get(&parent).map(|s| s.to_string()).unwrap_or_else(|| meta[i].1.clone())),
            None => (meta[i].0.unwrap_or(0), meta[i].1.clone()),
        };
        let ci = *cat_index.entry(cat_id).or_insert_with(|| {
            cats.push(SidebarCategory {
                id: cat_id,
                archive: cat_name.to_lowercase().contains("archive"),
                name: cat_name.trim().to_string(),
                channels: Vec::new(),
            });
            cats.len() - 1
        });
        let threads = channels
            .iter()
            .enumerate()
            .filter(|(_, t)| t.parent == Some(i))
            .map(|(ti, _)| ti)
            .collect::<Vec<_>>();
        cats[ci].channels.push(SidebarChannel { channel: i, threads });
    }

    let pos = |id: u64| api_pos.get(&id).map(|p| p.0).unwrap_or(i64::MAX);
    for cat in &mut cats {
        cat.channels.sort_by_key(|sc| {
            let c = &channels[sc.channel];
            (c.kind == ChannelKind::Voice, pos(c.id), c.name.clone())
        });
        for sc in &mut cat.channels {
            sc.threads.sort_by_key(|&t| std::cmp::Reverse(channels[t].messages.last().map(|m| m.ts).unwrap_or(0)));
        }
    }
    // Uncategorised first, then by position, archive last.
    cats.sort_by_key(|c| (c.archive, c.id != 0, pos(c.id), c.name.clone()));
    cats
}
