#!/usr/bin/env python3
"""Generate a made-up Discord server export for trying the viewer and for screenshots.

Usage: python3 demo/make_demo.py [OUTPUT_DIR]     (default: demo/output)

Writes the same layout DiscordChatExporter + export/export.sh produce:
  OUTPUT_DIR/<category>/<channel> [<id>].json
  OUTPUT_DIR/server/{guild,channels}.json, icon.png
  OUTPUT_DIR/media/...   images, GIFs, video, audio and emoji, all generated

Needs ImageMagick (`magick`) and ffmpeg. Everyone and everything in it is made up.
"""
import json
import random
import shutil
import subprocess
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent
OUT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / "output"
MEDIA = OUT / "media"
FONT = next((f for f in ["/System/Library/Fonts/Supplemental/Arial Bold.ttf",
                         "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"] if Path(f).exists()), None)
GUILD_ID = "1100000000000000001"
rng = random.Random(7)


def run(*args):
    subprocess.run([str(a) for a in args], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def magick(*args):
    run("magick", *args)


def ffmpeg(*args):
    run("ffmpeg", "-y", "-v", "error", *args)


def media(name):
    return MEDIA / name


# ---------------------------------------------------------------------------
# Generated media

def landscape(name, sky, hills, sun, size=(1400, 900), stars=False):
    w, h = size
    draw = []
    if stars:
        for _ in range(90):
            x, y = rng.randrange(w), rng.randrange(int(h * 0.55))
            draw += ["-fill", "#ffffffcc", "-draw", f"circle {x},{y} {x+1},{y+1}"]
    sx, sy, r, col = sun
    draw += ["-fill", col, "-draw", f"circle {sx},{sy} {sx+r},{sy}"]
    for i, colr in enumerate(hills):
        base = int(h * (0.55 + i * 0.12))
        pts = [(0, h), (0, base)]
        x = 0
        while x < w:
            x += rng.randint(120, 260)
            pts.append((min(x, w), base + rng.randint(-90, 50)))
        pts.append((w, h))
        draw += ["-fill", colr, "-draw", "polygon " + " ".join(f"{px},{py}" for px, py in pts)]
    magick("-size", f"{w}x{h}", f"gradient:{sky[0]}-{sky[1]}", *draw, "-blur", "0x0.6", "-quality", "88", media(name))


def plasma_art(name, colors, seed, size="1000x1300", swirl=140):
    magick("-seed", seed, "-size", size, f"plasma:{colors}", "-blur", "0x3", "-swirl", swirl,
           "-modulate", "100,150", "-quality", "90", media(name))


def pixel_art(name, colors, seed):
    magick("-seed", seed, "-size", "40x28", f"plasma:{colors}", "-posterize", "6", "-scale", "3000%", media(name))


def meme(name, top, bottom, seed):
    landscape("_meme_bg.jpg", ("#87CEEB", "#E0F6FF"), ["#6BBF59", "#3E8E41"], (300, 200, 70, "#FFE066"), (1000, 750))
    args = ["-fill", "white", "-stroke", "black", "-strokewidth", "3", "-pointsize", "64", "-gravity"]
    if FONT:
        args = ["-font", FONT] + args
    magick(media("_meme_bg.jpg"), *args, "north", "-annotate", "+0+30", top,
           *(["-gravity", "south", "-annotate", "+0+30", bottom] if bottom else []), media(name))
    media("_meme_bg.jpg").unlink()


def mandelbrot(name):
    ffmpeg("-f", "lavfi", "-i", "mandelbrot=s=1400x1000:start_scale=0.0004:start_x=-0.743643887037151:start_y=0.131825904205330:inner=period",
           "-frames:v", "1", media(name))


def avatar(name, colors, seed, face=True):
    draw = []
    if face:
        draw = ["-fill", "white", "-draw", "circle 92,110 92,84", "-draw", "circle 164,110 164,84",
                "-fill", "#1E1F22", "-draw", "circle 96,114 96,100", "-draw", "circle 168,114 168,100",
                "-fill", "none", "-stroke", "#1E1F22", "-strokewidth", "9", "-draw", "arc 88,130 168,190 20,160"]
    magick("-seed", seed, "-size", "256x256", f"plasma:{colors}", "-blur", "0x4", *draw, media(name))


def custom_emoji():
    magick("-size", "128x128", "xc:none", "-fill", "#5DBB63", "-draw", "circle 64,74 64,14",
           "-fill", "white", "-draw", "circle 40,40 40,18", "-draw", "circle 88,40 88,18",
           "-fill", "black", "-draw", "circle 42,42 42,31", "-draw", "circle 90,42 90,31",
           "-fill", "none", "-stroke", "#2E6B30", "-strokewidth", "7", "-draw", "arc 34,62 94,104 20,160",
           media("frogwave.png"))
    magick("-size", "128x128", "xc:none", "-fill", "#FF6FA8",
           "-draw", "path 'M 64,116 C 4,72 8,10 64,38 C 120,10 124,72 64,116 Z'", media("blobheart.png"))
    star = " ".join(f"{64 + (58 if i % 2 == 0 else 24) * __import__('math').sin(i * 3.14159 / 5):.1f},"
                    f"{64 - (58 if i % 2 == 0 else 24) * __import__('math').cos(i * 3.14159 / 5):.1f}" for i in range(10))
    magick("-size", "128x128", "xc:none", "-fill", "#FFD43B", "-stroke", "#F59F00", "-strokewidth", "4",
           "-draw", f"polygon {star}", media("goldstar.png"))
    # An animated one: a bobbing moth-ish blob.
    frames = []
    for i in range(8):
        f = media(f"_moth{i}.png")
        dy = [0, -4, -8, -10, -8, -4, 0, 2][i]
        wing = [48, 40, 30, 24, 30, 40, 48, 52][i]
        magick("-size", "128x128", "xc:none", "-fill", "#C9B8F0",
               "-draw", f"ellipse {64 - wing // 2},{60 + dy} {wing // 2 + 8},34 0,360",
               "-draw", f"ellipse {64 + wing // 2},{60 + dy} {wing // 2 + 8},34 0,360",
               "-fill", "#7B5EA7", "-draw", f"ellipse 64,{66 + dy} 12,30 0,360",
               "-fill", "white", "-draw", f"circle 58,{48 + dy} 58,{42 + dy}", "-draw", f"circle 70,{48 + dy} 70,{42 + dy}", f)
        frames.append(f)
    magick("-delay", "8", "-loop", "0", "-dispose", "background", *frames, media("mothbob.gif"))
    for f in frames:
        f.unlink()


SVG_EMOJI = {
    "😂": ("joy", '<circle cx="18" cy="18" r="17" fill="#FFCC4D"/><path d="M8 13q3-4 7 0M21 13q3-4 7 0" stroke="#664500" stroke-width="2" fill="none"/><path d="M8 20q10 12 20 0z" fill="#664500"/><path d="M3 15q-3 7 2 9q4-3 0-9zM33 15q3 7-2 9q-4-3 0-9z" fill="#5DADEC"/>'),
    "❤️": ("heart", '<path d="M18 33C4 23 1 15 4 9c3-6 11-6 14 0c3-6 11-6 14 0c3 6 0 14-14 24z" fill="#DD2E44"/>'),
    "🔥": ("fire", '<path d="M18 35C8 35 4 27 7 19c2-5 6-7 6-13c5 3 7 8 6 12c2-2 3-4 3-7c6 5 9 11 8 16c-1 5-5 8-12 8z" fill="#F4900C"/><path d="M18 35c-5 0-7-4-5-9c1-3 4-4 4-8c3 2 5 6 4 9c1-1 2-2 2-4c3 3 4 6 3 8c-1 3-4 4-8 4z" fill="#FFCC4D"/>'),
    "👀": ("eyes", '<ellipse cx="11" cy="18" rx="8" ry="11" fill="#F5F8FA"/><ellipse cx="25" cy="18" rx="8" ry="11" fill="#F5F8FA"/><circle cx="13" cy="20" r="4" fill="#292F33"/><circle cx="27" cy="20" r="4" fill="#292F33"/>'),
    "✨": ("sparkles", '<path d="M14 4l3 9l9 3l-9 3l-3 9l-3-9l-9-3l9-3z" fill="#FFAC33"/><path d="M28 2l1.5 4.5l4.5 1.5l-4.5 1.5l-1.5 4.5l-1.5-4.5l-4.5-1.5l4.5-1.5z" fill="#FFCC4D"/><path d="M28 24l1.5 4l4 1.5l-4 1.5l-1.5 4l-1.5-4l-4-1.5l4-1.5z" fill="#FFCC4D"/>'),
    "🍄": ("mushroom", '<path d="M3 18C3 8 33 8 33 18c0 3-30 3-30 0z" fill="#DD2E44"/><circle cx="11" cy="12" r="2.5" fill="#fff"/><circle cx="21" cy="10" r="2" fill="#fff"/><circle cx="27" cy="15" r="2" fill="#fff"/><path d="M13 20h10l1 13H12z" fill="#F5E6CC"/>'),
    "🌙": ("crescent_moon", '<path d="M24 3a15 15 0 1 0 9 22a12 12 0 1 1-9-22z" fill="#FFD983"/>'),
    "👍": ("thumbsup", '<path d="M6 16h6v17H6zM12 17l6-12c3 0 4 2 3 6l-1 4h10c2 0 3 2 2 4l-3 11c-1 2-2 3-4 3H12z" fill="#FFCC4D"/>'),
}


def svg_emoji():
    paths = {}
    for ch, (code, body) in SVG_EMOJI.items():
        p = media(f"{'-'.join(f'{ord(c):x}' for c in ch)}.svg")
        p.write_text(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 36 36">{body}</svg>')
        paths[ch] = (code, p)
    return paths


def videos():
    # A Mandelbrot zoom with a soft chord, as a video attachment.
    ffmpeg("-t", "8", "-f", "lavfi", "-i", "mandelbrot=s=960x720:rate=30:end_scale=0.001:end_pts=200", "-t", "8", "-f", "lavfi",
           "-i", "aevalsrc=0.15*sin(2*PI*220*t)+0.1*sin(2*PI*277*t)+0.08*sin(2*PI*330*t):s=48000",
           "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", media("mandelbrot-zoom.mp4"))
    # Game of Life as a Tenor-style looping "gif" (really an mp4) plus its poster.
    ffmpeg("-t", "5", "-f", "lavfi", "-i", "life=s=160x120:mold=10:rate=12:ratio=0.12:death_color=#2B2D31:life_color=#57F287:mold_color=#5865F2",
           "-vf", "scale=640:480:flags=neighbor", "-c:v", "libx264", "-pix_fmt", "yuv420p", media("life-loop.mp4"))
    ffmpeg("-i", media("life-loop.mp4"), "-frames:v", "1", media("life-loop-thumb.jpg"))
    # A real animated GIF attachment.
    ffmpeg("-t", "4", "-f", "lavfi", "-i", "cellauto=s=320x200:rule=110:rate=15:scroll=1",
           "-vf", "split[a][b];[a]palettegen=max_colors=8[p];[b][p]paletteuse", media("rule-110.gif"))
    # A little plucky arpeggio as an audio attachment.
    expr = "0.25*sin(2*PI*t*(220*pow(2,floor(mod(t*4,8))/12*2)))*exp(-mod(t,0.25)*9)"
    ffmpeg("-f", "lavfi", "-i", f"aevalsrc='{expr}':s=44100:d=16", "-c:a", "libmp3lame", "-q:a", "4", media("moth-lullaby.mp3"))


def make_media():
    if MEDIA.exists():
        shutil.rmtree(MEDIA)
    MEDIA.mkdir(parents=True)
    print("Drawing images…")
    landscape("sunset-hills.jpg", ("#FF7E5F", "#2B1055"), ["#3B2A5C", "#1F1838", "#120E22"], (980, 430, 90, "#FFD27F"))
    landscape("night-garden.jpg", ("#0B1026", "#2B3A67"), ["#1D3B2A", "#132A1E", "#0B1A12"], (300, 180, 50, "#F6F1D5"), stars=True)
    landscape("allotment-morning.jpg", ("#A8E0FF", "#FFF6E0"), ["#7FB069", "#4F772D", "#31572C"], (200, 160, 70, "#FFE9A8"))
    landscape("lake-trip.jpg", ("#5B86E5", "#36D1DC"), ["#2D6A4F", "#1B4332", "#081C15"], (1100, 200, 60, "#FFFDE7"), (1400, 1000))
    plasma_art("luna-moth.png", "#7FFFD4-#4B0082", 3)
    plasma_art("ink-study.png", "#F2E9E4-#22223B", 11, swirl=220)
    plasma_art("glaze-test.png", "#E76F51-#264653", 5, size="1200x900", swirl=90)
    pixel_art("pixel-forest.png", "#2D6A4F-#95D5B2", 9)
    pixel_art("pixel-dusk.png", "#F72585-#3A0CA3", 4)
    mandelbrot("fractal-wallpaper.png")
    meme("build-passes.jpg", "WHEN THE BUILD PASSES", "ON THE FIRST TRY", 1)
    meme("tomatoes.jpg", "ME: I'LL PLANT A FEW TOMATOES", "ALSO ME, AUGUST:", 2)
    magick("-size", "1280x720", "gradient:#141E30-#243B55", "-fill", "#FFD27F", "-draw", "circle 640,330 640,250",
           *(["-font", FONT] if FONT else []), "-fill", "white", "-pointsize", "70", "-gravity", "south",
           "-annotate", "+0+70", "lofi moth beats", media("lofi-thumb.jpg"))
    magick("-size", "400x400", "gradient:#B7E4C7-#2D6A4F", media("nodig-thumb.jpg"))
    magick("-size", "64x64", "xc:#1185FE", media("bsky-icon.png"))
    print("Drawing avatars and emoji…")
    for i, (uid, u) in enumerate(USERS.items()):
        avatar(f"avatar-{uid}.png", u["plasma"], 100 + i, face=not u.get("bot"))
    magick("-seed", "77", "-size", "512x512", "plasma:#FF9ECD-#5B2A86", "-blur", "0x6", "-fill", "#FFF5D6",
           "-draw", "circle 256,256 256,120", "-fill", "#C291E6", "-draw", "circle 300,220 300,110", media("guild-icon.png"))
    magick("-seed", "12", "-size", "320x320", "plasma:#FFD6A5-#FF7B54", "-blur", "0x5",
           *(["-font", FONT] if FONT else []), "-fill", "white", "-stroke", "#5B2A86", "-strokewidth", "6",
           "-pointsize", "120", "-gravity", "center", "-annotate", "+0+0", "gm!", media("sticker-gm.png"))
    custom_emoji()
    print("Rendering video and audio…")
    videos()
    (MEDIA / "planting-schedule.txt").write_text(
        "Week 1: broad beans, peas\nWeek 3: lettuce, radish\nWeek 5: tomatoes (indoors!)\nWeek 8: courgettes\n")


# ---------------------------------------------------------------------------
# People, channels and messages (all invented)

USERS = {
    "juni": {"name": "juniper.b", "nick": "Juni", "color": "#E67E22", "plasma": "#FFB347-#FF6961"},
    "moth": {"name": "mothlight", "nick": None, "color": "#9B59B6", "plasma": "#C9B8F0-#5B2A86"},
    "fern": {"name": "pixelfern", "nick": "fern", "color": "#2ECC71", "plasma": "#B7E4C7-#2D6A4F"},
    "kettle": {"name": "kettle_", "nick": "kettle", "color": "#3498DB", "plasma": "#A0E7E5-#1F6FB2"},
    "orb": {"name": "orbweaver", "nick": "orb", "color": "#E91E63", "plasma": "#FFAFCC-#7B2CBF"},
    "quill": {"name": "quill", "nick": None, "color": None, "plasma": "#FFF3B0-#9E9E9E"},
    "taddy": {"name": "tadpole", "nick": "taddy", "color": "#1ABC9C", "plasma": "#B9FBC0-#0F766E"},
    "archivist": {"name": "Archivist", "nick": None, "color": "#5865F2", "plasma": "#5865F2-#2B2D31", "bot": True},
}
USER_IDS = {k: str(1100000000000000100 + i) for i, k in enumerate(USERS)}
ROLES = {
    "juni": [("mods", "#E67E22", 10), ("gardeners", "#2ECC71", 4)],
    "moth": [("artists", "#9B59B6", 6)],
    "fern": [("gardeners", "#2ECC71", 4), ("artists", "#9B59B6", 6)],
    "kettle": [("mods", "#E67E22", 10)],
    "orb": [("artists", "#9B59B6", 6), ("music club", "#E91E63", 5)],
    "taddy": [("music club", "#E91E63", 5)],
}

CATEGORIES = [
    ("1100000000000000010", "🌿 hangout", [("general", "say hi, talk about your day"), ("gardening", "seeds, soil and slugs"), ("food", "what are you cooking"), ("games", "currently playing?")]),
    ("1100000000000000011", "🎨 art", [("art", "share things you've made or found"), ("photos", None), ("memes", "official meme inspection zone")]),
    ("1100000000000000012", "🎵 music", [("music", "recs, playlists, things you made")]),
    ("1100000000000000013", "📜 archive", [("old-general", "the before times"), ("minecraft-server", "RIP the server (2021-2023)")]),
]

FILLER = {
    "general": ["morning all", "anyone up?", "just got back from the shops, it's freezing", "the cat has claimed my chair again",
                "what's everyone doing this weekend", "i need a nap", "lmao", "that's so real", "omg", "same tbh",
                "has anyone seen my umbrella i think i left it at the pub", "it's raining AGAIN", "tea or coffee, go",
                "tea obviously", "coffee and i'm not sorry", "i finally fixed the wifi", "brb", "back",
                "do we have a movie night this week?", "i could do thursday", "going to bed, night!", "night!",
                "hahaha", "that's amazing", "no way", "honestly iconic", "i've been thinking about getting a bike"],
    "gardening": ["the tomatoes are finally going red", "slugs ate all my lettuce again", "anyone tried no-dig beds?",
                  "my basil is thriving and i don't know why", "frost forecast tonight, covering everything",
                  "how deep do you plant garlic?", "about 5cm, pointy end up", "the courgettes are out of control",
                  "free seeds at the library this saturday!", "my sunflower is taller than me now"],
    "food": ["made a lasagne, 10/10", "what do i do with 4kg of courgettes", "soup season is here", "best biscuit, wrong answers only",
             "tried making bread and it's a brick", "the brick bread is honestly still tasty", "ramen night!!",
             "who wants my sourdough starter"],
    "games": ["has anyone played the new zelda yet", "stuck on the water temple again", "100% completed it finally",
              "the speedrun community is insane", "co-op tonight?", "i'm in", "my save file corrupted 😭",
              "this boss is impossible", "just gotta git gud"],
    "art": ["new sketch!", "working on colours today", "this is so pretty", "the lighting in this is unreal",
            "wip, not sure about the background", "love the palette", "i tried gouache for the first time",
            "how do you get the texture like that?", "drawing every day this month", "day 12 of the challenge"],
    "photos": ["from my walk this morning", "the light was perfect", "phone camera but still", "golden hour hits different",
               "went to the coast", "foggy day"],
    "memes": ["lmaooo", "this is me", "too real", "i'm crying", "why is this so accurate", "sending this to my mum"],
    "music": ["on repeat all week", "this album is so good", "anyone going to the gig on friday?", "new playlist dropped",
              "i've been learning bass", "the bridge in this song 😭", "vinyl haul!", "what's everyone listening to"],
    "old-general": ["hello world", "first!", "this server is so cool", "what should we call the channels", "welcome everyone!"],
    "minecraft-server": ["server's back up", "who blew up my house", "it was a creeper i promise", "the castle is finished!",
                         "we need more iron", "server going down for updates"],
}
REACTION_POOL = ["😂", "❤️", "🔥", "👀", "✨", "👍", ":frogwave:", ":blobheart:", ":goldstar:", ":mothbob:"]


class Builder:
    def __init__(self, svgs):
        self.svgs = svgs
        self.seq = 0

    def snowflake(self, ts):
        self.seq += 1
        ms = int(ts.timestamp() * 1000) - 1420070400000
        return str((ms << 22) + self.seq)

    def user(self, key):
        u = USERS[key]
        return {
            "id": USER_IDS[key], "name": u["name"], "discriminator": "0000", "nickname": u["nick"] or u["name"],
            "color": u["color"], "isBot": bool(u.get("bot")),
            "roles": [{"id": str(1100000000000000900 + p), "name": n, "color": c, "position": p} for n, c, p in ROLES.get(key, [])],
            "avatarUrl": str(media(f"avatar-{key}.png")),
        }

    def emoji(self, token):
        if token.startswith(":"):
            name = token.strip(":")
            ext = "gif" if name == "mothbob" else "png"
            return {"id": str(1100000000000000500 + sum(map(ord, name))), "name": name, "code": name,
                    "isAnimated": ext == "gif", "imageUrl": str(media(f"{name}.{ext}"))}
        code, path = self.svgs[token]
        return {"id": "", "name": token, "code": code, "isAnimated": False, "imageUrl": str(path)}

    def message(self, ch, author, ts, content="", *, reply_to=None, attachments=(), embeds=(), reactions=None,
                pinned=False, edited=False, stickers=(), mentions=()):
        mid = self.snowflake(ts)
        inline = []
        for tok in list(SVG_EMOJI) + [":frogwave:", ":blobheart:", ":goldstar:", ":mothbob:"]:
            if tok in content:
                inline.append(self.emoji(tok))
        msg = {
            "id": mid, "type": "Reply" if reply_to else "Default",
            "timestamp": ts.isoformat(timespec="milliseconds"),
            "timestampEdited": (ts + timedelta(minutes=2)).isoformat(timespec="milliseconds") if edited else None,
            "callEndedTimestamp": None, "isPinned": pinned, "content": content, "author": self.user(author),
            "attachments": [], "embeds": list(embeds), "stickers": [], "reactions": [],
            "mentions": [self.user(m) for m in mentions], "inlineEmojis": inline,
        }
        for name in attachments:
            p = media(name)
            msg["attachments"].append({"id": self.snowflake(ts), "url": str(p), "fileName": name, "fileSizeBytes": p.stat().st_size})
        for name in stickers:
            msg["stickers"].append({"id": "1100000000000000700", "name": "gm", "format": "Png", "sourceUrl": str(media(name))})
        for tok, who in (reactions or {}).items():
            msg["reactions"].append({"emoji": self.emoji(tok), "count": len(who), "users": [self.user(w) for w in who]})
        if reply_to:
            msg["reference"] = {"type": "Default", "messageId": reply_to["id"], "channelId": ch, "guildId": GUILD_ID}
        return msg


def filler(b, ch_id, ch_name, start, end, n):
    lines = FILLER[ch_name]
    people = [k for k in USERS if k != "archivist"]
    times = sorted(start + timedelta(seconds=rng.uniform(0, (end - start).total_seconds())) for _ in range(n))
    out = []
    for ts in times:
        who = rng.choice(people)
        text = rng.choice(lines)
        reply = out[-1] if out and rng.random() < 0.08 else None
        reactions = None
        if rng.random() < 0.12:
            reactions = {rng.choice(REACTION_POOL): rng.sample(people, rng.randint(1, 3))}
        out.append(b.message(ch_id, who, ts, text, reply_to=reply, reactions=reactions))
    return out


def at(day, hh, mm):
    return datetime(2026, 10, day, hh, mm, tzinfo=timezone.utc)


def showcase(b):
    """Hand-written recent conversations, so the bottom of each channel looks lively."""
    s = {}

    m = []
    m.append(b.message("general", "kettle", at(17, 9, 2), "morning everyone ☕"))
    m.append(b.message("general", "juni", at(17, 9, 4), "morning!! the frost finally got the dahlias 😂",
                       reactions={"😂": ["kettle", "fern"], ":blobheart:": ["moth"]}))
    m.append(b.message("general", "fern", at(17, 9, 5), "noooo the dahlias"))
    m.append(b.message("general", "fern", at(17, 9, 5), "you can dig the tubers up and keep them in the shed over winter though"))
    m.append(b.message("general", "juni", at(17, 9, 7), "wait really? i've been letting them die every year like a fool :frogwave:", reply_to=m[-1],
                       mentions=["fern"]))
    m.append(b.message("general", "moth", at(17, 9, 31), "@Juni movie night is still on for thursday right?", mentions=["juni"]))
    m.append(b.message("general", "juni", at(17, 9, 33), "yes!! 8pm, bring snacks. we're watching **Spirited Away** unless someone has a ~~better~~ different idea",
                       reply_to=m[-1], mentions=["moth"], reactions={"🔥": ["moth", "orb", "taddy"], "👍": ["kettle"]}, edited=True))
    m.append(b.message("general", "orb", at(17, 9, 40), "the ending still gets me every time. ||the train scene|| 😭"))
    m.append(b.message("general", "taddy", at(17, 10, 12), "https://tenor.com/view/demo-life-gif-1100",
                       embeds=[{"title": None, "url": "https://tenor.com/view/demo-life-gif-1100", "timestamp": None, "description": None,
                                "thumbnail": {"url": str(media("life-loop-thumb.jpg")), "width": 640, "height": 480},
                                "video": {"url": str(media("life-loop.mp4")), "width": 640, "height": 480},
                                "images": [], "fields": [], "inlineEmojis": []}],
                       reactions={"👀": ["juni", "kettle"]}))
    m.append(b.message("general", "taddy", at(17, 10, 13), "conway would be proud"))
    m.append(b.message("general", "kettle", at(17, 10, 20), "anyway i put the planting schedule for the allotment together, shout if i got anything wrong",
                       attachments=["planting-schedule.txt"], pinned=True, reactions={"✨": ["juni", "fern", "quill"]}))
    m.append(b.message("general", "quill", at(17, 10, 26), "this is great, thank you :goldstar:"))
    m.append(b.message("general", "archivist", at(17, 12, 0), "📌 Reminder: movie night is **Thursday at 8pm** in #general. Snacks encouraged."))
    m.append(b.message("general", "fern", at(17, 12, 3), "", stickers=["sticker-gm.png"]))
    m.append(b.message("general", "moth", at(17, 12, 5), ":mothbob:"))
    s["general"] = m

    m = []
    m.append(b.message("art", "moth", at(15, 20, 14), "finished the luna moth piece!! took me about two weeks on and off",
                       attachments=["luna-moth.png"], pinned=True,
                       reactions={"❤️": ["juni", "fern", "orb", "taddy", "kettle"], ":blobheart:": ["orb", "quill"], "✨": ["fern"]}))
    m.append(b.message("art", "orb", at(15, 20, 20), "MOTH this is incredible", reply_to=m[-1], mentions=["moth"]))
    m.append(b.message("art", "fern", at(15, 20, 41), "been doing pixel art again, two little scenes", attachments=["pixel-forest.png", "pixel-dusk.png"],
                       reactions={"🔥": ["moth", "taddy"]}))
    m.append(b.message("art", "kettle", at(16, 11, 2), "https://bsky.app/profile/inkmoth.example/post/demo",
                       embeds=[{"title": None, "url": "https://bsky.app/profile/inkmoth.example/post/demo", "timestamp": None,
                                "description": "late night ink study, trying a looser line #art #inktober", "color": "#1185FE",
                                "author": {"name": "inkmoth (@inkmoth.example)", "url": "https://bsky.app/profile/inkmoth.example",
                                           "iconUrl": str(media("avatar-moth.png"))},
                                "images": [{"url": str(media("ink-study.png")), "width": 1000, "height": 1300}],
                                "fields": [], "footer": {"text": "Bluesky", "iconUrl": str(media("bsky-icon.png"))}, "inlineEmojis": []}]))
    m.append(b.message("art", "taddy", at(16, 18, 30), "rendered a little fractal zoom with sound 🍄", attachments=["mandelbrot-zoom.mp4"]))
    m.append(b.message("art", "juni", at(16, 18, 45), "and a cellular automaton for good measure", attachments=["rule-110.gif"],
                       reactions={"👀": ["taddy"]}))
    m.append(b.message("art", "quill", at(17, 8, 50), "glaze test from pottery class, it came out way more dramatic than planned",
                       attachments=["glaze-test.png"], reactions={"😂": ["juni"], "🔥": ["moth", "fern", "orb"]}))
    s["art"] = m

    m = []
    m.append(b.message("photos", "juni", at(12, 7, 40), "allotment at 7am, nobody else around", attachments=["allotment-morning.jpg"]))
    m.append(b.message("photos", "kettle", at(13, 19, 2), "the sunset on the drive home", attachments=["sunset-hills.jpg"],
                       reactions={"❤️": ["juni", "orb"]}))
    m.append(b.message("photos", "orb", at(14, 23, 30), "stargazing in the garden 🌙", attachments=["night-garden.jpg", "lake-trip.jpg"]))
    m.append(b.message("photos", "taddy", at(16, 14, 0), "made a wallpaper out of the fractal render", attachments=["fractal-wallpaper.png"]))
    s["photos"] = m

    m = []
    m.append(b.message("memes", "kettle", at(14, 16, 0), "", attachments=["build-passes.jpg"], reactions={"😂": ["juni", "fern", "taddy", "orb"]}))
    m.append(b.message("memes", "fern", at(15, 9, 0), "this one's for juni", attachments=["tomatoes.jpg"], reactions={"😂": ["juni", "moth"]}))
    s["memes"] = m

    m = []
    m.append(b.message("music", "orb", at(16, 21, 0), "made a little loop tonight, it's very rough but i like it", attachments=["moth-lullaby.mp3"],
                       reactions={"❤️": ["moth", "taddy"]}))
    m.append(b.message("music", "taddy", at(16, 21, 30), "perfect writing music https://www.youtube.com/watch?v=demo",
                       embeds=[{"title": "lofi moth beats to garden to", "url": "https://www.youtube.com/watch?v=demo", "timestamp": None,
                                "description": "Three hours of soft beats, rain and the occasional moth. Made for long evenings potting up seedlings.",
                                "color": "#FF0000", "author": {"name": "Night Garden Radio", "url": "https://www.youtube.com/@example"},
                                "thumbnail": {"url": str(media("lofi-thumb.jpg")), "width": 1280, "height": 720},
                                "video": {"url": "https://www.youtube.com/embed/demo", "width": 1280, "height": 720},
                                "images": [], "fields": [], "inlineEmojis": []}]))
    s["music"] = m

    m = []
    m.append(b.message("gardening", "fern", at(13, 10, 0), "good intro to no-dig if anyone's curious https://example.com/blog/no-dig-gardening",
                       embeds=[{"title": "No-dig gardening: a beginner's guide", "url": "https://example.com/blog/no-dig-gardening",
                                "timestamp": None, "description": "Why you can skip the digging, how to start a bed with cardboard and compost, and what to plant first.",
                                "color": "#4E9F3D", "author": None, "thumbnail": {"url": str(media("nodig-thumb.jpg")), "width": 400, "height": 400},
                                "images": [], "fields": [], "inlineEmojis": []}]))
    s["gardening"] = m
    return s


def main():
    for tool in ("magick", "ffmpeg"):
        if not shutil.which(tool):
            sys.exit(f"{tool} not found: install ImageMagick and ffmpeg first")
    if OUT.exists():
        for p in OUT.iterdir():
            shutil.rmtree(p) if p.is_dir() else p.unlink()
    make_media()
    svgs = svg_emoji()
    b = Builder(svgs)
    print("Writing channels…")

    channel_ids = {}
    api_channels = []
    pos = 0
    for cat_id, cat_name, chans in CATEGORIES:
        api_channels.append({"id": cat_id, "type": 4, "name": cat_name, "position": len(api_channels), "parent_id": None})
        for name, topic in chans:
            cid = str(1100000000000001000 + pos)
            channel_ids[name] = (cid, cat_id, cat_name, topic)
            api_channels.append({"id": cid, "type": 0, "name": name, "position": pos, "parent_id": cat_id, "topic": topic})
            pos += 1

    shows = showcase(b)
    counts = {"general": 900, "gardening": 260, "food": 180, "games": 220, "art": 300, "photos": 90, "memes": 120,
              "music": 200, "old-general": 400, "minecraft-server": 350}
    for name, (cid, cat_id, cat_name, topic) in channel_ids.items():
        archive = cat_name.endswith("archive")
        start = datetime(2021, 3, 1, tzinfo=timezone.utc) if archive else datetime(2023, 1, 10, tzinfo=timezone.utc)
        end = datetime(2023, 5, 1, tzinfo=timezone.utc) if archive else datetime(2026, 10, 10, tzinfo=timezone.utc)
        msgs = filler(b, cid, name, start, end, counts[name]) + shows.get(name, [])
        for msg in msgs:
            if "reference" in msg:
                msg["reference"]["channelId"] = cid
        write_channel(cat_id, cat_name, cid, name, topic, "GuildTextChat", msgs)

    # A thread under #art.
    tid = "1100000000000002000"
    t = [b.message(tid, "moth", at(10, 18, 0), "weekly prompt: **moths**! post anything moth-related here 🌙"),
         b.message(tid, "fern", at(10, 19, 12), "pixel moth incoming", attachments=["pixel-dusk.png"]),
         b.message(tid, "orb", at(11, 9, 3), ":mothbob: :mothbob: :mothbob:")]
    write_channel(channel_ids["art"][0], "art", tid, "weekly prompt: moths", None, "GuildPublicThread", t)

    server = OUT / "server"
    server.mkdir(parents=True, exist_ok=True)
    (server / "guild.json").write_text(json.dumps({"id": GUILD_ID, "name": "the night garden"}, indent=2))
    (server / "channels.json").write_text(json.dumps(api_channels, indent=2, ensure_ascii=False))
    shutil.copy(media("guild-icon.png"), server / "icon.png")
    total = sum(counts.values()) + sum(len(v) for v in shows.values()) + len(t)
    print(f"Done: {total} made-up messages in {OUT}")
    print(f"View it with: cargo run --release -- {OUT}")


def write_channel(cat_id, cat_name, cid, name, topic, kind, msgs):
    d = OUT / cat_name
    d.mkdir(parents=True, exist_ok=True)
    doc = {
        "guild": {"id": GUILD_ID, "name": "the night garden", "iconUrl": str(media("guild-icon.png"))},
        "channel": {"id": cid, "type": kind, "categoryId": cat_id, "category": cat_name, "name": name, "topic": topic},
        "dateRange": {"after": None, "before": None},
        "exportedAt": "2026-10-18T12:00:00+00:00",
        "messages": msgs,
        "messageCount": len(msgs),
    }
    (d / f"{name} [{cid}].json").write_text(json.dumps(doc, indent=1, ensure_ascii=False))


if __name__ == "__main__":
    main()
