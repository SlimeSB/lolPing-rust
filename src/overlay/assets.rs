use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::AnimationDecoder;
use tiny_skia::{IntSize, Pixmap};

use crate::emotes::EmoteArt;

pub struct Assets {
    images: HashMap<PathBuf, Option<Arc<Pixmap>>>,
}

impl Default for Assets {
    fn default() -> Self {
        Self::new()
    }
}

impl Assets {
    pub fn new() -> Self {
        Self { images: HashMap::new() }
    }

    /// Any still image by absolute path: ping textures, bundled emote icons, imported emote stills.
    pub fn image(&mut self, path: impl AsRef<Path>) -> Option<Arc<Pixmap>> {
        let path = path.as_ref();
        if let Some(hit) = self.images.get(path) {
            return hit.clone();
        }
        let loaded = std::fs::read(path).ok().and_then(|bytes| decode_image(&bytes)).map(Arc::new);
        self.images.insert(path.to_path_buf(), loaded.clone());
        loaded
    }
}

/// Frames one paint may decode after falling behind, so a stalled loop cannot cascade into a long one.
const MAX_CATCH_UP: usize = 8;

/// Plays an animated WebP a frame at a time: an emote on screen holds one frame, not the whole animation.
pub struct AnimPlayer {
    bytes: Arc<[u8]>,
    frames: image::Frames<'static>,
    current: Pixmap,
    delay: f32,
    /// How long the current frame has shown, and where on the clock this pass began.
    shown: f32,
    offset: f32,
    /// Passes left after this one; None plays for as long as the emote is on screen.
    passes_left: Option<u32>,
}

impl AnimPlayer {
    /// The player for an emote's art: something to stream, or None for a still image or a placeholder.
    pub fn for_art(art: &EmoteArt) -> Option<Self> {
        match art {
            EmoteArt::Bundled(path) => Self::open(Path::new(path)),
            EmoteArt::Custom { path, animated: true } => Self::open(Path::new(path)),
            _ => None,
        }
    }

    /// Opens `path` and decodes its first frame.
    pub fn open(path: &Path) -> Option<Self> {
        let bytes: Arc<[u8]> = std::fs::read(path).ok()?.into();
        let (mut frames, passes_left) = frames_of(bytes.clone())?;
        let (current, delay) = next_frame(&mut frames)?;
        Some(Self { bytes, frames, current, delay, shown: 0.0, offset: 0.0, passes_left })
    }

    /// The frame shown `elapsed` seconds in, decoding only the frames it has to.
    pub fn frame(&mut self, elapsed: f32) -> &Pixmap {
        for _ in 0..MAX_CATCH_UP {
            if elapsed - self.offset < self.shown + self.delay {
                break;
            }
            self.shown += self.delay;
            match next_frame(&mut self.frames) {
                Some((frame, delay)) => {
                    self.current = frame;
                    self.delay = delay;
                }
                None => {
                    // The animation ended: another pass when it has one left, and its last frame stays otherwise.
                    let pass = self.shown;
                    if !self.next_pass() {
                        break;
                    }
                    self.offset += pass;
                }
            }
        }
        &self.current
    }

    /// Starts the animation over when the file allows another pass.
    fn next_pass(&mut self) -> bool {
        match self.passes_left {
            Some(0) => false,
            Some(left) => {
                self.passes_left = Some(left - 1);
                self.restart()
            }
            None => self.restart(),
        }
    }

    /// Starts the animation over from its first frame.
    fn restart(&mut self) -> bool {
        let Some((mut frames, _)) = frames_of(self.bytes.clone()) else {
            return false;
        };
        let Some((current, delay)) = next_frame(&mut frames) else {
            return false;
        };
        self.frames = frames;
        self.current = current;
        self.delay = delay;
        self.shown = 0.0;
        true
    }
}

/// The animation's frames, and the passes left after the first: a file that loops once plays once.
fn frames_of(bytes: Arc<[u8]>) -> Option<(image::Frames<'static>, Option<u32>)> {
    let decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
    let passes_left = match decoder.loop_count() {
        image::metadata::LoopCount::Infinite => None,
        image::metadata::LoopCount::Finite(loops) => Some(loops.get().saturating_sub(1)),
    };
    Some((decoder.into_frames(), passes_left))
}

/// The animation's next frame as a premultiplied pixmap, with how long it shows.
fn next_frame(frames: &mut image::Frames<'static>) -> Option<(Pixmap, f32)> {
    let frame = frames.next()?.ok()?;
    let (numerator, denominator) = frame.delay().numer_denom_ms();
    // numer_denom_ms is milliseconds, whatever the name suggests.
    let delay = if denominator == 0 { 0.0 } else { numerator as f32 / denominator as f32 / 1000.0 };
    let buffer = frame.into_buffer();
    let (width, height) = buffer.dimensions();
    Some((to_pixmap(buffer.into_raw(), width, height)?, delay))
}

fn premultiply(rgba: &mut [u8]) {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let a = u32::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = (u32::from(*channel) * a / 255) as u8;
        }
    }
}

fn to_pixmap(mut rgba: Vec<u8>, width: u32, height: u32) -> Option<Pixmap> {
    premultiply(&mut rgba);
    let size = IntSize::from_wh(width, height)?;
    Pixmap::from_vec(rgba, size)
}

pub fn decode_image(bytes: &[u8]) -> Option<Pixmap> {
    let image = image::load_from_memory(bytes).ok()?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    to_pixmap(rgba.into_raw(), width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_player_streams_one_frame_at_a_time() {
        let mut player = AnimPlayer::open(Path::new("assets/emotes/poro_ride.webp")).expect("the animation");
        assert_eq!((player.current.width(), player.current.height()), (320, 320));
        assert_eq!(player.current.data().len(), 320 * 320 * 4, "the player holds one frame, not the animation");

        // Painted the way the overlay paints it: 60 times a second, for longer than the animation runs.
        let first = player.current.data().to_vec();
        let mut moved = false;
        for step in 1..=200 {
            moved |= player.frame(step as f32 / 60.0).data() != first.as_slice();
        }
        assert!(moved, "the animation never advanced past its first frame");
    }

    #[test]
    fn a_one_shot_animation_holds_its_last_frame() {
        let mut player = AnimPlayer::open(Path::new("assets/emotes/wahaha.webp")).expect("the animation");
        // Painted past the end of the animation: the bundled files loop once, so it must not play again.
        for step in 0..=200 {
            player.frame(step as f32 / 60.0);
        }
        let settled = player.frame(3.4).data().to_vec();
        assert_eq!(player.frame(9.0).data(), settled.as_slice(), "a one-shot animation played a second time");
    }

    #[test]
    fn a_looping_animation_runs_in_step_with_the_clock() {
        let path = Path::new("target/test-anims/looping.webp");
        write_looping_asset(path);

        let mut player = AnimPlayer::open(path).expect("the animation");
        let first = player.frame(0.05).data().to_vec();
        // Painted at 60 Hz until the pass wraps, then asked for the same point of the pass after it.
        let mut step = 0;
        while player.offset == 0.0 && step < 600 {
            step += 1;
            player.frame(step as f32 / 60.0);
        }
        assert!(player.offset > 0.0, "the looping animation never started a second pass");
        assert_eq!(
            player.frame(player.offset + 0.05).data(),
            first.as_slice(),
            "the second pass ran out of step with the clock"
        );
    }

    /// A bundled animation with its loop count set to forever, written where the test can open it.
    fn write_looping_asset(path: &Path) {
        let mut bytes = std::fs::read("assets/emotes/wahaha.webp").expect("the bundled animation");
        let anim = bytes.windows(4).position(|w| w == b"ANIM").expect("an ANIM chunk");
        // ANIM holds its size, then a background colour and the loop count: 0 loops forever.
        bytes[anim + 12..anim + 14].copy_from_slice(&0u16.to_le_bytes());
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("the test directory");
        std::fs::write(path, bytes).expect("the looping animation");
    }

    #[test]
    fn a_missing_animation_has_no_player() {
        assert!(AnimPlayer::open(Path::new("assets/emotes/nope.webp")).is_none());
        assert!(AnimPlayer::for_art(&EmoteArt::Placeholder).is_none());
    }
}
