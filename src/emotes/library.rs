use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use image::codecs::webp::WebPEncoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, DynamicImage, ExtendedColorType};
use sha2::{Digest, Sha256};

use super::sniff::{self, ImageType};
use super::{
    custom_hash, custom_still_file, is_custom_ref, CustomEmote, EmoteFile, ImportError, ANIM_MAX_BYTES, ANIM_MAX_SIDE, CUSTOM_NAME_MAX,
    INPUT_MAX_BYTES, INPUT_MAX_SIDE, MAX_CUSTOM_EMOTES, STILL_FALLBACK_SIDE, STILL_MAX_BYTES, STILL_SIDE,
};

pub type SaveResult = Result<CustomEmote, ImportError>;

/// Windows can briefly lock the target (antivirus, indexer); retried three times in total.
const RENAME_RETRY_DELAYS_MS: [u64; 2] = [50, 100];

fn is_transient(err: &std::io::Error) -> bool {
    // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION, ERROR_BUSY.
    matches!(err.raw_os_error(), Some(5) | Some(32) | Some(33) | Some(170))
}

fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt = 0;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(err) => {
                if attempt >= RENAME_RETRY_DELAYS_MS.len() || !is_transient(&err) {
                    return Err(err);
                }
                thread::sleep(Duration::from_millis(RENAME_RETRY_DELAYS_MS[attempt]));
                attempt += 1;
            }
        }
    }
}

fn is_file(path: &Path, when_unknown: bool) -> bool {
    match fs::metadata(path) {
        Ok(meta) => meta.is_file(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => when_unknown,
    }
}

fn is_unsafe_char(c: char) -> bool {
    matches!(c as u32,
        0x0000..=0x001f
            | 0x007f..=0x009f
            | 0x00ad
            | 0x034f
            | 0x061c
            | 0x115f
            | 0x1160
            | 0x180e
            | 0x200b..=0x200f
            | 0x2028..=0x202e
            | 0x2060..=0x2069
            | 0x3164
            | 0xfeff
            | 0xffa0)
}

fn sanitize_name(raw: &str, fallback: &str, max: usize) -> String {
    let cleaned: String = raw.chars().filter(|c| !is_unsafe_char(*c)).collect();
    let name: String = cleaned.trim().chars().take(max).collect();
    let name = name.trim();
    if name.is_empty() { fallback.to_string() } else { name.to_string() }
}

fn sha256_hex32(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    hex::encode(digest)[..32].to_string()
}

fn longest(size: sniff::Size) -> u32 {
    size.width.max(size.height)
}

/// The still the settings window encoded: a still WebP of at most STILL_MAX_BYTES and STILL_SIDE px.
fn check_still(b: &[u8]) -> Option<ImportError> {
    if sniff::sniff_image(b) != Some(ImageType::Webp) || sniff::is_animated_webp(b) {
        return Some(ImportError::Type);
    }
    if b.len() > STILL_MAX_BYTES {
        return Some(ImportError::Size);
    }
    match sniff::webp_size(b) {
        Some(size) if longest(size) <= STILL_SIDE => None,
        _ => Some(ImportError::Type),
    }
}

/// The original animation: a GIF or WebP, small enough to decode on every emote.
fn check_anim(b: &[u8]) -> Option<ImportError> {
    let Some(kind) = sniff::sniff_image(b) else {
        return Some(ImportError::Type);
    };
    if kind != ImageType::Gif && kind != ImageType::Webp {
        return Some(ImportError::Type);
    }
    if b.len() > ANIM_MAX_BYTES {
        return Some(ImportError::AnimSize);
    }
    let size = if kind == ImageType::Gif { sniff::gif_extent(b) } else { sniff::webp_size(b) };
    match size {
        None => Some(ImportError::Type),
        Some(size) if longest(size) > ANIM_MAX_SIDE => Some(ImportError::AnimSize),
        Some(_) => None,
    }
}

pub struct EmoteLibrary {
    pub dir: PathBuf,
}

impl EmoteLibrary {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn save(&self, file: &EmoteFile, existing: &[CustomEmote]) -> SaveResult {
        let error = check_still(&file.still).or_else(|| file.anim.as_deref().and_then(check_anim));
        if let Some(error) = error {
            return Err(error);
        }
        let hash = sha256_hex32(&file.still);
        let id = format!("c:{hash}");
        if existing.iter().any(|c| c.id == id) {
            return Err(ImportError::Duplicate);
        }
        if existing.len() >= MAX_CUSTOM_EMOTES {
            return Err(ImportError::Full);
        }

        let anim_name = file
            .anim
            .as_deref()
            .map(|a| format!("{hash}.anim.{}", sniff::sniff_image(a).map(ImageType::ext).unwrap_or("bin")));
        let mut files: Vec<(String, &[u8])> = vec![(format!("{hash}.webp"), file.still.as_slice())];
        if let (Some(anim), Some(name)) = (file.anim.as_deref(), anim_name.as_ref()) {
            files.push((name.clone(), anim));
        }
        let stale: Vec<String> = [format!("{hash}.anim.gif"), format!("{hash}.anim.webp")]
            .into_iter()
            .filter(|n| Some(n) != anim_name.as_ref())
            .collect();
        if let Err(err) = self.write(&files, &stale) {
            eprintln!("[emotes] import failed: {err}");
            return Err(ImportError::Disk);
        }

        let raw: String = file.name.chars().take(256).collect();
        let name = sanitize_name(&raw, &hash[..6], CUSTOM_NAME_MAX);
        Ok(CustomEmote { id, name, animated: file.anim.is_some() })
    }

    pub fn remove(&self, id: &str) {
        if !is_custom_ref(id) {
            return;
        }
        let hash = custom_hash(id);
        for name in [format!("{hash}.webp"), format!("{hash}.anim.gif"), format!("{hash}.anim.webp")] {
            if let Err(err) = fs::remove_file(self.dir.join(&name)) {
                if err.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("[emotes] could not delete {name} {err}");
                }
            }
        }
    }

    pub fn present(&self, list: &[CustomEmote]) -> Vec<CustomEmote> {
        list.iter()
            .filter(|c| is_custom_ref(&c.id) && is_file(&self.dir.join(custom_still_file(custom_hash(&c.id))), true))
            .cloned()
            .collect()
    }

    /// Every file goes to a temp name first, then all are renamed. On any failure none of them is left behind.
    fn write(&self, files: &[(String, &[u8])], stale: &[String]) -> std::io::Result<()> {
        let paths: Vec<PathBuf> = files.iter().map(|(name, _)| self.dir.join(name)).collect();
        let temps: Vec<PathBuf> = paths.iter().map(|p| PathBuf::from(format!("{}.tmp", p.display()))).collect();
        let result = (|| {
            fs::create_dir_all(&self.dir)?;
            for name in stale {
                match fs::remove_file(self.dir.join(name)) {
                    Ok(()) => {}
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => return Err(err),
                }
            }
            for (i, (_, data)) in files.iter().enumerate() {
                fs::write(&temps[i], data)?;
            }
            for i in 0..files.len() {
                rename_with_retry(&temps[i], &paths[i])?;
            }
            Ok(())
        })();
        if result.is_err() {
            for path in temps.iter().chain(paths.iter()) {
                let _ = fs::remove_file(path);
            }
        }
        result
    }
}

/// Scales (w, h) so its longest side is `side`, never up, and never to less than a pixel.
pub fn fit_size(w: u32, h: u32, side: u32) -> (u32, u32) {
    let scale = (side as f64 / w.max(h) as f64).min(1.0);
    (((w as f64 * scale).round() as u32).max(1), ((h as f64 * scale).round() as u32).max(1))
}

pub fn check_input(b: &[u8]) -> Option<ImportError> {
    if sniff::sniff_image(b).is_none() {
        return Some(ImportError::Type);
    }
    (b.len() > INPUT_MAX_BYTES).then_some(ImportError::Size)
}

pub fn check_dimensions(b: &[u8]) -> Option<ImportError> {
    let Some(size) = sniff::image_size(b) else {
        return Some(ImportError::Type);
    };
    (longest(size) > INPUT_MAX_SIDE).then_some(ImportError::Size)
}

pub fn check_animation(b: &[u8], width: u32, height: u32) -> Option<ImportError> {
    (b.len() > ANIM_MAX_BYTES || width.max(height) > ANIM_MAX_SIDE).then_some(ImportError::AnimSize)
}

/// The file name without its extension (main sanitises it).
pub fn name_from_file(file_name: &str) -> String {
    match file_name.rfind('.') {
        Some(i) if i >= 1 => file_name[..i].to_string(),
        _ => file_name.to_string(),
    }
}

fn decode_first(b: &[u8], kind: ImageType) -> Option<(DynamicImage, bool)> {
    match kind {
        ImageType::Png => Some((image::load_from_memory_with_format(b, image::ImageFormat::Png).ok()?, false)),
        ImageType::Jpeg => Some((image::load_from_memory_with_format(b, image::ImageFormat::Jpeg).ok()?, false)),
        ImageType::Gif => {
            let mut frames = image::codecs::gif::GifDecoder::new(Cursor::new(b)).ok()?.into_frames();
            let first = frames.next()?.ok()?;
            let animated = frames.next().is_some();
            Some((DynamicImage::ImageRgba8(first.into_buffer()), animated))
        }
        ImageType::Webp => {
            // A still WebP has no frames for into_frames; the decoder's read_image covers both kinds.
            let decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(b)).ok()?;
            let animated = decoder.has_animation();
            let image = DynamicImage::from_decoder(decoder).ok()?;
            Some((image, animated))
        }
    }
}

/// The first encoding of `frame` that fits STILL_MAX_BYTES, or null when even the smallest is too big.
fn encode_still(frame: &DynamicImage) -> Option<Vec<u8>> {
    // The crate's WebP encoder is lossless only, so the reference's quality ladder collapses to its two sides.
    for side in [STILL_SIDE, STILL_FALLBACK_SIDE] {
        let (width, height) = fit_size(frame.width(), frame.height(), side);
        let image = frame.resize(width, height, FilterType::Lanczos3).to_rgba8();
        let mut out = Vec::new();
        if WebPEncoder::new_lossless(&mut out)
            .encode(image.as_raw(), width, height, ExtendedColorType::Rgba8)
            .is_ok()
            && out.len() <= STILL_MAX_BYTES
        {
            return Some(out);
        }
    }
    None
}

pub fn prepare_emote(b: &[u8], file_name: &str) -> Result<EmoteFile, ImportError> {
    if b.len() > INPUT_MAX_BYTES {
        // Refused without reading the rest of what may be a huge file.
        let head = &b[..b.len().min(16)];
        return Err(if sniff::sniff_image(head).is_some() { ImportError::Size } else { ImportError::Type });
    }
    if let Some(error) = check_input(b).or_else(|| check_dimensions(b)) {
        return Err(error);
    }
    let kind = sniff::sniff_image(b).expect("checked above");
    let (frame, frames_over_one) = decode_first(b, kind).ok_or(ImportError::Type)?;
    let animated = (kind == ImageType::Gif || kind == ImageType::Webp) && frames_over_one;
    if animated {
        if let Some(error) = check_animation(b, frame.width(), frame.height()) {
            return Err(error);
        }
    }
    let still = encode_still(&frame).ok_or(ImportError::Detail)?;
    Ok(EmoteFile { name: name_from_file(file_name), still, anim: animated.then(|| b.to_vec()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn webp_still(width: u32, height: u32) -> Vec<u8> {
        let bits = (width - 1) | ((height - 1) << 14);
        let mut body = vec![0x2f];
        body.extend_from_slice(&bits.to_le_bytes());
        let mut b = b"RIFF".to_vec();
        b.extend_from_slice(&((4 + 8 + body.len()) as u32).to_le_bytes());
        b.extend_from_slice(b"WEBP");
        b.extend_from_slice(b"VP8L");
        b.extend_from_slice(&(body.len() as u32).to_le_bytes());
        b.extend_from_slice(&body);
        b
    }

    fn anim_webp(width: u32, height: u32) -> Vec<u8> {
        let mut body = vec![0x02, 0, 0, 0];
        body.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
        body.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
        let mut b = b"RIFF".to_vec();
        b.extend_from_slice(&((4 + 8 + body.len()) as u32).to_le_bytes());
        b.extend_from_slice(b"WEBP");
        b.extend_from_slice(b"VP8X");
        b.extend_from_slice(&(body.len() as u32).to_le_bytes());
        b.extend_from_slice(&body);
        b
    }

    fn file(name: &str, still: Vec<u8>) -> EmoteFile {
        EmoteFile { name: name.to_string(), still, anim: None }
    }

    fn encoded_still(side: u32) -> Vec<u8> {
        let frame = DynamicImage::new_rgba8(side, side / 2);
        encode_still(&frame).expect("encodes")
    }

    #[test]
    fn still_must_be_a_still_webp() {
        let lib = EmoteLibrary::new(std::env::temp_dir().join("lolping-test-none"));
        assert_eq!(lib.save(&file("a.png", vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]), &[]).unwrap_err(), ImportError::Type);
        assert_eq!(lib.save(&file("a.webp", anim_webp(160, 160)), &[]).unwrap_err(), ImportError::Type);
        assert_eq!(lib.save(&file("a.webp", webp_still(320, 320)), &[]).unwrap_err(), ImportError::Type);
    }

    #[test]
    fn still_over_the_byte_cap_is_too_big() {
        let lib = EmoteLibrary::new(std::env::temp_dir().join("lolping-test-none"));
        let mut big = webp_still(160, 160);
        big.resize(STILL_MAX_BYTES + 1, 0);
        assert_eq!(lib.save(&file("a.webp", big), &[]).unwrap_err(), ImportError::Size);
    }

    #[test]
    fn animated_original_must_be_small() {
        let lib = EmoteLibrary::new(std::env::temp_dir().join("lolping-test-none"));
        let mut f = file("a.gif", encoded_still(STILL_SIDE));
        f.anim = Some(vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        assert_eq!(lib.save(&f, &[]).unwrap_err(), ImportError::Type);
        let mut f = file("a.gif", encoded_still(STILL_SIDE));
        f.anim = Some(anim_webp(2000, 10));
        assert_eq!(lib.save(&f, &[]).unwrap_err(), ImportError::AnimSize);
    }

    #[test]
    fn duplicate_and_full_are_refused() {
        let lib = EmoteLibrary::new(std::env::temp_dir().join("lolping-test-none"));
        let still = encoded_still(STILL_SIDE);
        let first = lib.save(&file("name", still.clone()), &[]).expect("saves");
        assert!(first.id.starts_with("c:") && first.id.len() == 34);
        assert_eq!(first.name, "name");
        assert_eq!(lib.save(&file("name", still.clone()), std::slice::from_ref(&first)).unwrap_err(), ImportError::Duplicate);

        let full: Vec<CustomEmote> = (0..MAX_CUSTOM_EMOTES)
            .map(|i| CustomEmote { id: format!("c:{i:032x}"), name: "x".to_string(), animated: false })
            .collect();
        assert_eq!(lib.save(&file("name", still), &full).unwrap_err(), ImportError::Full);
    }

    #[test]
    fn save_writes_then_remove_and_present() {
        let dir = std::env::temp_dir().join(format!("lolping-emotes-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let lib = EmoteLibrary::new(&dir);
        let still = encoded_still(STILL_SIDE);
        let mut f = file("my emote.png", still.clone());
        f.anim = Some(anim_webp(200, 100));
        let saved = lib.save(&f, &[]).expect("saves");
        assert!(saved.animated);
        assert!(dir.join(format!("{}.webp", custom_hash(&saved.id))).is_file());
        assert!(dir.join(format!("{}.anim.webp", custom_hash(&saved.id))).is_file());
        assert_eq!(lib.present(std::slice::from_ref(&saved)), vec![saved.clone()]);
        lib.remove(&saved.id);
        assert!(lib.present(std::slice::from_ref(&saved)).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fit_size_never_upscales() {
        assert_eq!(fit_size(320, 160, STILL_SIDE), (160, 80));
        assert_eq!(fit_size(160, 160, STILL_SIDE), (160, 160));
        assert_eq!(fit_size(80, 40, STILL_SIDE), (80, 40));
        assert_eq!(fit_size(1000, 1, STILL_FALLBACK_SIDE), (128, 1));
    }

    #[test]
    fn name_from_file_drops_the_extension() {
        assert_eq!(name_from_file("kitten.png"), "kitten");
        assert_eq!(name_from_file("a.b.c.webp"), "a.b.c");
        assert_eq!(name_from_file("noext"), "noext");
    }

    #[test]
    fn prepare_rejects_wrong_type_and_keeps_the_still() {
        assert_eq!(prepare_emote(b"not an image", "x.png").unwrap_err(), ImportError::Type);
        let still = encoded_still(STILL_SIDE);
        let prepared = prepare_emote(&still, "kitten.webp").expect("prepares");
        assert_eq!(prepared.name, "kitten");
        assert!(prepared.anim.is_none());
        assert_eq!(sniff::sniff_image(&prepared.still), Some(ImageType::Webp));
        assert!(!sniff::is_animated_webp(&prepared.still));
        assert!(check_still(&prepared.still).is_none());
    }

    #[test]
    fn prepare_keeps_a_multi_frame_original() {
        let mut gif = Vec::new();
        {
            let mut frames = image::codecs::gif::GifEncoder::new(&mut gif);
            for _ in 0..2 {
                frames.encode_frame(image::Frame::new(image::RgbaImage::new(20, 10))).expect("frame");
            }
        }
        let prepared = prepare_emote(&gif, "anim.gif").expect("prepares");
        assert_eq!(prepared.anim.as_deref(), Some(gif.as_slice()));
        assert!(prepared.still.len() <= STILL_MAX_BYTES);
    }

    #[test]
    fn prepare_rejects_an_oversized_animation() {
        let mut gif = Vec::new();
        {
            let mut frames = image::codecs::gif::GifEncoder::new(&mut gif);
            for _ in 0..2 {
                frames.encode_frame(image::Frame::new(image::RgbaImage::new(ANIM_MAX_SIDE + 1, 4))).expect("frame");
            }
        }
        assert_eq!(prepare_emote(&gif, "anim.gif").unwrap_err(), ImportError::AnimSize);
    }

    #[test]
    fn sanitize_keeps_names_visible() {
        assert_eq!(sanitize_name("  hi\nthere  ", "fb", 24), "hithere");
        assert_eq!(sanitize_name("\u{200b}\u{feff}", "fb", 24), "fb");
        assert_eq!(sanitize_name("abcdef", "fb", 3), "abc");
    }
}
