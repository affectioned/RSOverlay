# VRChat OSC output

RSOverlay talks to VRChat's OSC input (UDP `127.0.0.1:9000` by default,
override with `--osc-addr`, disable with `--no-osc`).

## Chatbox

`/chatbox/input` — the Chat page composer, and (opt-in) the
now-playing bridge: `♪ Title — Artist` on track change, refreshed every
20 s, never faster than every 2 s, truncated to VRChat's 144 chars.

## Battery avatar parameters

Opt-in: Battery card → **send to VRChat avatar**, or in `config.toml`:

```toml
[osc]
battery_params = true
battery_prefix = "RSO/Battery"   # default
```

| Parameter | Type | Meaning |
|---|---|---|
| `RSO/Battery/HMD` | float 0–1 | headset charge |
| `RSO/Battery/LeftHand` | float 0–1 | left controller |
| `RSO/Battery/RightHand` | float 0–1 | right controller |
| `RSO/Battery/Tracker<N>` | float 0–1 | tracker at OpenVR index N |
| `RSO/Battery/Charging/<Device>` | bool | charging state |

Add the parameters you want to your avatar's expression parameters
(non-synced is enough for local displays; synced if others should see
them). Values are sent when a level moves ≥ 1 % or charging flips, and
all of them are re-sent every 30 s so an avatar switched to mid-session
picks them up. Readings come from SteamVR every 5 s, so they only flow
in `--vr` mode (or with `--demo-batteries` for testing).
