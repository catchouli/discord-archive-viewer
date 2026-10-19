#!/usr/bin/env bash
# Export every channel (including threads and forum posts) of a Discord server
# to JSON, with attachments, avatars and emoji downloaded locally, using
# DiscordChatExporter (https://github.com/Tyrrrz/DiscordChatExporter).
#
# Usage: export/export.sh [SERVER_ID] [extra DiscordChatExporter options...]
#   e.g. export/export.sh 123456789012345678 --parallel 3
#
# SERVER_ID can be left out if it's saved in private/server-id.
# The bot token comes from $DISCORD_TOKEN, or private/token.
#
# Each run writes a fresh snapshot to private/data/exports/<UTC timestamp>/ and
# points private/data/latest at it. Media goes to the shared private/data/media/
# folder and is reused between runs. Server info, icon, channel order and the
# full emoji/sticker sets go to <snapshot>/server/ (see server_assets.py).
set -euo pipefail

DCE_VERSION="2.48"

TOOL_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$TOOL_DIR/.." && pwd)"
PRIVATE="$ROOT/private"
BIN="$PRIVATE/bin/DiscordChatExporter.Cli"
TOKEN_FILE="$PRIVATE/token"

if [[ $# -gt 0 && "$1" =~ ^[0-9]+$ ]]; then
  GUILD_ID="$1"
  shift
elif [[ -s "$PRIVATE/server-id" ]]; then
  GUILD_ID="$(tr -d '[:space:]' < "$PRIVATE/server-id")"
else
  echo "Usage: $0 SERVER_ID [extra DiscordChatExporter options...]" >&2
  echo "(or save the server ID in private/server-id)" >&2
  exit 1
fi

# Download the CLI if it isn't there yet, checking it against the SHA-256 that
# GitHub publishes for each file of the release. Update these with DCE_VERSION.
if [[ ! -x "$BIN" ]]; then
  case "$(uname -s)-$(uname -m)" in
    Darwin-arm64)  asset="osx-arm64";   sha256="623f9d2dce568e17a46b8fbd366a18dca49803d386216f4ba24507d2c000fee9" ;;
    Darwin-x86_64) asset="osx-x64";     sha256="91b4eae3525df85d084969004f3a287edad4eeaafd664e4869b26bb8422e2e88" ;;
    Linux-x86_64)  asset="linux-x64";   sha256="3e253e28ec7ea034b2201443fa84571142945299296541ecbe196ffceef8bc3c" ;;
    Linux-aarch64) asset="linux-arm64"; sha256="02a47fc8e0192fd509fbb082aadd9322035b18feae96849699fefc424a1e3379" ;;
    *) echo "Unsupported platform: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
  esac
  echo "Downloading DiscordChatExporter $DCE_VERSION ($asset)..."
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  curl -fsSL -o "$tmp/dce.zip" \
    "https://github.com/Tyrrrz/DiscordChatExporter/releases/download/$DCE_VERSION/DiscordChatExporter.Cli.$asset.zip"
  if command -v sha256sum >/dev/null; then
    actual="$(sha256sum "$tmp/dce.zip" | cut -d' ' -f1)"
  else
    actual="$(shasum -a 256 "$tmp/dce.zip" | cut -d' ' -f1)"
  fi
  if [[ "$actual" != "$sha256" ]]; then
    echo "Checksum mismatch for DiscordChatExporter.Cli.$asset.zip, not installing it." >&2
    echo "  expected $sha256" >&2
    echo "  got      $actual" >&2
    exit 1
  fi
  mkdir -p "$PRIVATE/bin"
  unzip -oq "$tmp/dce.zip" -d "$PRIVATE/bin"
  chmod +x "$BIN"
  [[ "$(uname -s)" == Darwin ]] && xattr -dr com.apple.quarantine "$PRIVATE/bin" 2>/dev/null || true
fi

# Passed via the environment so the token doesn't show up in `ps`.
if [[ -z "${DISCORD_TOKEN:-}" ]]; then
  if [[ ! -s "$TOKEN_FILE" ]]; then
    echo "Missing bot token: set DISCORD_TOKEN or put it in private/token" >&2
    exit 1
  fi
  DISCORD_TOKEN="$(tr -d '[:space:]' < "$TOKEN_FILE")"
fi
export DISCORD_TOKEN
export FUCK_RUSSIA=true  # hides DiscordChatExporter's startup banner

STAMP="$(date -u +%Y-%m-%dT%H%M%SZ)"
OUT="$PRIVATE/data/exports/$STAMP"
MEDIA="$PRIVATE/data/media"
mkdir -p "$OUT" "$MEDIA"

echo "Exporting server $GUILD_ID to private/data/exports/$STAMP/"
"$BIN" exportguild \
  --guild "$GUILD_ID" \
  --format Json \
  --output "$OUT/%T/%C [%c].json" \
  --include-threads All \
  --media --reuse-media \
  --media-dir "$MEDIA" \
  --utc \
  "$@" 2>&1 | tee "$OUT/export.log"

echo
echo "Saving server info, icon, full emoji and sticker sets to private/data/exports/$STAMP/server/"
python3 -I "$TOOL_DIR/server_assets.py" "$GUILD_ID" "$OUT/server" 2>&1 | tee -a "$OUT/export.log"

ln -sfn "exports/$STAMP" "$PRIVATE/data/latest"
echo
echo "Done: $(find "$OUT" -name '*.json' | wc -l | tr -d ' ') channel files in private/data/exports/$STAMP/ (private/data/latest points here)"
echo "Media: $(du -sh "$MEDIA" | cut -f1) in private/data/media/"
