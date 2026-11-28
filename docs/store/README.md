# Store screenshots

1920×1080 renders of the VR preview scene: real RSOverlay UI, mock window
contents and a demo track (no real desktop or listening history).

The settings overview renders each Settings card offscreen with the
default config. Its Stats numbers are measured live (CPU, RAM, VRAM,
UI frame time); "overlay fps 90" is representative of a 90 Hz headset.

| File | Preset |
| --- | --- |
| `settings-overview.png` | every Settings card, default config (`--settings-cards` + `python tools/make-settings-overview.py`) |
| `overview.png` | all of the below in one shareable image (`python tools/make-overview.py`) |
| `vr-work.png` | editor + notes + terminal arc, stats widget, keyboard |
| `vr-gaming.png` | game guide, media/macros + battery widgets, wrist expanded |
| `vr-social.png` | dashboard on Chat, keyboard, notifications + media |
| `vr-dashboard.png` | dashboard Home, popped-out stats widget |
| `vr-wrist-compact.png` | wrist close-up, compact set (More), Media theme with a demo album cover |
| `vr-wrist-expanded.png` | wrist close-up, everything shown (Less), demo album cover |

Regenerate (after UI changes):

```sh
for s in work gaming social dashboard wrist-compact wrist-expanded; do
  cargo run -p rsoverlay-app -- --vr-preview --scene $s --screenshot docs/store/vr-$s.png --exit-after 300
done
python tools/make-overview.py   # rebuild overview.png (needs Pillow)

cargo run -p rsoverlay-app -- --settings-cards target/settings-cards
python tools/make-settings-overview.py   # rebuild settings-overview.png
```

Interactive: `cargo run -p rsoverlay-app -- --vr-preview`, then keys 1-6
switch presets, drag to look, scroll to dolly, R resets the view.
