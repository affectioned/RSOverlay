# XSOverlay feature parity — gap analysis

Feature inventory taken from XSOverlay Beta's user-facing settings model
and subsystem names (2026-11-24). **Feature level only**: this lists
*what* XSO lets users do. No XSO code, assets, layouts, sound packs or
strings are reused in RSOverlay; every implementation here is written
from scratch against public Windows / OpenVR APIs.

Legend: ✅ shipped · 🟡 partial · ❌ missing · ⛔ non-goal

## Core overlay & windows

| XSO feature | RSO | Notes |
|---|---|---|
| Desktop mirror | ✅ | M14 (GPU path actually live since `03ad266`) |
| Per-window capture | ✅ | M31, WGC |
| Window previews in picker | ❌ | thumbnails in the source dropdown |
| Multiple window overlays at once | ❌ | next step after M31 |
| Attach overlay to world / head / left hand / right hand | 🟡 | anchor enum exists; per-window + live hand tracking missing |
| Gather overlays into grid + auto-recenter | ❌ | |
| Curved overlays + curve bias | 🟡 | curvature ✅ (M24), bias ❌ |
| Clip angle (hide when viewed edge-on) | ❌ | cheap: dot(view, normal) threshold |
| Default overlay scale / opacity | ✅ | size + opacity (M35, compositor alpha) |
| Per-overlay min/max capture FPS | ❌ | throttles capture thread |
| Force higher-quality overlays | 🟡 | `overlay_texture_size` config |

## Input

| XSO feature | RSO | Notes |
|---|---|---|
| Laser mouse: click / long-press / drag | ❌ | blocked on VR controller input |
| Grab / push / scale gestures (+ sensitivity, dampening, smart roll) | ❌ | |
| Pointer scale, scroll speed, double-click delay | ❌ | |
| Automatic mouse control | ❌ | |
| Touch-injection input method (alt. to mouse) | ❌ | `InjectTouchInput` |
| Block input to background app (on hover / in layout mode) | ❌ | |
| Haptics strength | ❌ | |
| Gamepad (XInput) + sim-control bindings | ❌ | flight-sim crowd |
| Elevated-window input (admin permissions helper) | ❌ | needs an elevated helper process |

## Keyboard

| XSO feature | RSO | Notes |
|---|---|---|
| Virtual keyboard + chords | ✅ | M18 / M26 |
| Physical layouts | ✅ | M27 (5 layouts; XSO also has JIS) |
| Typing sounds + swappable sound packs | ❌ | |
| Keyboard opacity | ❌ | |
| Favorite layout quick-switch | ❌ | |

## Wrist / dashboard

| XSO feature | RSO | Notes |
|---|---|---|
| Wrist overlay offsets / rotation / scale / opacity / clip angle | 🟡 | placement card; rotation, opacity, clip ❌ |
| Dominant-hand setting | ❌ | |
| Hide wrist overlay | ❌ | |
| Wrist background theme image | ❌ | |
| 12/24h + date format | 🟡 | follows system locale; no override |
| Battery % on wrist + font scale | ✅ | M29 card |
| Low-battery warning % | ✅ | M29 |
| Low-battery **sound** | ❌ | toast only |

## System

| XSO feature | RSO | Notes |
|---|---|---|
| Master volume + mute | ✅ | M32 |
| **Per-app volume / mute mixer** | ✅ | M32 |
| CPU / GPU load + temp, RAM, frametime | ✅ | M36: CPU/GPU/RAM/VRAM + fps; temps ⛔ (ring-0 driver) |
| Night light (scheduled warm tint, intensity) | ✅ | M35, compositor colour multiply |
| Media auto-detect + controls | ✅ | M5 / M20 |
| Media-derived theming / adaptive accent | ❌ | album-art palette → accent |
| Dark theme + accent color | ✅ | AMOLED + accent picker |
| Themes folder | ❌ | |
| Notification API | ✅ | M2 |
| Notification scale / offsets | ❌ | |
| OSC device events (battery etc. → VRChat) | ✅ | M37 — see [OSC.md](OSC.md) |
| Discord Rich Presence | ✅ | M25 |
| Localization | 🟡 | 9 languages vs XSO's ~40 |
| Plugin applications | ❌ | WASM-only per ROADMAP non-goals |
| Analytics | ⛔ | telemetry off by default, ever |

## RSO-only (XSO doesn't have)

Now-playing → VRChat chatbox bridge (M28), macro / StreamDeck grid (M30),
VRChat chatbox composer (M8), AMOLED true-black theme, no Chromium.

## Suggested order (value ÷ cost)

Done: volume mixer (M32), night light + opacity (M35), system stats
(M36), battery OSC (M37). Also shipped outside the list: dashboard
redesign (M33) and motion system (M34).

Next:
1. **Clip angle** — hide overlays viewed edge-on; needs HMD pose.
2. **Multiple window overlays** — biggest UX jump after M31.
3. **VR controller input** (laser click/drag, grab/scale) — unlocks
   the whole Input section; the real foundation still missing.
4. **Keyboard sound packs + opacity.**
