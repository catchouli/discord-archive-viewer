//! Background decoding of images, animations, SVG emoji and video posters,
//! plus a GPU texture cache with least-recently-used eviction.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use egui::{ColorImage, Context, TextureHandle, TextureId, TextureOptions, Vec2, vec2};
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader};

use crate::model::{MediaKind, MediaRef, ext_kind, is_local};

/// Texture size buckets (longest side, in pixels).
pub const BUCKETS: [u32; 8] = [64, 128, 256, 512, 1024, 1536, 2048, 4096];

pub fn bucket_for(px: f32) -> u32 {
    BUCKETS.iter().copied().find(|&b| b as f32 >= px).unwrap_or(4096)
}

/// Total GPU memory we allow cached textures to use before evicting.
const TEXTURE_BUDGET: usize = 1536 << 20;
/// Max memory for one animation's frames; bigger ones are decoded at a smaller size.
const ANIMATION_BUDGET: usize = 192 << 20;
const MAX_FRAMES: usize = 600;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum JobKind {
    Image,
    VideoPoster,
}

type Key = (MediaRef, u32, JobKind);

struct Job {
    key: Key,
    stamp: u64,
}

enum Decoded {
    Static(ColorImage),
    Anim(Vec<(ColorImage, f32)>),
}

struct JobResult {
    key: Key,
    result: Result<Decoded, String>,
}

struct Queue {
    jobs: Vec<Job>,
}

enum Entry {
    Static(TextureHandle),
    Anim { frames: Vec<(TextureHandle, f32)>, total: f32 },
}

struct Cached {
    entry: Entry,
    size: [usize; 2],
    bytes: usize,
    last_used: u64,
}

pub enum Tex {
    Ready { id: TextureId, size: Vec2 },
    Loading,
    Failed,
}

pub struct MediaCache {
    ctx: Context,
    queue: Arc<(Mutex<Queue>, Condvar)>,
    frame: Arc<AtomicU64>,
    rx: Receiver<JobResult>,
    textures: HashMap<Key, Cached>,
    pending: HashSet<Key>,
    failed: HashSet<Key>,
    /// Natural size (aspect) of media, from file headers or decoded results.
    dims: HashMap<MediaRef, Option<[u32; 2]>>,
    kinds: HashMap<MediaRef, MediaKind>,
    bytes: usize,
    frame_no: u64,
}

pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        if cfg!(target_os = "macos") { home.join("Library/Caches") } else { home.join(".cache") }
    });
    base.join("discord-archive-viewer")
}

impl MediaCache {
    pub fn new(ctx: Context) -> Self {
        let queue = Arc::new((Mutex::new(Queue { jobs: Vec::new() }), Condvar::new()));
        let frame = Arc::new(AtomicU64::new(0));
        let (tx, rx) = channel();
        let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
        for i in 0..workers {
            let queue = queue.clone();
            let frame = frame.clone();
            let tx: Sender<JobResult> = tx.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name(format!("media-{i}"))
                .spawn(move || worker(queue, frame, tx, ctx))
                .expect("spawn media worker");
        }
        let _ = std::fs::create_dir_all(cache_dir().join("posters"));
        Self {
            ctx,
            queue,
            frame,
            rx,
            textures: HashMap::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            dims: HashMap::new(),
            kinds: HashMap::new(),
            bytes: 0,
            frame_no: 0,
        }
    }

    /// Call once per frame before drawing: uploads finished decodes.
    pub fn begin_frame(&mut self) {
        self.frame_no += 1;
        self.frame.store(self.frame_no, Ordering::Relaxed);
        let start = std::time::Instant::now();
        while let Ok(res) = self.rx.try_recv() {
            self.pending.remove(&res.key);
            match res.result {
                Ok(decoded) => self.insert(res.key, decoded),
                Err(e) if e == "cancelled" => {}
                Err(e) => {
                    if !e.is_empty() {
                        eprintln!("media: {}: {e}", res.key.0);
                    }
                    self.failed.insert(res.key);
                }
            }
            // Spread big uploads over several frames.
            if start.elapsed() > Duration::from_millis(12) {
                self.ctx.request_repaint();
                break;
            }
        }
    }

    /// Call once per frame after drawing: evicts least recently used textures over budget.
    pub fn end_frame(&mut self) {
        if self.bytes <= TEXTURE_BUDGET {
            return;
        }
        let mut by_age: Vec<(u64, Key)> = self.textures.iter().map(|(k, c)| (c.last_used, k.clone())).collect();
        by_age.sort_by_key(|(t, _)| *t);
        for (t, k) in by_age {
            if self.bytes <= TEXTURE_BUDGET * 3 / 4 || t + 2 >= self.frame_no {
                break;
            }
            if let Some(c) = self.textures.remove(&k) {
                self.bytes -= c.bytes;
            }
        }
    }

    fn insert(&mut self, key: Key, decoded: Decoded) {
        let name = format!("{}@{}", key.0, key.1);
        let (entry, size, bytes) = match decoded {
            Decoded::Static(img) => {
                let size = img.size;
                let bytes = size[0] * size[1] * 4;
                (Entry::Static(self.ctx.load_texture(name, img, TextureOptions::LINEAR)), size, bytes)
            }
            Decoded::Anim(frames) => {
                let size = frames[0].0.size;
                let bytes = size[0] * size[1] * 4 * frames.len();
                let total = frames.iter().map(|f| f.1).sum::<f32>().max(0.01);
                let frames = frames
                    .into_iter()
                    .enumerate()
                    .map(|(i, (img, d))| (self.ctx.load_texture(format!("{name}#{i}"), img, TextureOptions::LINEAR), d))
                    .collect();
                (Entry::Anim { frames, total }, size, bytes)
            }
        };
        if key.2 == JobKind::VideoPoster || !self.dims.get(&key.0).is_some_and(|d| d.is_some()) || key.1 >= 1024 {
            // Decoded pixels know the true orientation; prefer them over header sizes.
            self.dims.insert(key.0.clone(), Some([size[0] as u32, size[1] as u32]));
        }
        self.bytes += bytes;
        self.textures.insert(key, Cached { entry, size, bytes, last_used: self.frame_no });
    }

    fn request(&mut self, key: Key) {
        if self.pending.contains(&key) {
            // Refresh its stamp so workers keep prioritising it.
            let mut q = self.queue.0.lock().unwrap();
            if let Some(j) = q.jobs.iter_mut().find(|j| j.key == key) {
                j.stamp = self.frame_no;
            } else {
                drop(q);
            }
            return;
        }
        self.pending.insert(key.clone());
        let (lock, cv) = &*self.queue;
        lock.lock().unwrap().jobs.push(Job { key, stamp: self.frame_no });
        cv.notify_one();
    }

    fn get(&mut self, key: Key) -> Tex {
        if let Some(c) = self.textures.get_mut(&key) {
            c.last_used = self.frame_no;
            let size = vec2(c.size[0] as f32, c.size[1] as f32);
            let id = match &c.entry {
                Entry::Static(t) => t.id(),
                Entry::Anim { frames, total } => {
                    let t = (self.ctx.input(|i| i.time) as f32) % total;
                    let mut acc = 0.0;
                    let mut pick = &frames[0];
                    for f in frames {
                        pick = f;
                        acc += f.1;
                        if acc > t {
                            break;
                        }
                    }
                    self.ctx.request_repaint_after(Duration::from_secs_f32((acc - t).max(0.01)));
                    pick.0.id()
                }
            };
            return Tex::Ready { id, size };
        }
        if self.failed.contains(&key) || !is_local(&key.0) {
            return Tex::Failed;
        }
        self.request(key);
        Tex::Loading
    }

    /// Texture for an image at roughly `px` pixels on its longest side.
    /// While a sharper version loads, a smaller cached one is returned if there is one.
    pub fn image(&mut self, path: &MediaRef, px: f32) -> Tex {
        let bucket = bucket_for(px);
        let key = (path.clone(), bucket, JobKind::Image);
        match self.get(key) {
            Tex::Loading => {
                for &b in BUCKETS.iter().rev().filter(|&&b| b != bucket) {
                    let k = (path.clone(), b, JobKind::Image);
                    if self.textures.contains_key(&k) {
                        return self.get(k);
                    }
                }
                Tex::Loading
            }
            t => t,
        }
    }

    /// First-frame poster for a video, extracted with ffmpeg and cached on disk.
    pub fn poster(&mut self, path: &MediaRef) -> Tex {
        self.get((path.clone(), 1024, JobKind::VideoPoster))
    }

    /// Natural size of an image (fast: reads only the header), or of a decoded poster.
    pub fn dims(&mut self, path: &MediaRef) -> Option<[u32; 2]> {
        if let Some(d) = self.dims.get(path) {
            return *d;
        }
        let d = if is_local(path) {
            imagesize::size(path.as_ref()).ok().map(|s| [s.width as u32, s.height as u32]).filter(|d| d[0] > 0 && d[1] > 0)
        } else {
            None
        };
        // JPEG EXIF rotation isn't visible in the header; the decoded size corrects it later.
        self.dims.insert(path.clone(), d);
        d
    }

    /// What kind of media a file is: by extension, then by sniffing its first bytes.
    pub fn kind(&mut self, path: &MediaRef, name_hint: Option<&str>) -> MediaKind {
        if let Some(k) = self.kinds.get(path) {
            return *k;
        }
        let mut k = name_hint.map(ext_kind).unwrap_or(MediaKind::Other);
        if k == MediaKind::Other {
            k = ext_kind(path);
        }
        if k == MediaKind::Other && is_local(path) {
            k = sniff(Path::new(path.as_ref()));
        }
        self.kinds.insert(path.clone(), k);
        k
    }
}

pub fn sniff(path: &Path) -> MediaKind {
    let mut buf = [0u8; 64];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut buf)).unwrap_or(0);
    let b = &buf[..n];
    if b.starts_with(&[0x89, b'P', b'N', b'G'])
        || b.starts_with(&[0xFF, 0xD8, 0xFF])
        || b.starts_with(b"GIF8")
        || b.starts_with(b"BM")
        || (b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP"))
        || b.get(4..12) == Some(b"ftypavif")
    {
        MediaKind::Image
    } else if b.get(4..8) == Some(b"ftyp") {
        if b.get(8..11) == Some(b"M4A") { MediaKind::Audio } else { MediaKind::Video }
    } else if b.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        MediaKind::Video
    } else if b.starts_with(b"OggS")
        || b.starts_with(b"ID3")
        || b.starts_with(b"fLaC")
        || (b.len() > 1 && b[0] == 0xFF && b[1] & 0xE0 == 0xE0)
        || (b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WAVE"))
    {
        MediaKind::Audio
    } else if std::str::from_utf8(b).is_ok_and(|s| s.trim_start().starts_with("<svg") || s.contains("<svg")) {
        MediaKind::Svg
    } else {
        MediaKind::Other
    }
}

// ---------------------------------------------------------------------------
// Worker side

fn worker(queue: Arc<(Mutex<Queue>, Condvar)>, frame: Arc<AtomicU64>, tx: Sender<JobResult>, ctx: Context) {
    loop {
        let job = {
            let (lock, cv) = &*queue;
            let mut q = lock.lock().unwrap();
            loop {
                // Most recently requested first: that's what's on screen.
                if let Some(i) = q.jobs.iter().enumerate().max_by_key(|(_, j)| j.stamp).map(|(i, _)| i) {
                    break q.jobs.swap_remove(i);
                }
                q = cv.wait(q).unwrap();
            }
        };
        // Scrolled away before we got to it.
        if job.stamp + 30 < frame.load(Ordering::Relaxed) {
            let _ = tx.send(JobResult { key: job.key, result: Err("cancelled".into()) });
            continue;
        }
        let (path, bucket, kind) = &job.key;
        let result = std::panic::catch_unwind(|| match kind {
            JobKind::Image => decode_any(Path::new(path.as_ref()), *bucket),
            JobKind::VideoPoster => video_poster(Path::new(path.as_ref()), *bucket),
        })
        .unwrap_or_else(|_| Err("decoder panicked".into()));
        if tx.send(JobResult { key: job.key, result }).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

fn to_color_image(img: DynamicImage) -> ColorImage {
    let rgba = img.into_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    ColorImage::from_rgba_unmultiplied(size, rgba.as_raw())
}

fn fit(w: u32, h: u32, max: u32) -> (u32, u32) {
    if w <= max && h <= max {
        return (w, h);
    }
    let s = max as f32 / w.max(h) as f32;
    (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1))
}

fn shrink(img: DynamicImage, max: u32) -> DynamicImage {
    let (w, h) = fit(img.width(), img.height(), max);
    if (w, h) == (img.width(), img.height()) {
        img
    } else {
        img.resize_exact(w, h, image::imageops::FilterType::Triangle)
    }
}

fn decode_any(path: &Path, bucket: u32) -> Result<Decoded, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    if crate::model::ext_kind(&path.to_string_lossy()) == MediaKind::Svg || sniff(path) == MediaKind::Svg {
        return decode_svg(&data, bucket);
    }
    let reader = ImageReader::new(Cursor::new(&data)).with_guessed_format().map_err(|e| e.to_string())?;
    let format = reader.format();
    match format {
        Some(ImageFormat::Gif) => {
            let dec = image::codecs::gif::GifDecoder::new(Cursor::new(&data)).map_err(|e| e.to_string())?;
            animation(dec.into_frames(), bucket)
        }
        Some(ImageFormat::WebP) => {
            let dec = image::codecs::webp::WebPDecoder::new(Cursor::new(&data)).map_err(|e| e.to_string())?;
            if dec.has_animation() {
                animation(dec.into_frames(), bucket)
            } else {
                Ok(Decoded::Static(to_color_image(shrink(DynamicImage::from_decoder(dec).map_err(|e| e.to_string())?, bucket))))
            }
        }
        Some(ImageFormat::Png) => {
            let dec = image::codecs::png::PngDecoder::new(Cursor::new(&data)).map_err(|e| e.to_string())?;
            if dec.is_apng().unwrap_or(false) {
                animation(dec.apng().map_err(|e| e.to_string())?.into_frames(), bucket)
            } else {
                Ok(Decoded::Static(to_color_image(shrink(DynamicImage::from_decoder(dec).map_err(|e| e.to_string())?, bucket))))
            }
        }
        Some(_) => {
            let mut dec = reader.into_decoder().map_err(|e| e.to_string())?;
            let orientation = dec.orientation().ok();
            let mut img = DynamicImage::from_decoder(dec).map_err(|e| e.to_string())?;
            if let Some(o) = orientation {
                img.apply_orientation(o);
            }
            Ok(Decoded::Static(to_color_image(shrink(img, bucket))))
        }
        None => Err(String::new()),
    }
}

fn animation(frames: image::Frames<'_>, bucket: u32) -> Result<Decoded, String> {
    let mut raw = Vec::new();
    for f in frames.take(MAX_FRAMES) {
        let f = f.map_err(|e| e.to_string())?;
        let (n, d) = f.delay().numer_denom_ms();
        let mut delay = n as f32 / d.max(1) as f32 / 1000.0;
        if delay < 0.02 {
            delay = 0.1; // browsers treat 0/10ms GIF delays as 100ms
        }
        raw.push((f.into_buffer(), delay));
    }
    if raw.is_empty() {
        return Err("no frames".into());
    }
    let (w, h) = raw[0].0.dimensions();
    if raw.len() == 1 {
        let img = DynamicImage::ImageRgba8(raw.pop().unwrap().0);
        return Ok(Decoded::Static(to_color_image(shrink(img, bucket))));
    }
    // Keep the whole animation within budget.
    let mut max = bucket;
    while max > 64 {
        let (fw, fh) = fit(w, h, max);
        if (fw * fh * 4) as usize * raw.len() <= ANIMATION_BUDGET {
            break;
        }
        max /= 2;
    }
    Ok(Decoded::Anim(
        raw.into_iter().map(|(buf, d)| (to_color_image(shrink(DynamicImage::ImageRgba8(buf), max)), d)).collect(),
    ))
}

fn decode_svg(data: &[u8], bucket: u32) -> Result<Decoded, String> {
    let tree = resvg::usvg::Tree::from_data(data, &resvg::usvg::Options::default()).map_err(|e| e.to_string())?;
    let size = tree.size();
    let (w, h) = fit((size.width() * 64.0) as u32, (size.height() * 64.0) as u32, bucket);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w.max(1), h.max(1)).ok_or("bad svg size")?;
    let t = resvg::tiny_skia::Transform::from_scale(w as f32 / size.width(), h as f32 / size.height());
    resvg::render(&tree, t, &mut pixmap.as_mut());
    // tiny-skia pixels are premultiplied.
    Ok(Decoded::Static(ColorImage::from_rgba_premultiplied([w as usize, h as usize], pixmap.data())))
}

fn video_poster(path: &Path, bucket: u32) -> Result<Decoded, String> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0).hash(&mut h);
    let out = cache_dir().join("posters").join(format!("{:016x}.jpg", h.finish()));
    if !out.exists() {
        let tmp = out.with_extension("tmp.jpg");
        let status = std::process::Command::new(crate::player::ffmpeg())
            .args(["-v", "error", "-y", "-i"])
            .arg(path)
            .args(["-frames:v", "1", "-vf", "scale='min(1280,iw)':-2", "-q:v", "4"])
            .arg(&tmp)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| format!("ffmpeg: {e}"))?;
        if !status.success() || !tmp.exists() {
            // Remember the failure so we don't retry every launch.
            let _ = std::fs::write(&out, b"");
            return Err(String::new());
        }
        std::fs::rename(&tmp, &out).map_err(|e| e.to_string())?;
    }
    if std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0) == 0 {
        return Err(String::new());
    }
    decode_any(&out, bucket)
}
