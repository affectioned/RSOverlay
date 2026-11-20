# RSOverlay — Architecture

## Design goals

1. **Fast cold start.** First frame in VR under 1s. No heavy runtime (no Mono,
   no CEF, no Node, no Unity player).
2. **Small resident footprint.** Under 150 MB idle with one desktop mirror.
3. **GPU-to-GPU everything.** No CPU readbacks in the capture or render path.
4. **Preview-first iteration.** Everything the HMD path does, the desktop
   preview path also does, except the final texture submission step. Keep the
   gap between the two paths small enough to notice drift immediately.
5. **Clean-room.** No code from XSOverlay/OVR Toolkit. Byte-level compat with
   the XSOverlay notification websocket schema only, through independent
   reimplementation.

## Crate graph

```
                 rsoverlay-app
                 /     |      \
          preview.rs   vr.rs   (clap, winit, signal)
            |            |
            v            v
    +---------------------------+
    |     rsoverlay-render      |   wgpu, panel_view, blit
    +---------------------------+
             |         |
             v         v
    rsoverlay-core   rsoverlay-openvr   rsoverlay-capture
    (config,         (hand-FFI,         (DXGI DDA, WGC)
     panel,          OpenVr,
     RuntimeMode)    Overlay)
             |
             v
    rsoverlay-notify  (ws schema, XSOverlay compat)
    rsoverlay-input   (SendInput, TouchInput)
    rsoverlay-ui      (egui panels)
```

Dependency rules:

- `rsoverlay-app` is the only crate that depends on both `rsoverlay-openvr`
  and `winit`. Everything else is runtime-mode agnostic.
- `rsoverlay-render` never imports `winit` or `OpenVr`. It produces textures;
  the app wires presentation.
- `rsoverlay-core` depends on nothing in the workspace. It defines the shared
  types (`Panel`, `Anchor`, `RuntimeMode`, `Config`) everyone agrees on.

## Runtime modes

`RuntimeMode::Preview` and `RuntimeMode::Vr` branch exactly once — at app
startup, when the CLI is parsed. Every subsystem downstream of that reads
`RuntimeMode` from its own context; nothing else branches on it. This keeps
the two paths from drifting.

### Preview mode

1. Open a winit window.
2. Build a `wgpu::Surface` on the window.
3. Build a `GpuCtx` and an `OverlayTarget` (offscreen BGRA8Srgb texture).
4. Each frame: render panels into `OverlayTarget`, then blit onto the window
   surface via a fullscreen-triangle pass.

The window **shows exactly the overlay texture** — no desktop chrome, no UI
scaffolding around it. What you see is what the HMD will see.

### VR mode

1. `OpenVr::init()` with `EVRApplicationType_Overlay`.
2. `VR_GetGenericInterface("FnTable:IVROverlay_024")` for the overlay fn
   table.
3. `CreateOverlay`, `SetOverlayWidthInMeters`, `SetOverlayTransformAbsolute`,
   `ShowOverlay`.
4. Each frame: render panels into `OverlayTarget`, then
   `SetOverlayTexture(d3d11_handle)`.

Step 4 texture submission is the one subsystem where preview and VR diverge.
It is isolated to `rsoverlay-app/src/vr.rs` and gated by
`RuntimeMode::is_vr()`.

## Render pipeline

Panels → `OverlayTarget` → presentation backend.

- `OverlayTarget` is a BGRA8UnormSrgb 2D texture with `RENDER_ATTACHMENT |
  TEXTURE_BINDING | COPY_SRC` usage. One per overlay.
- Panel kinds plug into `panel_view::render_panel`. P0 scope: `SolidColor`.
  Later: `Desktop`, `Window`, `Ui`.
- Presentation is per-mode. Preview blits via WGSL; VR submits via OpenVR.

## Threading model

- Main thread: winit event loop (preview) or OpenVR tick loop (vr).
- A dedicated tokio runtime handles the websocket notification server
  (future milestone) and the Ctrl+C signal.
- Capture runs on a dedicated thread per source, feeding textures into the
  render pipeline via `wgpu::Queue::write_texture` or shared handles.

## Error policy

- **Libs** use `thiserror` enums with specific variants.
- **App** uses `anyhow` with `.context()` at every boundary.
- **No `unwrap`/`expect`** outside `#[cfg(test)]` or `tests/`.
- **No `unsafe`** outside FFI boundaries (`rsoverlay-openvr` and the Windows
  capture crates). Every `unsafe` block carries a `# Safety` comment.

## Cross-platform story

The workspace is written so Linux support stays viable even though Windows is
the primary target. Compiling on Linux today produces a working **preview**
binary (window + egui + OSC + notifications). Platform-specific features gate
cleanly:

| Crate | Windows | Linux | Why |
|-------|---------|-------|-----|
| `rsoverlay-core` | ✅ | ✅ | pure types |
| `rsoverlay-render` | ✅ | ✅ | wgpu is cross-platform (DX12 / Vulkan / GL) |
| `rsoverlay-ui` | ✅ | ✅ | egui + chrono; SMTC `media::poll` stubs to `None` on non-Windows |
| `rsoverlay-notify` | ✅ | ✅ | tokio + tungstenite; portable |
| `rsoverlay-osc` | ✅ | ✅ | UDP only |
| `rsoverlay-capture` | ✅ | stub | DXGI Desktop Duplication is Windows-only. Linux needs a PipeWire screencast backend via xdg-desktop-portal. |
| `rsoverlay-input` | ✅ | stub | `SendInput`. Linux needs uinput or ydotool. |
| `rsoverlay-openvr` | ✅ | ⚠ | OpenVR has Linux builds of `libopenvr_api.so`, and the FFI already loads by platform-specific filename. Untested end-to-end on Linux. |
| `rsoverlay-app` | ✅ | ✅ | `cfg(windows)` gates the VR-mode D3D11 submission; Linux build will have Preview but `--vr` will err until the Linux submission path lands. |

Minimum Linux milestones to reach parity with Windows:
1. PipeWire screencast capture backend in `rsoverlay-capture`.
2. uinput / libei input injection in `rsoverlay-input`.
3. Linux OpenVR submission path in `rsoverlay-app/vr.rs` (OpenGL texture via
   `ETextureType_OpenGL` is simplest — avoids Vulkan interop complexity).

## Clean-room notes

- OpenVR FFI is hand-rolled from the public C ABI (`openvr_capi.h`) and the
  stable `FnTable:IVROverlay_NNN` interface accessor. No code from the
  `openvr` or `openvr-sys` crates on crates.io is linked.
- The notification websocket schema (`rsoverlay-notify::Notification`) matches
  XSOverlay's message shape by field name and type so existing clients work
  unmodified, but contains no XSOverlay code.
- Every subsystem (capture, rendering, input, UI, websocket) is written from
  scratch against the underlying OS or Rust crate APIs, not ported.
