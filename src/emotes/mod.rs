pub mod catalog;
pub mod library;
pub mod sniff;

use std::collections::HashSet;
use std::path::Path;

use crate::i18n::{Lang, LANG_ZH_CN};

pub use catalog::{DEFAULT_CLICK_EMOTE, DEFAULT_EMOTE_WHEEL};

pub type EmoteRef = String;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CustomEmote {
    pub id: EmoteRef,
    pub name: String,
    pub animated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmoteArt {
    Bundled(String),
    Custom { path: String, animated: bool },
    Placeholder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportError {
    Type,
    Size,
    AnimSize,
    Detail,
    Full,
    Duplicate,
    Disk,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmoteFile {
    pub name: String,
    pub still: Vec<u8>,
    pub anim: Option<Vec<u8>>,
}

pub const MAX_CUSTOM_EMOTES: usize = 24;
pub const EMOTE_DURATION_S: f32 = 3.0;
pub const CUSTOM_NAME_MAX: usize = 24;
pub const STILL_MAX_BYTES: usize = 12_288;
pub const STILL_SIDE: u32 = 160;
pub const STILL_FALLBACK_SIDE: u32 = 128;
pub const INPUT_MAX_BYTES: usize = 8_388_608;
pub const INPUT_MAX_SIDE: u32 = 8192;
pub const ANIM_MAX_BYTES: usize = 5_242_880;
pub const ANIM_MAX_SIDE: u32 = 1024;

pub fn is_slug(v: &str) -> bool {
    let bytes = v.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 40
        && bytes.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

pub fn is_custom_ref(v: &str) -> bool {
    let Some(hash) = v.strip_prefix("c:") else {
        return false;
    };
    hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn is_emote_ref_shape(v: &str) -> bool {
    is_slug(v) || is_custom_ref(v)
}

pub fn custom_hash(reference: &str) -> &str {
    &reference[2..]
}

pub fn is_bundled(reference: &str) -> bool {
    is_slug(reference) && catalog::by_slug(reference).is_some()
}

pub fn emote_name(reference: &str, lang: Lang, custom: &[CustomEmote]) -> String {
    if let Some(bundled) = catalog::by_slug(reference) {
        return if lang == LANG_ZH_CN { bundled.name_zh } else { bundled.name }.to_string();
    }
    custom
        .iter()
        .find(|c| c.id == reference)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "?".to_string())
}

pub fn emote_anim_file(slug: &str) -> String {
    format!("{slug}.webp")
}

pub fn emote_icon_file(slug: &str) -> String {
    format!("{slug}.png")
}

pub fn emote_sound_file(slug: &str) -> String {
    format!("{slug}.ogg")
}

pub fn custom_still_file(hash: &str) -> String {
    format!("{hash}.webp")
}

/// The animation an imported emote was saved as: its codec is in the extension, and a still stands in when it is gone.
fn custom_anim_file(custom_dir: &Path, hash: &str) -> Option<String> {
    ["webp", "gif"]
        .into_iter()
        .map(|ext| custom_dir.join(format!("{hash}.anim.{ext}")))
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

pub fn all_emote_asset_files() -> Vec<String> {
    let mut files = Vec::new();
    for e in catalog::EMOTES {
        files.push(format!("emotes/{}.webp", e.slug));
        files.push(format!("emotes/{}.png", e.slug));
        if e.has_sound {
            files.push(format!("emotes/{}.ogg", e.slug));
        }
    }
    files
}

fn is_known(reference: &str, custom: &[CustomEmote]) -> bool {
    is_bundled(reference) || (is_custom_ref(reference) && custom.iter().any(|c| c.id == reference))
}

pub fn resolve_emote_art(
    reference: &str,
    custom: &[CustomEmote],
    remote: bool,
    bundled_dir: &Path,
    custom_dir: &Path,
) -> EmoteArt {
    if is_bundled(reference) {
        return EmoteArt::Bundled(bundled_dir.join(emote_anim_file(reference)).to_string_lossy().into_owned());
    }
    let emote = if is_custom_ref(reference) {
        custom.iter().find(|c| c.id == reference)
    } else {
        None
    };
    let Some(emote) = emote else {
        return EmoteArt::Placeholder;
    };
    let hash = custom_hash(reference);
    if emote.animated && !remote {
        if let Some(path) = custom_anim_file(custom_dir, hash) {
            return EmoteArt::Custom { path, animated: true };
        }
    }
    let path = custom_dir.join(custom_still_file(hash)).to_string_lossy().into_owned();
    EmoteArt::Custom { path, animated: false }
}

fn default_wheel() -> Vec<EmoteRef> {
    DEFAULT_EMOTE_WHEEL.iter().map(|s| s.to_string()).collect()
}

pub fn normalize_emote_wheel(raw: &serde_json::Value, custom: &[CustomEmote]) -> Vec<EmoteRef> {
    let slots = DEFAULT_EMOTE_WHEEL.len();
    let serde_json::Value::Array(items) = raw else {
        return default_wheel();
    };
    let refs: Option<Vec<EmoteRef>> = items.iter().map(|v| v.as_str().map(str::to_string)).collect();
    let Some(mut wheel) = refs else {
        return default_wheel();
    };
    if wheel.len() != slots || !wheel.iter().all(|r| is_emote_ref_shape(r)) {
        return default_wheel();
    }
    let mut seen = HashSet::new();
    if !wheel.iter().all(|r| seen.insert(r.clone())) {
        return default_wheel();
    }
    let mut used: HashSet<EmoteRef> = wheel.iter().filter(|r| is_known(r, custom)).cloned().collect();
    for i in 0..slots {
        if is_known(&wheel[i], custom) {
            continue;
        }
        let pick = if used.contains(DEFAULT_EMOTE_WHEEL[i]) {
            catalog::EMOTES
                .iter()
                .find(|e| !used.contains(e.slug))
                .map(|e| e.slug.to_string())
                .unwrap_or_else(|| DEFAULT_EMOTE_WHEEL[i].to_string())
        } else {
            DEFAULT_EMOTE_WHEEL[i].to_string()
        };
        used.insert(pick.clone());
        wheel[i] = pick;
    }
    wheel
}

pub fn normalize_click_emote(raw: &serde_json::Value, custom: &[CustomEmote]) -> EmoteRef {
    match raw.as_str() {
        Some(s) if is_emote_ref_shape(s) && is_known(s, custom) => s.to_string(),
        _ => DEFAULT_CLICK_EMOTE.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(id: &str) -> CustomEmote {
        CustomEmote { id: id.to_string(), name: "x".to_string(), animated: false }
    }

    #[test]
    fn slug_and_custom_refs_are_shaped() {
        assert!(is_slug("facepalm"));
        assert!(is_slug("a"));
        assert!(is_slug(&"a".repeat(40)));
        assert!(!is_slug(""));
        assert!(!is_slug("Facepalm"));
        assert!(!is_slug("a-b"));
        assert!(!is_slug(&"a".repeat(41)));

        assert!(is_custom_ref(&format!("c:{}", "0".repeat(32))));
        assert!(is_custom_ref(&format!("c:{}", "abcdef0123456789abcdef0123456789")));
        assert!(!is_custom_ref(&format!("c:{}", "0".repeat(31))));
        assert!(!is_custom_ref(&format!("c:{}", "A".repeat(32))));
        assert!(!is_custom_ref(&format!("c:{}", "g".repeat(32))));
        assert!(!is_custom_ref("facepalm"));

        assert!(is_emote_ref_shape("facepalm"));
        assert!(is_emote_ref_shape(&format!("c:{}", "0".repeat(32))));
        assert!(!is_emote_ref_shape("nope!"));
        assert!(!is_emote_ref_shape(""));
    }

    #[test]
    fn hash_and_bundled_lookup() {
        assert_eq!(custom_hash(&format!("c:{}", "a1".repeat(16))), "a1".repeat(16));
        assert!(is_bundled("facepalm"));
        assert!(is_bundled("such_foolishness"));
        assert!(!is_bundled("not_in_catalog"));
        assert!(!is_bundled(&format!("c:{}", "0".repeat(32))));
    }

    #[test]
    fn catalog_is_complete() {
        assert_eq!(catalog::EMOTES.len(), 44);
        assert_eq!(catalog::DEFAULT_EMOTE_WHEEL.len(), 8);
        assert_eq!(catalog::DEFAULT_CLICK_EMOTE, "facepalm");
        for slug in catalog::DEFAULT_EMOTE_WHEEL {
            assert!(catalog::by_slug(slug).is_some(), "{slug}");
        }
    }

    #[test]
    fn names_use_the_language() {
        assert_eq!(emote_name("facepalm", crate::i18n::LANG_EN, &[]), "Facepalm");
        assert_eq!(emote_name("facepalm", LANG_ZH_CN, &[]), "捂脸");
        assert_eq!(emote_name("c:00", crate::i18n::LANG_EN, &[custom("c:00")]), "x");
        assert_eq!(emote_name("c:00", crate::i18n::LANG_EN, &[]), "?");
    }

    #[test]
    fn art_resolves_bundled_custom_and_placeholder() {
        let id = format!("c:{}", "aa".repeat(16));
        let animated = CustomEmote { id: id.clone(), name: "a".to_string(), animated: true };
        let bundled = Path::new("C:/app/assets/emotes");
        let custom = Path::new("target/test-emotes");
        let expect = |dir: &Path, name: &str| dir.join(name).to_string_lossy().into_owned();
        assert_eq!(
            resolve_emote_art("facepalm", &[], false, bundled, custom),
            EmoteArt::Bundled(expect(bundled, "facepalm.webp"))
        );
        assert_eq!(
            resolve_emote_art(&format!("c:{}", "bb".repeat(16)), &[], false, bundled, custom),
            EmoteArt::Placeholder
        );
        // An imported animation is stored as `<hash>.anim.<ext>`, and a missing one leaves its still.
        let anim = expect(custom, &format!("{}.anim.webp", "aa".repeat(16)));
        std::fs::create_dir_all(custom).expect("the test directory");
        std::fs::write(&anim, b"RIFF").expect("a placeholder animation");
        assert_eq!(
            resolve_emote_art(&id, std::slice::from_ref(&animated), false, bundled, custom),
            EmoteArt::Custom { path: anim.clone(), animated: true }
        );
        std::fs::remove_file(&anim).expect("removing the placeholder animation");
        assert_eq!(
            resolve_emote_art(&id, std::slice::from_ref(&animated), false, bundled, custom),
            EmoteArt::Custom { path: expect(custom, &format!("{}.webp", "aa".repeat(16))), animated: false }
        );
        assert_eq!(
            resolve_emote_art(&id, &[animated], true, bundled, custom),
            EmoteArt::Custom { path: expect(custom, &format!("{}.webp", "aa".repeat(16))), animated: false }
        );
    }

    #[test]
    fn wheel_falls_back_and_fills_dead_slots() {
        let v = |s: &str| serde_json::json!(s);
        assert_eq!(normalize_emote_wheel(&serde_json::Value::Null, &[]), default_wheel());
        assert_eq!(normalize_emote_wheel(&serde_json::json!([1, 2, 3]), &[]), default_wheel());
        assert_eq!(
            normalize_emote_wheel(&serde_json::json!(["facepalm", "facepalm", "a", "b", "c", "d", "e", "f"]), &[]),
            default_wheel()
        );

        let mut raw: Vec<serde_json::Value> = DEFAULT_EMOTE_WHEEL.iter().map(|s| v(s)).collect();
        raw[0] = v("gone_emote");
        let wheel = normalize_emote_wheel(&serde_json::json!(raw), &[]);
        assert_eq!(wheel[0], DEFAULT_EMOTE_WHEEL[0]);

        let mut raw: Vec<serde_json::Value> = DEFAULT_EMOTE_WHEEL.iter().map(|s| v(s)).collect();
        raw[0] = v("unworthy");
        let wheel = normalize_emote_wheel(&serde_json::json!(raw), &[]);
        assert_eq!(wheel[0], "unworthy");
        assert_eq!(wheel[1..].iter().map(String::as_str).collect::<Vec<_>>(), DEFAULT_EMOTE_WHEEL[1..].to_vec());
    }

    #[test]
    fn click_emote_falls_back() {
        let ref32 = format!("c:{}", "ab".repeat(16));
        assert_eq!(normalize_click_emote(&serde_json::Value::Null, &[]), "facepalm");
        assert_eq!(normalize_click_emote(&serde_json::json!("nice"), &[]), "nice");
        assert_eq!(normalize_click_emote(&serde_json::json!("gone"), &[]), "facepalm");
        assert_eq!(normalize_click_emote(&serde_json::json!(7), &[]), "facepalm");
        assert_eq!(normalize_click_emote(&serde_json::json!(ref32), &[custom(&ref32)]), ref32);
        assert_eq!(normalize_click_emote(&serde_json::json!(ref32), &[]), "facepalm");
    }

    #[test]
    fn asset_files_match_the_catalog() {
        let files = all_emote_asset_files();
        assert_eq!(files.len(), 44 * 3);
        assert!(files.contains(&"emotes/facepalm.webp".to_string()));
        assert!(files.contains(&"emotes/facepalm.ogg".to_string()));
        assert_eq!(emote_anim_file("nice"), "nice.webp");
    }
}
