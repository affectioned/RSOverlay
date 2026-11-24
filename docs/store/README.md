# Store screenshots

1920×1080 renders of the VR preview scene: real RSOverlay UI, mock window
contents and a demo track (no real desktop or listening history).

| File | Preset |
| --- | --- |
| `vr-work.png` | editor + notes + terminal arc, stats widget, keyboard |
| `vr-gaming.png` | game guide, media/macros + battery widgets, wrist expanded |
| `vr-social.png` | dashboard on Chat, keyboard, notifications + media |
| `vr-dashboard.png` | dashboard Home, popped-out stats widget |
| `vr-wrist.png` | wrist close-up, expanded |

Regenerate (after UI changes):

```sh
for s in work gaming social dashboard wrist; do
  cargo run -p rsoverlay-app -- --vr-preview --scene $s --screenshot docs/store/vr-$s.png --exit-after 300
done
```

Interactive: `cargo run -p rsoverlay-app -- --vr-preview`, then keys 1-5
switch presets, drag to look, scroll to dolly, R resets the view.
