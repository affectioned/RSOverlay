# RSOverlay — Roadmap

Fed by two public wishlists, both mined clean-room — only user-facing feature
requests were extracted, no XSOverlay / OVR Toolkit internals.

- **XSOverlay Issue Tracker** — https://github.com/Xiexe/XSOverlay-Issue-Tracker/issues
- **OVR Toolkit Steam Discussions** — https://steamcommunity.com/app/1068820/discussions

---

## What's shipped (M0 → M10)

Preview mode (no HMD needed), OpenVR red-square submission, DXGI desktop
mirror, egui dashboard with AMOLED theme + hand-authored SVG icon set,
runtime accent/card customization, SMTC now-playing, XSOverlay-compatible
WebSocket notification server with live toasts, VRChat OSC `/chatbox/input`,
`SendInput` mirror-to-desktop click forwarding.

## Near-term (next 5 milestones, chosen by signal)

Ranked by cross-wishlist demand × implementation cost in a Rust + wgpu stack.

1. **Notifications overhaul.** Clear-all, multi-stack (done — polish next),
   scrollable history pane. *XSO #127/#128/#131 cluster, 3 `Planned` tags.*
2. **Window capture (Windows Graphics Capture API).** Per-window mirrors
   alongside the whole-monitor path. Unlocks every "Discord window in VR"
   request. *OVR top complaint cluster + XSO #30/#117/#139.*
3. **Keyboard layouts.** Save keyboard position/scale per layout +
   favorites + quick-switch. Pure serde work. *XSO #185/#316/#337, all
   `Planned`; OVR #6 physical-keyboard calibration.*
4. **Battery / HMD stats widget.** Configurable warning thresholds, hide
   toggles per device, support for Vive-wireless, Index wand, Quest. *XSO
   #203/#216/#339; OVR #17.*
5. **WASM plugin API (`wasmtime`).** The single most-reframed ask — users
   want extensibility but we don't ship a dylib loader. Wasm sandbox turns
   this from a security problem into a differentiator. *XSO #44/#150.*

## Mid-term

- **Reset-settings tray action** (XSO #160, second-highest signal) + full
  `config.toml` hot-reload with watcher.
- **Macros / virtual StreamDeck** — bindable actions in-overlay firing
  arbitrary SendInput shortcuts (OBS scene switching, mute-mic, ShadowPlay).
  *OVR top-5.*
- **Service discovery via mDNS** so companion apps autoconnect to the
  notification server (XSO #188).
- **"Apply as new center" / HMD-anchored panels.** Already on our anchor
  enum — needs UI + persistence. *XSO #64, OVR #16 head-locked pin.*
- **Toolbar whitelist / blacklist**, window frame chrome toggles (OVR #7).
- **Horizontal scroll forwarding** on captured windows (XSO #115).
- **Discord / media button bar** (XSO #119/#120 — mute/deafen, next/prev).
- **Song-title OSC bridge to VRChat chatbox** — SMTC → chat. Existing pieces
  already in-crate.
- **Hand-tracking input** (OVR #5) — unlocks fingertip typing. Needs
  SteamVR hand-tracking consumption via OpenXR skeletal tracking.

## Long-term

- **OpenXR path alongside OpenVR.** Opens the door for the flight-sim
  crowd on non-SteamVR runtimes. *OVR #2.*
- **Linux / Wayland capture backend** via PipeWire + xdg-desktop-portal.
  Preview binary already runs on Linux today. *OVR #1 — the single loudest
  OVRT ask.*
- **End-user customization beyond accent.** Density presets, font scale,
  layout grid override, per-panel opacity, saved "profiles" users can
  swap at runtime. Groundwork is in place (`theme.rs` tokens, Appearance
  card) — this expands the surface.
- **`wry` WebView2 opt-in browser overlay.** Explicitly opt-in — not the
  always-on Chromium dependency XSO/OVRT ship.

## Explicit non-goals

- **Chromium baked in by default.** Chromium is the architectural choice
  that makes XSOverlay cost 1 GB RAM. `wry` is opt-in only.
- **Unity workshop asset pipeline** (OVR has one). Not reimplementing.
- **Cloud STT dependencies** for voice-to-chatbox (OVR's Google-Cloud
  path). Local-only speech, or not at all, when we get there.
- **Dylib plugins.** Ever. WASM + `wasmtime` is the plugin surface.
- **Telemetry on by default.** Opt-in only.
