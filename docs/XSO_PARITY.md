# XSOverlay feature parity: gap analysis

Feature inventory of what XSOverlay lets users do: the options in its
settings and its Steam store feature list (2026-11-24). **Feature level
only.** Features and behaviour are compared, never implementations: no
XSO code, assets, layouts, sound packs or strings are reused in
RSOverlay, and every implementation here is written from scratch
against public Windows / OpenVR APIs.

Goal: **behave** like XSO (bindings, wrist, grab/scale, layouts,
keyboard) so moving over is easy, with RSOverlay's own look.

Legend: ✅ shipped · 🟡 partial · ❌ missing · ⛔ non-goal
Last reviewed: M60 (2026-11-24).

## Core overlay & windows

| XSO feature | RSO | Notes |
|---|---|---|
| Launch with SteamVR (startup overlay app) | ✅ | M58: `.vrmanifest` registered via IVRApplications; auto-launch on first run, toggle in SteamVR › Startup; exits when SteamVR quits |
| Desktop mirror | ✅ | M14, GPU-to-GPU; cursor drawn on monitor mirrors (M47) |
| Per-window capture | ✅ | M31, Windows Graphics Capture, GPU-to-GPU |
| Window picker with previews | 🟡 | Desktop page tiles with real app icons + search filter (M47/M50); no live thumbnails |
| Multiple window overlays at once | ✅ | M44: up to 6 windows, 14 floating panels in total |
| Attach overlay to world / head / left hand / right hand | ✅ | M44 windows, M46 widgets, M59 dashboard; grabbing a docked panel undocks it |
| Gather overlays + recenter | 🟡 | Recenter arcs every world panel (and the keyboard) in front of you; no grid view |
| Curved overlays + curve bias | 🟡 | curvature ✅ (M24); bias ❌ |
| Clip angle (hide when viewed edge-on) | ✅ | M44, default 60°, every panel |
| Default overlay scale / opacity | ✅ | M35 (global); per-window content opacity on each window toolbar (M64) |
| Per-overlay min/max capture FPS | 🟡 | automatic: panels you aren't looking at refresh ~5×/s (M55); no per-panel setting |
| Force higher-quality overlays | 🟡 | `overlay_texture_size` config |
| Layouts (save / load panel arrangements) | ✅ | M53: 8 slots, favourite slot auto-loads at startup, from the wrist or Layouts page |

## Input

| XSO feature | RSO | Notes |
|---|---|---|
| SteamVR Input with XSO's default bindings | ✅ | M49: Index, Touch, Vive, WMR; rebindable in SteamVR |
| Laser mouse: click / drag / scroll | ✅ | M38 + M49: either hand, stick scroll, click stabilisation |
| Right / middle click | ✅ | M49: trackpad modifiers while clicking |
| Long-press | ✅ | M63: hold still → right click (opt-in, 0.3–1.5 s); moving still drags, early release still clicks |
| Grab / push / scale gestures | ✅ | M44: grab rides the laser and faces you, stick push/pull + scale, two-hand scale, live size readout (M49) |
| Grab sensitivity / dampening / smart roll settings | 🟡 | pose smoothing built in; no user settings |
| Pointer scale, scroll speed, double-click delay | 🟡 | scroll speed + cursor smoothing + laser angle ✅ (M53/M51); double-click delay ✅ (M63, snaps a quick second click onto the first); pointer scale ❌ |
| Automatic mouse control | ❌ | |
| Touch-injection input method (alt. to mouse) | ❌ | `InjectTouchInput` |
| Block input to background app (on hover / in layout mode) | ❌ | |
| Haptics | 🟡 | click + grab ticks; no strength setting |
| Gamepad (XInput) + sim-control bindings | ❌ | flight-sim crowd |
| Elevated-window input | ✅ | M45: optional run-as-administrator relaunch; elevated windows show a "use your PC" notice when not elevated |

## Keyboard

| XSO feature | RSO | Notes |
|---|---|---|
| Virtual keyboard + chords | ✅ | M18 / M26, physical-style with scan codes (M46) |
| Physical layouts | ✅ | M27 + labels from the live Windows layout; XSO also has JIS |
| Key hold behaviour | ✅ | M46: **real hold** by default (better than XSO's repeated presses in games); auto-repeat opt-in |
| Sticky / locked modifiers, VRChat chatbox mode, compact / full | ✅ | M46–M55 |
| Typing sounds + swappable sound packs | ✅ | M62: synthesised built-in clicks (key / space / enter / backspace / modifier), volume, user WAV packs in `sounds/<pack>/` |
| Keyboard opacity | ✅ | M62, Settings › Keyboard |
| Favourite layout quick-switch | ❌ | |

## Wrist / dashboard

| XSO feature | RSO | Notes |
|---|---|---|
| Wrist toolbar stack (actions, layouts, clock, media, devices) | ✅ | M54; compact by default with **More** (M59); every button labelled |
| Wrist toolbar button customisation | ✅ | M55 |
| Wrist offsets / rotation / scale / opacity / clip angle | ✅ | grab to move + scale, opacity (M44), clip angle shared with all panels |
| Dominant-hand setting | ✅ | Laser hand Right / Left; the wrist goes on the other hand |
| Hide wrist overlay | ✅ | M66, Settings › VR; dashboard still summoned by double-press |
| Wrist background theme image | ❌ | |
| 12/24h + date format | 🟡 | follows system locale; no override |
| Battery % on wrist | ✅ | M29 / M54, with playtime estimates (M51) |
| Low-battery warning % | ✅ | M29 |
| Low-battery **sound** | ✅ | M66: synthesised chime with the toast (Battery card toggle) |

## System

| XSO feature | RSO | Notes |
|---|---|---|
| Master volume + mute | ✅ | M32 |
| Per-app volume / mute mixer | ✅ | M32 |
| CPU / GPU load + temp, RAM, frametime | ✅ | M36: CPU/GPU/RAM/VRAM + fps; temps ⛔ (needs a ring-0 driver) |
| Night light (scheduled warm tint, intensity) | ✅ | M35 |
| Media auto-detect + controls | ✅ | M5 / M20 |
| Media-derived theming | ✅ | M55: accent follows album art (opt-in) |
| Dark theme + accent colour | ✅ | AMOLED black + accent presets / picker |
| Themes folder | ❌ | |
| Notification API | ✅ | M2, XSO-compatible JSON |
| Notification scale / offsets | ❌ | |
| OSC device events (battery etc. → VRChat) | ✅ | M37, see [OSC.md](OSC.md) |
| Discord Rich Presence | 🟡 | M25; opt-in with your own Discord application ID (`--discord-app-id`), no project app is registered |
| Localization | 🟡 | 9 languages vs XSO's ~40 |
| Plugin applications | ❌ | WASM-only, see ROADMAP non-goals |
| Analytics | ⛔ | no telemetry by default, ever |

## RSO-only (XSO doesn't have these)

- Pop-out widgets from any dashboard card; drop onto each other to merge
  into tabs, split back out (M46)
- Global search across settings, pages and windows (M50)
- Real key hold on the keyboard (M46)
- Battery playtime estimates (M51)
- Live measurement of RSOverlay's own footprint: CPU, RAM, VRAM, frame
  time (M52)
- Now-playing → VRChat chatbox bridge (M28), chatbox composer (M8)
- Macro / StreamDeck grid (M30)
- VR preview: every panel in 3D on the desktop, layout presets (M60)
- AMOLED true-black design, no Chromium / Unity / Mono

## Next parity gaps (value ÷ cost)

1. **Window thumbnails** in the Desktop picker.
2. **Notification scale / offsets.**
3. Touch injection, gamepad bindings, curve bias (smaller audiences).
