# Custom sounds

RSOverlay makes a few sounds, and every one of them can be swapped for
your own. Out of the box they're recordings, made by people, nothing
generated and nothing borrowed from other overlays:

- **Typing:** real keystrokes of a Keychron V1 Ultra with red linear
  switches, several takes per key, played shuffled.
- **Notification, warning, error, low battery:** sounds from Kenney's
  [Interface Sounds](https://kenney.nl/assets/interface-sounds) pack
  (CC0, public domain): `confirmation_004`, `question_002`, `error_006`
  and `minimize_006`, converted to 16-bit WAV. Picked to stay mellow, no
  shrill high tones. Thanks, Kenney.

The typing pack is cut from
["Typing on Keychron V1 Ultra (Red Linear Switch)"](https://commons.wikimedia.org/wiki/File:Typing_on_Keychron_V1_Ultra_(Red_Linear_Switch).wav)
by C40115 on Wikimedia Commons, licensed
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/): single strokes
were cut out, levelled and faded.

| Sound | Plays when | Change it in |
|---|---|---|
| Typing clicks | You press a key on the VR keyboard | Settings › **Keyboard** › Sound pack |
| Notification | An app sends a normal notification | Settings › **Notifications** |
| Warning | An app sends a warning notification | Settings › **Notifications** |
| Error | An app sends an error notification | Settings › **Notifications** |
| Low battery | A headset / controller / tracker drops below your warning % | Settings › **Notifications** |

Every slot can also be **Silent**, and each has its own volume slider.

## 1. Find the sounds folder

```
%APPDATA%\RSOverlay\sounds
```

Paste that into the Explorer address bar, or press
**Settings › Notifications › Open sounds folder** (it creates the folder
if it doesn't exist yet).

## 2. Make your files the right format

RSOverlay plays **WAV** files that are:

- **16-bit PCM** (the most common kind of WAV)
- **2 MB or smaller** (a click is a few KB; a chime is maybe 50 KB)
- any sample rate (44.1 kHz or 48 kHz are typical), mono or stereo

MP3, OGG, FLAC and 24/32-bit WAVs aren't played. A file that doesn't
qualify quietly falls back to the built-in sound, so nothing breaks; check
the log if a sound isn't changing.

**Converting with Audacity:** open the file → *File › Export Audio* →
Format **WAV (Microsoft)**, Encoding **Signed 16-bit PCM**.

**Converting with ffmpeg:**

```
ffmpeg -i input.mp3 -ac 1 -ar 44100 -c:a pcm_s16le output.wav
```

Tips:

- **Trim the silence** at the start. Even 50 ms of silence makes a key
  click feel laggy.
- **Keep typing clicks short**, under ~100 ms. A new click cuts the
  previous one off, so long sounds get chopped while you type.
- Use sounds you have the right to use. Freesound.org filtered by
  CC0 is a good source.

## 3. Alert sounds (notification, warning, error, low battery)

1. Put your `.wav` files **directly in** the sounds folder (not in a
   subfolder), e.g. `sounds\ping.wav`, `sounds\uh-oh.wav`.
2. Open **Settings › Notifications** and press **Refresh**.
3. Pick a file for each slot. **▶** plays a test.
4. **Sound volume** sets the level for all alerts.

The same file can be used for several slots.

### For app developers

Apps that send notifications to RSOverlay (the XSOverlay-compatible API,
see [NOTIFICATION_API.md](NOTIFICATION_API.md)) choose the **category**
with `audioPath`: `"default"`, `"warning"`, `"error"` or `"none"`. The
*user* decides which sound each category plays. File paths in
`audioPath` are not played, for safety.

## 4. Typing sound packs

A pack is a **folder** inside `sounds` with up to five files:

```
%APPDATA%\RSOverlay\sounds\
└── Thocky\
    ├── key.wav         every normal key
    ├── space.wav       space bar
    ├── enter.wav       Enter
    ├── backspace.wav   Backspace
    └── modifier.wav    Shift, Ctrl, Alt, Win, Caps
```

Every file is optional: whatever is missing uses the recorded Keychron
sound. A
pack with only `key.wav` changes every key to that sound except the four
special ones.

**Several takes per key** sound much more natural: add `key2.wav`,
`key3.wav` ... (up to `key8.wav`, and the same for `space`, `enter`,
`backspace`, `modifier`). RSOverlay picks one at random for each press,
never the same one twice in a row.

1. Create the folder (its name is the pack's name) and add your files.
2. Open **Settings › Keyboard**, press **Refresh**, and pick the pack
   under **Sound pack**.
3. **Volume** and the **Typing sounds** on/off switch are on the same
   card.

## Troubleshooting

| Problem | Fix |
|---|---|
| My file isn't in the list | Press **Refresh**. Make sure it ends in `.wav` and sits directly in `sounds` (alerts) or in a pack folder (typing). |
| It still plays the built-in sound | The file isn't 16-bit PCM or is over 2 MB. Re-export it (see step 2). |
| No sound at all | Check the slot isn't **Silent**, the volume isn't 0, and Windows' volume mixer isn't muting RSOverlay. |
| Typing sounds are choppy | Shorten the clip and trim the silence at its start. |
