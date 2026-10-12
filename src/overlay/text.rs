use std::path::Path;

use ab_glyph::{Font, FontArc, Glyph, PxScale, ScaleFont};
use tiny_skia::{Color, Pixmap};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Weight {
    Regular,
    Semibold,
    Bold,
}

impl Weight {
    fn index(self) -> usize {
        match self {
            Weight::Regular => 0,
            Weight::Semibold => 1,
            Weight::Bold => 2,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Segoe UI has no Chinese, so each weight keeps a CJK face to fall back on, the way a browser would.
struct Face {
    main: FontArc,
    cjk: Option<FontArc>,
}

pub struct Text {
    faces: [Face; 3],
}

/// Windows faces with Chinese, in the order the settings window also prefers.
const CJK_FACES: [(&str, u32); 4] = [("Deng.ttf", 0), ("msyh.ttc", 0), ("simhei.ttf", 0), ("simsun.ttc", 0)];

fn load(dir: &Path, file: &str) -> FontArc {
    let path = dir.join(file);
    let bytes = std::fs::read(&path).unwrap_or_default();
    FontArc::try_from_vec(bytes).unwrap_or_else(|_| FontArc::try_from_vec(Vec::new()).expect("empty font"))
}

fn load_cjk(dir: &Path) -> Option<FontArc> {
    CJK_FACES.iter().find_map(|(file, index)| {
        let bytes = std::fs::read(dir.join(file)).ok()?;
        let font = ab_glyph::FontVec::try_from_vec_and_index(bytes, *index).ok()?;
        Some(FontArc::new(font))
    })
}

/// The face that can draw this character, and its glyph.
fn pick(face: &Face, ch: char) -> (&FontArc, ab_glyph::GlyphId) {
    let id = face.main.glyph_id(ch);
    if id.0 != 0 {
        return (&face.main, id);
    }
    let Some(cjk) = &face.cjk else {
        return (&face.main, id);
    };
    let id = cjk.glyph_id(ch);
    if id.0 != 0 {
        (cjk, id)
    } else {
        (&face.main, id)
    }
}

impl Text {
    pub fn load(fonts_dir: &Path) -> Self {
        let cjk = load_cjk(fonts_dir);
        Self {
            faces: [
                Face { main: load(fonts_dir, "segoeui.ttf"), cjk: cjk.clone() },
                Face { main: load(fonts_dir, "seguisb.ttf"), cjk: cjk.clone() },
                Face { main: load(fonts_dir, "segoeuib.ttf"), cjk },
            ],
        }
    }

    fn face(&self, weight: Weight) -> &Face {
        &self.faces[weight.index()]
    }

    pub fn width(&self, text: &str, size: f32, weight: Weight, letter_spacing: f32) -> f32 {
        let face = self.face(weight);
        let scale = PxScale::from(size);
        let mut width = 0.0;
        let mut previous: Option<(&FontArc, ab_glyph::GlyphId)> = None;
        for ch in text.chars() {
            let (font, id) = pick(face, ch);
            if let Some((previous_font, previous_id)) = previous {
                if std::ptr::eq(previous_font, font) {
                    width += font.as_scaled(scale).kern(previous_id, id);
                }
            }
            width += font.as_scaled(scale).h_advance(id) + letter_spacing;
            previous = Some((font, id));
        }
        (width - letter_spacing).max(0.0)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        canvas: &mut Pixmap,
        text: &str,
        x: f32,
        baseline_y: f32,
        size: f32,
        weight: Weight,
        color: Color,
        align: Align,
        letter_spacing: f32,
    ) {
        let face = self.face(weight);
        let scale = PxScale::from(size);
        let start_x = match align {
            Align::Left => x,
            Align::Center => x - self.width(text, size, weight, letter_spacing) / 2.0,
            Align::Right => x - self.width(text, size, weight, letter_spacing),
        };
        let mut cursor = start_x;
        let mut previous: Option<(&FontArc, ab_glyph::GlyphId)> = None;
        for ch in text.chars() {
            let (font, id) = pick(face, ch);
            if let Some((previous_font, previous_id)) = previous {
                if std::ptr::eq(previous_font, font) {
                    cursor += font.as_scaled(scale).kern(previous_id, id);
                }
            }
            let glyph: Glyph = id.with_scale_and_position(scale, ab_glyph::point(cursor, baseline_y));
            if let Some(outlined) = font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|gx, gy, coverage| {
                    let px = bounds.min.x as i32 + gx as i32;
                    let py = bounds.min.y as i32 + gy as i32;
                    blend(canvas, px, py, color, coverage);
                });
            }
            cursor += font.as_scaled(scale).h_advance(id) + letter_spacing;
            previous = Some((font, id));
        }
    }
}

fn blend(canvas: &mut Pixmap, x: i32, y: i32, color: Color, coverage: f32) {
    let width = canvas.width() as i32;
    let height = canvas.height() as i32;
    if x < 0 || y < 0 || x >= width || y >= height {
        return;
    }
    let c = color.to_color_u8();
    let alpha = (u32::from(c.alpha()) * (coverage.clamp(0.0, 1.0) * 255.0).round() as u32) / 255;
    if alpha == 0 {
        return;
    }
    let index = (y as usize * width as usize + x as usize) * 4;
    let inv = 255 - alpha;
    let src = [
        u32::from(c.red()) * alpha / 255,
        u32::from(c.green()) * alpha / 255,
        u32::from(c.blue()) * alpha / 255,
        alpha,
    ];
    let data = canvas.data_mut();
    for channel in 0..4 {
        data[index + channel] = (src[channel] + u32::from(data[index + channel]) * inv / 255).min(255) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_comes_from_a_fallback_face() {
        let text = Text::load(&crate::paths::system_fonts());
        let face = text.face(Weight::Semibold);
        let Some(_) = &face.cjk else {
            eprintln!("no Chinese face in the system fonts: nothing to check");
            return;
        };
        let (latin, _) = pick(face, 'A');
        let (chinese, id) = pick(face, '危');
        assert!(!std::ptr::eq(latin, chinese), "Chinese must come from the fallback face");
        assert_ne!(id.0, 0, "the fallback face has no glyph for 危");

        let mut canvas = Pixmap::new(80, 30).expect("canvas");
        text.draw(&mut canvas, "危险", 2.0, 22.0, 16.0, Weight::Semibold, Color::BLACK, Align::Left, 0.0);
        let inked = canvas.data().as_chunks::<4>().0.iter().filter(|px| px[3] > 0).count();
        assert!(inked > 100, "Chinese text left almost no ink: {inked} pixels");
    }
}
