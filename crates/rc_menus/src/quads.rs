//! Works out where every picture and letter of a menu goes, as a list of
//! textured rectangles ("quads"). Both the game (with Bevy) and the
//! software check in `render.rs` draw this same list, so a screenshot of
//! the game can be compared with a picture made straight from the
//! original's textures.
//!
//! Units: the menu area is 0..1 across and 0..1 down. The original lays
//! menus out for a 640x480 screen: a picture drawn at scale 1 covers as
//! many of those 640x480 pixels as it has texels, and fonts work the same
//! way. We checked that base size against the main menu: each planet
//! (64 or 128 texels) sits inside its ring, which is sized as a fraction of
//! the screen; at 640x480 every planet fits its ring with a small margin,
//! at 1024x768 the Kamino ring would be twice the planet's size.
//!
//! How each `EMenuScaleMode` sizes a picture is native code in the
//! original, so it is our reading of the enum's own comments:
//! - Scale: texels times the scale.
//! - Fit: the scale is the size as a fraction of the screen.
//! - Stretch / FitStretch: the same sizes, but the picture is cut into a
//!   3x3 grid; the corners keep their size and the middle row and column
//!   stretch ("split texture and fill in gaps with stretched blocks of the
//!   middle texels").

use crate::layout::{
    pivot_fraction, Kind, MenuLayout, Sprite, Text, MSCM_FIT, MSCM_FIT_STRETCH, MSCM_STRETCH,
};
use crate::library::FontData;

/// The screen size the original's menu numbers are written for.
pub const BASE_WIDTH: f32 = 640.0;
pub const BASE_HEIGHT: f32 = 480.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Quad {
    /// Full object path of the texture.
    pub texture: String,
    /// Part of the texture, in texels: x0, y0, x1, y1.
    pub src: [f32; 4],
    /// Where on the menu area: x0, y0, x1, y1 in 0..1.
    pub dst: [f32; 4],
    /// Tint, red green blue alpha, 0..255.
    pub color: [u8; 4],
    pub render_style: u8,
    /// Which widget it belongs to (for logs and checks).
    pub widget: String,
}

/// What the quad builder needs to know about textures and fonts.
pub trait Sizes {
    /// Width and height in texels.
    fn texture_size(&mut self, path: &str) -> Option<(f32, f32)>;
    fn font(&mut self, path: &str) -> Option<FontData>;
}

/// All quads for a menu, in drawing order. `focused` is the index (into
/// `layout.widgets`) of the button that has focus.
pub fn menu_quads(layout: &MenuLayout, focused: Option<usize>, sizes: &mut dyn Sizes) -> Vec<Quad> {
    let mut out = Vec::new();
    for (i, w) in layout.widgets.iter().enumerate() {
        if !w.shown_on_pc() {
            continue;
        }
        match &w.kind {
            Kind::Sprite(s) => sprite_quads(s, &w.name, sizes, &mut out),
            Kind::Text(t) => text_quads(t, &w.name, sizes, &mut out),
            Kind::Button(b) => {
                let (under, bg, t) = if focused == Some(i) {
                    (&b.under_focused, &b.background_focused, &b.focused)
                } else {
                    (&b.under_blurred, &b.background_blurred, &b.blurred)
                };
                sprite_quads(under, &w.name, sizes, &mut out);
                sprite_quads(bg, &w.name, sizes, &mut out);
                text_quads(t, &w.name, sizes, &mut out);
            }
        }
    }
    out
}

/// The box a sprite covers, without drawing it (used for mouse areas).
pub fn sprite_box(s: &Sprite, sizes: &mut dyn Sizes) -> Option<[f32; 4]> {
    let path = s.texture.as_ref()?;
    let (tw, th) = sizes.texture_size(path)?;
    let (cw, ch) = crop_size(s, tw, th);
    let (w, h) = match s.scale_mode {
        MSCM_FIT | MSCM_FIT_STRETCH => (s.scale[0], s.scale[1]),
        _ => (cw * s.scale[0] / BASE_WIDTH, ch * s.scale[1] / BASE_HEIGHT),
    };
    let (px, py) = pivot_fraction(s.pivot);
    let x0 = s.pos[0] - w * px;
    let y0 = s.pos[1] - h * py;
    Some([x0, y0, x0 + w, y0 + h])
}

fn crop(s: &Sprite, tw: f32, th: f32) -> [f32; 4] {
    if s.coords == [0, 0, 0, 0] {
        [0.0, 0.0, tw, th]
    } else {
        s.coords.map(|c| c as f32)
    }
}

fn crop_size(s: &Sprite, tw: f32, th: f32) -> (f32, f32) {
    let c = crop(s, tw, th);
    ((c[2] - c[0]).abs(), (c[3] - c[1]).abs())
}

fn sprite_quads(s: &Sprite, widget: &str, sizes: &mut dyn Sizes, out: &mut Vec<Quad>) {
    let Some(path) = s.texture.clone() else {
        return;
    };
    let Some((tw, th)) = sizes.texture_size(&path) else {
        return;
    };
    let Some(dst) = sprite_box(s, sizes) else {
        return;
    };
    let src = crop(s, tw, th);
    let quad = |src: [f32; 4], dst: [f32; 4]| Quad {
        texture: path.clone(),
        src,
        dst,
        color: s.color,
        render_style: s.render_style,
        widget: widget.to_string(),
    };
    if s.scale_mode != MSCM_STRETCH && s.scale_mode != MSCM_FIT_STRETCH {
        out.push(quad(src, dst));
        return;
    }
    // Nine pieces. Corners are half the picture, at the original's pixel
    // size, shrunk when the box is too small to hold two of them.
    let (cw, ch) = ((src[2] - src[0]) / 2.0, (src[3] - src[1]) / 2.0);
    let corner_w = (cw / BASE_WIDTH).min((dst[2] - dst[0]) / 2.0);
    let corner_h = (ch / BASE_HEIGHT).min((dst[3] - dst[1]) / 2.0);
    // The middle strip is the two centre texels (whole texels: the GPU
    // path drops pieces cut through the middle of a texel).
    let sx = [src[0], src[0] + cw - 1.0, src[0] + cw + 1.0, src[2]];
    let sy = [src[1], src[1] + ch - 1.0, src[1] + ch + 1.0, src[3]];
    let dx = [dst[0], dst[0] + corner_w, dst[2] - corner_w, dst[2]];
    let dy = [dst[1], dst[1] + corner_h, dst[3] - corner_h, dst[3]];
    for row in 0..3 {
        for col in 0..3 {
            if dx[col + 1] - dx[col] <= 0.0 || dy[row + 1] - dy[row] <= 0.0 {
                continue;
            }
            out.push(quad(
                [sx[col], sy[row], sx[col + 1], sy[row + 1]],
                [dx[col], dy[row], dx[col + 1], dy[row + 1]],
            ));
        }
    }
}

/// The width of a line of text in base pixels, before scaling.
fn line_width(font: &FontData, text: &str, kerning: i32) -> f32 {
    text.chars().map(|c| advance(font, c, kerning)).sum()
}

fn advance(font: &FontData, c: char, kerning: i32) -> f32 {
    match font.glyph(c) {
        Some(g) => (g.width + font.kerning + kerning) as f32,
        // Characters the font lacks take no space.
        None => 0.0,
    }
}

/// The lines actually shown. Without word wrap, one line cut down with
/// "..." when wider than `MaxSizeX` (the original's `bEllipsisOnLeft` is
/// not handled). With word wrap, the text breaks at spaces into lines that
/// fit; a single word wider than the limit keeps its own line.
fn lines(t: &Text, font: &FontData) -> Vec<String> {
    if t.max_size_x <= 0.0 || t.squeeze {
        return vec![t.text.clone()];
    }
    let max = t.max_size_x * BASE_WIDTH / t.scale[0];
    let fits = |s: &str| line_width(font, s, t.kerning) <= max;
    if t.word_wrap {
        let mut out: Vec<String> = Vec::new();
        let mut line = String::new();
        for word in t.text.split(' ') {
            let longer = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if fits(&longer) || line.is_empty() {
                line = longer;
            } else {
                out.push(std::mem::replace(&mut line, word.to_string()));
            }
        }
        out.push(line);
        return out;
    }
    if fits(&t.text) {
        return vec![t.text.clone()];
    }
    let mut s: String = t.text.clone();
    while !s.is_empty() && !fits(&format!("{s}...")) {
        s.pop();
    }
    vec![format!("{s}...")]
}

/// Size of a block of lines in menu units: widest line, and total height.
fn block_size(t: &Text, font: &FontData, lines: &[String]) -> (f32, f32) {
    let widest = lines
        .iter()
        .map(|l| line_width(font, l, t.kerning))
        .fold(0.0, f32::max);
    (
        widest * t.scale[0] / BASE_WIDTH,
        lines.len() as f32 * font.line_height as f32 * t.scale[1] / BASE_HEIGHT,
    )
}

/// A squeezed text (`Text::squeeze`) narrowed just enough to fit its
/// `max_size_x`; other texts unchanged.
fn squeezed<'a>(t: &'a Text, font: &FontData) -> std::borrow::Cow<'a, Text> {
    if !t.squeeze || t.max_size_x <= 0.0 {
        return std::borrow::Cow::Borrowed(t);
    }
    let wide = line_width(font, &t.text, t.kerning) * t.scale[0] / BASE_WIDTH;
    if wide <= t.max_size_x {
        return std::borrow::Cow::Borrowed(t);
    }
    let mut n = t.clone();
    n.scale[0] *= t.max_size_x / wide;
    std::borrow::Cow::Owned(n)
}

fn text_quads(t: &Text, widget: &str, sizes: &mut dyn Sizes, out: &mut Vec<Quad>) {
    let Some(font) = t.font.as_deref().and_then(|f| sizes.font(f)) else {
        return;
    };
    let t = &*squeezed(t, &font);
    let lines = lines(t, &font);
    if lines.iter().all(|l| l.is_empty()) {
        return;
    }
    // The pivot places the whole block; each line lines up the same way
    // inside it (left, centred or right).
    let (bw, bh) = block_size(t, &font, &lines);
    let (px, py) = pivot_fraction(t.pivot);
    let x0 = t.pos[0] - bw * px;
    let mut y = t.pos[1] - bh * py;
    let line_h = font.line_height as f32 * t.scale[1] / BASE_HEIGHT;
    for text in &lines {
        let w = line_width(&font, text, t.kerning) * t.scale[0] / BASE_WIDTH;
        let mut x = x0 + (bw - w) * px;
        for c in text.chars() {
            let Some(g) = font.glyph(c) else {
                continue;
            };
            let Some(page) = font.pages.get(g.page as usize) else {
                continue;
            };
            let gw = g.width as f32 * t.scale[0] / BASE_WIDTH;
            let gh = g.height as f32 * t.scale[1] / BASE_HEIGHT;
            out.push(Quad {
                texture: page.clone(),
                src: [
                    g.u as f32,
                    g.v as f32,
                    (g.u + g.width) as f32,
                    (g.v + g.height) as f32,
                ],
                dst: [x, y, x + gw, y + gh],
                color: t.color,
                render_style: t.render_style,
                widget: widget.to_string(),
            });
            x += advance(&font, c, t.kerning) * t.scale[0] / BASE_WIDTH;
        }
        y += line_h;
    }
}

/// The box a text covers (used for logs and checks).
pub fn text_box(t: &Text, sizes: &mut dyn Sizes) -> Option<[f32; 4]> {
    let font = sizes.font(t.font.as_deref()?)?;
    let t = &*squeezed(t, &font);
    let lines = lines(t, &font);
    let (w, h) = block_size(t, &font, &lines);
    let (px, py) = pivot_fraction(t.pivot);
    let x0 = t.pos[0] - w * px;
    let y0 = t.pos[1] - h * py;
    Some([x0, y0, x0 + w, y0 + h])
}

/// Texts on a page that run into each other or into a picture button (the
/// arrows beside a chooser): every pair of shown texts (labels, and button
/// texts), or text and picture button, from different widgets whose boxes
/// cross,
/// with how much they cross as a fraction of the menu width. The menu is
/// always drawn in a 4:3 area scaled to the window, so what overlaps here
/// overlaps at every window size and shape, and what doesn't, doesn't.
pub fn text_overlaps(layout: &MenuLayout, sizes: &mut dyn Sizes) -> Vec<(String, String, f32)> {
    let mut boxes: Vec<(String, [f32; 4])> = Vec::new();
    let mut pictures: Vec<(String, [f32; 4])> = Vec::new();
    for w in &layout.widgets {
        if !w.shown_on_pc() {
            continue;
        }
        if let Kind::Button(b) = &w.kind {
            if b.blurred.text.is_empty() && b.background_blurred.color[3] > 0 {
                if let Some(bx) = sprite_box(&b.background_blurred, sizes) {
                    pictures.push((w.name.clone(), bx));
                }
            }
        }
        let t = match &w.kind {
            Kind::Text(t) => t,
            Kind::Button(b) => &b.blurred,
            Kind::Sprite(_) => continue,
        };
        if t.text.trim().is_empty() || t.color[3] == 0 {
            continue;
        }
        if let Some(b) = text_box(t, sizes) {
            boxes.push((w.name.clone(), b));
        }
    }
    let mut out = Vec::new();
    let texts = boxes.len();
    boxes.extend(pictures);
    for (i, (a, ab)) in boxes[..texts].iter().enumerate() {
        for (b, bb) in &boxes[i + 1..] {
            let x = ab[2].min(bb[2]) - ab[0].max(bb[0]);
            let y = ab[3].min(bb[3]) - ab[1].max(bb[1]);
            // Letters' boxes include a little space above and below, so
            // rows that only touch aren't counted.
            if x > 0.002 && y > 0.004 {
                out.push((a.clone(), b.clone(), x));
            }
        }
    }
    out
}
