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

## Voice typing mute

While you dictate with the keyboard's mic button, RSOverlay mutes your
VRChat mic and unmutes it afterwards (Settings › Keyboard › **Mute
VRChat while dictating**, on by default):

- It listens on VRChat's OSC output port, UDP `127.0.0.1:9001`, for
  `/avatar/parameters/MuteSelf`, so it only ever touches a mic it knows
  is live.
- It mutes by pressing `/input/Voice` (0 → 1 → 0), which flips mute with
  VRChat's default Toggle Voice. After the recording it presses it again,
  only if the mic is still muted.

Turning the setting off releases port 9001 for other OSC apps. If
another app already holds the port, the mic is simply left alone.
