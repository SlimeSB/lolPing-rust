#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageType {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl ImageType {
    pub fn ext(self) -> &'static str {
        match self {
            ImageType::Png => "png",
            ImageType::Jpeg => "jpeg",
            ImageType::Gif => "gif",
            ImageType::Webp => "webp",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

fn starts_with(b: &[u8], at: usize, sig: &[u8]) -> bool {
    b.len() >= at + sig.len() && &b[at..at + sig.len()] == sig
}

fn byte(b: &[u8], at: usize) -> u32 {
    b.get(at).copied().unwrap_or(0) as u32
}

fn u16(b: &[u8], at: usize) -> u32 {
    byte(b, at) | (byte(b, at + 1) << 8)
}

fn u24(b: &[u8], at: usize) -> u32 {
    byte(b, at) | (byte(b, at + 1) << 8) | (byte(b, at + 2) << 16)
}

fn be16(b: &[u8], at: usize) -> u32 {
    (byte(b, at) << 8) | byte(b, at + 1)
}

fn be32(b: &[u8], at: usize) -> u32 {
    (byte(b, at) << 24) | (byte(b, at + 1) << 16) | (byte(b, at + 2) << 8) | byte(b, at + 3)
}

fn sized(width: u32, height: u32) -> Option<Size> {
    (width > 0 && height > 0).then_some(Size { width, height })
}

pub fn sniff_image(b: &[u8]) -> Option<ImageType> {
    if starts_with(b, 0, &PNG_SIGNATURE) {
        return Some(ImageType::Png);
    }
    if starts_with(b, 0, &[0xff, 0xd8, 0xff]) {
        return Some(ImageType::Jpeg);
    }
    if starts_with(b, 0, b"GIF87a") || starts_with(b, 0, b"GIF89a") {
        return Some(ImageType::Gif);
    }
    if starts_with(b, 0, b"RIFF") && starts_with(b, 8, b"WEBP") {
        return Some(ImageType::Webp);
    }
    None
}

pub fn webp_size(b: &[u8]) -> Option<Size> {
    if sniff_image(b) != Some(ImageType::Webp) {
        return None;
    }
    if starts_with(b, 12, b"VP8 ") {
        // Frame tag (3 bytes), start code, then 14-bit width and height; the top two bits are a scale hint.
        if b.len() < 30 || !starts_with(b, 23, &[0x9d, 0x01, 0x2a]) {
            return None;
        }
        return sized(u16(b, 26) & 0x3fff, u16(b, 28) & 0x3fff);
    }
    if starts_with(b, 12, b"VP8L") {
        // Signature byte, then width - 1 and height - 1 as 14 bits each.
        if b.len() < 25 || b[20] != 0x2f {
            return None;
        }
        let bits = u32::from_le_bytes([b[21], b[22], b[23], b[24]]);
        return Some(Size { width: (bits & 0x3fff) + 1, height: ((bits >> 14) & 0x3fff) + 1 });
    }
    if starts_with(b, 12, b"VP8X") {
        // Flags and reserved bytes, then the canvas width - 1 and height - 1 as 24 bits each.
        if b.len() < 30 {
            return None;
        }
        return Some(Size { width: u24(b, 24) + 1, height: u24(b, 27) + 1 });
    }
    None
}

pub fn gif_size(b: &[u8]) -> Option<Size> {
    if sniff_image(b) != Some(ImageType::Gif) || b.len() < 10 {
        return None;
    }
    sized(u16(b, 6), u16(b, 8))
}

fn skip_sub_blocks(b: &[u8], mut at: usize) -> usize {
    while at < b.len() {
        let length = b[at] as usize;
        at += 1 + length;
        if length == 0 {
            break;
        }
    }
    at
}

pub fn gif_extent(b: &[u8]) -> Option<Size> {
    if sniff_image(b) != Some(ImageType::Gif) || b.len() < 10 {
        return None;
    }
    let mut width = u16(b, 6);
    let mut height = u16(b, 8);
    let mut at = 13; // after the header and the logical screen descriptor
    if b.len() > 10 && (b[10] & 0x80) != 0 {
        at += 3 << ((b[10] & 7) + 1); // global colour table
    }
    while at < b.len() {
        let block = b[at];
        if block == 0x21 {
            at = skip_sub_blocks(b, at + 2); // an extension: its label, then data sub-blocks
        } else if block == 0x2c {
            if at + 10 > b.len() {
                break;
            }
            width = width.max(u16(b, at + 1) + u16(b, at + 5));
            height = height.max(u16(b, at + 3) + u16(b, at + 7));
            let packed = b[at + 9];
            at += 10;
            if (packed & 0x80) != 0 {
                at += 3 << ((packed & 7) + 1); // local colour table
            }
            at = skip_sub_blocks(b, at + 1); // the LZW code size, then the compressed data
        } else {
            break; // the trailer, or something that isn't a block
        }
    }
    sized(width, height)
}

pub fn is_animated_webp(b: &[u8]) -> bool {
    // The flags byte follows the VP8X chunk header: bits 5 to 2 are ICC, alpha, EXIF and XMP, bit 1 is animation.
    sniff_image(b) == Some(ImageType::Webp) && starts_with(b, 12, b"VP8X") && b.len() > 20 && (b[20] & 0x02) != 0
}

fn png_size(b: &[u8]) -> Option<Size> {
    if b.len() < 24 || !starts_with(b, 12, b"IHDR") {
        return None;
    }
    sized(be32(b, 16), be32(b, 20))
}

fn jpeg_size(b: &[u8]) -> Option<Size> {
    let mut i = 2; // after the SOI marker
    while i + 1 < b.len() {
        if b[i] != 0xff {
            i += 1; // extraneous bytes between segments, which decoders skip
            continue;
        }
        let marker = b[i + 1];
        if marker == 0xff {
            i += 1; // fill byte
            continue;
        }
        if marker == 0x00 || marker == 0x01 || (0xd0..=0xd9).contains(&marker) {
            i += 2; // a stuffed zero, TEM, RSTn, SOI or EOI: none of them has a length
            continue;
        }
        if marker == 0xda {
            return None; // the scan starts: no frame header came first
        }
        if i + 4 > b.len() {
            return None;
        }
        let length = be16(b, i + 2) as usize;
        if length < 2 {
            return None;
        }
        // SOF0 to SOF15 except DHT (C4), JPG (C8) and DAC (CC): precision, height, width, then components.
        if (0xc0..=0xcf).contains(&marker) && marker != 0xc4 && marker != 0xc8 && marker != 0xcc {
            return if length >= 8 && i + 9 <= b.len() { sized(be16(b, i + 7), be16(b, i + 5)) } else { None };
        }
        i += 2 + length;
    }
    None
}

pub fn image_size(b: &[u8]) -> Option<Size> {
    match sniff_image(b)? {
        ImageType::Png => png_size(b),
        ImageType::Jpeg => jpeg_size(b),
        ImageType::Gif => gif_extent(b),
        ImageType::Webp => webp_size(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn riff(chunk: &[u8], body: &[u8]) -> Vec<u8> {
        let mut b = b"RIFF".to_vec();
        b.extend_from_slice(&((4 + 8 + body.len()) as u32).to_le_bytes());
        b.extend_from_slice(b"WEBP");
        b.extend_from_slice(chunk);
        b.extend_from_slice(&(body.len() as u32).to_le_bytes());
        b.extend_from_slice(body);
        b
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut b = PNG_SIGNATURE.to_vec();
        b.extend_from_slice(&13u32.to_be_bytes());
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&width.to_be_bytes());
        b.extend_from_slice(&height.to_be_bytes());
        b
    }

    fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut b = vec![0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08];
        b.extend_from_slice(&height.to_be_bytes());
        b.extend_from_slice(&width.to_be_bytes());
        b.extend_from_slice(&[0x03, 0x01, 0x11, 0x00]);
        b
    }

    fn gif(width: u16, height: u16) -> Vec<u8> {
        let mut b = b"GIF89a".to_vec();
        b.extend_from_slice(&width.to_le_bytes());
        b.extend_from_slice(&height.to_le_bytes());
        b.extend_from_slice(&[0x00, 0x00, 0x00, 0x3b]);
        b
    }

    fn webp_vp8(width: u16, height: u16) -> Vec<u8> {
        let mut body = vec![0x00, 0x00, 0x00, 0x9d, 0x01, 0x2a];
        body.extend_from_slice(&width.to_le_bytes());
        body.extend_from_slice(&height.to_le_bytes());
        riff(b"VP8 ", &body)
    }

    fn webp_vp8l(width: u32, height: u32) -> Vec<u8> {
        let bits = (width - 1) | ((height - 1) << 14);
        let mut body = vec![0x2f];
        body.extend_from_slice(&bits.to_le_bytes());
        riff(b"VP8L", &body)
    }

    fn webp_vp8x(width: u32, height: u32, flags: u8) -> Vec<u8> {
        let mut body = vec![flags, 0, 0, 0];
        body.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
        body.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
        riff(b"VP8X", &body)
    }

    #[test]
    fn sniff_names_by_magic_bytes() {
        assert_eq!(sniff_image(&png(4, 4)), Some(ImageType::Png));
        assert_eq!(sniff_image(&jpeg(4, 4)), Some(ImageType::Jpeg));
        assert_eq!(sniff_image(&gif(4, 4)), Some(ImageType::Gif));
        assert_eq!(sniff_image(&webp_vp8(4, 4)), Some(ImageType::Webp));
        assert_eq!(sniff_image(b"not an image"), None);
        assert_eq!(sniff_image(b""), None);
    }

    #[test]
    fn sizes_come_from_the_header() {
        assert_eq!(image_size(&png(1920, 1080)), Some(Size { width: 1920, height: 1080 }));
        assert_eq!(image_size(&jpeg(48, 32)), Some(Size { width: 48, height: 32 }));
        assert_eq!(image_size(&gif(100, 50)), Some(Size { width: 100, height: 50 }));
        assert_eq!(image_size(&webp_vp8(64, 48)), Some(Size { width: 64, height: 48 }));
        assert_eq!(image_size(&webp_vp8l(160, 90)), Some(Size { width: 160, height: 90 }));
        assert_eq!(image_size(&webp_vp8x(320, 240, 0x00)), Some(Size { width: 320, height: 240 }));
    }

    #[test]
    fn animated_webp_is_flagged() {
        assert!(is_animated_webp(&webp_vp8x(160, 160, 0x02)));
        assert!(!is_animated_webp(&webp_vp8x(160, 160, 0x10)));
        assert!(!is_animated_webp(&webp_vp8l(160, 160)));
    }

    #[test]
    fn truncated_headers_are_unreadable() {
        assert_eq!(sniff_image(&PNG_SIGNATURE), Some(ImageType::Png));
        assert_eq!(image_size(&PNG_SIGNATURE), None);
        assert_eq!(image_size(&png(0, 0)), None);
        let mut cut = webp_vp8x(160, 160, 0x02);
        cut.truncate(20);
        assert_eq!(sniff_image(&cut), Some(ImageType::Webp));
        assert_eq!(image_size(&cut), None);
        assert!(!is_animated_webp(&cut));
        let mut cutl = webp_vp8l(160, 160);
        cutl.truncate(22);
        assert_eq!(image_size(&cutl), None);
        assert_eq!(image_size(b"GIF89a"), None);
        assert_eq!(image_size(&[0xff, 0xd8, 0xff, 0xc0]), None);
    }

    #[test]
    fn gif_extent_grows_to_a_larger_frame() {
        let mut b = gif(100, 50);
        b.truncate(13);
        b.push(0x2c); // an image descriptor reaching past the logical screen
        b.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x90, 0x00, 0x40, 0x00, 0x00]);
        b.push(0x02); // the LZW minimum code size
        b.push(0x00); // the end of the LZW data
        b.push(0x3b);
        assert_eq!(gif_extent(&b), Some(Size { width: 144, height: 64 }));
    }
}
