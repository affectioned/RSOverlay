# Testing without a headset

RSOverlay is built so changes can be verified automatically, by a person
or an AI agent, without putting on a headset. Headset testing is for what
genuinely needs SteamVR (listed at the end).

```sh
cargo test --workspace                 # everything below that runs anywhere
cargo test --workspace -- --ignored    # + live / desktop tests (see each)
cargo clippy --workspace --all-targets -- -D warnings
```

## Harnesses

| What | Where | How it works |
|---|---|---|
| **VR interaction** | `crates/rsoverlay-app/src/vr_logic.rs` (`sim` tests) | The VR loop's decisions (laser picking with layers, grab pose with / without tilt, stick lock, what shows or hides, render throttling, toolbar auto-hide, focus) live in pure functions the loop calls. Tests drive them with scripted controller sessions against `SimOverlay`, a stand-in for SteamVR's overlay ray hits. |
| **UI with laser input** | `crates/rsoverlay-ui/src/views.rs` (`laser_integration_tests`, `search_integration_tests`, `gamepad_capture_tests`) | Real egui frames fed with scripted laser samples / pad states; assert on clicks, pages, highlights, saved settings. |
| **Updater** | `crates/rsoverlay-core/src/update.rs` (`sandbox`), `crates/rsoverlay-ui/src/updates.rs` | Fake release server (transport), throwaway exe in a temp folder: verified download with progress, tampered / wrong files, off-site URLs, locked-exe rollback, restart waiting for the old process, and the UI's install states. |
| **Updater, live** | `update::live` (ignored) | Real GitHub API and a real release-asset download through the CDN redirect. |
| **Gamepad** | `crates/rsoverlay-input/src/gamepad.rs` | Scripted fake pads: combining, 2 s rescans, hot-plug, one-shot chords. |
| **Touch input** | `crates/rsoverlay-input/src/touch.rs` (`harness`, ignored) | Injects real touches into a small topmost test window of our own and checks the pointer messages it receives. Only injects after confirming that window is under the point, so nothing else on the desktop is touched. Needs an interactive desktop. |
| **Window program names** | `crates/rsoverlay-capture/src/window.rs` (ignored) | Lists the real open windows with their program. |
| **Images** | `--settings-cards DIR`, `--vr-preview --scene <preset> --screenshot <png> --exit-after N` | Headless renders of every Settings card and of the VR scenes (see `docs/store/README.md`). Look at them to check layout. |
| **Version stamping** | `RSO_VERSION=… RSO_NOTES="$(cat docs/releases/<tag>.md)" cargo run -- --settings-cards …` | Renders the Updates card as a release build would show it. |

## Rules for new work

- Every feature or fix comes with a way to verify it here: a unit /
  scripted-session test, a fake for the OS or network part, or a render
  to look at. Build the harness as part of the change.
- Never test with side effects on the real desktop or SteamVR (injecting
  into other apps, launching SteamVR); use a sandbox like the ones above.
- Only call something "needs headset testing" when no harness can reach
  it, and say why.

## Still needs a headset

- **Block game input while pointing**: depends on SteamVR's own
  laser-mouse mode (`MakeOverlaysInteractiveIfVisible`), which only the
  real compositor implements.
- **How things feel**: comfort of grab smoothing, text sharpness at a
  given resolution on a specific headset.
