use std::time::Instant;

use crate::emotes::{CustomEmote, EmoteArt, EmoteRef};
use crate::overlay::assets::AnimPlayer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelKind {
    Ping,
    Emote,
}

#[derive(Clone, Debug)]
pub struct Tag {
    pub name: String,
    pub color: usize,
}

#[derive(Clone, Debug)]
pub struct WheelItem {
    pub key: String,
    pub icon: String,
}

#[derive(Clone, Debug)]
pub struct Wheel {
    pub kind: WheelKind,
    pub x: f32,
    pub y: f32,
    pub hot: Option<usize>,
    pub items: Vec<WheelItem>,
    pub hub: String,
    pub opened: Instant,
    pub closed: Option<Instant>,
}

#[derive(Clone, Debug)]
pub struct PingInstance {
    pub ping: String,
    pub x: f32,
    pub y: f32,
    pub start: Instant,
    pub size_px: f32,
    pub duration_s: f32,
    pub tag: Option<Tag>,
}

pub struct EmoteInstance {
    pub art: EmoteArt,
    /// Streams an animated emote; None for a still image or a placeholder.
    pub player: Option<AnimPlayer>,
    pub owner: String,
    pub x: f32,
    pub y: f32,
    pub start: Instant,
    pub size_px: f32,
    pub tag: Option<Tag>,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub title: String,
    pub body: String,
    pub start: Instant,
    pub leaving: Option<Instant>,
}

#[derive(Default)]
pub struct Scene {
    pub display_id: u32,
    pub wheel: Option<Wheel>,
    pub pings: Vec<PingInstance>,
    pub emotes: Vec<EmoteInstance>,
    pub toast: Option<Toast>,
}

#[derive(Clone, Debug)]
pub struct Visuals {
    pub ping_size_px: f32,
    pub ping_duration_s: f32,
    pub emote_size_px: f32,
    pub wheel: Vec<String>,
    pub click_ping: String,
    pub emote_wheel: Vec<EmoteRef>,
    pub click_emote: EmoteRef,
    pub custom_emotes: Vec<CustomEmote>,
    pub tick_sound: bool,
    pub emote_sound: bool,
}

impl Default for Visuals {
    fn default() -> Self {
        Self {
            ping_size_px: 110.0,
            ping_duration_s: 3.2,
            emote_size_px: 150.0,
            wheel: Vec::new(),
            click_ping: "generic".into(),
            emote_wheel: Vec::new(),
            click_emote: "facepalm".into(),
            custom_emotes: Vec::new(),
            tick_sound: true,
            emote_sound: true,
        }
    }
}
