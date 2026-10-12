# lolPing, Design Spec

**Date:** 2026-10-12
**Status:** implemented in the Rust app, version 0.6.0 (`Cargo.toml`)

## 1. Purpose

A personal desktop toy for Windows that recreates League of Legends' ping wheel anywhere on the desktop. Hold the trigger key (Alt by default), drag, and a League-style ping wheel opens. Release on a slice to drop that ping, with its animation and its original sound, at the point where the wheel opened. A second key opens an emote wheel that works the same way (`src/settings/model.rs:180`).

- **Audience:** a single user on their own machine. Users on the same network, or over internet relays, can join a room and see each other's pings and emotes (section 10).
- **Capture:** pings must appear in whole-screen capture (Discord "Screen", OBS Display Capture, screenshots). Single-window capture is not supported.
- **Success criteria:**
  1. Alt+drag opens the wheel over any normal app without that app receiving the drag.
  2. Releasing on a slice shows the ping with animation and sound with no noticeable delay.
  3. With the option off, Alt+click behaves as it did without lolPing.
  4. A hotkey turns everything on and off.
  5. Every setting applies live, with no Save button.

## 2. Decisions log

| Topic | Decision |
|---|---|
| Platform | Windows only, multi-monitor, mixed DPI. The `windows` crate is an unconditional dependency and the crate targets `x86_64-pc-windows-msvc` only. |
| Stack | Rust, one process, no runtime dependencies. `tiny-skia` draws the overlays, `eframe`/`egui` draws the settings window, `rodio` plays sound. |
| Default trigger | Hold **Alt** and drag with the left mouse button to open the ping wheel (`src/input/decision.rs:99`). |
| Emote key | Hold **Ctrl** and drag to open the emote wheel (`src/settings/model.rs:180`). |
| Trigger + click (no drag) | Configurable, **off** by default. Off: the click is replayed untouched. On: it places a **generic** ping (or the chosen emote) and the click is swallowed. |
| Toggle | A global hotkey (default **Ctrl+Alt+P**) turns pinging on/off, mirrored in the tray (`src/input/decision.rs:106`). |
| Wheel slots | 8 slices, top then clockwise (`src/pings.rs:26`). The user can put any of the 11 pings on any slot, and pick the trigger+click ping. |
| Excluded pings | Defend/Hold, Target and the yellow "caution" circle: they need a target, so they wait until lolPing can pick one. |
| Textures | Only standard textures. **Never colourblind (`_cb`) variants.** |
| Sound | Original League sounds, played by the app itself, not by the overlays. |

## 3. Assets

The assets ship in `assets/` next to the executable, and from the source tree while developing (`src/paths.rs:15`). They come from the user's local League install and are (c) Riot Games.

### 3.1 Ping table (single source of truth: `src/pings.rs:11`)

Each entry is an id, a name, an icon, one or two pieces of on-screen art, a sound name and a ring colour. Slot is the default layout (`DEFAULT_WHEEL`, `src/pings.rs:26`); the user can rearrange it.

| id | Slot | Wheel icon and on-screen art (`assets/textures/`) | Sound (`assets/sounds/`) | Ring colour |
|---|---|---|---|---|
| `danger` | N | `pingwheel_retreatrender` | `SRP_12` | `#ff3b55` |
| `push` | NE | `push`, `pingwheel_pushrender` | `SRP_9` | `#35d27a` |
| `omw` | E | `omw` (the flat ground render looks wrong) | `OnMyWay` | `#2e8bff` |
| `allin` | SE | `all_in`, `pingwheel_allin_render_01` + `_02` (two halves that clash together) | `SRP_4` | `#f2b320` |
| `assist` | S | `assist_me` | `ComeHere` | `#56d43c` |
| `needvision` | SW | `need_vision` | `SRP_7` | `#56d43c` |
| `missing` | W | `enemy_missing` | `MIA` | `#f2c518` |
| `enemyvision` | NW | `enemy_vision`, `pingwheel_enemyvisionrender` | `AreaIsWarded` | `#ff3b55` |
| `generic` | (trigger+click) | `generic_ping`, `pingwheel_basicpingrender` | `Base` | `#2e8bff` |
| `bait` | (unslotted) | `pingwheel_baitrender` | `SRP_11` | `#efbc12` |
| `visioncleared` | (unslotted) | `vision_cleared`, `pingwheel_visionclearedrender` | `SRP_6` | `#2b80ff` |
| wheel tick | | | `button` | |

`FALLBACK_TEXTURE` is `generic_ping` and `TICK_SOUND` is `button` (`src/pings.rs:39`). `all_texture_names` and `all_sound_names` walk the table, so a new ping is one row and nothing else. Unused extracted files stay in `assets/` but are never referenced.

### 3.2 Emote catalog (single source of truth: `src/emotes/catalog.rs:10`)

44 bundled emotes, each with a slug, the League emote id, an English and a Chinese name, and whether it has a sound. Files live in `assets/emotes/` as `<slug>.webp` (animation), `<slug>.png` (icon) and, when there is one, `<slug>.ogg` (`src/emotes/mod.rs:119`). `DEFAULT_EMOTE_WHEEL` is 8 slugs and `DEFAULT_CLICK_EMOTE` is `facepalm` (`src/emotes/catalog.rs:57`).

The app checks every asset it will load once at startup, in `missing_assets` (`src/app.rs:822`), and lists what is missing in the settings window under About. The window covers the pings, the emotes and their sounds.

## 4. Architecture

One process, no inter-process channel. `src/lib.rs` is only the module list (`src/lib.rs:1`); `src/main.rs` owns the process and the control window; `src/app.rs` owns everything else and is the only place the parts meet.

```
main thread (src/main.rs:47)
  App::tick() every 16 ms while the message loop waits
    |-- input-hook thread   WH_MOUSE_LL + WH_KEYBOARD_LL, its own message queue
    |-- overlay thread      one layered window per display, its own message pump
    |-- room manager thread transports + a pump thread per transport
    |-- settings window     egui host thread, one per process
    |-- settings writer     debounced settings.json writer
    `-- tray               the shell's icon, polled from the main loop (src/app.rs:290)
```

- **`src/app.rs`** owns the paths, the emote library, the settings store and its current `Settings`, the language, the enabled flag, the overlay handle, the input hook handle, the audio device, the tray, the settings window and the room (`src/app.rs:32`). `App::tick` drains the five channels in a fixed order, hook then overlay then room then window then tray (`src/app.rs:141`). Nothing else in the program mutates shared state.
- **`src/main.rs`** sets per-monitor DPI awareness v2 before anything else, takes the single-instance mutex `Local\lolPing`, and creates a hidden control window of class `lolPing` (`src/main.rs:47`). A second launch finds that window and posts `WM_APP+1` to it, which the running instance turns into "open the settings window". The control window also receives `WM_POWERBROADCAST` with `PBT_APMRESUMEAUTOMATIC`, which becomes a wake call so the LAN socket rebinds and the relays reconnect (`src/main.rs:20`).
- **`--hidden`** (what the autostart entry passes) skips opening the settings window; a manual launch opens it.
- **`src/lib.rs`** exposes the 17 modules so `main.rs` can use the crate as a library. That split exists so the whole app can be unit tested without a Windows message loop.

The main loop waits with `MsgWaitForMultipleObjectsEx(None, 16, QS_ALLINPUT, MWMO_INPUTAVAILABLE)`, so it wakes for either a message or the next frame, and `App::tick` runs on every pass.

## 5. Input

### 5.1 The hook (`src/input/hook.rs`)

`hook::spawn` starts one named thread, `input-hook`, which installs `WH_MOUSE_LL` and `WH_KEYBOARD_LL` with `SetWindowsHookExW` and then runs a `GetMessageW` loop (`src/input/hook.rs:75`, `:273`). The `Decision` state machine lives in a thread local, so a hook callback takes no lock and does nothing but consult the machine and return the result.

The thread answers three kinds of message:

- `WM_APP+1` carries a `HookCommand` (`Config`, `Enabled`, `Suspend`, `Shutdown`) as a boxed pointer posted by `HookHandle::send` with `PostThreadMessageW` (`src/input/hook.rs:33`, `:55`).
- `WM_APP+2` carries a list of inputs to inject. This indirection matters: `SendInput` called from inside a hook callback waits on the hook chain that is currently running, so the callback posts the injection to its own queue and sends it from the loop instead (`src/input/hook.rs:149`).
- A 1 s `SetTimer` tick resyncs the modifier state from `GetAsyncKeyState` and runs the stuck-gesture watchdog.

Mouse and keyboard callbacks build one `InputEvent` each and hand it to the machine. A callback returns 1 to swallow the event, and `CallNextHookEx` otherwise. Events this app injected are recognised by the marker `0x4C50_494E` ("LPIN") in `dwExtraInfo` plus the `LLMHF_INJECTED`/`LLKHF_INJECTED` flag, so the app never reacts to itself (`src/input/hook.rs:25`). The callback never touches the OS except for `GetAsyncKeyState`.

The hook and the main loop meet through a channel: the machine returns an `Emit` per event, the hook thread sends it on a `Sender<Emit>` that `App::poll_hook` drains on the main thread (`src/app.rs:169`). Commands travel the other way as posted messages, which keeps the two threads free of shared state.

`app::spawn_hook` waits up to 5 s for the thread to report that the hooks are in place; anything else leaves `App.hook` as `None`, which is what the tray's warning state and the settings window's "stopped" card are keyed on (`src/app.rs:901`). `restart_hook` tears the thread down and starts a new one, re-applying `Suspend` if the settings window is capturing a key.

### 5.2 The decision state machine (`src/input/decision.rs`)

Pure logic: no Windows call, no clock, no I/O. Every event returns an `Outcome` with three fields, `swallow`, `emits` and `injects` (`src/input/decision.rs:170`).

States are `Idle`, `Pending` (a trigger claimed the press, movement has not passed the threshold), `Wheel` and `SwallowUp` (`src/input/decision.rs:177`). `SwallowUp` exists for one reason: when a gesture is cancelled, the press was already swallowed, so the matching release must be swallowed too, even though the wheel is gone. A later press of the same button resets it, which is how a lost release is recovered.

| Situation | Action |
|---|---|
| Disabled or suspended | Pass everything through; only the toggle hotkey is checked |
| Left button down while a trigger is held | **Swallow**, record the press point, go to `Pending` |
| `Pending`, movement over `dragThresholdPx` | Emit `WheelOpen` at the **press** point and `WheelMove`, go to `Wheel` |
| `Wheel`, mouse move | Emit `WheelMove`. Movement is not swallowed, so the cursor still moves |
| `Wheel`, release | **Swallow**, emit `WheelRelease` at the release point, go to `Idle` |
| `Pending`, release (trigger+click) | Option on: **swallow**, emit `Click` at the press point. Option off: **replay** the down and up with `SendInput` |
| `Wheel`, right button down or Esc | **Swallow**, emit `Cancel` |
| Trigger released during `Pending` | Same as a release: replay, or ping if the option is on |
| Trigger released during `Wheel` | Emit `Cancel` |
| Toggle hotkey | **Swallow**, flip enabled, emit `Toggled`; if it disabled mid-gesture, cancel first |
| No mouse event for 10 s while `Pending` or `Wheel` | Emit `Cancel` (`WATCHDOG_MS`, `src/input/decision.rs:11`) |

**Invariant:** for every swallowed press the paired release is swallowed too, or both are replayed. An app never sees an unmatched press.

Three details a reader cannot see from one function:

- **Two wheels, one press.** Each wheel has its own trigger, its own click flag and its own drag button, read through a `Spec` (`src/input/decision.rs:197`). A press is only claimed when exactly one wheel's trigger claims it; when both do (Alt and Ctrl held together, for example), the press passes through untouched rather than the code guessing which wheel the user meant.
- **The gesture's owner ends it.** Only the trigger that opened the gesture can close it, so the other wheel's key can come and go mid-drag without disturbing anything (`src/input/decision.rs:466`).
- **Key triggers are kept from the OS.** Caps Lock and a custom trigger key have their down and up swallowed for as long as pinging is on, so Caps Lock does not toggle and a custom key does not type (`src/input/decision.rs:455`).

### 5.3 Modifier side effects

Windows opens the menu bar (Alt) or the Start menu (Win) when the key goes up with no other key in between. When a gesture swallowed something while Alt or Win was held, the key-up is swallowed and replayed after a no-op key, `VK_NONAME` (0xFC): down, up, then the real key-up, in that order (`src/input/decision.rs:9`, `:480`). A plain Alt tap is never touched.

With a side mouse button as the trigger, that button is the drag button. A side-button click without movement is replayed so the app underneath still gets its Back/Forward, or it pings if the trigger+click option is on.

### 5.4 Key tables (`src/keys.rs`)

One file holds every conversion between the three names a key can have:

- `code_to_vk` maps the physical-key name the settings window reports (`KeyP`, `Digit7`, `Numpad3`, `F24`, `CapsLock`, ...) to a Windows virtual-key code (`src/keys.rs:43`).
- `vk_label` goes the other way, for anything that shows a key to the user.
- `mod_labels` and `hotkey_label` turn a `Hotkey` into `Ctrl + Alt + P`; `event_mods` packs the four modifier flags into the bitmask the hook config uses (`MOD_CTRL` 1, `MOD_ALT` 2, `MOD_SHIFT` 4, `MOD_WIN` 8, `src/keys.rs:3`).
- `UNSAFE_TRIGGER_VKS` lists the nine keys a custom trigger may never use, because the trigger key is swallowed system-wide: Enter, Tab, Backspace, Space, Escape and the four arrows (`src/keys.rs:18`).
- `HOTKEY_PRIMARY_MODS` is Ctrl, Alt and Win: a toggle hotkey without one of them would swallow ordinary typing (`src/keys.rs:9`).

## 6. Geometry, displays and the wheel

### 6.1 The wheel (`src/geometry.rs`)

`WHEEL` is the whole geometry in one constant: inner radius 86, outer 188, the cardinal wedges reach 288, icons sit at radius 137 and are 48 px (`src/geometry.rs:9`).

`pick_slice(dx, dy, dead_zone)` is the only selection rule (`src/geometry.rs:13`). It returns `None` inside the dead zone, otherwise 0 to 7 with 0 at the top and going clockwise, computed as `(atan2 + 90 + 22.5) mod 360` divided into 45 degree sectors, so each sector is centred on its compass direction. The overlay passes the wheel's inner radius as the dead zone, which is why releasing near the centre cancels.

### 6.2 Displays (`src/display.rs`)

`display::displays` enumerates monitors with `EnumDisplayMonitors` and asks each one for its rectangle and its effective DPI with `GetDpiForMonitor` (`src/display.rs:60`). Each `DisplayMap` carries the id, the physical rectangle, the DIP rectangle and the scale (`src/display.rs:27`). The scale is `dpi / 96`, and the DIP rectangle is the physical one divided by that scale, so a 150% display reports 1536x864 for a 2304x1296 panel.

The four conversions are:

- `physical_to_dip`: a physical point to global DIP coordinates, using the display under the point (`src/display.rs:96`).
- `physical_to_local`: a physical point to pixels inside one overlay, either the display under the point or a named display. Passing the named display is what keeps wheel movement coherent when the cursor crosses onto another monitor.
- `local_to_physical`: the inverse, used when a room mate's ping has to be placed on this machine.
- `display_at_physical`: the display containing the point, or the nearest one when the point is off every display, by squared distance to the rectangle.

Room pings need a stable display number across machines. `number_displays` puts the primary first as #1, then the rest sorted by x then y, which is left to right and top to bottom (`src/display.rs:122`). `to_shared` turns a point inside one overlay into that number plus a fraction of the display's size, clamped to 0..1 (`src/display.rs:139`). `resolve_target` picks the display with that number, falling back to the primary when this machine does not have it (`src/display.rs:148`).

## 7. Overlay

### 7.1 One window per monitor (`src/overlay/mod.rs`, `src/overlay/window.rs`)

`Overlay::start` runs everything on one named thread, `overlay` (`src/overlay/mod.rs:68`). At startup it enumerates the displays and creates one `LayeredWindow` per display, exactly covering that display's physical rectangle, and shows it (`src/overlay/mod.rs:129`).

`LayeredWindow::new` creates a `WS_POPUP` window with `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE` (`src/overlay/window.rs:31`). `TRANSPARENT` makes the window click-through, so the mouse never lands on it; `TOPMOST` and `NOACTIVATE` keep it above other windows without stealing focus; `TOOLWINDOW` keeps it out of the taskbar. Pixels are composited with per-pixel alpha through `UpdateLayeredWindow` with `ULW_ALPHA`, fed by a 32-bit top-down `CreateDIBSection` (`src/overlay/window.rs:94`). The window class is `lolping-overlay` and its procedure is a plain `DefWindowProcW`: the window never receives input, it only gets painted.

The overlay thread pumps its own messages with `PeekMessageW`, because the layered windows it created belong to that thread. It then advances and paints the scenes. The frame budget is one frame per 16.667 ms and only applies while something is animating (`FRAME`, `src/overlay/mod.rs:24`). When nothing moves the thread blocks on the command channel with a 100 ms timeout, so an idle overlay costs nothing. That split is the reason a ping never delays the main loop and the main loop never delays a ping.

### 7.2 What the overlay is told (`src/overlay/mod.rs:42`)

The main thread sends commands on one channel: `Visuals` (the whole look, resent on every settings change), `OpenWheel`, `MoveWheel`, `ReleaseWheel`, `CancelWheel`, `SpawnPing`, `SpawnEmote`, `ClearEmote`, `Toast`, `Shutdown`. The overlay answers on another: `Hover` when the selected slice changes, `Released` with the chosen item's key and the wheel centre, and `Cancelled`. The overlay never reads a settings file and never decides anything about rooms: the main thread owns both.

Per display there is a `Scene`: the wheel (if open), the pings, the emotes and the toast (`src/overlay/scene.rs:68`). At most 60 pings are kept; past that the oldest is dropped (`MAX_PINGS`, `src/overlay/mod.rs:30`). A wheel release spawns its ping or emote at the wheel's centre through `spawn_at`, which is the same code path a trigger+click takes (`src/overlay/mod.rs:227`).

An emote belongs to an owner, one per screen. Spawning a new emote for an owner removes that owner's previous emote on every display, which is why the overlay sends `ClearEmote` with the exception of the display it just spawned on. That is what makes a room mate's new emote replace their old one everywhere instead of leaving a stale one on the other monitor (`src/overlay/mod.rs:428`).

### 7.3 The render pipeline (`src/overlay/paint.rs`)

Each frame allocates one `tiny-skia` pixmap the size of the display, paints the scene into it in physical pixels, and hands the premultiplied bytes to the window (`src/overlay/mod.rs:491`). Painting order is pings, emotes, wheel, toast (`src/overlay/paint.rs:33`).

The animation code is a small port of the timing model the look was designed with. `Ease` is a cubic Bezier with a Newton solve and a bisection fallback, and `keys` walks keyframe pairs applying the easing per segment (`src/overlay/paint.rs:53`, `:120`). Everything else is expressed in those two functions.

- **A ping** fades over the last 13% of its life, has a soft coloured ground ellipse, and expands three rings at 0, 0.22 s and 1.4 s, each over 0.65 s from 0.25x to 1.35x (`src/overlay/paint.rs:438`). The art pops from 0.4 to 1.15 to 1.0 over 0.28 s and then bobs 5 px on a 1.6 s cycle. The glow under the art is the image's alpha, blurred and tinted, drawn first (`src/overlay/paint.rs:346`).
- **All In** is the two-art case: the halves slide in, a white flash, then a 250 ms shake, which lives in `paint_clash_ping` (`src/overlay/paint.rs:501`).
- **The wheel** is eight 45 degree sectors, a wedge gradient between the inner and outer radii, cardinal wedges reaching the extended radius, a gold-tinted divider on every edge, and the hovered slice brightened to its own reach (`src/overlay/paint.rs:732`). The centre is a teal radial disc with a `#c8aa6e` ring, the trigger+click icon above the label, and the label is `PING` or `EMOTE` depending on the wheel (`src/overlay/paint.rs:861`). It opens from 0.85 scale with a fade over 0.12 s (`WHEEL_FADE_S`, `src/overlay/paint.rs:730`). Idle icons are tinted to gold by `tint_icon`, which folds the grayscale, sepia, saturation and brightness steps into one 3x3 matrix applied on a device-sized copy of the icon (`src/overlay/paint.rs:371`).
- **An emote** is drawn inside a speech bubble, bobs, and fades out over its last 0.35 s (`src/overlay/paint.rs:629`, `:659`).
- **A room tag** is a small pill above the ping or emote with the sender's name and colour, fading out with the art (`src/overlay/paint.rs:590`). Colours are indexed into a fixed palette so a peer can never send raw styling (`src/room/colors.rs:2`).

### 7.4 Images and animations (`src/overlay/assets.rs`)

`Assets` is a path-keyed cache of decoded pixmaps, including negative results, so a missing file is not re-read every frame (`src/overlay/assets.rs:11`).

`AnimPlayer` streams an animated WebP one frame at a time instead of decoding the whole animation into memory (`src/overlay/assets.rs:42`). It decodes the next frame only when the clock passes the current frame's delay, and caps catch-up at 8 frames so a stalled thread cannot turn into a long decode burst. A file whose loop count is finite plays that many passes and then holds its last frame; a file that loops forever restarts in step with the clock (`src/overlay/assets.rs:73`). Frames are premultiplied before they become a pixmap, which is what `tiny-skia` and the layered window both expect.

### 7.5 Text (`src/overlay/text.rs`)

`Text` holds three faces, Regular, Semibold and Bold, loaded from the Windows font folder: `segoeui.ttf`, `seguisb.ttf` and `segoeuib.ttf` (`src/overlay/text.rs:75`). Segoe UI has no Chinese, so each weight also holds a CJK fallback, the first of Deng, Microsoft YaHei, SimHei and SimSun that exists (`src/overlay/text.rs:41`). `pick` chooses per character: it takes the main face's glyph, and only when that glyph is missing falls back, so Latin text keeps Segoe metrics (`src/overlay/text.rs:58`).

`width` and `draw` apply kerning between consecutive glyphs of the same face and add letter spacing after every glyph; `draw` supports left, centre and right alignment and blends coverage straight into the premultiplied pixmap (`src/overlay/text.rs:90`, `:109`). This is the only text drawing code in the overlays, so the wheel label, the `BACK` hint, room names and toasts all go through it. The settings window has its own text stack, described in section 9.3.

## 8. Audio (`src/audio.rs`)

One `Audio` owns a rodio mixer device and a table of decoded sounds (`src/audio.rs:20`). At startup it loads every `.wav` in `assets/sounds` under its file stem, and every `.ogg` in `assets/emotes` under the prefix `emote:` (`src/audio.rs:96`). The overlay has no sound at all: the main thread plays a ping's sound when the ping is created, wherever it came from (`src/app.rs:680`).

WAV files go through rodio's decoder. Opus-in-Ogg does not, so emote clips take a hand-written path: read the `OpusHead` header for the channel count and the pre-skip, decode every packet with a 48 kHz Opus decoder, then trim the output to the last page's granule position and drop the pre-skip samples (`src/audio.rs:28`). Both numbers matter: the granule is the stream's true length, and the pre-skip is encoder delay, so without them a clip is either padded or clipped.

Playback is a fresh `Player` per sound connected to the shared mixer, so several pings can overlap (`src/audio.rs:108`). At most 8 voices are kept; a ninth drops the oldest, which is the safety ceiling that stops a room full of people from stacking sound forever (`MAX_VOICES`, `src/audio.rs:9`). Muting and volume are applied at play time, and a ping whose sound file is missing is simply silent.

## 9. Settings

### 9.1 The model (`src/settings/model.rs`)

`Settings` is one flat struct with camelCase JSON names (`src/settings/model.rs:121`). It holds the ping trigger and the emote key, the wheel and the click ping, the toggle hotkey, the drag threshold, the ping and emote sizes, the ping duration, the volume and its mute flag, the tick and emote sound flags, the language, the room identity, the room preferences and the emote library.

Defaults: ping trigger Alt, emote key Ctrl, drag threshold 8 px, click ping off, the eight standard wheel slots, toggle Ctrl+Alt+P, ping size 110 px, ping duration 3.2 s, volume 70, tick sound on, emote size 150 px, emote sound on, no custom emotes.

Every number has a range and a step in one table, `LIMITS` (`src/settings/model.rs:208`): drag distance 2 to 40, ping size 60 to 220, ping duration 1.0 to 8.0, volume 0 to 100, incoming ping limit 0 to 20 (0 means unlimited), emote size 80 to 300.

`normalize(raw, identity)` is the only way anything becomes a `Settings`, and it never fails: it reads each field, clamps numbers to their limit and rounds them to their step, drops unknown enum values and malformed values back to the default, rebuilds the wheel only when it is exactly 8 distinct known ids, filters the custom emote list field by field, and sanitises the display name (`src/settings/model.rs:388`). A settings file with a bogus trigger, a broken wheel and a name full of control characters normalises to a valid object rather than to an error.

Three rules live here rather than in the window:

- **The three keys cannot clash.** The ping trigger and the emote key cannot be the same key, and the emote key cannot be the toggle hotkey's key. Caps Lock and a custom key with code 0x14 are recognised as one physical key. `triggers_clash` answers the question, and `normalize` turns a clashing emote key off; `patch_clashes` answers it for a proposed patch before it is applied, so the settings window can refuse and keep the old value (`src/settings/model.rs:288`, `:297`).
- **A toggle hotkey needs Ctrl, Alt or Win.** A Shift-only hotkey would swallow ordinary capital letters system-wide, so it falls back to the default (`src/settings/model.rs:306`).
- **`merge`** applies a patch by serialising the current settings, overwriting the named fields, and running the result through `normalize` again (`src/settings/model.rs:442`). There is no second validation path: what the file on disk, the settings window and the tray all go through is `normalize`.

The machine's own defaults, the user name and a random tag colour, arrive as an `Identity` so a fresh install gets a usable room profile (`src/app.rs:66`, `src/app.rs:933`).

### 9.2 The store (`src/settings/store.rs`)

The file is `%APPDATA%\lolPing\settings.json` (`src/settings/store.rs:17`). `SettingsStore` owns the current settings and a writer thread that serialises every write (`src/settings/store.rs:30`).

- A save is debounced 300 ms, so dragging a slider does not write 60 times (`DEFAULT_DEBOUNCE_MS`, `src/settings/store.rs:14`). A flush bypasses the debounce and waits for the ack; `Drop` flushes, so quitting never loses the last change.
- A write goes to `settings.json.tmp` and is then renamed over the target, with retries on the transient Windows sharing errors (`src/settings/store.rs:202`, `:215`). A reader never sees a half-written file.
- A file that cannot be read or parsed is copied to `settings.bak.json` and the defaults are used; the reason is pushed onto a problem list that the settings window shows (`src/settings/store.rs:139`).
- A file from an older version is missing the newer keys; `load` notices and writes the filled-in object once (`src/settings/store.rs:70`).
- `first_run` and `emote_key_clashed` are reported to the window so it can greet a new user and explain that an emote key was turned off.

### 9.3 The settings window (`src/settings_window.rs`, `src/settings_window/ui.rs`, `src/settings_window/capture.rs`)

The window is `eframe`/`egui`. winit allows one event loop per process, so all windows run on one host thread created once and reused (`HOST`, `src/settings_window.rs:115`). It is 880x720, minimum 560x420, and CJK text needs an installed font, so the window loads the first Chinese face it finds from the system font folder into egui (`src/settings_window.rs:153`). Closing the window does not end the app: the host reports `Closed`, `App.poll_window` drops the handle, and the next "open settings" creates a new one.

Communication is two channels. The window sends `UiEvent` (a patch, an import, a removal, a preview, capture on/off, restart the hook, a room command, quit) and the app sends `WindowMsg` (status, the settings, focus, close, an import result, the room view) (`src/settings_window.rs:50`, `:70`). The window never touches the settings file or the hook: it proposes a patch and the app decides.

The page is one scrolling column of sections: status, general, pings, wheel, emotes, room, app, about (`src/settings_window/ui.rs:20`). The status card carries the master switch and a live hint naming the trigger and the hotkey; when the hook failed it offers a restart. A switch or slider edit sends a one-field patch and the app applies it live, which is why there is no Save button.

**Key capture** (`src/settings_window/capture.rs`) is a small evaluator over physical key names: Escape cancels, a repeated key is ignored, a lone modifier is ignored, an unknown key and a forbidden key each produce their own hint, and for a trigger the modifiers are dropped so the trigger is the bare key (`src/settings_window/capture.rs:24`). A toggle hotkey additionally requires Ctrl, Alt or Win. While a capture is active the app tells the hook thread to suspend, so the key being pressed does not also fire the hook. The forbidden set for a trigger is the nine unsafe keys plus the other wheel's key, so the window cannot even propose a clash (`src/settings_window.rs:186`).

**The wheel editor** is one widget used for both wheels: a small wheel with the 8 slots beside a list of everything that can go on it. Placing an id that is already on the wheel swaps the two slots instead of duplicating it (`place_on_slot`, `src/settings_window.rs:423`). A ping tile previews its sound and spawns the ping at the centre of the primary display (`src/app.rs:614`).

**Importing an emote** accepts a file from a file dialog or from a drop on the window, refuses anything over the input cap before reading it further, and sends the bytes to the app, which does the real validation (`src/settings_window.rs:360`, `src/settings_window.rs:379`).

## 10. Rooms

A room is a code plus the peers that share it. The manager owns membership, the wire format and the crypto; the transports own nothing but moving sealed bytes.

### 10.1 The code (`src/room/code.rs`)

A code is 9 random characters in Crockford base32 plus a Luhn mod 32 check character, so a code read aloud cannot be misheard and a single wrong character is caught (`ROOM_ALPHABET`, `src/room/code.rs:2`, `check_char` at `:16`). It is shown as `PING-XXXXX-XXXXX` (`format_room_code`, `src/room/code.rs:43`).

`parse_room_code` takes what the user typed or pasted and returns `Valid`, `Invalid` or `Mistyped` (`src/room/code.rs:68`). It splits the text into alphanumeric tokens, maps look-alike letters to digits (O to 0, I and L to 1), and only then tries the shapes a code can take: a `PING` prefix with the token boundaries in the right places, or the whole text as one code. It never glues neighbouring words onto a code, which is why a sentence containing a code does not join a room. Clipboard reads pass `strict`, which accepts only a well-formed `PING` code or a lone code; nobody typed that one, so anything ambiguous is refused (`src/app.rs:437`).

### 10.2 Crypto (`src/room/crypto.rs`)

`derive_room_keys` runs the code through scrypt with log2(N) 15, r 8, p 1 and the fixed salt `lolping/room/v1`, then expands the 32-byte result with HKDF-SHA256 into two keys (`src/room/crypto.rs:21`). One is the AES-256-GCM message key; the other is the relay topic, a 64-hex-character string. The scrypt work is what makes guessing a code from a relay topic impractical, and the topic says nothing about the code without it.

A packet is one format byte `0x01`, a fresh 12-byte random nonce and the ciphertext with its tag, and the format byte is also the additional authenticated data, so a packet from a future format fails to open rather than being misread (`seal`, `src/room/crypto.rs:35`; `open`, `:52`). The packet cap is 1200 bytes, which is what keeps a room packet inside one UDP datagram (`MAX_PACKET`, `src/room/crypto.rs:7`). Keys live in the manager and never reach a transport, the overlay or the settings window.

### 10.3 The wire protocol (`src/room/protocol.rs`)

Four message kinds, all carrying the sender's peer id, a per-sender sequence number and a millisecond timestamp (`RoomMessage`, `src/room/protocol.rs:46`):

- `presence`: protocol version, app version, name, tag colour, status (`on`, `paused` or `muted`).
- `ping`: the ping id, the display number and the position as fractions of that display's size.
- `emote`: the same, with an emote reference instead of a ping id.
- `bye`.

Encoding is compact UTF-8 JSON with positions rounded to four decimals, which keeps packets small and makes two identical positions compare equal (`encode_message`, `src/room/protocol.rs:74`). Decoding rebuilds the message from known fields only and rejects anything missing or out of range (`validate_message`, `src/room/protocol.rs:98`): the peer id must be 16 hex characters, integers must be exactly representable and non-negative, positions must be inside 0 to 1, the display number must be 1 to 16, the tag colour must be an index in the palette, a ping id must exist in the ping table, an emote reference must be the right shape, and a name is stripped of control, zero-width and bidi characters and cut to 16 characters (`src/room/mod.rs:16`). `PROTOCOL_VERSION` is 1 and a peer on another version is shown as needing an update rather than being trusted (`src/room/protocol.rs:11`).

One version comparison deserves a note: `is_older_than` compares three numeric groups as plain numbers, so `0.10.0` is correctly newer than `0.9.0`, and it is used to mark members too old to see emotes (`src/room/protocol.rs:154`).

### 10.4 Transports

A transport moves sealed packets and reports three things: a packet from a peer key, a peer that said bye, and its own link state (`TransportEvent`, `src/room/transport.rs:21`). A peer key is transport-local: `lan:<ip>:<port>` or `net:<relay pubkey>` (`src/room/transport.rs:5`). Each transport runs on its own thread and is driven by commands: start with the room keys, stop, shutdown, send to one peer, broadcast, wake after sleep, retry (`TransportCommand`, `src/room/transport.rs:29`). The `shared` flag on the handle tells the manager whether one send reaches every member, which is true for relays and false for LAN.

**LAN** (`src/room/lan.rs`) is one UDP socket on port 47474, bound before the handle is returned so an occupied port reports failure instead of a dead handle (`LAN_PORT`, `src/room/lan.rs:19`; `start_with`, `:60`). A broadcast goes to 255.255.255.255 and to the directed broadcast of every IPv4 interface, because Windows sends the former out of only one interface; the interface list comes from the TCP/IP registry key and is refreshed every 5 s (`broadcast_targets`, `src/room/lan.rs:221`; `interface_addresses`, `:237`). A ping is unicast to the address a member's packets came from.

Two guards sit in front of decryption. The flood guard allows 100 packets per second per source IP and 800 per second in total, and forgets the per-address budgets once it has 1024 addresses, so a spoofed-source flood cannot exhaust memory (`DEFAULT_FLOOD_LIMITS`, `src/room/flood.rs:13`; `allow`, `:27`). The receive loop then reads at most 64 datagrams per 5 ms tick, so one busy socket cannot starve the command channel (`src/room/lan.rs:172`). Both limits are checked before the AES work, which is the point: a flood costs a comparison, not a decrypt.

Both guards, and every other rate in the app, are the same token bucket: one bucket per key holding one second's worth of tokens, refilled continuously and never above the burst size, with a rate of 0 meaning unlimited and a way to forget a key when a peer leaves (`src/room/rate.rs:23`, `:28`, `:43`). The clock is injected so the bucket can be tested without waiting (`Clock`, `src/room/rate.rs:5`).

A stop waits up to 250 ms for the last queued send to leave the socket before closing it, so a `bye` still gets out (`drain`, `src/room/lan.rs:151`). A wake drops the socket and rebinds it, which is what recovers a socket that went deaf across sleep.

**Relays** (`src/room/relay.rs`, `src/room/nostr.rs`) carry the same sealed packet as one ephemeral Nostr event, kind 24747, with tag `x` set to the room topic and the packet base64 in the content (`ROOM_EVENT_KIND`, `src/room/nostr.rs:6`). Every member connects to all six relays in the list, so two members meet while they share one that works (`RELAYS`, `src/room/relay.rs:22`). Each join makes a fresh key pair, and events are signed with BIP-340 Schnorr over the SHA-256 of the NIP-01 serialisation; the receiving side verifies both the signature and the id before it looks at the content (`make_key_pair`, `src/room/nostr.rs:33`; `verify_event`, `:71`). There is no server of our own and no WebRTC, and members never see each other's IP.

`RelayConnection` is a small state machine, Connecting, Live, Cooling, Down, Retired, judged by one signal: our own events coming back (`src/room/relay.rs:92`). No echo for 25 s means down, and reconnects back off from 2 s to 60 s with 20% jitter, resetting after 60 s of health (`src/room/relay.rs:351`). A `rate-limited` rejection cools the relay for 30 s instead of dropping it. `blocked`, `restricted`, `auth-required`, `pow` or `invalid`, a message over 64 KB, or more than 400 messages per second retires it for the session (`src/room/relay.rs:253`, `:291`).

`RelayPool` holds the six, drops duplicate event ids through a 512-entry ring, and caps intake at 200 events per second (`src/room/relay.rs:527`). Its status is Ok while any relay echoes and Unavailable 15 s after none does, which is what the room page's Retry button is for. `RelaySession` owns the topic and the key pair, signs and publishes a packet, and on stop drains for up to 500 ms so a `bye` that was already sent can still be acknowledged (`src/room/relay.rs:635`). A relay event whose content is longer than 1600 bytes is dropped before decoding.

### 10.5 The manager (`src/room/manager.rs`)

`Room::start` builds the manager thread with the LAN transport and, unless internet is off, the relay transport, plus a pump thread per transport that forwards that transport's events into the manager's single input channel (`src/room/manager.rs:1151`, `:544`). The manager publishes its state as an `Arc<RoomState>` behind a mutex, which is what the tray and the settings window read without blocking the manager (`src/room/manager.rs:69`).

Membership rules, each with its constant:

- **8 members** at most; a join that settles with 8 already present leaves and reports "room is full" (`MAX_MEMBERS`, `src/room/manager.rs:15`; `settled`, `:605`).
- **Joining settles** 3 s after the internet transport leaves Starting, or at a 15 s cap when it never does; with internet off it settles immediately (`SETTLE_MS`, `:19`; `SETTLE_CAP_MS`, `:20`; `check_settle`, `:592`). That window is what lets the relays answer before the room is called active.
- **Presence** is re-sent every 2 s on LAN and every 10 s over relays, and on every link-up and profile change; over a shared transport it is at most once a second (`PRESENCE_MS`, `:24`; `announce`, `:998`).
- **Liveness**: a LAN path is alive for 6 s and an internet path for 25 s; a member with no live path is dropped after 15 s on LAN or 30 s over relays (`ALIVE_MS`, `:25`; `DROP_MS`, `:26`; `tick`, `:659`). After a sleep the grace period restarts from the wake time, so nobody is dropped for messages that arrived while the machine was asleep.
- **Path choice** is LAN while it is alive, otherwise internet, otherwise nothing (`route`, `src/room/manager.rs:935`). A member with no path is shown as Connecting for 20 s and Unreachable after that.
- **Replays** are rejected by a per-peer sequence window plus a timestamp check within 10 minutes, and a peer that said bye stays rejected for the rest of the session (`ReplayGuard`, `src/room/manager.rs:206`; `MAX_SKEW_MS`, `:27`). Packets outside the window never touch the member list; after three stale presence packets the state carries a "clock is off" hint (`STALE_STRIKES`, `:30`).
- **Rate**: each member's incoming pings and emotes pass a token bucket set by the user's 1 to 20 per second, where 0 is unlimited; pings and emotes share it (`on_action`, `src/room/manager.rs:815`). A separate cap of 5 per second with bursts of 10 applies to what we send over a shared transport, shared between pings and emotes, because relays dislike chatter (`RELAY_PING_RATE`, `:16`). Presence and bye are never capped.
- **Muting**: a room mute or a per-member mute drops that member's pings and emotes, as does pausing pinging and as does a member whose protocol version differs.

A peer id is 16 random hex characters made fresh on every join, so a peer that said bye is not confused with a peer that rejoins, and the manager's own messages are recognised and ignored (`src/room/manager.rs:463`, `:723`). Sending a ping is one sealed packet per member over that member's own path, and one broadcast over each shared transport, whichever members use it (`send_action`, `src/room/manager.rs:889`).

Mapping between machines is the display arithmetic of section 6.2: our own ping becomes a display number and a fraction, and an incoming ping becomes a physical point on the display with that number, or the primary when this machine does not have it (`remote_point`, `src/app.rs:806`).

## 11. Emotes (`src/emotes/`)

An emote reference is either a bundled slug or `c:` plus 32 hex characters, the hash of an imported still (`is_slug`, `src/emotes/mod.rs:57`; `is_custom_ref`, `:64`). `resolve_emote_art` turns a reference into what to draw: a bundled animation, an imported animation or still, or a placeholder (`src/emotes/mod.rs:135`). A remote reference resolves to the still, never to the animation, so a room mate's emote costs one image rather than an animation. An unknown but well-formed reference becomes the placeholder rather than nothing, which is what makes a member without the file still see that something happened.

**Sniffing** (`src/emotes/sniff.rs`) identifies an image by magic bytes, not by extension or by decoding: PNG, JPEG, GIF and WebP (`sniff_image`, `src/emotes/sniff.rs:56`). It then reads the size out of the header, which is what makes validation cheap: PNG and JPEG from their frame headers, GIF from the logical screen and every frame rectangle, WebP from whichever of the three chunk formats the file uses (`image_size`, `src/emotes/sniff.rs:199`). It also detects an animated WebP from the animation flag in the extended header, and that flag is what decides whether the original is kept at all (`is_animated_webp`, `src/emotes/sniff.rs:152`).

**Import** has two halves. The settings window reads the file and refuses anything over the input cap before handing the bytes over (`src/settings_window.rs:379`). The app then runs `prepare_emote`, which sniffs, checks the size and the pixel dimensions, decodes only the first frame to detect whether the file is animated, and encodes a still (`src/emotes/library.rs:285`). The still is a lossless WebP of at most 12,288 bytes on a longest side of 160 px, retried at 128 px; the encoder has no quality ladder, so those two sides are the whole ladder (`encode_still`, `src/emotes/library.rs:268`; `STILL_MAX_BYTES`, `src/emotes/mod.rs:49`). An animated original is kept only when it is at most 5 MB and 1024 px, because it will be decoded again on every spawn (`check_anim`, `src/emotes/library.rs:101`).

`EmoteLibrary::save` names the emote by the first 32 hex characters of the SHA-256 of the still, which makes the same image imported twice the same emote and lets the library refuse a duplicate (`save`, `src/emotes/library.rs:128`). It refuses past 24 custom emotes, writes every file to a temporary name and only then renames them, and removes a stale animation left by an earlier import of the same image, so a failed import leaves nothing behind (`write`, `src/emotes/library.rs:186`). `present` filters the stored list down to emotes whose still still exists, and the app runs it at startup and repairs the wheel slots that used a removed emote (`src/emotes/library.rs:178`; `src/app.rs:69`).

**Rendering an emote** is the overlay's job: `AnimPlayer` streams the frames for an animation, a still is drawn as is, and the placeholder is the bubble with a question mark (`src/overlay/assets.rs:56`). A bundled emote also plays its `.ogg` sound, but only when the emote has one and the sound setting is on (`src/app.rs:690`).

**Over a room** an emote is an `emote` message, the same fields as a ping with a reference in place of the ping id. It follows exactly a ping's routes and gates: the same pause and mute rules, the same per-member incoming limit, the same 5-per-second shared cap, and the same display mapping (`on_action`, `src/room/manager.rs:815`). Members whose presence says an app older than 0.5.0 are marked as unable to see emotes (`EMOTES_SINCE`, `src/room/manager.rs:32`).

## 12. Tray, autostart and paths

**Tray** (`src/tray.rs`) is one shell icon with three states: normal, dimmed and warning. The three icons are generated at startup from the generic ping texture: the normal one, a halved-luminance version for disabled, and a red-tinted version for a failure (`load_icons`, `src/tray.rs:58`). Each is resized to 32 px. The menu holds the enabled check box, a restart item that only appears when the hook failed, a Room submenu, Settings and Quit; a left click opens the settings (`command_for`, `src/tray.rs:136`). The Room submenu shows the code and member count when in a room, or Join from Clipboard and Create when not, and carries the room mute check box (`room_items`, `src/tray.rs:186`). The tray only ever reports a command; `App::poll_tray` performs it (`src/app.rs:290`).

**Autostart** (`src/autostart.rs`) writes or deletes the value `lolPing` under `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`, pointing at this executable with `--hidden`, which is the flag `main.rs` reads to stay in the tray (`src/autostart.rs:8`, `:16`). It is applied when the setting changes, not on every start.

**Paths** (`src/paths.rs`) are resolved once at startup into an `AppPaths`: the data folder `%APPDATA%\lolPing`, the asset folder next to the executable when it exists and otherwise the source tree's `assets`, and the four asset subfolders (`src/paths.rs:4`, `:15`). Fonts are not assets: they come from `%SystemRoot%\Fonts`, so both the overlays and the settings window draw with the machine's own faces (`src/paths.rs:45`). `AppPaths::overlay` hands the overlay exactly the four paths it needs, so the overlay cannot reach the data folder at all (`src/paths.rs:29`).

The clipboard is the one piece of shared OS state outside these three: `read_text` and `write_text` open the clipboard, read or write `CF_UNICODETEXT` and close it, and are used only for joining a room from a copied code and for copying the current code (`src/clipboard.rs:8`, `:36`).

## 13. Error handling

| Failure | Behaviour |
|---|---|
| The hooks cannot be installed | The app keeps running without them; the tray shows a warning and the settings window offers a restart (`src/app.rs:901`, `:735`) |
| The LAN port is taken | The room starts without a LAN transport, and rebinds on the next room start or wake (`src/room/manager.rs:463`) |
| Every relay is down | The room stays on LAN; the room page shows the internet as unavailable and offers Retry |
| A stuck gesture | The 10 s watchdog cancels it; swallowed presses and releases are always paired (`src/input/decision.rs:11`) |
| A settings file that will not parse | It is copied to `settings.bak.json`, defaults are used, and the reason is listed in the settings window (`src/settings/store.rs:139`) |
| A settings write fails | The failure is listed in the settings window; the app keeps running with the in-memory settings |
| An emote import is bad, too big, animated and oversized, a duplicate or over the 24 limit | Each case has its own message; nothing is written (`src/emotes/mod.rs:29`, `src/app.rs:588`) |
| An emote's files are deleted outside the app | The next start drops it from the list and repairs the wheel slots that used it (`src/app.rs:69`) |
| A missing texture or sound | The asset is listed once at startup under About; the ping falls back to `generic_ping` and plays no sound (`src/pings.rs:40`) |
| The emote key clashes with another lolPing key | The emote key is turned off and the settings window says so (`src/settings/store.rs:70`) |
| The machine sleeps | On resume the LAN socket is rebound, the relays drop their backoff, and the member grace periods restart (`src/main.rs:20`, `src/room/manager.rs:659`) |

**Known limitations** (listed under About in the settings window):

- Elevated windows such as Task Manager do not receive the hook, because Windows blocks it, so the wheel cannot be triggered over them.
- Exclusive-fullscreen games draw above the overlay.
- Single-window screen capture does not include the overlay.
- The emote key defaults to Ctrl, so Ctrl+drag no longer reaches the app underneath while pinging is on.

## 14. Testing

The tests live in the modules they cover, as `#[cfg(test)]` blocks, and the pure parts of the app are pure precisely so they can be tested without a Windows session.

- **Input** (`src/input/decision.rs:588` and on): the machine is driven through event sequences and its `Outcome` asserted: a drag opens the wheel and releases on a slice, a plain click passes through, Alt+click replays both buttons, the click ping places a ping, Esc and right-click cancel and swallow their release, the toggle hotkey flips once and ignores autorepeat, a suspended machine passes everything, the watchdog cancels, the mask key is emitted in the right order, a plain Alt tap is not masked, Caps Lock and custom keys do not reach the OS, and two triggers held together pass the press through.
- **Geometry and displays**: slice boundaries and the dead zone (`src/geometry.rs:21`), and DIP conversion, nearest-display fallback, cross-display targeting and the shared fraction clamp (`src/display.rs:152`).
- **Keys**: code-to-VK, labels, and the accelerator spelling (`src/keys.rs:174`).
- **Crypto**: known-answer vectors for the derived keys and the topic, a fresh nonce per packet, and a rejection for every single flipped byte (`src/room/crypto.rs:61`).
- **Protocol**: round trips for every message kind, four-decimal rounding, and rejection of out-of-range fields (`src/room/protocol.rs:207`).
- **Rooms**: the manager is tested against fake transports, which is why it talks to a `Net` trait rather than a concrete handle (`src/room/manager.rs:123`). The tests cover joining, the 8-member limit, presence and its answers, path choice and fallback, the drop timeouts, replay and clock skew, the rate limits, and that pings and emotes share the relay cap.
- **Relays**: a fake socket drives the connection and pool state machines through connect, echo, health timeout, backoff, cooling and retirement (`src/room/relay.rs:947`).
- **Settings**: defaults round-trip, every numeric limit clamps, garbage fields fall back, and the trigger clash rules (`src/settings/model.rs:517`). The store is tested against a temporary directory for the debounce, the atomic write and the corrupt-file backup (`src/settings/store.rs:287`).
- **Emotes**: magic-byte sniffing and header sizes, the animated flag, the still constraints, the duplicate and full cases, and the all-or-nothing write (`src/emotes/sniff.rs:208`, `src/emotes/library.rs:306`).
- **Overlay**: the animation player advances, a one-shot animation holds its last frame, and a looping one stays in step with the clock (`src/overlay/assets.rs:168`). Text rendering checks that Chinese comes from the fallback face and leaves ink (`src/overlay/text.rs:177`).
- **Audio**: an Opus emote clip and a WAV ping clip both decode to PCM (`src/audio.rs:127`).
