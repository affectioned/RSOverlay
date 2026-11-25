# RSOverlay — Roadmap

Fed by two public wishlists — only user-facing feature requests were
extracted — plus a feature-level inventory of XSOverlay's user-facing options
(see [XSO_PARITY.md](XSO_PARITY.md)). No XSOverlay / OVR Toolkit code,
assets or strings are reused; every implementation is written from
scratch against public Windows / OpenVR APIs.

- **XSOverlay Issue Tracker** — https://github.com/Xiexe/XSOverlay-Issue-Tracker/issues
- **OVR Toolkit Steam Discussions** — https://steamcommunity.com/app/1068820/discussions

---

## What's shipped (M0 → M60)

Last reviewed 2026-11-24. Parity detail lives in
[XSO_PARITY.md](XSO_PARITY.md).

- **Foundations (M0–M17):** desktop preview, OpenVR overlay FFI, DXGI
  desktop mirror, egui dashboard, XSO-compatible notification WebSocket,
  VRChat OSC chatbox, SendInput click forwarding, VR preview, placement
  system, the real dashboard submitted to SteamVR via DX12.
- **Everyday features (M18–M38):** virtual keyboard + chords, persistent
  config with hot-reload, notification history, media controls,
  Reset defaults, curved overlays, Discord Rich Presence, 9 languages,
  now-playing → chatbox, battery widget, macros, per-window capture
  (WGC), volume mixer, dashboard redesign + motion, night light, system
  stats, battery → VRChat OSC, laser click / drag / scroll.
- **XSO-style VR shell (M44–M56):** wrist toolbar on the off hand,
  summoned dashboard, multiple window overlays, XSO grab model (push /
  pull / scale / two-hand), dock to world / head / hand, clip angle,
  UAC awareness + run as admin, pop-out widgets with merge / split, real
  key hold, SteamVR Input with XSO default bindings, global search,
  battery playtime, Performance card, 8 layout slots, look-based
  throttling, media theme, wrist customisation.
- **Own identity + polish (M57–M60):** RSOverlay's own design (AMOLED
  black, Inter, rose accent), labelled navigation everywhere, compact
  wrist with More, dock + close on every panel, launch with SteamVR, VR
  preview scenes and Steam store screenshots.
- **M61:** window edge docking (drop a window beside another → it snaps
  flush left / right / above / below and the two move together until
  Unlink); widgets dock too; resizing a docked panel pushes its
  neighbours so seams stay flush; layout slots remember docked groups.
  User-editable compact wrist (pick any actions + clock, batteries,
  media, layout slots, views) from Settings or in VR (More → Edit, tap
  to show / hide); batteries on the compact wrist by default. Wrist
  modules: session timer, CPU / GPU, RAM and Alerts status chips.
- **M62:** keyboard opacity; typing sounds (built-in synthesised clicks,
  volume, swappable WAV packs in `sounds/<pack>/`).
- **M63:** laser long-press → right click (opt-in) and double-click
  delay (a quick second click snaps onto the first).
- **M64:** per-window opacity — a slider on every window toolbar fades
  just the captured content (toolbar stays solid) so you can watch your
  game and still see the world; saved with layouts.
- **M65:** persistence audit — wrist pose / size and keyboard toggles
  (pinned, auto-repeat, compact / full, VRChat) survive restarts
  (`[vr_state]`); Restore last session (on by default) reopens the open
  windows at startup; config change detection is structural; pending
  saves flush on exit; round-trip test guards every setting.
- **M66:** hide the wrist toolbar (Settings › VR); low-battery chime.

## XSOverlay parity: remaining gaps

Ranked by value ÷ cost; full table in [XSO_PARITY.md](XSO_PARITY.md).

1. **Window thumbnails** in the Desktop picker.
2. **Notification scale / offsets.**
3. Touch injection, gamepad bindings, curve bias.

## Working in VR (productivity track)

RSOverlay should be a daily driver for people who *work* in VR, not
only play. Foundations already shipped: window overlays with a real
mouse (press / drag / right-click / wheel), 8 layout slots, a
physical-style keyboard that types in any language, global search,
macros, DPI-correct input, dock-anywhere panels. The VR preview's
"work" scene (`--vr-preview --scene work`) shows the target setup.

1. **Focus mode** — one toggle that silences notifications/toasts and
   dims or throttles panels you aren't looking at, so the window you're
   working in owns your attention. Auto-off when you leave it.
2. **Work layout preset** — one tap builds a large curved main screen
   plus side windows at reading distance (a built-in layout template
   users can then tweak and save to a slot).
3. **Window resize from VR** — set the *real* window's size/DPI from the
   overlay toolbar so text stays readable at the panel's physical size
   (e.g. "fit to panel" / 1280×800 / 1920×1080 presets).
4. **Multi-monitor wall** — every monitor at once, arranged in an arc
   around you, with per-monitor docking and recenter.

## Mid-term

- **Notifications polish.** History + clear-all shipped (M21); next:
  per-app mute, scrollable history in a widget, size / offset settings.
  *XSO #127/#128/#131.*
- **Per-layout keyboard presets.** Layout slots already store the
  keyboard pose (M53); next: favourites + quick-switch.
  *XSO #185/#316/#337; OVR #6.*
- **Service discovery via mDNS** so companion apps autoconnect to the
  notification server (XSO #188).
- **Toolbar whitelist / blacklist**, window frame chrome toggles (OVR #7).
- **Horizontal scroll forwarding** on captured windows (XSO #115).
- **Discord button bar** (mute / deafen) next to the media controls
  (XSO #119/#120).
- **Per-device battery hide toggles** (battery widget follow-up).
- **Hand-tracking input** (OVR #5): fingertip typing via OpenXR
  skeletal tracking.

## Long-term

- **OpenXR path alongside OpenVR.** Opens the door for the flight-sim
  crowd on non-SteamVR runtimes. *OVR #2.*
- **Linux / Wayland capture backend** via PipeWire + xdg-desktop-portal.
  *OVR #1, the single loudest OVRT ask.*
- **End-user customization beyond accent.** Density presets, layout
  grid override, per-panel opacity, saved "profiles" users can swap at
  runtime. VR text size already ships (Accessibility); groundwork is in
  `theme.rs` tokens + the Appearance card.
- **`wry` WebView2 opt-in browser overlay.** Explicitly opt-in — not the
  always-on Chromium dependency XSO/OVRT ship.

## Explicit non-goals

- **Clipboard history / clipboard capture.** Reading or storing what
  users copy (passwords, tokens, private messages) breaks the project's
  privacy & security rule. The keyboard's Copy / Paste buttons only send
  the normal Ctrl+C / Ctrl+V keystrokes; RSOverlay never reads or keeps
  clipboard contents.

- **Chromium baked in by default.** Chromium is the architectural choice
  that makes XSOverlay cost 1 GB RAM. `wry` is opt-in only.
- **Unity workshop asset pipeline** (OVR has one). Not reimplementing.
- **Cloud STT dependencies** for voice-to-chatbox (OVR's Google-Cloud
  path). Local-only speech, or not at all, when we get there.
- **Dylib plugins.** Ever. WASM + `wasmtime` is the plugin surface.
- **Telemetry on by default.** Opt-in only.
