#!/usr/bin/env python3
"""Render the menu bar icon of the resident state (run once after the app icon changes).

Usage: python3 scripts/gen-tray-icon.py   (needs Pillow)

Produces src-tauri/icons/tray-mac.png: the app icon itself, in its own colours, for
the macOS menu bar — the icon stands for vemry and should read as it (a monochrome
template "v" was tried and read as foreign to the logo). The tray shows the image 18
points high, so the file is 36 px for a Retina menu bar, and the tile fills it: the
coloured application icons next to it use the full 18 points, and a 16-point tile
read as small beside them. Windows and Linux show icons/32x32.png as it is (tray.rs).
"""
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "src-tauri" / "icons"
OUT = ICONS / "tray-mac.png"

SIZE = 36  # 18 points at 2×
TILE = 36  # the full height: the neighbouring coloured icons use all of it

src = Image.open(ICONS / "128x128@2x.png").convert("RGBA")
tile = src.resize((TILE, TILE), Image.LANCZOS)
out = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
out.paste(tile, ((SIZE - TILE) // 2, (SIZE - TILE) // 2), tile)
out.save(OUT)
print(f"wrote {OUT.relative_to(ROOT)} ({SIZE}×{SIZE}, tile {TILE} px, colour)")
