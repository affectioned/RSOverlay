# RSOverlay — Roadmap

Fed by two public wishlists — only user-facing feature requests were
extracted — plus a feature-level inventory of XSOverlay's user-facing options
(see [XSO_PARITY.md](XSO_PARITY.md)). No XSOverlay / OVR Toolkit code,
assets or strings are reused; every implementation is written from
scratch against public Windows / OpenVR APIs.

- **XSOverlay Issue Tracker** — https://github.com/Xiexe/XSOverlay-Issue-Tracker/issues
- **OVR Toolkit Steam Discussions** — https://steamcommunity.com/app/1068820/discussions

---

## What's shipped (M0 → M78)

Last reviewed 2026-11-26. Parity detail lives in
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
- **M67:** window thumbnails in the Desktop picker (PrintWindow on a
  worker thread, every 2 s while the page is open; icon fallback).
- **M68:** notifications in VR (they were only drained by the desktop
  window): head-locked toast panel with size / height / distance;
  custom sounds for every alert (notification / warning / error / low
  battery: built-in, silent or any WAV in `sounds/`); client
  `audioPath` picks a category, never a file.
- **M69:** PC performance for VR players: busiest-core meter, SteamVR
  frame timing (game fps vs refresh, GPU headroom, reprojection), VR FPS
  and VRAM wrist chips; RSOverlay's own stats moved to Settings ›
  RSOverlay performance. Media polling moved off the UI thread.
- **M70 (first tester feedback):** the laser hits the panel drawn on top
  (explicit SteamVR sort layers; fixes the keyboard "ray angle");
  windows hide with the dashboard unless pinned; panels step aside for
  the SteamVR menu; one stick action at a time while grabbing; tilt with
  the controller; dedicated right / middle click actions; pointer size;
  panel resolution 1× / 1.5× / 2×.
- **M71:** releases: tag-driven GitHub builds (pre-releases for
  testers), opt-in update notice (never installs anything), `steam`
  build feature, SteamPipe beta upload job (off until the App ID exists),
  achievements draft ([ACHIEVEMENTS.md](ACHIEVEMENTS.md)).
- **M72:** sideways scrolling (stick X, tilt-wheel on captured windows);
  hide battery devices; mute notifications per app.
- **M73:** Focus mode on the wrist: notifications wait in history, the
  windows you aren't working in dim.
- **M74:** resize the real window from its VR toolbar (Size menu).
- **M75:** All monitors (every monitor on an arc around you, left to
  right as on the desk, pinned) and Big screen (main monitor, 2.2 m and
  curved, pinned; curvature saved with layouts).
- **M76 (parity sweep):** vibration strength, grab smoothing, clock
  (12 h / 24 h) and date format, keyboard language quick-switch;
  XSO_PARITY.md re-reviewed. Release notes are hand-written per tag
  (`docs/releases/`).
- **M77 (parity, continued):** no console window (log file + crash
  dialog, Report a problem / Open logs); mouse follows the laser
  (optional); gamepad shortcuts; wrist background image; touch input for
  windows; block game input while pointing (experimental); refresh rate
  for panels you aren't looking at.
- **M78:** in-app updates (Install update: download with progress,
  SHA-256 check, swap, restart) and what's new before / after updating;
  Focus follow-ups (dimmed windows throttled, auto-off when the last
  window closes); auto-hide window toolbars; per-window "Keep smooth
  when not looking"; hide apps from the window picker; language picker
  hidden until the UI is translated.

## XSOverlay parity: remaining gaps

Ranked by value ÷ cost; full table in [XSO_PARITY.md](XSO_PARITY.md).

See [XSO_PARITY.md](XSO_PARITY.md) › Remaining gaps.

## Working in VR (productivity track)

RSOverlay should be a daily driver for people who *work* in VR, not
only play. Foundations already shipped: window overlays with a real
mouse (press / drag / right-click / wheel), 8 layout slots, a
physical-style keyboard that types in any language, global search,
macros, DPI-correct input, dock-anywhere panels. The VR preview's
"work" scene (`--vr-preview --scene work`) shows the target setup.

1. **Translate the UI** — route every string through the i18n tables,
   then translations checked by native speakers (the language picker is
   hidden until then).
   - **Automatic language:** on first start, use the language Windows
     itself is displayed in (the Windows display / UI language), not the
     user's region or location. Falls back to English when that language
     isn't translated yet. Choosing a language in Settings overrides it
     and is kept.

## Replacing OpenVR Advanced Settings

Planned 2026-11-28 with testers. OVRAS's playspace features (drag and
turn, offsets, recenter handling, fix floor, auto-turn) live in the
separate app **RSPlayspace**. The rest worth keeping comes into
RSOverlay, in this order:

1. **Mic mute and push-to-talk, system-wide.** A wrist button and a
   SteamVR binding to mute the Windows mic itself (WASAPI), so Discord,
   VRChat and everything else are muted at once. Modes: toggle, hold to
   talk, hold to mute. A mute icon on the wrist and a short sound on mute
   and unmute. Reuses the mic code from voice typing.
   - **Discord mute preset:** for people who want Discord's own mute (the
     red icon), the button can also send Discord's mute hotkey through a
     ready-made macro. Ties in with the Discord button bar below.
2. **Audio device switching in VR.** Pick the output (headset, speakers)
   and the mic from the volume panel, without taking the headset off.
3. **Session stats on the wrist.** Cable turn counter (how far you've
   physically turned; RSPlayspace already has a tested counter that
   survives recenters and tracking gaps), distance walked, dropped and
   reprojected frames.
4. **Chaperone helpers** (least certain; SteamVR covers some of this):
   saved chaperone profiles, show the bounds when you get close, a
   proximity warning.

Left out on purpose: video and colour tweaks, motion smoothing toggles
and SteamVR settings mirrors (SteamVR's own settings do these), and
OVRAS's keyboard tools (RSOverlay's keyboard already covers them).

## Mid-term

- **Notifications polish.** History (also as a pop-out widget),
  clear-all, VR size / offset and per-app mute shipped; next: per-app
  sounds. *XSO #127/#128/#131.*
- **Per-layout keyboard presets.** Layout slots already store the
  keyboard pose (M53); next: favourites + quick-switch.
  *XSO #185/#316/#337; OVR #6.*
- **Service discovery via mDNS** so companion apps autoconnect to the
  notification server (XSO #188).
- **Discord button bar** (mute / deafen) next to the media controls
  (XSO #119/#120).
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
