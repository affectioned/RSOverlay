# XSOverlay feature parity: gap analysis

Feature inventory of what XSOverlay lets users do: the options in its
settings and its Steam store feature list. **Feature level only.**
Features and observable behaviour are compared, never implementations: no
XSO code, assets, layouts, sound packs or strings are reused in
RSOverlay, and every implementation here is written from scratch
against public Windows / OpenVR APIs.

Rule of thumb: where XSO already has a feature, match how it behaves so
moving over is easy, unless testers ask for something different (then
their request wins). RSOverlay keeps its own look.

Legend: ✅ shipped · 🟡 partial · ❌ missing · ⛔ non-goal
Last reviewed: M77 (2026-11-26).

## Core overlay & windows

| XSO feature | RSO | Notes |
|---|---|---|
| Launch with SteamVR (startup overlay app) | ✅ | M58: registered via IVRApplications; auto-launch on first run, toggle in SteamVR › Startup; exits with SteamVR |
| Desktop mirror | ✅ | M14, GPU-to-GPU; cursor drawn on monitor mirrors (M47) |
| Per-window capture | ✅ | M31, Windows Graphics Capture, GPU-to-GPU |
| Window picker with previews | ✅ | Desktop page tiles with app icons, filter (M47/M50), live thumbnails (M67) |
| Multiple window overlays at once | ✅ | M44: up to 6 windows, 14 floating panels in total |
| Attach overlay to world / head / left hand / right hand | ✅ | M44 windows, M46 widgets, M59 dashboard; grabbing a docked panel undocks it |
| Layout mode (unpinned windows hide with the dashboard, pinned stay) | ✅ | M70: Pin on every window / widget toolbar, saved with layouts; toggle in Settings › VR panels |
| Hide while the SteamVR dashboard is open | ✅ | M70 (IVROverlay IsDashboardVisible), on by default |
| Gather overlays + recenter | 🟡 | Recenter arcs every world panel (and the keyboard) in front of you; All monitors arranges monitors on an arc (M75); no grid view |
| Curved overlays | ✅ | M24, Big screen M75, saved with layouts |
| Curve bias | ⛔ | OpenVR's overlay curvature has no bias parameter; faking it would mean warping textures ourselves |
| Clip angle (hide when viewed edge-on) | ✅ | M44, default 60°, every panel |
| Default overlay scale / opacity | ✅ | M35 (global); per-window content opacity on each window toolbar (M64) |
| Per-overlay min/max capture FPS | ✅ | M77 global rate for panels you aren't looking at (1–90 fps); M78 per window: Window menu › Keep smooth when not looking |
| Force higher-quality overlays | ✅ | M70: Settings › VR panels › Resolution (Standard / High 1.5× / Ultra 2×), live |
| Resize the real window from VR | ✅ | M74: Size menu on window toolbars (RSO-only in this form) |
| Layouts (save / load panel arrangements) | ✅ | M53: 8 slots, startup slot, last session restored (M65) |

## Input

| XSO feature | RSO | Notes |
|---|---|---|
| SteamVR Input with default bindings | ✅ | M49: Index, Touch, Vive, WMR; rebindable in SteamVR |
| Laser mouse: click / drag / scroll | ✅ | M38 + M49: either hand, stick scroll (vertical + sideways, M72), click stabilisation |
| Right / middle click | ✅ | trackpad / button modifiers (M49) and dedicated bindable Right / Middle click actions (M70) |
| Long-press | ✅ | M63: hold still → right click (opt-in, 0.3–1.5 s) |
| Grab / push / scale gestures | ✅ | M44: rides the laser, stick push/pull + scale, two-hand scale, live size readout; one stick action at a time (M70) |
| Rotate grabbed panels | ✅ | M70: Tilt panels with the controller (off by default: panels stay upright, facing you) |
| Grab sensitivity / dampening | ✅ | M76: Grab smoothing slider (Settings › Pointer) |
| Smart roll | ⛔ | covered by upright-by-default + optional tilt |
| Pointer scale, scroll speed, double-click delay | ✅ | pointer size (M70), scroll speed + cursor smoothing + laser angle (M51/M53), double-click delay (M63) |
| Haptics strength | ✅ | M76: Vibration 0–150% (0 = off), Settings › Pointer |
| Automatic mouse control | ✅ | M77: Mouse follows the laser (on by default); off = the cursor only moves on click / drag |
| Touch-injection input method (alt. to mouse) | ✅ | M77: Touch input for windows (`InjectTouchInput`); right / middle click and the wheel stay on the mouse. Not yet tried on a touch-first app in a headset |
| Block input to background app (on hover) | 🟡 | M77, experimental opt-in: SteamVR's MakeOverlaysInteractiveIfVisible on the panel being pointed at. Needs headset testing (SteamVR may also show its own pointer) |
| Gamepad (XInput) bindings | ✅ | M77: Settings › Gamepad, chords for dashboard / recenter / keyboard (off by default) |
| Sim-control (HOTAS / DirectInput) bindings | ❌ | XInput pads only so far |
| Elevated-window input | ✅ | M45: optional run-as-administrator relaunch; elevated windows show a "use your PC" notice |

## Keyboard

| XSO feature | RSO | Notes |
|---|---|---|
| Virtual keyboard + chords | ✅ | M18 / M26, physical-style with scan codes (M46) |
| Physical layouts | ✅ | key caps labelled from the live Windows layout of the focused window |
| Layout quick-switch | ✅ | M76: language pill on the VR keyboard (shown with 2+ Windows layouts) switches the focused window to the next one; caps relabel |
| Key hold behaviour | ✅ | M46: real hold by default (games); auto-repeat opt-in |
| Sticky / locked modifiers, VRChat chatbox mode, compact / full | ✅ | M46–M55 |
| Typing sounds + swappable sound packs | ✅ | M62: built-in clicks, volume, user WAV packs in `sounds/<pack>/` |
| Keyboard opacity | ✅ | M62 |

## Wrist / dashboard

| XSO feature | RSO | Notes |
|---|---|---|
| Wrist toolbar stack (actions, layouts, clock, media, devices) | ✅ | M54; compact by default with More (M59); every button labelled |
| Wrist toolbar customisation | ✅ | M55 / M61: pick any actions, modules and status chips, in Settings or in VR (Edit) |
| Wrist offsets / rotation / scale / opacity / clip angle | ✅ | grab to move + scale (saved, M65), opacity, shared clip angle |
| Dominant-hand setting | ✅ | Laser hand Right / Left; the wrist goes on the other hand |
| Hide wrist overlay | ✅ | M66 |
| Wrist background theme image | ✅ | M77: any PNG / JPG in `backgrounds/`, darkened behind the clock + media |
| 12/24h + date format | ✅ | M76: System / 12 h / 24 h and three date styles (Settings › Wrist toolbar), wrist + dashboard |
| Battery % on wrist | ✅ | M29 / M54, playtime estimates (M51), hide devices (M72) |
| Low-battery warning % + sound | ✅ | M29 / M66 |

## System

| XSO feature | RSO | Notes |
|---|---|---|
| Master volume + mute, per-app mixer | ✅ | M32 |
| CPU / GPU load, RAM, frametime | ✅ | M69: PC performance with busiest core, VRAM and SteamVR frame timing (fps vs Hz, headroom, reprojection); temps ⛔ (needs a ring-0 driver) |
| Night light | ✅ | M35 |
| Media auto-detect + controls | ✅ | M5 / M20; polling off the UI thread (M69) |
| Media-derived theming | ✅ | M55 |
| Dark theme + accent colour | ✅ | AMOLED black + accent presets / picker |
| Themes folder | 🟡 | accent picker + wrist backgrounds; full colour themes need the theme colours to become runtime tokens first |
| Notification API | ✅ | M2, same JSON shape, so existing senders (VRCX etc.) work |
| Notification scale / offsets / sounds | ✅ | M68: VR toast panel size / height / distance, per-category custom sounds; per-app mute (M72) |
| Do-not-disturb | ✅ | M73: Focus mode on the wrist (also dims the windows you aren't working in) |
| OSC device events (battery etc. → VRChat) | ✅ | M37, see [OSC.md](OSC.md) |
| Discord Rich Presence | ✅ | M25 + built-in app, live toggle and sharing choices in Settings › Discord |
| Localization | ❌ | English only for now. Translation tables exist for 9 languages (~40 keys) but the UI doesn't use them yet, so the language picker is hidden. Planned: route every string through the tables, then translations (ideally checked by native speakers), with the language picked automatically on first start from the language Windows is displayed in, not region or location (Settings can override) |
| Updates | ✅ | M71: GitHub builds show an opt-in notice (never auto-install); Steam builds update through Steam |
| Plugin applications | ❌ | WASM-only, see ROADMAP non-goals |
| Analytics | ⛔ | no telemetry |

## RSO-only (XSO doesn't have these)

- Pop-out widgets from any dashboard card; merge into tabs, split back out (M46)
- Global search across settings, pages and windows (M50)
- Real key hold on the keyboard (M46)
- Battery playtime estimates (M51)
- PC performance aimed at "why is my game lagging": busiest core, VRAM, SteamVR reprojection (M69)
- RSOverlay's own footprint measured live (Settings › RSOverlay performance)
- Window edge docking into walls that move together (M61)
- All monitors arc and Big screen (M75); resize real windows from VR (M74)
- Now-playing → VRChat chatbox bridge (M28), chatbox composer (M8)
- Macro / StreamDeck grid (M30)
- VR preview: every panel in 3D on the desktop (M60)
- No Chromium / Unity / Mono

## Remaining gaps (value ÷ cost)

1. Translate the whole UI (move every string into the i18n tables), then
   more languages; on first start use the language Windows is displayed
   in (not region / location).
2. Headset-test the experimental input blocking; promote or replace it.
3. Full colour themes (theme colours as runtime tokens, `themes/` files).
4. HOTAS / DirectInput sim bindings.
