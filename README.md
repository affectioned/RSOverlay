# RSOverlay

An open-source SteamVR overlay written in Rust. Feature-parity target: **XSOverlay**
and **OVR Toolkit**. Different backend: faster, smaller, more secure.

> **NOT affiliated with or derived from XSOverlay or OVR Toolkit.** Clean-room
> implementation. We read their public APIs only — specifically the XSOverlay
> websocket notification message shape, for byte-level API compat so existing
> clients (VRCX etc.) keep working. No code is copied, ported, or
> reverse-engineered from either.

> **Repo status: private.** Shipping on Steam + public GitHub repo later. For
> now this stays internal.

## Vision

The space is dominated by two closed-source tools built on Unity + Mono +
Chromium. They work, but they pay for it — 1 GB RAM for a notification bar is
not a technical constraint, it's an architectural choice.

RSOverlay rejects that stack. Tiny render loop in `wgpu`, DXGI Desktop
Duplication for mirrors (GPU-to-GPU, no readback), `tokio` for the websocket
API, `egui` for UI. One static Rust binary. No Mono runtime, no Chromium, no
Unity player.

## Features (rough milestones)

| Tier | Feature | Status |
|------|---------|--------|
| **P0** | **Desktop preview mode (no HMD required)** | **working** |
| **P0** | **OpenVR overlay registration + texture submission (D3D11)** | **working** |
| **P0** | **Desktop mirror (DXGI Desktop Duplication)** | **working** |
| **P0** | **Input injection (`SendInput`, mirror → real desktop)** | **working** |
| P0 | Window mirror (Windows Graphics Capture) | planned |
| P0 | Virtual keyboard | planned |
| P0 | Panel grab / resize / anchor (wrist, HMD, world) | planned |
| **P1** | **XSOverlay-compatible websocket API (`127.0.0.1:42069`)** | **working** |
| **P1** | **Toast notifications (XSOverlay JSON compat)** | **working** |
| **P1** | **Media controls — now-playing via SMTC** | **working** (read-only) |
| **P1** | **VRChat OSC chatbox (`/chatbox/input`)** | **working** |
| P1 | Wrist / radial menu | planned |
| **P2** | **Clock / stats widget** | **working** |
| P2 | `wry` WebView2 browser overlay (opt-in) | planned |
| P2 | Battery / controller stats | planned |
| P2 | Discord / OBS bridges | planned |
| P2 | Hotkeys (global + VR controller) | planned |
| P2 | WASM plugin API (`wasmtime` only, never dylib loading) | planned |

## Build

```bash
cargo build --workspace
cargo clippy --workspace -- -D warnings
cargo deny check
```

## Run

### Desktop preview mode (default — no HMD required)

```bash
cargo run -p rsoverlay-app
# or explicitly
cargo run -p rsoverlay-app -- --preview
```

Opens a window that presents the overlay texture exactly as VR would submit
it. Every subsystem — capture, UI, websocket API, panel logic — runs
identically to VR; only the final texture presentation step differs.
Iterate here, verify in VR once per milestone.

### SteamVR mode

```bash
cargo run -p rsoverlay-app -- --vr
```

Requires a running SteamVR compositor + HMD. Current milestone: a solid-red
BGRA texture is submitted to the overlay at (0, 1.5, -2) in standing-universe
space. Richer rendering (egui dashboard in VR) is gated behind the
wgpu → DXGI shared-handle interop milestone.

### Test the notification server

```bash
# From an elevated / regular PowerShell:
.\tools\send-toast.ps1 -Title "Build green" -Content "cargo clippy clean"
```

### Common flags

```
--preview / --vr             mode (default: --preview)
--list-monitors              print monitors and exit
--monitor <idx>              which monitor to mirror (default 0)
--no-mirror                  disable desktop capture
--notify-bind <addr>         notification server bind (default 127.0.0.1:42069)
--no-notify                  disable the notification server
--osc-addr <addr>            VRChat OSC input address (default 127.0.0.1:9000)
--no-osc                     disable the OSC sender
```

## Performance targets

- Cold start → first frame in VR: **< 1s**
- Idle RAM with one desktop mirror: **< 150 MB**
- Idle CPU: **< 1%**
- Desktop mirror latency: **< 16 ms**
- Stripped binary: **< 20 MB**
- Zero allocations per frame in the render hot path

## Security

- Websocket API binds `127.0.0.1` only, token-gated for non-local origins
- No `eval`, no shell-outs, no RCE surface in the notification path
- User data under `%APPDATA%\RSOverlay\` only
- `cargo-deny` + `cargo-audit` in CI, `cargo-fuzz` on the websocket parser
- Signed releases (cosign + Authenticode) at ship time
- No telemetry by default
- Plugins (future): `wasmtime` sandbox only — never dylib loading

## Stack

| Layer | Crate |
|-------|-------|
| OpenVR | hand-rolled FFI (`rsoverlay-openvr`) |
| Render | `wgpu` (DX12 / DX11 backend for OpenVR interop) |
| Desktop capture | DXGI Desktop Duplication via `windows-rs`, GPU-to-GPU |
| Window capture | Windows Graphics Capture API |
| Websocket | `tokio` + `tokio-tungstenite` |
| UI | `egui` + `egui-wgpu` |
| Config | `serde` + TOML, hot-reload |
| Input injection | `windows-rs` `SendInput` + `TouchInput` |
| Logging | `tracing` |
| Errors | `anyhow` (app) / `thiserror` (libs) |
| Browser overlay | `wry` (WebView2), opt-in |

Rust stable, edition 2021. `unsafe` only at FFI boundaries. No
`unwrap`/`expect` outside tests. Dual-licensed **MIT OR Apache-2.0**. Windows
x64 first, Linux x64 is a stretch.

## Layout

```
rsoverlay/
├── Cargo.toml                 # workspace
├── deny.toml                  # cargo-deny config
├── crates/
│   ├── rsoverlay-core/        # lifecycle, panel registry, config
│   ├── rsoverlay-openvr/      # hand-rolled FFI
│   ├── rsoverlay-capture/     # DXGI desktop + window capture
│   ├── rsoverlay-notify/      # websocket API (XSOverlay-compat schema)
│   ├── rsoverlay-render/      # wgpu renderer
│   ├── rsoverlay-input/       # laser, keyboard, SendInput
│   ├── rsoverlay-ui/          # egui panels
│   └── rsoverlay-app/         # binary: preview + VR entry points
└── docs/
    ├── ARCHITECTURE.md
    └── NOTIFICATION_API.md
```

## License

Dual-licensed under either of [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE), at your option.
