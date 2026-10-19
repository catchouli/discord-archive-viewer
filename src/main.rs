//! Discord export viewer: browse, search and view media from a DiscordChatExporter JSON export.
//!
//! Usage: discord-archive-viewer [EXPORT_DIR] [--media MEDIA_DIR]
//!
//! EXPORT_DIR defaults to $DISCORD_VIEWER_DATA, or the nearest `private/data/latest`
//! (what `export/export.sh` writes) or `data/latest`, looking up from the current
//! directory and the executable's location.

mod app;
mod lightbox;
mod markdown;
mod media;
mod model;
mod player;
mod render;
mod script;
mod search;
mod theme;
mod view;

use std::path::{Path, PathBuf};

fn find_data_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("DISCORD_VIEWER_DATA") {
        return Some(PathBuf::from(p));
    }
    let mut starts: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        starts.push(dir);
    }
    starts.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    starts
        .iter()
        .flat_map(|s| s.ancestors())
        .flat_map(|d| [d.join("private/data/latest"), d.join("data/latest")])
        .find(|p| p.is_dir())
}

fn main() -> eframe::Result {
    let mut args = std::env::args().skip(1);
    let mut data_dir = None;
    let mut media_dir = None;
    let mut check = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--media" => media_dir = args.next().map(PathBuf::from),
            "--check" => check = true,
            "-h" | "--help" => {
                println!("Usage: discord-archive-viewer [EXPORT_DIR] [--media MEDIA_DIR] [--check]");
                println!("--check loads the export, parses every message and reports problems, without opening a window.");
                println!("EXPORT_DIR defaults to the nearest private/data/latest folder (what export/export.sh writes).");
                return Ok(());
            }
            _ => data_dir = Some(PathBuf::from(a)),
        }
    }
    let Some(data_dir) = data_dir.or_else(find_data_dir) else {
        eprintln!("Couldn't find an export. Run export/export.sh first, or pass an export folder as an argument.");
        eprintln!("To try the viewer with made-up data: demo/make_demo.sh, then cargo run --release -- demo/output");
        std::process::exit(1);
    };
    if check {
        return check_export(&data_dir, media_dir.as_deref());
    }
    if !player::ffmpeg_available() {
        eprintln!("ffmpeg not found: videos will show without previews and won't play inline.");
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Discord Archive Viewer")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([820.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native("discord-archive-viewer", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, data_dir, media_dir)))))
}

/// Loads everything headlessly and reports what would be shown, to sanity-check an export.
fn check_export(data_dir: &Path, media_dir: Option<&Path>) -> eframe::Result {
    let t = std::time::Instant::now();
    let a = match model::load(data_dir, media_dir, &|_| {}) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    println!("Loaded {} messages, {} channels, {} users in {:.2}s", a.total_messages, a.channels.len(), a.users.len(), t.elapsed().as_secs_f32());
    for cat in &a.sidebar {
        let names: Vec<String> = cat
            .channels
            .iter()
            .map(|c| {
                let threads = c.threads.len();
                let name = &a.channels[c.channel].name;
                if threads > 0 { format!("{name} (+{threads} threads)") } else { name.clone() }
            })
            .collect();
        println!("  [{}{}] {}", cat.name, if cat.archive { ", archive" } else { "" }, names.join(", "));
    }

    let t = std::time::Instant::now();
    let mut blocks = 0usize;
    let exists = |n: &str| a.channel_by_name.contains_key(n);
    for c in &a.channels {
        for m in &c.messages {
            let pats: Vec<(&str, Option<model::MediaRef>)> = m.emojis.iter().map(|e| (e.pattern.as_str(), e.image.clone())).collect();
            let cx = markdown::Context { emojis: &pats, mentions: &[], channel_exists: &exists };
            blocks += markdown::parse(&m.content, &cx).len();
        }
    }
    println!("Parsed markdown for every message ({blocks} blocks) in {:.2}s", t.elapsed().as_secs_f32());

    let (mut local, mut missing, mut remote) = (0, 0, 0);
    let mut missing_examples = Vec::new();
    for c in &a.channels {
        for m in &c.messages {
            for att in &m.attachments {
                if !model::is_local(&att.url) {
                    remote += 1;
                } else if Path::new(att.url.as_ref()).exists() {
                    local += 1;
                } else {
                    missing += 1;
                    if missing_examples.len() < 3 {
                        missing_examples.push(att.url.to_string());
                    }
                }
            }
        }
    }
    println!("Attachments: {local} on disk, {missing} missing, {remote} remote-only");
    for e in missing_examples {
        println!("  missing: {e}");
    }

    for q in ["the", "has:image", "has:video", "\"good morning\""] {
        let t = std::time::Instant::now();
        let parsed = search::parse(q);
        let hits = search::run(&a, &parsed);
        println!("Search {q:?}: {} hits in {:.1} ms", hits.len(), t.elapsed().as_secs_f64() * 1000.0);
    }
    println!("ffmpeg: {}", if player::ffmpeg_available() { player::ffmpeg().display().to_string() } else { "not found".into() });
    Ok(())
}
