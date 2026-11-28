"""Compose docs/store/*.png into one shareable overview image.

Run from the repo root after regenerating the store shots:
    python tools/make-overview.py            # -> docs/store/overview.png
Needs Pillow (pip install pillow). Uses the UI's own Inter font.
"""
import sys
from PIL import Image, ImageDraw, ImageFont

OUT = sys.argv[1] if len(sys.argv) > 1 else 'docs/store/overview.png'
WRIST_CROP = (240, 180, 1680, 990)  # 16:9 around the wrist
SHOTS = [
    ('vr-work', 'Working in VR', 'editor, notes and terminal around you · keyboard', None),
    ('vr-dashboard', 'Dashboard', 'labelled sidebar · search · pop-out widgets', None),
    ('vr-social', 'Social VR', 'chatbox + keyboard · notifications · now playing', None),
    ('vr-gaming', 'In game', 'guide pinned aside · media widget · wrist with every battery', None),
    ('vr-wrist-compact', 'Wrist toolbar', 'the everyday buttons + More · album cover while music plays', (240, 270, 1680, 1080)),
    ('vr-wrist-expanded', 'Wrist toolbar, More', 'every action, layout slots, batteries, stats', WRIST_CROP),
]
TW, TH = 1152, 648
PAD, GAP, CAP, HEAD = 64, 40, 96, 210
BG, FG, MUTED = (0, 0, 0), (242, 242, 244), (156, 156, 166)
ROSE, EDGE = (232, 143, 162), (37, 37, 41)


def font(weight, size):
    return ImageFont.truetype(f'crates/rsoverlay-ui/src/fonts/Inter-{weight}.ttf', size)


w = PAD * 2 + TW * 2 + GAP
h = HEAD + 3 * (TH + CAP) + 2 * GAP + PAD
img = Image.new('RGB', (w, h), BG)
d = ImageDraw.Draw(img)
d.text((PAD, 56), 'RSOverlay', font=font('Medium', 76), fill=FG)
d.rounded_rectangle((PAD + 2, 150, PAD + 66, 156), 3, fill=ROSE)
d.text((w - PAD, 92), 'Open-source SteamVR overlay · written in Rust', font=font('Regular', 34), fill=MUTED, anchor='ra')
for i, (name, title, desc, crop) in enumerate(SHOTS):
    x = PAD + (i % 2) * (TW + GAP)
    y = HEAD + (i // 2) * (TH + CAP + GAP)
    src = Image.open(f'docs/store/{name}.png').convert('RGB')
    tile = (src.crop(crop) if crop else src).resize((TW, TH), Image.LANCZOS)
    mask = Image.new('L', (TW, TH), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, TW - 1, TH - 1), 22, fill=255)
    img.paste(tile, (x, y), mask)
    d.rounded_rectangle((x, y, x + TW - 1, y + TH - 1), 22, outline=EDGE, width=2)
    d.text((x + 4, y + TH + 22), title, font=font('Medium', 36), fill=FG)
    d.text((x + 4, y + TH + 66), desc, font=font('Regular', 24), fill=MUTED)
img.save(OUT, optimize=True)
print(OUT, img.size)
