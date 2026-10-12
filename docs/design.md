# lolPing, Design Spec

**Date:** 2026-10-12
**Status:** implemented in the Rust app, version 0.6.0 (`Cargo.toml`)

This note holds what `src/` cannot say: the decisions behind it, the shape of what peers exchange, and the invariants that hold across modules. Everything else is the code.

## 1. Purpose

A desktop toy for Windows that recreates League of Legends' ping wheel anywhere on the desktop. Hold the trigger key (Alt by default), drag, release on a slice: that ping lands, with its animation and its original sound, at the point where the wheel opened. A second key opens an emote wheel that works the same way.

- **Capture:** pings must appear in whole-screen capture (Discord, OBS, screenshots). Single-window capture is not supported.
- **Success:** the wheel opens over any normal app without that app receiving the drag; the ping shows with no noticeable delay; with the option off, Alt + click behaves as it did without lolPing; a hotkey turns everything on and off; every setting applies live, with no Save button.

## 2. Decisions

| Topic | Decision |
|---|---|
| Platform | Windows only, multi-monitor, mixed DPI. `windows` is an unconditional dependency and the crate targets `x86_64-pc-windows-msvc`. |
| Stack | Rust, one process, no runtime dependencies. `tiny-skia` draws the overlays, `eframe`/`egui` the settings window, `rodio` the sound, `tray-icon` the tray. |
| Input | A low-level hook owns the mouse and keyboard on its own thread and swallows the gesture, so the app underneath never sees the press. |
| Trigger | Hold Alt and drag with the left button for the wheel, Ctrl for the emote wheel. Both are configurable, and the two cannot share a key. |
| Trigger + click | Off by default, so ordinary Alt + click shortcuts keep working. On, it places the centre ping and swallows the click. |
| Fullscreen | On by default: a press that would start a gesture passes through when the foreground window covers the clicked monitor, so a game keeps every click. |
| Toggle | A global hotkey, Ctrl + Alt + P by default, mirrored in the tray. It needs Ctrl, Alt or Win, or it would swallow ordinary typing. |
| Wheel | 8 slices, top then clockwise, plus one centre slot. Any of the 11 pings fits any slot. |
| Excluded pings | Defend/Hold, Target and the caution circle: they need a target, so they wait until lolPing can pick one. |
| Textures | Only standard textures. Never the colourblind (`_cb`) variants. |
| Sound | Original League sounds, played by the app itself, never by the overlays. |
| Overlay | One click-through, always-on-top layered window per monitor, painted by the app rather than by the compositor. |
| Emotes | 44 bundled, plus up to 24 of your own. An imported animation plays locally; a peer gets a still or a placeholder. |
| Updates | None. The app touches the network only for rooms. |
| Settings file | `%APPDATA%\lolPing\settings.json`, camelCase keys, and the shape the older app wrote. |

## 3. What peers exchange

A room code is 9 random Crockford base32 characters plus a Luhn mod 32 check character, shown as `PING-XXXXX-XXXXX`, so a code read aloud cannot be misheard. `scrypt` over the code (log2 N 15, r 8, p 1, the fixed salt `lolping/room/v1`) gives a 32-byte secret, which HKDF-SHA256 splits into two keys: the AES-256-GCM message key, and the relay topic as 64 hex characters.

A packet is a format byte `0x01`, a fresh 12-byte random nonce and the ciphertext with its tag; the format byte is also the additional authenticated data, so a packet from a future format fails to open rather than being misread. A packet is capped at 1200 bytes, which keeps it inside one UDP datagram. Keys stay in the room manager: no transport, no overlay and no settings page ever sees them.

Four message kinds, each carrying the sender's 16-hex-character peer id, a per-sender sequence number and a millisecond timestamp:

| Kind | Carries |
|---|---|
| `presence` | Protocol version, app version, name, tag colour, status (`on`, `paused`, `muted`) |
| `ping` | Ping id, display number, position as a fraction of that display |
| `emote` | The same, with an emote reference instead of a ping id |
| `bye` | Nothing: the sender is leaving |

Positions are rounded to four decimals, which keeps packets small and makes two identical positions compare equal. An emote reference is a bundled slug, or `c:` plus the SHA-256 prefix of an imported still, so a peer that lacks the image still knows something happened.

Two transports carry the same sealed packets, and the manager picks per member: LAN while it is alive, otherwise the relays.

- **LAN** is one UDP socket on port 47474, broadcasting to the directed broadcast of every interface (Windows sends a plain `255.255.255.255` out of only one) and unicasting to the address a member's packets came from.
- **Relays** are six public Nostr relays carrying one ephemeral kind-24747 event per packet, tagged with the room topic and signed BIP-340. The receiving side verifies the signature and the id before it looks at the content. There is no server of ours, no WebRTC, and members never see each other's IP address.

A room holds 8 members. A peer id is made fresh on every join, so a peer that said bye is never confused with one that rejoins. Replays are rejected by a per-peer sequence window plus a 10-minute timestamp skew, and both transports rate-limit before they decrypt, so a flood costs a comparison rather than an AES operation.

## 4. Invariants

- For every swallowed press the paired release is swallowed too, or both are replayed: an app never sees an unmatched press.
- A press is claimed only when exactly one wheel's trigger claims it, no other program's chord is holding a trigger, and no fullscreen app owns that screen; otherwise it passes through rather than the code guessing.
- Only the main loop mutates shared state. The hook thread, the overlay thread, the transports, the settings writer and the window talk to it over channels.
- The overlay never reads a settings file and never decides anything about rooms.
- Every number, enum and string that comes from disk or from a peer passes one normalising path and cannot fail: a broken value becomes the default, never an error.
- The wheel, the ping and the emote geometry is defined once, in `src/geometry.rs` and `src/pings.rs`, and the room position is a fraction of a display so it survives a different resolution at the other end.
