"""Regenerate the branded bitmaps of the Windows NSIS installer (run once after the logo changes).

Usage: python3 scripts/gen-nsis-assets.py   (needs Pillow)

What it produces (referenced by tauri.conf.json's bundle>windows>nsis; NSIS only accepts
24-bit BMPs, and the sizes are fixed):
- src-tauri/icons/nsis-sidebar.bmp  164x314  the left column of the welcome and finish pages:
  the app panel's background colour plus the logo and the wordmark
- src-tauri/icons/nsis-header.bmp   150x57   each page's header: a small logo on white (MUI2's
  header is white by default, so it blends in)

The installer's and uninstaller's exe icons reuse icons/icon.ico directly and need no generating.
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "src-tauri" / "icons"

# Kept consistent with src/app.css's light-theme tokens
BG_PANEL = (244, 244, 242)  # --bg-panel
BORDER = (231, 229, 228)    # --border
FG = (28, 25, 23)           # --fg
WHITE = (255, 255, 255)

logo_src = Image.open(ICONS / "128x128@2x.png").convert("RGBA")


def paste_logo(canvas, size, cx, cy):
    logo = logo_src.resize((size, size), Image.LANCZOS)
    canvas.paste(logo, (cx - size // 2, cy - size // 2), logo)


def load_font(px):
    for path in ("/System/Library/Fonts/HelveticaNeue.ttc",
                 "/System/Library/Fonts/Helvetica.ttc",
                 "C:/Windows/Fonts/segoeui.ttf"):
        try:
            return ImageFont.truetype(path, px)
        except OSError:
            continue
    raise SystemExit("no usable font found; add a local font path in load_font")


def draw_tracked(draw, center_x, y, text, font, fill, tracking):
    widths = [draw.textlength(ch, font=font) for ch in text]
    total = sum(widths) + tracking * (len(text) - 1)
    x = center_x - total / 2
    for ch, w in zip(text, widths):
        draw.text((x, y), ch, font=font, fill=fill)
        x += w + tracking


S = 2  # drawn at 2x and scaled back down, for cleaner antialiasing

# the sidebar, 164x314
w, h = 164 * S, 314 * S
img = Image.new("RGBA", (w, h), BG_PANEL + (255,))
d = ImageDraw.Draw(img)
paste_logo(img, 76 * S, w // 2, 96 * S)
draw_tracked(d, w // 2, 158 * S, "vemry", load_font(23 * S), FG, tracking=int(1.5 * S))
d.rectangle([w - S, 0, w - 1, h - 1], fill=BORDER)  # a 1px right border once scaled back to 1x
img.resize((164, 314), Image.LANCZOS).convert("RGB").save(ICONS / "nsis-sidebar.bmp", "BMP")

# the header, 150x57
w, h = 150 * S, 57 * S
img = Image.new("RGBA", (w, h), WHITE + (255,))
paste_logo(img, 30 * S, w // 2, h // 2)
img.resize((150, 57), Image.LANCZOS).convert("RGB").save(ICONS / "nsis-header.bmp", "BMP")

print("generated nsis-sidebar.bmp / nsis-header.bmp")
