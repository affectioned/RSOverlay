# Steam achievements (draft)

A first draft for review. Nothing here is implemented yet: achievements
need the Steamworks App ID and SDK. They're unlocked locally from things
RSOverlay already knows; no extra data is collected or sent anywhere but
Steam.

Guidelines: fun, discoverable, teach a feature, never grindy, no
achievements for spending money or for being online with others.

## Getting started

| ID | Name | How to unlock |
|---|---|---|
| `FIRST_LIGHT` | First Light | Start RSOverlay in VR for the first time |
| `SUMMONED` | Summoned | Open the dashboard with a double-press |
| `WINDOW_SHOPPING` | Window Shopping | Open a desktop window in VR |
| `HANDS_ON` | Hands On | Grab and move a panel |
| `WRIST_WATCH` | It's a Smartwatch Now | Customise the compact wrist toolbar |

## Getting comfortable

| ID | Name | How to unlock |
|---|---|---|
| `INTERIOR_DESIGNER` | Interior Designer | Save a layout |
| `SNAP_TO_IT` | Snap To It | Edge-dock two windows together |
| `PINNED` | Pin Collector | Pin 3 windows at once |
| `SEE_THROUGH` | See-Through | Lower a window's opacity below 50% |
| `CHATTERBOX` | Chatterbox | Send a VRChat chatbox message from the VR keyboard |
| `TOUCH_TYPIST` | Touch Typist | Type 1,000 characters on the VR keyboard |
| `NIGHT_OWL` | Night Owl | Have night light switch on by itself on its schedule |
| `MY_SOUNDS` | DJ | Use a custom notification sound or typing sound pack |

## Long haul (hidden until unlocked)

| ID | Name | How to unlock |
|---|---|---|
| `MARATHON` | Marathon | One VR session longer than 4 hours |
| `LOW_BATTERY_HERO` | Saved by the Bell | Get a low-battery warning and charge that controller before it dies |
| `FULL_HOUSE` | Full House | Have 6 windows open at once |
| `PERFORMANCE_DETECTIVE` | Performance Detective | Open PC performance while the game is reprojecting |
| `CENTURY` | Century | Use RSOverlay in VR on 100 different days |

Open questions for review:
- Keep `PERFORMANCE_DETECTIVE`? It only unlocks while the game is struggling.
- Do we want a few silly hidden ones (e.g. "Upside Down": rotate a window
  past 150° with Tilt on)?
