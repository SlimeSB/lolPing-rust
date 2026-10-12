use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

use rodio::{
    buffer::SamplesBuffer, ChannelCount, Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, SampleRate, Source,
};

const MAX_VOICES: usize = 8;
/// Opus always decodes at 48 kHz and its longest packet is 120 ms.
const OPUS_RATE: u32 = 48_000;
const OPUS_MAX_PACKET_SAMPLES: usize = 5_760;

struct Sound {
    channels: ChannelCount,
    sample_rate: SampleRate,
    samples: Vec<f32>,
}

pub struct Audio {
    sink: MixerDeviceSink,
    sounds: HashMap<String, Sound>,
    voices: Vec<Player>,
    muted: bool,
}

/// Opus in Ogg: the emote clips ship as Opus, which rodio's decoder set cannot read.
fn load_opus(bytes: &[u8]) -> Option<Sound> {
    let mut reader = ogg::PacketReader::new(Cursor::new(bytes));
    let head = reader.read_packet().ok()??.data;
    if !head.starts_with(b"OpusHead") {
        return None;
    }
    let channels = ChannelCount::new(*head.get(9)? as u16)?;
    let pre_skip = usize::from(u16::from_le_bytes([*head.get(10)?, *head.get(11)?]));
    let count = channels.get() as usize;
    let mut decoder = opus_rs::OpusDecoder::new(OPUS_RATE as i32, count).ok()?;
    let mut samples = Vec::new();
    let mut buffer = vec![0.0f32; OPUS_MAX_PACKET_SAMPLES * count];
    let mut granule = 0u64;
    while let Some(packet) = reader.read_packet().ok()? {
        if packet.data.starts_with(b"OpusTags") {
            continue;
        }
        let decoded = decoder.decode(&packet.data, OPUS_MAX_PACKET_SAMPLES, &mut buffer).ok()?;
        samples.extend_from_slice(&buffer[..decoded * count]);
        if packet.last_in_stream() {
            granule = packet.absgp_page();
        }
    }
    // The last page's granule is the stream's true length; the first `pre_skip` samples are encoder delay.
    samples.truncate((granule as usize * count).min(samples.len()));
    samples.drain(..(pre_skip * count).min(samples.len()));
    (!samples.is_empty()).then(|| Sound {
        channels,
        sample_rate: SampleRate::new(OPUS_RATE).expect("48 kHz is not zero"),
        samples,
    })
}

fn load(path: &Path) -> Option<Sound> {
    let bytes = std::fs::read(path).ok()?;
    if let Some(sound) = load_opus(&bytes) {
        return Some(sound);
    }
    let decoder = Decoder::new(Cursor::new(bytes)).ok()?;
    let channels = decoder.channels();
    let sample_rate = decoder.sample_rate();
    let samples: Vec<f32> = decoder.collect();
    if samples.is_empty() {
        return None;
    }
    Some(Sound { channels, sample_rate, samples })
}

fn load_dir(sounds: &mut HashMap<String, Sound>, dir: &Path, extension: &str, prefix: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let matches = path.extension().is_some_and(|e| e.eq_ignore_ascii_case(extension));
        if !matches {
            continue;
        }
        let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        if let Some(sound) = load(&path) {
            sounds.insert(format!("{prefix}{stem}"), sound);
        }
    }
}

impl Audio {
    pub fn new(sounds_dir: &Path, emotes_dir: &Path) -> Option<Self> {
        let sink = DeviceSinkBuilder::open_default_sink().ok()?;
        let mut sounds = HashMap::new();
        load_dir(&mut sounds, sounds_dir, "wav", "");
        load_dir(&mut sounds, emotes_dir, "ogg", "emote:");
        Some(Self { sink, sounds, voices: Vec::new(), muted: false })
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn play(&mut self, name: &str, volume: f32) {
        if self.muted || volume <= 0.0 {
            return;
        }
        let Some(sound) = self.sounds.get(name) else {
            return;
        };
        self.voices.retain(|voice| !voice.empty());
        while self.voices.len() >= MAX_VOICES {
            self.voices.remove(0);
        }
        let player = Player::connect_new(self.sink.mixer());
        player.set_volume(volume.clamp(0.0, 1.0));
        player.append(SamplesBuffer::new(sound.channels, sound.sample_rate, sound.samples.clone()));
        player.play();
        self.voices.push(player);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_emote_clip_decodes_to_pcm() {
        let sound = load(Path::new("assets/emotes/wahaha.ogg")).expect("the Opus emote clip");
        assert_eq!(sound.channels.get(), 1);
        assert_eq!(sound.sample_rate.get(), OPUS_RATE);
        assert!(sound.samples.len() > OPUS_RATE as usize, "the clip is shorter than a second");
        assert!(sound.samples.iter().any(|s| s.abs() > 0.01), "the clip is silent");
    }

    #[test]
    fn a_ping_clip_still_decodes() {
        let sound = load(Path::new("assets/sounds/button.wav")).expect("the wav ping clip");
        assert!(!sound.samples.is_empty());
    }
}

