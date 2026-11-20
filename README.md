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
| P0 | OpenVR overlay registration + texture submission | scaffolded |
| P0 | Desktop & window mirror (DXGI DDA + WGC) | scaffolded |
| P0 | Laser pointer + virtual keyboard | scaffolded |
| P0 | Panel grab / resize / anchor (wrist, HMD, world) | scaffolded |
| **P0** | **Desktop preview mode (no HMD required)** | **working** |
| P1 | XSOverlay-compatible websocket API (`127.0.0.1:42069`) | schema done |
| P1 | Toast notifications with icons | planned |
| P1 | Media controls (SMTC) | planned |
| P1 | Wrist / radial menu | planned |
| P1 | VRChat OSC chatbox | planned |
| P2 | `wry` WebView2 browser overlay (opt-in) | planned |
| P2 | Clock / battery / stats widget | planned |
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

Requires a running SteamVR compositor + HMD. Current milestone registers the
overlay at (0, 1.5, -2) in standing-universe space; texture submission lands
next milestone.

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
