# Brand

| File | What |
| --- | --- |
| `rsoverlay-logo.svg` | The logo tile (source of truth), minimal: two layered windows, the one behind dimmed, in one rose stroke on the flat black tile both apps share. |
| `rsoverlay-logo-1024.png` | The tile rendered at 1024 px (Discord art asset `rsoverlay-mark`, store art). |
| `crates/rsoverlay-ui/src/icons/brand.svg` | The same mark in white for the app (tinted at runtime), next to the wordmark. |

RSPlayspace uses the same tile and stroke with its own mark (its
`docs/brand`).

Regenerate the PNG after editing the SVG (resvg 0.38, the renderer the app
uses): `resvg -w 1024 docs/brand/rsoverlay-logo.svg docs/brand/rsoverlay-logo-1024.png`.

**Discord:** the presence shows the art asset uploaded to RSOverlay's
Discord application, not this file. After changing the logo, upload the
new PNG in the Discord Developer Portal › the app › Rich Presence › Art
Assets, under the same key `rsoverlay-mark` (delete the old one first).
