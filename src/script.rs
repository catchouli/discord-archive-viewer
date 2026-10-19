//! A tiny scripting hook for automated screenshots and smoke tests.
//!
//! Set `DISCORD_VIEWER_SCRIPT` to commands separated by `|`, e.g.
//! `channel:art|sleep:1500|shot:/tmp/art.png|search:from:juni has:image|sleep:800|shot:/tmp/s.png|quit`
//!
//! Commands: `channel:<name>`, `at:<0..1>` (jump to that fraction of the channel),
//! `jump:<message id>`, `search:<query>`, `pins`, `lightbox:<n>`, `lbvideo:<n>`,
//! `lbnext`, `closelb`, `sleep:<ms>`, `shot:<png path>`, `quit`.

use std::collections::VecDeque;

pub struct Script {
    cmds: VecDeque<String>,
    wait_until: f64,
}

pub enum Cmd {
    Channel(String),
    At(f32),
    Jump(u64),
    Search(String),
    Pins,
    Lightbox { n: isize, video: bool },
    LightboxNext,
    CloseLightbox,
    Quit,
}

impl Script {
    pub fn from_env() -> Option<Self> {
        let s = std::env::var("DISCORD_VIEWER_SCRIPT").ok()?;
        Some(Self { cmds: s.split('|').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect(), wait_until: 0.0 })
    }

    /// Returns the next command to run this frame, handling sleeps and screenshots itself.
    pub fn next(&mut self, ctx: &egui::Context) -> Option<Cmd> {
        let now = ctx.input(|i| i.time);
        ctx.request_repaint();
        if now < self.wait_until {
            return None;
        }
        let cmd = self.cmds.pop_front()?;
        let (name, arg) = cmd.split_once(':').unwrap_or((cmd.as_str(), ""));
        eprintln!("script: {cmd}");
        match name {
            "sleep" => {
                self.wait_until = now + arg.parse::<f64>().unwrap_or(500.0) / 1000.0;
                None
            }
            "shot" => {
                let path = arg.to_string();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                    let [w, h] = img.size;
                    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                    match image::RgbaImage::from_raw(w as u32, h as u32, bytes).map(|i| i.save(&path)) {
                        Some(Ok(())) => eprintln!("script: saved {path}"),
                        other => eprintln!("script: couldn't save {path}: {other:?}"),
                    }
                })));
                // Give the screenshot a moment to be taken before changing anything.
                self.wait_until = now + 0.3;
                None
            }
            "channel" => Some(Cmd::Channel(arg.to_string())),
            "at" => Some(Cmd::At(arg.parse().unwrap_or(0.5))),
            "jump" => arg.parse().ok().map(Cmd::Jump),
            "search" => Some(Cmd::Search(arg.to_string())),
            "pins" => Some(Cmd::Pins),
            "lightbox" => Some(Cmd::Lightbox { n: arg.parse().unwrap_or(0), video: false }),
            "lbvideo" => Some(Cmd::Lightbox { n: arg.parse().unwrap_or(0), video: true }),
            "lbnext" => Some(Cmd::LightboxNext),
            "closelb" => Some(Cmd::CloseLightbox),
            "quit" => Some(Cmd::Quit),
            other => {
                eprintln!("script: unknown command {other}");
                None
            }
        }
    }
}
