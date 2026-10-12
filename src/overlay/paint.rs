use std::sync::Arc;
use std::time::Instant;

use tiny_skia::{
    Color, FillRule, FilterQuality, GradientStop, Mask, Paint, Path as SkPath, PathBuilder, Pattern, Pixmap, Point,
    RadialGradient, Rect, Shader, SpreadMode, Stroke, Transform,
};

use crate::emotes::{EmoteArt, EMOTE_DURATION_S};
use crate::geometry::WHEEL;
use crate::overlay::assets::Assets;
use crate::overlay::scene::{EmoteInstance, PingInstance, Scene, Tag, Visuals, Wheel, WheelKind};
use crate::overlay::text::{Align, Text, Weight};
use crate::overlay::Paths;
use crate::pings::{ping_by_id, FALLBACK_TEXTURE};

pub struct PaintCtx<'a> {
    pub canvas: &'a mut Pixmap,
    pub assets: &'a mut Assets,
    pub text: &'a Text,
    pub visuals: &'a Visuals,
    pub now: Instant,
    pub scale: f32,
    pub paths: &'a Paths,
}

impl PaintCtx<'_> {
    pub fn tf(&self) -> Transform {
        Transform::from_scale(self.scale, self.scale)
    }
}

pub fn paint(scene: &mut Scene, ctx: &mut PaintCtx) {
    for ping in &scene.pings {
        paint_ping(ping, ctx);
    }
    for emote in &mut scene.emotes {
        paint_emote(emote, ctx);
    }
    if let Some(wheel) = &scene.wheel {
        paint_wheel(wheel, ctx);
    }
    if let Some(toast) = &scene.toast {
        paint_toast(toast, ctx);
    }
}

fn elapsed(start: Instant, now: Instant) -> f32 {
    now.saturating_duration_since(start).as_secs_f32()
}

#[derive(Clone, Copy)]
struct Ease {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
}

impl Ease {
    const LINEAR: Ease = Ease { x1: 0.0, y1: 0.0, x2: 1.0, y2: 1.0 };
    const OUT: Ease = Ease { x1: 0.0, y1: 0.0, x2: 0.58, y2: 1.0 };
    const IN: Ease = Ease { x1: 0.42, y1: 0.0, x2: 1.0, y2: 1.0 };
    const IN_OUT: Ease = Ease { x1: 0.42, y1: 0.0, x2: 0.58, y2: 1.0 };
    const POP: Ease = Ease { x1: 0.2, y1: 1.6, x2: 0.4, y2: 1.0 };
    const SLIDE: Ease = Ease { x1: 0.5, y1: 0.0, x2: 0.9, y2: 0.5 };
    const TOAST: Ease = Ease { x1: 0.1, y1: 0.9, x2: 0.2, y2: 1.0 };

    fn at(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if self.x1 == self.y1 && self.x2 == self.y2 {
            return t;
        }
        let u = solve_bezier(t, self.x1, self.x2);
        bezier(u, self.y1, self.y2)
    }
}

fn bezier(u: f32, p1: f32, p2: f32) -> f32 {
    let v = 1.0 - u;
    3.0 * v * v * u * p1 + 3.0 * v * u * u * p2 + u * u * u
}

fn bezier_slope(u: f32, p1: f32, p2: f32) -> f32 {
    let v = 1.0 - u;
    3.0 * v * v * p1 + 6.0 * v * u * (p2 - p1) + 3.0 * u * u * (1.0 - p2)
}

fn solve_bezier(t: f32, p1: f32, p2: f32) -> f32 {
    let mut u = t;
    for _ in 0..8 {
        let error = bezier(u, p1, p2) - t;
        if error.abs() < 1e-5 {
            return u;
        }
        let slope = bezier_slope(u, p1, p2);
        if slope.abs() < 1e-6 {
            break;
        }
        u = (u - error / slope).clamp(0.0, 1.0);
    }
    let (mut low, mut high) = (0.0f32, 1.0f32);
    let mut u = t;
    for _ in 0..24 {
        let error = bezier(u, p1, p2) - t;
        if error.abs() < 1e-5 {
            break;
        }
        if error < 0.0 {
            low = u;
        } else {
            high = u;
        }
        u = 0.5 * (low + high);
    }
    u
}

// The timing function applies per keyframe segment, as in CSS.
fn keys(t: f32, frames: &[(f32, f32)], ease: Ease) -> f32 {
    let first = frames[0];
    if t <= first.0 {
        return first.1;
    }
    for pair in frames.windows(2) {
        let (p0, v0) = pair[0];
        let (p1, v1) = pair[1];
        if t <= p1 {
            let local = if p1 > p0 { (t - p0) / (p1 - p0) } else { 1.0 };
            return v0 + (v1 - v0) * ease.at(local);
        }
    }
    frames[frames.len() - 1].1
}

fn css_color(css: &str) -> Color {
    let hex = css.trim_start_matches('#');
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0);
    match hex.len() {
        6 => Color::from_rgba8(byte(0), byte(2), byte(4), 255),
        8 => Color::from_rgba8(byte(0), byte(2), byte(4), byte(6)),
        _ => Color::WHITE,
    }
}

fn rgb(value: u32) -> Color {
    Color::from_rgba8((value >> 16) as u8, (value >> 8) as u8, value as u8, 255)
}

fn with_alpha(color: Color, a: f32) -> Color {
    Color::from_rgba(color.red(), color.green(), color.blue(), (color.alpha() * a).clamp(0.0, 1.0))
        .unwrap_or(Color::TRANSPARENT)
}

fn tag_color(index: usize) -> Color {
    crate::room::colors::TAG_COLORS
        .get(index)
        .map(|css| css_color(css))
        .unwrap_or(Color::WHITE)
}

fn polar(r: f32, degrees: f32) -> (f32, f32) {
    let a = degrees.to_radians();
    (r * a.cos(), r * a.sin())
}

fn rect_path(x: f32, y: f32, w: f32, h: f32) -> Option<SkPath> {
    Rect::from_xywh(x, y, w, h).map(PathBuilder::from_rect)
}

fn oval_path(x: f32, y: f32, w: f32, h: f32) -> Option<SkPath> {
    Rect::from_xywh(x, y, w, h).and_then(PathBuilder::from_oval)
}

fn circle_path(cx: f32, cy: f32, r: f32) -> Option<SkPath> {
    PathBuilder::from_circle(cx, cy, r)
}

// A circular arc as one cubic: exact enough for the 45° spans the wheel uses.
fn arc_to(builder: &mut PathBuilder, cx: f32, cy: f32, r: f32, from: f32, to: f32) {
    let span = to - from;
    let k = 4.0 / 3.0 * (span / 4.0).tan();
    let (x0, y0) = polar(r, from.to_degrees());
    let (x1, y1) = polar(r, to.to_degrees());
    let c0 = (cx + x0 - k * y0, cy + y0 + k * x0);
    let c1 = (cx + x1 + k * y1, cy + y1 - k * x1);
    builder.cubic_to(c0.0, c0.1, c1.0, c1.1, cx + x1, cy + y1);
}

fn round_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<SkPath> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let r = r.max(0.0).min(w * 0.5).min(h * 0.5);
    let mut builder = PathBuilder::new();
    builder.move_to(x + r, y);
    builder.line_to(x + w - r, y);
    arc_to(&mut builder, x + w - r, y + r, r, -std::f32::consts::FRAC_PI_2, 0.0);
    builder.line_to(x + w, y + h - r);
    arc_to(&mut builder, x + w - r, y + h - r, r, 0.0, std::f32::consts::FRAC_PI_2);
    builder.line_to(x + r, y + h);
    arc_to(&mut builder, x + r, y + h - r, r, std::f32::consts::FRAC_PI_2, std::f32::consts::PI);
    builder.line_to(x, y + r);
    arc_to(&mut builder, x + r, y + r, r, std::f32::consts::PI, std::f32::consts::PI * 1.5);
    builder.close();
    builder.finish()
}

fn sector(r0: f32, r1: f32, from: f32, to: f32) -> Option<SkPath> {
    let (x0, y0) = polar(r0, from);
    let (x1, y1) = polar(r1, from);
    let (x3, y3) = polar(r0, to);
    let mut builder = PathBuilder::new();
    builder.move_to(x0, y0);
    builder.line_to(x1, y1);
    arc_to(&mut builder, 0.0, 0.0, r1, from.to_radians(), to.to_radians());
    builder.line_to(x3, y3);
    arc_to(&mut builder, 0.0, 0.0, r0, to.to_radians(), from.to_radians());
    builder.close();
    builder.finish()
}

fn radial(cx: f32, cy: f32, rx: f32, ry: f32, stops: Vec<GradientStop>) -> Option<Shader<'static>> {
    if rx <= 0.0 || ry <= 0.0 {
        return None;
    }
    let transform = Transform::from_scale(rx, ry).post_concat(Transform::from_translate(cx, cy));
    RadialGradient::new(Point::from_xy(0.0, 0.0), 0.0, Point::from_xy(0.0, 0.0), 1.0, stops, SpreadMode::Pad, transform)
}

fn fill(canvas: &mut Pixmap, path: &SkPath, shader: Shader, opacity: f32, ts: Transform) {
    if opacity <= 0.0 {
        return;
    }
    let mut paint = Paint { shader, ..Default::default() };
    if opacity < 1.0 {
        paint.shader.apply_opacity(opacity);
    }
    canvas.fill_path(path, &paint, FillRule::Winding, ts, None);
}

fn fill_color(canvas: &mut Pixmap, path: &SkPath, color: Color, opacity: f32, ts: Transform) {
    fill(canvas, path, Shader::SolidColor(with_alpha(color, opacity)), 1.0, ts);
}

fn stroke_shader(canvas: &mut Pixmap, path: &SkPath, shader: Shader, width: f32, opacity: f32, ts: Transform) {
    if opacity <= 0.0 || width <= 0.0 {
        return;
    }
    let mut paint = Paint { shader, ..Default::default() };
    if opacity < 1.0 {
        paint.shader.apply_opacity(opacity);
    }
    canvas.stroke_path(path, &paint, &Stroke { width, ..Default::default() }, ts, None);
}

fn stroke_color(canvas: &mut Pixmap, path: &SkPath, color: Color, width: f32, opacity: f32, ts: Transform) {
    stroke_shader(canvas, path, Shader::SolidColor(with_alpha(color, opacity)), width, 1.0, ts);
}

fn draw_image(
    canvas: &mut Pixmap,
    img: &Pixmap,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    ts: Transform,
    opacity: f32,
    clip: Option<&Mask>,
) {
    if opacity <= 0.0 || w <= 0.0 || h <= 0.0 || img.width() == 0 || img.height() == 0 {
        return;
    }
    let Some(path) = rect_path(x, y, w, h) else { return };
    let transform = Transform::from_scale(w / img.width() as f32, h / img.height() as f32)
        .post_concat(Transform::from_translate(x, y));
    let shader = Pattern::new(img.as_ref(), SpreadMode::Pad, FilterQuality::Bilinear, opacity, transform);
    canvas.fill_path(&path, &Paint { shader, ..Default::default() }, FillRule::Winding, ts, clip);
}

// Only scales and translations reach here, so the mapped box stays axis aligned.
fn map_box(ts: Transform, x: f32, y: f32, w: f32, h: f32) -> (f32, f32, f32, f32) {
    let mut top_left = Point::from_xy(x, y);
    let mut bottom_right = Point::from_xy(x + w, y + h);
    ts.map_point(&mut top_left);
    ts.map_point(&mut bottom_right);
    (top_left.x, top_left.y, bottom_right.x - top_left.x, bottom_right.y - top_left.y)
}

fn box_blur(data: &mut [u8], w: usize, h: usize, radius: usize, passes: usize) {
    if radius == 0 || w == 0 || h == 0 {
        return;
    }
    let window = (2 * radius + 1) as u32;
    let mut scratch = vec![0u8; data.len()];
    for _ in 0..passes {
        for y in 0..h {
            let mut sum = [0u32; 4];
            for x in 0..=radius.min(w - 1) {
                for c in 0..4 {
                    sum[c] += u32::from(data[(y * w + x) * 4 + c]);
                }
            }
            for c in 0..4 {
                sum[c] += u32::from(data[y * w * 4 + c]) * radius as u32;
            }
            for x in 0..w {
                for c in 0..4 {
                    scratch[(y * w + x) * 4 + c] = (sum[c] / window) as u8;
                }
                let leaving = x.saturating_sub(radius);
                let entering = (x + radius + 1).min(w - 1);
                for c in 0..4 {
                    sum[c] -= u32::from(data[(y * w + leaving) * 4 + c]);
                    sum[c] += u32::from(data[(y * w + entering) * 4 + c]);
                }
            }
        }
        for x in 0..w {
            let mut sum = [0u32; 4];
            for y in 0..=radius.min(h - 1) {
                for c in 0..4 {
                    sum[c] += u32::from(scratch[(y * w + x) * 4 + c]);
                }
            }
            for c in 0..4 {
                sum[c] += u32::from(scratch[x * 4 + c]) * radius as u32;
            }
            for y in 0..h {
                for c in 0..4 {
                    data[(y * w + x) * 4 + c] = (sum[c] / window) as u8;
                }
                let leaving = y.saturating_sub(radius);
                let entering = (y + radius + 1).min(h - 1);
                for c in 0..4 {
                    sum[c] -= u32::from(scratch[(leaving * w + x) * 4 + c]);
                    sum[c] += u32::from(scratch[(entering * w + x) * 4 + c]);
                }
            }
        }
    }
}

// The CSS drop-shadow: blur the image's alpha, tint it, draw it under the image.
fn draw_glow(canvas: &mut Pixmap, img: &Pixmap, x: f32, y: f32, w: f32, h: f32, blur: f32, color: Color, opacity: f32) {
    if opacity <= 0.0 || blur < 0.5 || w <= 0.0 || h <= 0.0 || img.width() == 0 || img.height() == 0 {
        return;
    }
    let k = (64.0 / w.max(h)).min(1.0);
    let (sw, sh) = (w * k, h * k);
    let pad = (blur * k).ceil() + 1.0;
    let Some(mut glow) = Pixmap::new((sw + 2.0 * pad).ceil() as u32, (sh + 2.0 * pad).ceil() as u32) else {
        return;
    };
    draw_image(&mut glow, img, pad, pad, sw, sh, Transform::identity(), 1.0, None);
    let (gw, gh) = (glow.width() as usize, glow.height() as usize);
    box_blur(glow.data_mut(), gw, gh, ((blur * k) * 0.5).round().max(1.0) as usize, 3);
    for px in glow.data_mut().as_chunks_mut::<4>().0 {
        let a = f32::from(px[3]) / 255.0 * color.alpha();
        px[0] = (color.red() * a * 255.0).round() as u8;
        px[1] = (color.green() * a * 255.0).round() as u8;
        px[2] = (color.blue() * a * 255.0).round() as u8;
        px[3] = (a * 255.0).round() as u8;
    }
    let side = 1.0 / k;
    draw_image(canvas, &glow, x - pad * side, y - pad * side, (sw + 2.0 * pad) * side, (sh + 2.0 * pad) * side, Transform::identity(), opacity, None);
}

// The CSS filter chain grayscale, sepia, saturate, brighten, folded into one 3x3 matrix.
fn tint_icon(pixmap: &mut Pixmap) {
    const GRAYSCALE: [[f32; 3]; 3] = [[0.2126, 0.7152, 0.0722]; 3];
    const SEPIA: [[f32; 3]; 3] = [[0.393, 0.769, 0.189], [0.349, 0.686, 0.168], [0.272, 0.534, 0.131]];
    const LUMINANCE: [[f32; 3]; 3] = [[0.213, 0.715, 0.072], [0.213, 0.715, 0.072], [0.213, 0.715, 0.072]];
    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let mix = |a: [[f32; 3]; 3], b: [[f32; 3]; 3], amount: f32| {
        let mut out = [[0.0f32; 3]; 3];
        for r in 0..3 {
            for c in 0..3 {
                out[r][c] = a[r][c] * (1.0 - amount) + b[r][c] * amount;
            }
        }
        out
    };
    let multiply = |a: [[f32; 3]; 3], b: [[f32; 3]; 3]| {
        let mut out = [[0.0f32; 3]; 3];
        for r in 0..3 {
            for c in 0..3 {
                out[r][c] = (0..3).map(|i| a[r][i] * b[i][c]).sum();
            }
        }
        out
    };
    let brightness = {
        let mut m = IDENTITY;
        for row in &mut m {
            for value in row.iter_mut() {
                *value *= 1.15;
            }
        }
        m
    };
    let matrix = multiply(multiply(multiply(brightness, mix(IDENTITY, LUMINANCE, 1.3)), mix(IDENTITY, SEPIA, 0.75)), GRAYSCALE);
    for px in pixmap.data_mut().as_chunks_mut::<4>().0 {
        let alpha = f32::from(px[3]) / 255.0;
        if alpha <= 0.0 {
            continue;
        }
        let src = [f32::from(px[0]) / alpha, f32::from(px[1]) / alpha, f32::from(px[2]) / alpha];
        for channel in 0..3 {
            let value: f32 = (0..3).map(|i| matrix[channel][i] * src[i]).sum();
            px[channel] = (value.clamp(0.0, 255.0) * alpha).round() as u8;
        }
    }
}

fn text_at(
    ctx: &mut PaintCtx,
    text: &str,
    x: f32,
    baseline: f32,
    size: f32,
    weight: Weight,
    color: Color,
    align: Align,
    spacing: f32,
    ts: Transform,
) {
    if color.alpha() <= 0.0 || text.is_empty() {
        return;
    }
    let device = ts.get_scale().0;
    let mut point = Point::from_xy(x, baseline);
    ts.map_point(&mut point);
    ctx.text.draw(ctx.canvas, text, point.x, point.y, size * device, weight, color, align, spacing * device);
}

fn paint_ping(ping: &PingInstance, ctx: &mut PaintCtx) {
    let Some(def) = ping_by_id(&ping.ping) else { return };
    let size = ping.size_px;
    let duration = ping.duration_s;
    if size <= 0.0 || duration <= 0.0 {
        return;
    }
    let t = elapsed(ping.start, ctx.now);
    let life = keys(t / duration, &[(0.0, 1.0), (0.87, 1.0), (1.0, 0.0)], Ease::LINEAR);
    if life <= 0.0 {
        return;
    }
    let color = rgb(def.color);
    let base = Transform::from_translate(ping.x, ping.y).post_concat(ctx.tf());

    let ground = keys(t / duration, &[(0.0, 0.0), (0.10, 0.75), (0.85, 0.55), (1.0, 0.0)], Ease::OUT);
    if let (Some(path), Some(shader)) = (
        oval_path(-0.42 * size, 0.2 * size, 0.84 * size, 0.24 * size),
        radial(
            0.0,
            0.32 * size,
            0.42 * size,
            0.12 * size,
            vec![
                GradientStop::new(0.0, with_alpha(color, 1.0)),
                GradientStop::new(1.0, with_alpha(color, 0.0)),
            ],
        ),
    ) {
        fill(ctx.canvas, &path, shader, ground, base);
    }

    for delay in [0.0f32, 0.22, 1.4] {
        let progress = (t - delay) / 0.65;
        if !(0.0..=1.0).contains(&progress) {
            continue;
        }
        let eased = Ease::OUT.at(progress);
        let ring_scale = 0.25 + 1.10 * eased;
        paint_ring(ctx.canvas, base, 0.545 * size * ring_scale, 3.0 * ring_scale, color, 0.95 * (1.0 - eased));
    }

    let (art_x, art_y) = (-0.5 * size, -0.73 * size);
    if def.art.len() == 2 {
        paint_clash_ping(ctx, def.art, size, t, life, color, base);
    } else if let Some(name) = def.art.first() {
        let pop = t / 0.28;
        let pop_scale = keys(pop, &[(0.0, 0.4), (0.6, 1.15), (1.0, 1.0)], Ease::POP);
        let pop_opacity = keys(pop, &[(0.0, 0.0), (0.6, 1.0), (1.0, 1.0)], Ease::POP);
        let bob = if t >= 0.3 { keys(((t - 0.3) % 1.6) / 1.6, &[(0.0, 0.0), (0.5, -5.0), (1.0, 0.0)], Ease::IN_OUT) } else { 0.0 };
        let ts = scale_about(0.0, -0.23 * size, pop_scale).post_concat(Transform::from_translate(0.0, bob)).post_concat(base);
        if let Some(img) = art_texture(ctx, name) {
            let (x, y, w, h) = map_box(ts, art_x, art_y, size, size);
            draw_glow(ctx.canvas, &img, x, y, w, h, 10.0 * (w / size), color, life * pop_opacity);
            draw_image(ctx.canvas, &img, art_x, art_y, size, size, ts, life * pop_opacity, None);
        }
    }

    if let Some(tag) = &ping.tag {
        paint_tag(ctx, tag, 0.0, 0.5 * size, duration, t, base);
    }
}

fn paint_clash_ping(ctx: &mut PaintCtx, art: &[&str], size: f32, t: f32, life: f32, color: Color, base: Transform) {
    let shake_t = ((t - 0.22) / 0.25).clamp(0.0, 1.0);
    let shake_x = keys(shake_t, &[(0.0, 0.0), (0.25, -3.0), (0.5, 3.0), (0.75, -2.0), (1.0, 0.0)], Ease::LINEAR);
    let shake_y = keys(shake_t, &[(0.0, 0.0), (0.25, 2.0), (0.5, -2.0), (0.75, -1.0), (1.0, 0.0)], Ease::LINEAR);
    let bob = if t >= 0.5 { keys(((t - 0.5) % 1.6) / 1.6, &[(0.0, 0.0), (0.5, -5.0), (1.0, 0.0)], Ease::IN_OUT) } else { 0.0 };
    let clash = Transform::from_translate(shake_x, shake_y + bob).post_concat(base);

    let slide = Ease::SLIDE.at((t / 0.22).clamp(0.0, 1.0));
    for (index, offset) in [(0, (0.64, -0.64)), (1, (-0.64, -0.64))] {
        let Some(name) = art.get(index) else { continue };
        let Some(img) = art_texture(ctx, name) else { continue };
        let ts = Transform::from_translate(offset.0 * size * (1.0 - slide), offset.1 * size * (1.0 - slide)).post_concat(clash);
        let (x, y, w, h) = map_box(ts, -0.5 * size, -0.73 * size, size, size);
        draw_glow(ctx.canvas, &img, x, y, w, h, 10.0 * (w / size), color, life * slide);
        draw_image(ctx.canvas, &img, -0.5 * size, -0.73 * size, size, size, ts, life * slide, None);
    }

    let flash = (t - 0.2) / 0.35;
    if !(0.0..=1.0).contains(&flash) {
        return;
    }
    let flash_opacity = keys(flash, &[(0.0, 0.0), (0.3, 1.0), (1.0, 0.0)], Ease::OUT);
    let radius = 0.25 * size * (0.3 + 1.9 * Ease::OUT.at(flash));
    if let (Some(path), Some(shader)) = (
        circle_path(0.0, -0.23 * size, radius),
        radial(
            0.0,
            -0.23 * size,
            radius,
            radius,
            vec![
                GradientStop::new(0.0, Color::WHITE),
                GradientStop::new(0.4, with_alpha(color, 1.0)),
                GradientStop::new(1.0, with_alpha(color, 0.0)),
            ],
        ),
    ) {
        fill(ctx.canvas, &path, shader, life * flash_opacity, base);
    }
}

fn scale_about(cx: f32, cy: f32, k: f32) -> Transform {
    Transform::from_translate(-cx, -cy)
        .post_concat(Transform::from_scale(k, k))
        .post_concat(Transform::from_translate(cx, cy))
}

// The 3 px border plus the 18 px outer and 12 px inset glows of the CSS box-shadow.
fn paint_ring(canvas: &mut Pixmap, base: Transform, r: f32, width: f32, color: Color, opacity: f32) {
    if opacity <= 0.0 || r <= 0.0 {
        return;
    }
    let outer_blur = width * 6.0;
    let inner_blur = width * 4.0;
    let ring_radius = r + (outer_blur - inner_blur) * 0.5;
    let halo_width = inner_blur + outer_blur;
    let reach = ring_radius + halo_width * 0.5;
    let at = |value: f32| (value / reach).clamp(0.0, 1.0);
    if let (Some(path), Some(shader)) = (
        circle_path(0.0, 0.0, ring_radius),
        radial(
            0.0,
            0.0,
            reach,
            reach,
            vec![
                GradientStop::new(at(r - inner_blur), with_alpha(color, 0.0)),
                GradientStop::new(at(r - width * 0.5), with_alpha(color, 0.5)),
                GradientStop::new(at(r + width * 0.5), with_alpha(color, 0.5)),
                GradientStop::new(1.0, with_alpha(color, 0.0)),
            ],
        ),
    ) {
        stroke_shader(canvas, &path, shader, halo_width, opacity, base);
    }
    if let Some(path) = circle_path(0.0, 0.0, r) {
        stroke_color(canvas, &path, color, width, opacity, base);
    }
}

fn art_texture(ctx: &mut PaintCtx, name: &str) -> Option<Arc<Pixmap>> {
    let path = ctx.paths.assets.join("textures").join(format!("{name}.png"));
    if let Some(img) = ctx.assets.image(&path) {
        return Some(img);
    }
    let fallback = ctx.paths.assets.join("textures").join(format!("{FALLBACK_TEXTURE}.png"));
    ctx.assets.image(&fallback)
}

fn paint_tag(ctx: &mut PaintCtx, tag: &Tag, x: f32, top: f32, duration: f32, t: f32, base: Transform) {
    let opacity = keys(t / duration, &[(0.0, 1.0), (0.87, 1.0), (1.0, 0.0)], Ease::LINEAR);
    if opacity <= 0.0 {
        return;
    }
    let size = 13.0;
    let spacing = 0.2;
    let name = fit_tag(ctx.text, &tag.name, 206.0, size, spacing);
    let width = ctx.text.width(&name, size, Weight::Semibold, spacing) + 14.0;
    let height = 20.0;
    let left = x - width * 0.5;
    let top = top + keys(t / 0.25, &[(0.0, 4.0), (1.0, 0.0)], Ease::OUT);
    if let Some(path) = round_rect(left, top, width, height, 4.0) {
        fill_color(ctx.canvas, &path, Color::from_rgba8(8, 12, 16, 158), opacity, base);
    }
    let baseline = top + 10.0 + size * 0.35;
    let color = tag_color(tag.color);
    text_at(ctx, &name, x, baseline + 1.0, size, Weight::Semibold, with_alpha(Color::BLACK, opacity * 0.55), Align::Center, spacing, base);
    text_at(ctx, &name, x, baseline, size, Weight::Semibold, with_alpha(color, opacity), Align::Center, spacing, base);
}

fn fit_tag(text: &Text, name: &str, max: f32, size: f32, spacing: f32) -> String {
    if text.width(name, size, Weight::Semibold, spacing) <= max {
        return name.to_string();
    }
    let mut out = String::new();
    let mut width = 0.0;
    for ch in name.chars() {
        let advance = text.width(&ch.to_string(), size, Weight::Semibold, spacing) + spacing;
        if width + advance > max {
            break;
        }
        width += advance;
        out.push(ch);
    }
    out.push('…');
    out
}

fn paint_emote(emote: &mut EmoteInstance, ctx: &mut PaintCtx) {
    let size = emote.size_px;
    if size <= 0.0 {
        return;
    }
    let t = elapsed(emote.start, ctx.now);
    let base = Transform::from_translate(emote.x, emote.y).post_concat(ctx.tf());
    match &emote.art {
        EmoteArt::Bundled(_) => match emote.player.as_mut() {
            Some(player) => {
                let frame = player.frame(t);
                draw_image(ctx.canvas, frame, -0.625 * size, -0.625 * size, 1.25 * size, 1.25 * size, base, 1.0, None);
            }
            None => paint_envelope(ctx, size, t, base, None),
        },
        EmoteArt::Custom { animated: true, .. } => {
            let frame = emote.player.as_mut().map(|player| player.frame(t));
            paint_envelope(ctx, size, t, base, frame);
        }
        EmoteArt::Custom { path, animated: false } => {
            let image = ctx.assets.image(path);
            paint_envelope(ctx, size, t, base, image.as_deref());
        }
        EmoteArt::Placeholder => paint_envelope(ctx, size, t, base, None),
    }
    if let Some(tag) = &emote.tag {
        paint_tag(ctx, tag, 0.0, 0.5 * size + 8.0, EMOTE_DURATION_S, t, base);
    }
}

fn paint_envelope(ctx: &mut PaintCtx, size: f32, t: f32, base: Transform, image: Option<&Pixmap>) {
    let bob = keys((t % 1.4) / 1.4, &[(0.0, 0.0), (0.25, -4.0), (0.75, 4.0), (1.0, 0.0)], Ease::IN_OUT);
    let out = (t - (EMOTE_DURATION_S - 0.35)) / 0.35;
    let (scale, opacity) = if out <= 0.0 {
        (keys(t / 0.25, &[(0.0, 0.0), (0.6, 1.15), (1.0, 1.0)], Ease::OUT), 1.0)
    } else {
        let eased = Ease::IN.at(out);
        (1.0 - 0.15 * eased, 1.0 - eased)
    };
    if opacity <= 0.0 {
        return;
    }
    let ts = scale_about(0.0, 0.0, scale).post_concat(Transform::from_translate(0.0, bob)).post_concat(base);
    match image {
        Some(img) => {
            let fit = (size / img.width() as f32).min(size / img.height() as f32);
            let (w, h) = (img.width() as f32 * fit, img.height() as f32 * fit);
            let (x, y, dw, dh) = map_box(ts, -w * 0.5, -h * 0.5, w, h);
            draw_glow(ctx.canvas, img, x, y, dw, dh, 8.0 * (dw / w), Color::from_rgba8(255, 255, 255, 136), opacity);
            draw_image(ctx.canvas, img, -w * 0.5, -h * 0.5, w, h, ts, opacity, None);
        }
        None => paint_bubble(ctx, size, ts, opacity),
    }
}

fn paint_bubble(ctx: &mut PaintCtx, size: f32, ts: Transform, opacity: f32) {
    let (x, y, w, h) = (-0.42 * size, -0.42 * size, 0.84 * size, 0.68 * size);
    let halo = 8.0;
    if let (Some(path), Some(shader)) = (
        oval_path(x - halo, y - halo, w + halo * 2.0, h + halo * 2.0),
        radial(
            0.0,
            0.0,
            w * 0.5 + halo,
            h * 0.5 + halo,
            vec![
                GradientStop::new(0.0, with_alpha(Color::WHITE, 0.0)),
                GradientStop::new((w * 0.5) / (w * 0.5 + halo), Color::from_rgba8(255, 255, 255, 136)),
                GradientStop::new(1.0, with_alpha(Color::WHITE, 0.0)),
            ],
        ),
    ) {
        fill(ctx.canvas, &path, shader, opacity, ts);
    }
    let (cream, gold) = (css_color("#f4efe2"), css_color("#c8aa6e"));
    if let Some(path) = oval_path(x, y, w, h) {
        fill_color(ctx.canvas, &path, cream, opacity, ts);
        stroke_color(ctx.canvas, &path, gold, 3.0, opacity, ts);
    }
    text_at(ctx, "?", 0.0, y + h * 0.5 + 0.4 * size * 0.35, 0.4 * size, Weight::Bold, with_alpha(css_color("#1f2a30"), opacity), Align::Center, 0.0, ts);

    let tail = 14.0;
    let left = x + w * 0.24;
    let top = y + h - 7.0;
    let centre = (left + tail * 0.5, top + tail * 0.5);
    let skew = 8.0f32.to_radians().tan();
    let matrix = Transform::from_skew(skew, skew).post_concat(Transform::from_rotate(45.0));
    let tail_ts = Transform::from_translate(-centre.0, -centre.1)
        .post_concat(matrix)
        .post_concat(Transform::from_translate(centre.0, centre.1))
        .post_concat(ts);
    let mut builder = PathBuilder::new();
    builder.move_to(left + tail, top);
    builder.line_to(left + tail, top + tail);
    builder.line_to(left, top + tail);
    if let Some(path) = builder.finish() {
        stroke_color(ctx.canvas, &path, gold, 3.0, opacity, tail_ts);
    }
}

/// How long the wheel takes to fade and scale in or out; the engine keeps repainting for exactly this long.
pub const WHEEL_FADE_S: f32 = 0.12;

fn paint_wheel(wheel: &Wheel, ctx: &mut PaintCtx) {
    let (wheel_scale, opacity) = match wheel.closed {
        Some(closed) => {
            let eased = Ease::OUT.at(elapsed(closed, ctx.now) / WHEEL_FADE_S);
            if eased >= 1.0 {
                return;
            }
            (1.0 - 0.15 * eased, 1.0 - eased)
        }
        None => {
            let eased = Ease::OUT.at(elapsed(wheel.opened, ctx.now) / WHEEL_FADE_S);
            (0.85 + 0.15 * eased, eased)
        }
    };
    if opacity <= 0.0 {
        return;
    }
    let ts = Transform::from_scale(wheel_scale, wheel_scale)
        .post_concat(Transform::from_translate(wheel.x, wheel.y))
        .post_concat(ctx.tf());

    let (inner, outer, ext) = (WHEEL.inner_r, WHEEL.outer_r, WHEEL.ext_r);
    let ext_stops = vec![
        GradientStop::new(outer / ext, with_alpha(css_color("#5c6066"), 0.55)),
        GradientStop::new(1.0, with_alpha(css_color("#5c6066"), 0.0)),
    ];
    for slice in (0..8).step_by(2) {
        let mid = -90.0 + slice as f32 * 45.0;
        if let (Some(path), Some(shader)) = (sector(outer, ext, mid - 22.5, mid + 22.5), radial(0.0, 0.0, ext, ext, ext_stops.clone())) {
            fill(ctx.canvas, &path, shader, opacity, ts);
        }
    }

    if let (Some(path), Some(shader)) = (
        circle_path(0.0, 0.0, (inner + outer) * 0.5),
        radial(
            0.0,
            0.0,
            outer,
            outer,
            vec![
                GradientStop::new(inner / outer, css_color("#2a2d31")),
                GradientStop::new(1.0, css_color("#4a4e54")),
            ],
        ),
    ) {
        stroke_shader(ctx.canvas, &path, shader, outer - inner, 0.92 * opacity, ts);
    }

    if let Some(hot) = wheel.hot.filter(|index| *index < 8) {
        let mid = -90.0 + hot as f32 * 45.0;
        let reach = if hot % 2 == 0 { ext } else { outer };
        if let (Some(path), Some(shader)) = (
            sector(inner, reach, mid - 22.5, mid + 22.5),
            radial(
                0.0,
                0.0,
                ext,
                ext,
                vec![
                    GradientStop::new(inner / ext, css_color("#8a8f96")),
                    GradientStop::new(outer / ext, css_color("#9aa0a8")),
                    GradientStop::new(1.0, with_alpha(css_color("#9aa0a8"), 0.0)),
                ],
            ),
        ) {
            fill(ctx.canvas, &path, shader, 0.55 * opacity, ts);
        }
    }

    for slice in 0..8 {
        let edge = -90.0 + slice as f32 * 45.0 - 22.5;
        let (x0, y0) = polar(inner, edge);
        let (x1, y1) = polar(outer, edge);
        let mut builder = PathBuilder::new();
        builder.move_to(x0, y0);
        builder.line_to(x1, y1);
        if let Some(path) = builder.finish() {
            stroke_color(ctx.canvas, &path, css_color("#a08a5c"), 1.2, 0.45 * opacity, ts);
        }
    }

    let round = wheel.kind == WheelKind::Emote;
    let clip = if round { wheel_clip(wheel, ctx, ts) } else { None };
    for (index, item) in wheel.items.iter().enumerate().take(8) {
        let Some(img) = ctx.assets.image(&item.icon) else { continue };
        let mid = -90.0 + index as f32 * 45.0;
        let (x, y) = polar(WHEEL.icon_r, mid);
        let hot = wheel.hot == Some(index);
        let size = WHEEL.icon_size * if hot { 1.18 } else { 1.0 };
        let (left, top) = (x - size * 0.5, y - size * 0.5);
        if hot || round {
            draw_image(ctx.canvas, &img, left, top, size, size, ts, opacity, clip.as_ref());
        } else {
            let device = ts.get_scale().0;
            let side = (size * device).ceil() as u32;
            let Some(mut icon) = Pixmap::new(side, side) else { continue };
            draw_image(&mut icon, &img, 0.0, 0.0, side as f32, side as f32, Transform::identity(), 1.0, None);
            tint_icon(&mut icon);
            draw_image(ctx.canvas, &icon, left, top, size, size, ts, opacity, None);
        }
    }

    if let Some(path) = circle_path(0.0, 0.0, inner + 4.0) {
        stroke_color(ctx.canvas, &path, css_color("#1a1a1a"), 6.0, 0.6 * opacity, ts);
    }
    if let (Some(path), Some(shader)) = (
        circle_path(0.0, 0.0, inner),
        radial(
            0.0,
            -10.0,
            inner,
            inner,
            vec![GradientStop::new(0.0, css_color("#2c5d60")), GradientStop::new(1.0, css_color("#11292c"))],
        ),
    ) {
        fill(ctx.canvas, &path, shader, opacity, ts);
    }
    if let Some(path) = circle_path(0.0, 0.0, inner) {
        stroke_color(ctx.canvas, &path, css_color("#c8aa6e"), 3.0, opacity, ts);
    }
    if let Some(path) = circle_path(0.0, 0.0, inner - 7.0) {
        stroke_color(ctx.canvas, &path, css_color("#8a7448"), 1.0, 0.6 * opacity, ts);
    }

    if let Some(hub) = ctx.assets.image(&wheel.hub) {
        draw_image(ctx.canvas, &hub, -20.0, -58.0, 40.0, 40.0, ts, opacity, clip.as_ref());
    }

    let label = if round { "EMOTE" } else { "PING" };
    text_at(ctx, label, 0.0, 6.0, 15.0, Weight::Semibold, with_alpha(css_color("#1fa9e0"), opacity), Align::Center, 1.0, ts);
    text_at(ctx, "BACK", 6.0, 50.0, 9.0, Weight::Semibold, with_alpha(css_color("#6c7a7c"), opacity), Align::Center, 0.5, ts);

    if let Some(path) = round_rect(-26.0, 40.0, 11.0, 16.0, 5.5) {
        stroke_color(ctx.canvas, &path, css_color("#6c7a7c"), 1.3, 0.55 * opacity, ts);
    }
    let mut builder = PathBuilder::new();
    builder.move_to(-20.5, 40.0);
    builder.line_to(-20.5, 47.0);
    builder.line_to(-15.0, 47.0);
    builder.line_to(-15.0, 45.5);
    arc_to(&mut builder, -20.5, 45.5, 5.5, 0.0, -std::f32::consts::FRAC_PI_2);
    if let Some(path) = builder.finish() {
        fill_color(ctx.canvas, &path, css_color("#6c7a7c"), 0.55 * opacity, ts);
    }
}

fn wheel_clip(wheel: &Wheel, ctx: &PaintCtx, ts: Transform) -> Option<Mask> {
    let mut mask = Mask::new(ctx.canvas.width(), ctx.canvas.height())?;
    for index in 0..wheel.items.len().min(8) {
        let size = WHEEL.icon_size * if wheel.hot == Some(index) { 1.18 } else { 1.0 };
        let (x, y) = polar(WHEEL.icon_r, -90.0 + index as f32 * 45.0);
        if let Some(path) = circle_path(x, y, size * 0.5) {
            mask.fill_path(&path, FillRule::Winding, true, ts);
        }
    }
    if let Some(path) = circle_path(0.0, -38.0, 20.0) {
        mask.fill_path(&path, FillRule::Winding, true, ts);
    }
    Some(mask)
}

/// The toast's fade in and out; it holds still in between, so the engine stops repainting there.
pub const TOAST_FADE_IN_S: f32 = 0.3;
pub const TOAST_FADE_OUT_S: f32 = 0.2;

fn paint_toast(toast: &crate::overlay::scene::Toast, ctx: &mut PaintCtx) {
    let (offset, opacity) = match toast.leaving {
        Some(left) => {
            let eased = Ease::IN.at(elapsed(left, ctx.now) / TOAST_FADE_OUT_S);
            (12.0 * eased, 1.0 - eased)
        }
        None => {
            let eased = Ease::TOAST.at(elapsed(toast.start, ctx.now) / TOAST_FADE_IN_S);
            (24.0 * (1.0 - eased), eased)
        }
    };
    if opacity <= 0.0 {
        return;
    }
    let title_size = 14.0;
    let body_size = 12.0;
    let text_width = ctx
        .text
        .width(&toast.title, title_size, Weight::Bold, 0.0)
        .max(ctx.text.width(&toast.body, body_size, Weight::Regular, 0.0));
    let width = (16.0 + 28.0 + 12.0 + text_width + 16.0).clamp(260.0, 420.0);
    let height = 64.0;
    let x = ctx.canvas.width() as f32 / ctx.scale - 24.0 - width;
    let y = ctx.canvas.height() as f32 / ctx.scale - 72.0 - height + offset;
    let ts = ctx.tf();

    for (layer, alpha) in [0.14f32, 0.10, 0.07, 0.04].into_iter().enumerate() {
        if let Some(path) = round_rect(x, y + 16.0 + layer as f32 * 5.0, width, height, 8.0) {
            fill_color(ctx.canvas, &path, Color::from_rgba8(0, 0, 0, 255), alpha * opacity, ts);
        }
    }
    if let Some(path) = round_rect(x, y, width, height, 8.0) {
        fill_color(ctx.canvas, &path, Color::from_rgba8(32, 32, 32, 235), opacity, ts);
        stroke_color(ctx.canvas, &path, Color::from_rgba8(255, 255, 255, 20), 1.0, opacity, ts);
    }

    let icon = ctx.paths.assets.join("textures").join(format!("{FALLBACK_TEXTURE}.png"));
    if let Some(img) = ctx.assets.image(&icon) {
        draw_image(ctx.canvas, &img, x + 16.0, y + (height - 28.0) * 0.5, 28.0, 28.0, ts, opacity, None);
    }

    let text_x = x + 16.0 + 28.0 + 12.0;
    let text_top = y + (height - 40.0) * 0.5;
    text_at(ctx, &toast.title, text_x, text_top + 10.0 + title_size * 0.35, title_size, Weight::Bold, with_alpha(Color::WHITE, opacity), Align::Left, 0.0, ts);
    text_at(
        ctx,
        &toast.body,
        text_x,
        text_top + 30.0 + body_size * 0.35,
        body_size,
        Weight::Regular,
        with_alpha(Color::WHITE, opacity * 0.6),
        Align::Left,
        0.0,
        ts,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::assets::AnimPlayer;
    use crate::overlay::scene::WheelItem;
    use std::path::PathBuf;
    use std::time::Duration;

    fn paths() -> Paths {
        Paths {
            assets: PathBuf::from("assets"),
            bundled_emotes: PathBuf::from("assets/emotes"),
            custom_emotes: PathBuf::from("target/test-emotes"),
        }
    }

    fn ctx<'a>(canvas: &'a mut Pixmap, assets: &'a mut Assets, text: &'a Text, visuals: &'a Visuals, now: Instant, paths: &'a Paths) -> PaintCtx<'a> {
        PaintCtx { canvas, assets, text, visuals, now, scale: 1.0, paths }
    }

    fn pixel(canvas: &Pixmap, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let px = &canvas.data()[(y as usize * canvas.width() as usize + x as usize) * 4..][..4];
        let alpha = f32::from(px[3]) / 255.0;
        let un = |v: u8| if alpha > 0.0 { (f32::from(v) / alpha).round().min(255.0) as u8 } else { 0 };
        (un(px[0]), un(px[1]), un(px[2]), px[3])
    }

    fn wheel_items() -> Vec<WheelItem> {
        crate::pings::DEFAULT_WHEEL
            .iter()
            .map(|id| {
                let icon = crate::pings::ping_by_id(id).expect("known ping").icon;
                WheelItem { key: (*id).to_string(), icon: format!("assets/textures/{icon}.png") }
            })
            .collect()
    }

    fn ping_scene(now: Instant) -> Scene {
        let ping = |ping: &str, x: f32, y: f32, age: u64, tag: Option<&str>| PingInstance {
            ping: ping.into(),
            x,
            y,
            start: now - Duration::from_millis(age),
            size_px: 110.0,
            duration_s: 3.2,
            tag: tag.map(|name| Tag { name: name.into(), color: 0 }),
        };
        Scene {
            display_id: 0,
            wheel: Some(Wheel {
                kind: WheelKind::Ping,
                x: 900.0,
                y: 540.0,
                hot: Some(2),
                items: wheel_items(),
                hub: "assets/textures/generic_ping.png".into(),
                opened: now - Duration::from_millis(400),
                closed: None,
            }),
            pings: vec![ping("danger", 400.0, 300.0, 700, Some("Alice")), ping("allin", 1550.0, 300.0, 320, Some("Bob"))],
            emotes: vec![
                EmoteInstance {
                    player: AnimPlayer::for_art(&EmoteArt::Bundled("assets/emotes/facepalm.webp".into())),
                    art: EmoteArt::Bundled("assets/emotes/facepalm.webp".into()),
                    owner: "self".into(),
                    x: 1300.0,
                    y: 800.0,
                    start: now - Duration::from_millis(400),
                    size_px: 150.0,
                    tag: Some(Tag { name: "Carol".into(), color: 2 }),
                },
                EmoteInstance {
                    player: None,
                    art: EmoteArt::Placeholder,
                    owner: "peer".into(),
                    x: 380.0,
                    y: 820.0,
                    start: now - Duration::from_millis(400),
                    size_px: 150.0,
                    tag: None,
                },
            ],
            toast: Some(crate::overlay::scene::Toast {
                title: "Emote copied".into(),
                body: "Paste it into the game chat".into(),
                start: now - Duration::from_millis(500),
                leaving: None,
            }),
        }
    }

    #[test]
    fn a_ping_and_an_open_wheel_leave_a_coloured_frame() {
        let now = Instant::now();
        let mut scene = ping_scene(now);
        let mut canvas = Pixmap::new(1920, 1080).expect("canvas");
        let mut assets = Assets::new();
        let visuals = Visuals::default();
        let paths = paths();
        let text = Text::load();
        paint(&mut scene, &mut ctx(&mut canvas, &mut assets, &text, &visuals, now, &paths));

        let painted = canvas.data().as_chunks::<4>().0.iter().filter(|px| px[3] > 0).count();
        assert!(painted > 0, "the frame is empty");

        let expected = rgb(0xff3b55);
        let mut best = f32::MAX;
        for y in 285..315 {
            for x in 385..415 {
                let (r, g, b, a) = pixel(&canvas, x, y);
                if a == 0 {
                    continue;
                }
                let distance = (f32::from(r) - expected.red() * 255.0).abs()
                    + (f32::from(g) - expected.green() * 255.0).abs()
                    + (f32::from(b) - expected.blue() * 255.0).abs();
                best = best.min(distance);
            }
        }
        assert!(best < 30.0, "no ping colour near the ping: closest channel sum {best}");
    }

    #[test]
    fn emote_wheel_icons_are_clipped_to_a_circle() {
        let now = Instant::now();
        let icon = PathBuf::from("target/test-emotes/bleed.png");
        std::fs::create_dir_all(icon.parent().expect("dir")).expect("test dir");
        image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 0, 0, 255])).save(&icon).expect("icon written");
        let icon = icon.to_string_lossy().into_owned();

        let mut scene = Scene {
            wheel: Some(Wheel {
                kind: WheelKind::Emote,
                x: 900.0,
                y: 540.0,
                hot: None,
                items: vec![WheelItem { key: "bleed".into(), icon: icon.clone() }],
                hub: icon,
                opened: now - Duration::from_secs(1),
                closed: None,
            }),
            ..Default::default()
        };
        let mut canvas = Pixmap::new(1920, 1080).expect("canvas");
        let mut assets = Assets::new();
        let visuals = Visuals::default();
        let paths = paths();
        let text = Text::load();
        paint(&mut scene, &mut ctx(&mut canvas, &mut assets, &text, &visuals, now, &paths));

        let (cx, cy) = (900u32, 540 - 137);
        let corner = pixel(&canvas, cx - 23, cy - 23);
        let middle = pixel(&canvas, cx, cy);
        assert!(corner.0 < 120, "the slice icon's corner should be clipped away, got {corner:?}");
        assert!(middle.3 > 200 && middle.0 > 200, "the slice icon should be painted, got {middle:?}");
    }

    #[test]
    fn an_empty_scene_leaves_an_empty_frame() {
        let now = Instant::now();
        let mut canvas = Pixmap::new(1920, 1080).expect("canvas");
        let mut assets = Assets::new();
        let visuals = Visuals::default();
        let paths = paths();
        let text = Text::load();
        paint(&mut Scene::default(), &mut ctx(&mut canvas, &mut assets, &text, &visuals, now, &paths));
        assert!(canvas.data().as_chunks::<4>().0.iter().all(|px| px[3] == 0), "an empty scene painted pixels");
    }

    #[test]
    fn writes_a_preview_frame() {
        let now = Instant::now();
        let mut scene = ping_scene(now);
        let mut canvas = Pixmap::new(1920, 1080).expect("canvas");
        let mut assets = Assets::new();
        let visuals = Visuals::default();
        let paths = paths();
        let text = Text::load();
        paint(&mut scene, &mut ctx(&mut canvas, &mut assets, &text, &visuals, now, &paths));

        let mut rgba = canvas.data().to_vec();
        for px in rgba.as_chunks_mut::<4>().0 {
            let alpha = f32::from(px[3]) / 255.0;
            if alpha > 0.0 {
                for channel in &mut px[..3] {
                    *channel = (f32::from(*channel) / alpha).round().min(255.0) as u8;
                }
            }
        }
        let image = image::RgbaImage::from_raw(1920, 1080, rgba).expect("frame");
        std::fs::create_dir_all("target").expect("target dir");
        image.save("target/overlay_preview.png").expect("preview written");
    }
}
