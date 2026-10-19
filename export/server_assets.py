#!/usr/bin/env python3
"""Save a Discord server's full custom emoji and sticker sets, plus server metadata.

Usage: DISCORD_TOKEN=... server_assets.py <guild_id> <output_dir>

Writes:
  <output_dir>/guild.json       server info (name, icon, roles, ...)
  <output_dir>/emojis.json      emoji list as returned by Discord
  <output_dir>/stickers.json    sticker list as returned by Discord
  <output_dir>/channels.json    channel list (order, categories, topics, ...)
  <output_dir>/emoji/           one image per emoji, named "<name> [<id>].<ext>"
  <output_dir>/stickers/        one file per sticker, named the same way
  <output_dir>/icon.*, banner.*, splash.*   server images, where set
"""
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

API = "https://discord.com/api/v10"
CDN = "https://cdn.discordapp.com"
USER_AGENT = "DiscordBot (https://github.com/Tyrrrz/DiscordChatExporter, 1.0) server-assets"

# Sticker format_type -> (file extension, URL host)
STICKER_FORMATS = {
    1: ("png", CDN),                            # PNG
    2: ("png", CDN),                            # APNG
    3: ("json", CDN),                           # Lottie
    4: ("gif", "https://media.discordapp.net"), # GIF
}


def fetch(url, auth=False):
    headers = {"User-Agent": USER_AGENT}
    if auth:
        headers["Authorization"] = f"Bot {os.environ['DISCORD_TOKEN']}"
    for attempt in range(5):
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=60) as r:
                return r.read()
        except urllib.error.HTTPError as e:
            if e.code == 429:
                time.sleep(float(e.headers.get("Retry-After", 2)) + 0.5)
                continue
            raise
    raise RuntimeError(f"Still rate limited after retries: {url}")


def safe_name(name):
    return re.sub(r'[/\\:*?"<>|\x00-\x1f]', "_", name).strip() or "unnamed"


def download(url, path):
    if path.exists():
        return
    try:
        path.write_bytes(fetch(url))
    except urllib.error.HTTPError as e:
        print(f"  ! {path.name}: HTTP {e.code}", file=sys.stderr)


def main():
    guild_id, out = sys.argv[1], Path(sys.argv[2])
    (out / "emoji").mkdir(parents=True, exist_ok=True)
    (out / "stickers").mkdir(parents=True, exist_ok=True)

    guild = json.loads(fetch(f"{API}/guilds/{guild_id}", auth=True))
    emojis = json.loads(fetch(f"{API}/guilds/{guild_id}/emojis", auth=True))
    stickers = json.loads(fetch(f"{API}/guilds/{guild_id}/stickers", auth=True))
    # Channel/category order, topics, NSFW flags etc., for rebuilding the sidebar.
    channels = json.loads(fetch(f"{API}/guilds/{guild_id}/channels", auth=True))
    for name, data in (("guild", guild), ("emojis", emojis), ("stickers", stickers), ("channels", channels)):
        (out / f"{name}.json").write_text(json.dumps(data, indent=2, ensure_ascii=False))

    # Server icon, banner and invite splash, at full size.
    for field, folder in (("icon", "icons"), ("banner", "banners"), ("splash", "splashes")):
        if h := guild.get(field):
            ext = "gif" if h.startswith("a_") else "png"
            download(f"{CDN}/{folder}/{guild_id}/{h}.{ext}?size=4096", out / f"{field}.{ext}")

    print(f"Downloading {len(emojis)} emoji...")
    for e in emojis:
        ext = "gif" if e.get("animated") else "png"
        download(f"{CDN}/emojis/{e['id']}.{ext}?size=4096",
                 out / "emoji" / f"{safe_name(e['name'])} [{e['id']}].{ext}")

    print(f"Downloading {len(stickers)} stickers...")
    for s in stickers:
        ext, host = STICKER_FORMATS.get(s.get("format_type"), ("png", CDN))
        download(f"{host}/stickers/{s['id']}.{ext}",
                 out / "stickers" / f"{safe_name(s['name'])} [{s['id']}].{ext}")

    print(f"Saved server assets to {out}")


if __name__ == "__main__":
    main()
