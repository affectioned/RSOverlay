# RSOverlay — Notification API

## Scope

RSOverlay implements the XSOverlay notification websocket shape at byte level,
so existing clients (VRCX, VRChat log watchers, custom scripts) work without
modification.

- Default bind: `127.0.0.1:42069`
- Path: `/api/v1/send`
- Transport: WebSocket, JSON frames
- Auth: localhost is unauthenticated by default; non-localhost origins require
  a bearer token configured in `%APPDATA%\RSOverlay\config.toml`.

## Message shape

Exactly matches XSOverlay's `/api/v1/send`:

```json
{
  "messageType": 1,
  "index": 0,
  "timeout": 3.0,
  "height": 110.0,
  "opacity": 1.0,
  "volume": 0.7,
  "audioPath": "default",
  "title": "Hello",
  "content": "RSOverlay says hi.",
  "useBase64Icon": false,
  "icon": "default",
  "sourceApp": "my-client"
}
```

### Fields

| Field | Type | Notes |
|-------|------|-------|
| `messageType` | u32 | 1 = notification. Other values reserved. |
| `index` | u32 | Notification lane index (0-based). |
| `timeout` | f32 | Seconds before auto-dismiss. |
| `height` | f32 | Pixel height of the toast at reference DPI. |
| `opacity` | f32 | 0..=1. |
| `volume` | f32 | 0..=1 audio volume for the chime. |
| `audioPath` | string | `"default"`, `"error"`, `"warning"`, or an absolute path to a WAV. |
| `title` | string | Toast heading. UTF-8. |
| `content` | string | Toast body. UTF-8. |
| `useBase64Icon` | bool | If true, `icon` is a base64 data URI. |
| `icon` | string | `"default"`, a known icon name, or (with the flag above) base64 bytes. |
| `sourceApp` | string | Free-form client identifier, shown in the UI for attribution. |

## Non-compat surface

Anything beyond the bare message shape is RSOverlay-native and lives under a
versioned path (`/api/v2/…`). Compat path never changes shape; new features
go to a new path.

## Fuzz corpus

`cargo fuzz run notification_parse` drives the schema deserializer with
`libfuzzer`. The seed corpus lives in `fuzz/corpus/notification_parse/`.
Required pass before every release.
