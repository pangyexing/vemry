#!/usr/bin/env python3
"""Regenerate every app icon from the one mark (run after the mark or its colours change).

Usage: python3 scripts/gen-app-icons.py   (needs pnpm with the Tauri CLI; the two scripts
it runs last need Pillow)

The mark is a knot tied in the corner of a block: a rounded square whose top edge runs
on past the corner into a loop, and the loop comes back down as the square's right side,
passing under the top edge. It is one stroke. Where the right side passes under the top
edge the path really stops and starts again (nothing is painted over the line in the
background colour), so the mark stays clean on any ground.

Writes into src-tauri/icons/:
- logo.svg: the mark alone in currentColor, for anything that draws it at text size
- the bundle icons `tauri icon` makes from a 1024 canvas with the tile inset to macOS's
  icon grid; only the files already tracked here are copied back (the command also makes
  iOS and Android sets this app does not ship)
- 32x32.png again, from a full-bleed tile: a Windows or Linux tray shows that file as it
  is (tray.rs), and an inset tile reads small there
then runs gen-tray-icon.py and gen-nsis-assets.py, which derive from 128x128@2x.png.
"""
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "src-tauri" / "icons"
SCRIPTS = ROOT / "scripts"

TILE = "#0f766e"  # --accent in src/app.css
INK = "#fafaf9"   # the light theme's paper

# The mark in its own units. The square spans 0..42 with corners of radius 12; the loop is
# a circle of radius 11 centred at (53, -11); the line is 11 wide. The right side stops at
# y = -11 above the top edge and starts again at y = 11 below it, which leaves 5.5 of clear
# gap on each side of the edge's 11-wide line.
MARK = ("M42 11L42 30A12 12 0 0 1 30 42L12 42A12 12 0 0 1 0 30L0 12A12 12 0 0 1 12 0"
        "L53 0A11 11 0 1 0 42 -11")
STROKE = 11
BOX_X, BOX_Y, BOX_SIDE = -5.5, -27.5, 75  # the square the stroked mark fills

MARK_OF_TILE = 0.6825  # the mark's share of the tile's side
CORNER = 0.224         # the tile's corner radius as a share of its side


def path(scale, dx, dy, colour):
    return (f'<path transform="translate({dx:.3f} {dy:.3f}) scale({scale:.5f})" d="{MARK}" '
            f'fill="none" stroke="{colour}" stroke-width="{STROKE}" stroke-linejoin="round"/>')


def tile_svg(inset):
    size = 1024
    side = size - 2 * inset
    mark = side * MARK_OF_TILE
    scale = mark / BOX_SIDE
    left = inset + (side - mark) / 2
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {size} {size}">'
            f'<rect x="{inset}" y="{inset}" width="{side}" height="{side}" rx="{side * CORNER:.1f}" fill="{TILE}"/>'
            f'{path(scale, left - BOX_X * scale, left - BOX_Y * scale, INK)}</svg>')


def logo_svg():
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{BOX_X} {BOX_Y} {BOX_SIDE} {BOX_SIDE}">'
            f'{path(1, 0, 0, "currentColor")}</svg>\n')


def tauri_icon(source, out, *extra):
    subprocess.run(["pnpm", "tauri", "icon", str(source), "-o", str(out), *extra],
                   cwd=ROOT, check=True, stdout=subprocess.DEVNULL)


(ICONS / "logo.svg").write_text(logo_svg())

with tempfile.TemporaryDirectory() as tmp:
    tmp = Path(tmp)
    (tmp / "app-icon.svg").write_text(tile_svg(inset=96))  # the old icon's 832 tile on 1024
    (tmp / "small.svg").write_text(tile_svg(inset=0))
    tauri_icon(tmp / "app-icon.svg", tmp / "full")
    tauri_icon(tmp / "small.svg", tmp / "small", "--png", "32")
    copied = []
    for made in sorted((tmp / "full").iterdir()):
        target = ICONS / made.name
        if made.is_file() and target.exists():
            shutil.copyfile(made, target)
            copied.append(made.name)
    shutil.copyfile(tmp / "small" / "32x32.png", ICONS / "32x32.png")

print(f"wrote logo.svg and {len(copied)} bundle icons; 32x32.png full-bleed", flush=True)
for script in ("gen-tray-icon.py", "gen-nsis-assets.py"):
    subprocess.run([sys.executable, str(SCRIPTS / script)], check=True)
