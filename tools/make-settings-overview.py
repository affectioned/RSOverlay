"""Pack every RSOverlay Settings card into one overview image.

Run from the repo root:
    cargo run -p rsoverlay-app -- --settings-cards target/settings-cards
    python tools/make-settings-overview.py   # -> docs/store/settings-overview.png
Needs Pillow. Cards come from the real UI (default config, demo data).
"""
import glob
import sys
from PIL import Image, ImageDraw, ImageFont

SRC = sys.argv[1] if len(sys.argv) > 1 else 'target/settings-cards'
OUT = sys.argv[2] if len(sys.argv) > 2 else 'docs/store/settings-overview.png'
COLS, PAD, GAP, HEAD = 3, 64, 36, 230
BG, FG, MUTED, ROSE = (0, 0, 0), (242, 242, 244), (156, 156, 166), (232, 143, 162)


def font(weight, size):
    return ImageFont.truetype(f'crates/rsoverlay-ui/src/fonts/Inter-{weight}.ttf', size)


def crop_card(path):
    """Trim the black around the card (the card surface is never pure black)."""
    im = Image.open(path).convert('RGB')
    box = im.point(lambda v: 255 if v > 6 else 0).convert('L').getbbox()
    return im.crop(box) if box else im


cards = [crop_card(p) for p in sorted(glob.glob(f'{SRC}/*.png'))]
if not cards:
    sys.exit(f'no cards in {SRC}: run --settings-cards first')
cw = max(c.width for c in cards)
# Masonry: each card goes to the currently shortest column, in page order.
heights = [0] * COLS
places = []
for c in cards:
    col = heights.index(min(heights))
    places.append((col, heights[col]))
    heights[col] += c.height + GAP
w = PAD * 2 + COLS * cw + (COLS - 1) * GAP
h = HEAD + max(heights) - GAP + PAD
img = Image.new('RGB', (w, h), BG)
d = ImageDraw.Draw(img)
d.text((PAD, 56), 'RSOverlay', font=font('Medium', 76), fill=FG)
d.rounded_rectangle((PAD + 2, 150, PAD + 66, 156), 3, fill=ROSE)
d.text((PAD + 410, 78), 'Settings', font=font('Regular', 56), fill=MUTED)
d.text((w - PAD, 92), f'All {len(cards)} settings cards, as shipped', font=font('Regular', 34), fill=MUTED, anchor='ra')
for c, (col, y) in zip(cards, places):
    img.paste(c, (PAD + col * (cw + GAP), HEAD + y))
img.save(OUT, optimize=True)
print(OUT, img.size)
