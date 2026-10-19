# Working on discord-viewer

## Layout

| Path | What it is |
| --- | --- |
| `src/model.rs` | Loads an export (the DiscordChatExporter JSON plus `server/`) in parallel and orders the sidebar |
| `src/markdown.rs` | Parses Discord's markdown |
| `src/search.rs` | Parses and runs search queries |
| `src/media.rs` | Background image decoding, file type sniffing and the texture cache |
| `src/player.rs` | Video and audio playback through ffmpeg |
| `src/view.rs` | The virtualised message list |
| `src/render.rs` | Draws messages, embeds, attachments and players |
| `src/lightbox.rs` | The full-screen media viewer |
| `src/app.rs` | Sidebar, header, search panel and app state |
| `src/script.rs` | Scripted runs for screenshots |
| `export/` | The export script and `server_assets.py`, which saves server info, emoji and stickers |
| `demo/make_demo.py` | Generates the made-up demo server |

## Checking an export

`--check` loads an export without opening a window. It parses every message, counts missing attachments and times a few searches:

```sh
cargo run --release -- --check demo/output
```

`cargo test` covers the markdown parser and search queries.

## Screenshots

The viewer can drive itself and save screenshots. Put commands separated by `|` in `DISCORD_VIEWER_SCRIPT`. The commands are listed at the top of `src/script.rs`.

The README screenshots come from the demo server:

```sh
python3 demo/make_demo.py
DISCORD_VIEWER_SCRIPT='channel:general|sleep:3000|shot:/tmp/chat.png|search:has:image|sleep:2500|shot:/tmp/search.png|quit' \
  cargo run --release -- demo/output
```

`jump:<message id>` scrolls to a message, and `lightbox:<n>` opens the n-th image in the current channel. Give images a couple of seconds to load before each `shot:`. Then convert the PNGs to `screenshots/<name>.jpg` plus a 640px-wide `<name>-thumb.jpg`.

Never take screenshots of a real export for the repo.
