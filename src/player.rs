//! Video and audio playback by piping ffmpeg's decoded output.
//!
//! Video comes out as raw RGBA frames at a fixed frame rate; audio as 48 kHz
//! stereo f32 samples played through rodio. When there's audio, the number of
//! samples actually played is the clock that video frames are timed against.

use std::io::Read;
use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use egui::{ColorImage, Context, TextureHandle, TextureId, TextureOptions, Vec2, vec2};

const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
const MAX_W: f32 = 1280.0;
const MAX_H: f32 = 960.0;

fn find_tool(name: &str) -> PathBuf {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    // Apps launched from Finder don't get Homebrew's PATH.
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file()).unwrap_or_else(|| PathBuf::from(name))
}

pub fn ffmpeg() -> &'static Path {
    static P: OnceLock<PathBuf> = OnceLock::new();
    P.get_or_init(|| find_tool("ffmpeg"))
}

pub fn ffprobe() -> &'static Path {
    static P: OnceLock<PathBuf> = OnceLock::new();
    P.get_or_init(|| find_tool("ffprobe"))
}

pub fn ffmpeg_available() -> bool {
    ffmpeg().is_file()
}

/// The audio device, opened on first use.
#[derive(Default)]
pub struct AudioOut {
    sink: Option<rodio::MixerDeviceSink>,
    failed: bool,
}

impl AudioOut {
    fn mixer(&mut self) -> Option<&rodio::mixer::Mixer> {
        if self.sink.is_none() && !self.failed {
            match rodio::DeviceSinkBuilder::open_default_sink() {
                Ok(mut s) => {
                    s.log_on_drop(false);
                    self.sink = Some(s);
                }
                Err(e) => {
                    eprintln!("audio: {e}");
                    self.failed = true;
                }
            }
        }
        self.sink.as_ref().map(|s| s.mixer())
    }
}

#[derive(Clone, Debug)]
pub struct Info {
    pub size: Option<[u32; 2]>,
    pub duration: f64,
    pub has_audio: bool,
    pub fps: f64,
}

#[derive(Clone, PartialEq, Debug)]
pub enum State {
    Probing,
    Playing,
    Paused,
    Ended,
    Error(String),
}

struct PcmSource {
    rx: Receiver<Vec<f32>>,
    buf: Vec<f32>,
    pos: usize,
    played: Arc<AtomicU64>,
    done: Arc<AtomicBool>,
}

impl Iterator for PcmSource {
    type Item = rodio::Sample;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.pos < self.buf.len() {
                self.pos += 1;
                self.played.fetch_add(1, Ordering::Relaxed);
                return Some(self.buf[self.pos - 1]);
            }
            match self.rx.try_recv() {
                Ok(b) => {
                    self.buf = b;
                    self.pos = 0;
                }
                // Still buffering: play silence without advancing the clock.
                Err(TryRecvError::Empty) => return Some(0.0),
                Err(TryRecvError::Disconnected) => {
                    self.done.store(true, Ordering::Relaxed);
                    return None;
                }
            }
        }
    }
}

impl rodio::Source for PcmSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> rodio::ChannelCount {
        NonZero::new(CHANNELS).unwrap()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        NonZero::new(RATE).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

struct KillOnDrop(Arc<Mutex<Option<Child>>>);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn spawn(args: &[&str], path: &Path) -> std::io::Result<(Child, std::process::ChildStdout)> {
    let mut cmd = Command::new(ffmpeg());
    cmd.args(["-v", "error", "-nostdin"]);
    let (before, after) = args.split_at(args.iter().position(|a| *a == "-i").unwrap_or(0));
    cmd.args(before).arg("-i").arg(path).args(&after[1..]);
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let out = child.stdout.take().unwrap();
    Ok((child, out))
}

struct Session {
    start: f64,
    frames: Option<Receiver<Vec<u8>>>,
    frame_size: [usize; 2],
    next_frame: u64,
    pending: Option<Vec<u8>>,
    video_done: bool,
    audio: Option<(rodio::Player, Arc<AtomicU64>, Arc<AtomicBool>)>,
    wall_start: Instant,
    paused_at: Option<Instant>,
    paused_total: Duration,
    _procs: Vec<KillOnDrop>,
}

impl Session {
    fn new(path: &Path, info: &Info, start: f64, audio: Option<&rodio::mixer::Mixer>, volume: f32) -> Result<Self, String> {
        let ss = format!("{start:.3}");
        let mut procs = Vec::new();
        let mut frames = None;
        let mut frame_size = [0, 0];
        if let Some([w, h]) = info.size {
            frame_size = [w as usize, h as usize];
            let vf = format!("fps={},scale={w}:{h}:flags=bilinear", info.fps);
            let (child, mut out) =
                spawn(&["-ss", &ss, "-i", "-map", "0:v:0", "-an", "-sn", "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgba", "-"], path)
                    .map_err(|e| format!("ffmpeg: {e}"))?;
            procs.push(KillOnDrop(Arc::new(Mutex::new(Some(child)))));
            let (tx, rx) = sync_channel::<Vec<u8>>(3);
            let n = (w * h * 4) as usize;
            std::thread::spawn(move || {
                loop {
                    let mut buf = vec![0u8; n];
                    if out.read_exact(&mut buf).is_err() || tx.send(buf).is_err() {
                        break;
                    }
                }
            });
            frames = Some(rx);
        }
        let mut audio_out = None;
        if info.has_audio
            && let Some(mixer) = audio
        {
            let (child, mut out) = spawn(
                &["-ss", &ss, "-i", "-map", "0:a:0", "-vn", "-sn", "-f", "f32le", "-ac", "2", "-ar", &RATE.to_string(), "-"],
                path,
            )
            .map_err(|e| format!("ffmpeg: {e}"))?;
            procs.push(KillOnDrop(Arc::new(Mutex::new(Some(child)))));
            let (tx, rx) = sync_channel::<Vec<f32>>(24);
            std::thread::spawn(move || {
                let mut bytes = vec![0u8; 4096 * 4];
                loop {
                    let mut filled = 0;
                    while filled < bytes.len() {
                        match out.read(&mut bytes[filled..]) {
                            Ok(0) | Err(_) => break,
                            Ok(k) => filled += k,
                        }
                    }
                    let usable = filled / 8 * 8; // whole stereo frames only
                    if usable > 0 {
                        let s: Vec<f32> =
                            bytes[..usable].as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
                        if tx.send(s).is_err() {
                            break;
                        }
                    }
                    if filled < bytes.len() {
                        break;
                    }
                }
            });
            let played = Arc::new(AtomicU64::new(0));
            let done = Arc::new(AtomicBool::new(false));
            let player = rodio::Player::connect_new(mixer);
            player.set_volume(volume);
            player.append(PcmSource { rx, buf: Vec::new(), pos: 0, played: played.clone(), done: done.clone() });
            audio_out = Some((player, played, done));
        }
        Ok(Self {
            start,
            frames,
            frame_size,
            next_frame: 0,
            pending: None,
            video_done: info.size.is_none(),
            audio: audio_out,
            wall_start: Instant::now(),
            paused_at: None,
            paused_total: Duration::ZERO,
            _procs: procs,
        })
    }

    fn audio_done(&self) -> bool {
        self.audio.as_ref().is_none_or(|a| a.2.load(Ordering::Relaxed))
    }

    fn clock(&self) -> f64 {
        if let Some((_, played, done)) = &self.audio
            && !done.load(Ordering::Relaxed)
        {
            return self.start + played.load(Ordering::Relaxed) as f64 / (RATE as f64 * CHANNELS as f64);
        }
        let now = self.paused_at.unwrap_or_else(Instant::now);
        self.start + (now - self.wall_start - self.paused_total).as_secs_f64()
    }

    fn set_paused(&mut self, paused: bool) {
        if let Some((p, ..)) = &self.audio {
            if paused { p.pause() } else { p.play() }
        }
        match (paused, self.paused_at) {
            (true, None) => self.paused_at = Some(Instant::now()),
            (false, Some(at)) => {
                self.paused_total += at.elapsed();
                self.paused_at = None;
            }
            _ => {}
        }
    }
}

pub struct Player {
    path: PathBuf,
    pub info: Option<Info>,
    probe: Option<Receiver<Result<Info, String>>>,
    session: Option<Session>,
    texture: Option<TextureHandle>,
    pub state: State,
    pub looping: bool,
    pub volume: f32,
    pub muted: bool,
    position: f64,
    /// Show one frame even while paused (after a seek).
    refresh_frame: bool,
    want_paused: bool,
}

fn probe(path: &Path) -> Result<Info, String> {
    let out = Command::new(ffprobe())
        .args(["-v", "error", "-print_format", "json", "-show_entries"])
        .arg("stream=codec_type,width,height,avg_frame_rate,r_frame_rate:stream_disposition=attached_pic:stream_side_data=rotation:stream_tags=rotate:format=duration")
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Couldn't run ffprobe: {e}"))?;
    if !out.status.success() {
        return Err("This file couldn't be read.".into());
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let has_audio = streams.iter().any(|s| s["codec_type"] == "audio");
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"].as_i64().unwrap_or(0) == 0);
    let duration = v["format"]["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0);
    let parse_rate = |s: &serde_json::Value| -> Option<f64> {
        let (n, d) = s.as_str()?.split_once('/')?;
        let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
        (d > 0.0 && n > 0.0).then_some(n / d)
    };
    let mut fps = 30.0;
    let size = video.and_then(|s| {
        fps = parse_rate(&s["avg_frame_rate"]).or_else(|| parse_rate(&s["r_frame_rate"])).unwrap_or(30.0).clamp(1.0, 60.0);
        let (mut w, mut h) = (s["width"].as_f64()?, s["height"].as_f64()?);
        let rotation = s["side_data_list"]
            .as_array()
            .and_then(|l| l.iter().find_map(|d| d["rotation"].as_f64()))
            .or_else(|| s["tags"]["rotate"].as_str().and_then(|r| r.parse().ok()))
            .unwrap_or(0.0);
        if (rotation.abs() - 90.0).abs() < 1.0 || (rotation.abs() - 270.0).abs() < 1.0 {
            std::mem::swap(&mut w, &mut h);
        }
        let s = (MAX_W / w as f32).min(MAX_H / h as f32).min(1.0) as f64;
        let even = |x: f64| ((x * s / 2.0).round() as u32).max(1) * 2;
        Some([even(w), even(h)])
    });
    if size.is_none() && !has_audio {
        return Err("No playable streams.".into());
    }
    Ok(Info { size, duration, has_audio, fps: (fps * 1000.0).round() / 1000.0 })
}

impl Player {
    pub fn new(path: &Path, autoplay: bool) -> Self {
        let (tx, rx) = sync_channel(1);
        let p = path.to_path_buf();
        std::thread::spawn(move || {
            let _ = tx.send(probe(&p));
        });
        Self {
            path: path.to_path_buf(),
            info: None,
            probe: Some(rx),
            session: None,
            texture: None,
            state: State::Probing,
            looping: false,
            volume: 1.0,
            muted: false,
            position: 0.0,
            refresh_frame: false,
            want_paused: !autoplay,
        }
    }

    pub fn duration(&self) -> f64 {
        self.info.as_ref().map(|i| i.duration).unwrap_or(0.0)
    }

    pub fn position(&self) -> f64 {
        self.position
    }

    pub fn video_size(&self) -> Option<Vec2> {
        let [w, h] = self.info.as_ref()?.size?;
        Some(vec2(w as f32, h as f32))
    }

    pub fn frame(&self) -> Option<(TextureId, Vec2)> {
        let t = self.texture.as_ref()?;
        Some((t.id(), t.size_vec2()))
    }

    pub fn is_playing(&self) -> bool {
        matches!(self.state, State::Playing | State::Probing) && !self.want_paused
    }

    fn effective_volume(&self) -> f32 {
        if self.muted { 0.0 } else { self.volume }
    }

    pub fn set_volume(&mut self, volume: f32, muted: bool) {
        self.volume = volume;
        self.muted = muted;
        let v = self.effective_volume();
        if let Some((p, ..)) = self.session.as_ref().and_then(|s| s.audio.as_ref()) {
            p.set_volume(v);
        }
    }

    fn start_at(&mut self, t: f64, audio: &mut AudioOut) {
        let Some(info) = self.info.clone() else { return };
        self.session = None; // kill the old processes first
        let mixer = if info.has_audio { audio.mixer() } else { None };
        match Session::new(&self.path, &info, t, mixer, self.effective_volume()) {
            Ok(mut s) => {
                if self.want_paused {
                    s.set_paused(true);
                    self.state = State::Paused;
                } else {
                    self.state = State::Playing;
                }
                self.session = Some(s);
                self.position = t;
                self.refresh_frame = true;
            }
            Err(e) => self.state = State::Error(e),
        }
    }

    pub fn toggle(&mut self, audio: &mut AudioOut) {
        match self.state {
            State::Ended => {
                self.want_paused = false;
                self.start_at(0.0, audio);
            }
            State::Playing => {
                self.want_paused = true;
                if let Some(s) = &mut self.session {
                    s.set_paused(true);
                }
                self.state = State::Paused;
            }
            State::Paused => {
                self.want_paused = false;
                if let Some(s) = &mut self.session {
                    s.set_paused(false);
                    self.state = State::Playing;
                } else {
                    self.start_at(self.position, audio);
                }
            }
            State::Probing => self.want_paused = !self.want_paused,
            State::Error(_) => {}
        }
    }

    pub fn seek(&mut self, t: f64, audio: &mut AudioOut) {
        let t = t.clamp(0.0, (self.duration() - 0.05).max(0.0));
        if self.state == State::Ended {
            self.want_paused = true;
        }
        self.start_at(t, audio);
    }

    /// Advance playback; call every frame the player is visible.
    pub fn update(&mut self, ctx: &Context, audio: &mut AudioOut) {
        if let Some(rx) = &self.probe {
            match rx.try_recv() {
                Ok(Ok(info)) => {
                    self.probe = None;
                    self.info = Some(info);
                    self.start_at(0.0, audio);
                    if self.want_paused {
                        self.refresh_frame = true;
                    }
                }
                Ok(Err(e)) => {
                    self.probe = None;
                    self.state = State::Error(e);
                }
                Err(_) => {
                    ctx.request_repaint_after(Duration::from_millis(30));
                    return;
                }
            }
        }
        let Some(fps) = self.info.as_ref().map(|i| i.fps) else { return };
        let dur = self.duration();
        let Some(s) = &mut self.session else { return };

        let now = s.clock();
        let mut latest: Option<Vec<u8>> = None;
        loop {
            if s.pending.is_none() {
                match s.frames.as_ref().map(|rx| rx.try_recv()) {
                    Some(Ok(f)) => s.pending = Some(f),
                    Some(Err(TryRecvError::Disconnected)) => s.video_done = true,
                    _ => {}
                }
            }
            let Some(f) = s.pending.take() else { break };
            let due = s.start + s.next_frame as f64 / fps;
            if due <= now || (self.refresh_frame && latest.is_none()) {
                s.next_frame += 1;
                latest = Some(f);
                if self.refresh_frame && self.state == State::Paused {
                    break;
                }
            } else {
                s.pending = Some(f);
                break;
            }
        }
        if let Some(px) = latest {
            self.refresh_frame = false;
            let img = ColorImage::from_rgba_unmultiplied(s.frame_size, &px);
            match &mut self.texture {
                Some(t) if t.size() == s.frame_size => t.set(img, TextureOptions::LINEAR),
                _ => self.texture = Some(ctx.load_texture(format!("video:{}", self.path.display()), img, TextureOptions::LINEAR)),
            }
        }
        if self.state == State::Playing {
            self.position = if dur > 0.0 { now.min(dur) } else { now };
            let ended = s.video_done && s.pending.is_none() && s.audio_done();
            if ended {
                if self.looping {
                    self.start_at(0.0, audio);
                } else {
                    self.state = State::Ended;
                    self.position = dur;
                    self.session = None;
                }
            }
            ctx.request_repaint_after(Duration::from_secs_f64((0.5 / fps).max(0.004)));
        }
    }
}
