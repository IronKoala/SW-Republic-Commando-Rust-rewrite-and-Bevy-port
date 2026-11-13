//! A small software drawer for menu quads. It paints a menu straight from
//! the original's decoded textures into a picture, with no GPU. The game's
//! screenshots are compared against it to check that the colours on screen
//! are the original's (see `rc_game --script`).

use crate::layout::{STY_ADDITIVE, STY_MASKED, STY_NORMAL, STY_TRANSLUCENT};
use crate::quads::Quad;
use rc_image::Rgba;

/// Something that hands out decoded textures by path.
pub trait Textures {
    fn get(&mut self, path: &str) -> Option<&Rgba>;
}

/// Paint `quads` onto a `width` x `height` picture that starts as `clear`.
pub fn render(
    quads: &[Quad],
    textures: &mut dyn Textures,
    width: usize,
    height: usize,
    clear: [u8; 4],
) -> Rgba {
    render_over(
        quads,
        textures,
        Rgba {
            width,
            height,
            pixels: clear.repeat(width * height),
        },
    )
}

/// [`render`] onto a picture already there (such as the menu's
/// background video frame).
pub fn render_over(quads: &[Quad], textures: &mut dyn Textures, base: Rgba) -> Rgba {
    let mut out = base;
    for q in quads {
        let Some(tex) = textures.get(&q.texture) else {
            continue;
        };
        draw_quad(&mut out, q, tex);
    }
    out
}

/// Bilinear sample at texel coordinates (centre of texel 0 is 0.5).
fn sample(tex: &Rgba, u: f32, v: f32) -> [f32; 4] {
    let x = (u - 0.5).clamp(0.0, (tex.width - 1) as f32);
    let y = (v - 0.5).clamp(0.0, (tex.height - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(tex.width - 1), (y0 + 1).min(tex.height - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let px = |x: usize, y: usize| {
        let i = (y * tex.width + x) * 4;
        [
            tex.pixels[i] as f32,
            tex.pixels[i + 1] as f32,
            tex.pixels[i + 2] as f32,
            tex.pixels[i + 3] as f32,
        ]
    };
    let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
    let mut o = [0.0; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bottom = c[k] + (d[k] - c[k]) * fx;
        o[k] = top + (bottom - top) * fy;
    }
    o
}

fn draw_quad(out: &mut Rgba, q: &Quad, tex: &Rgba) {
    let (w, h) = (out.width as f32, out.height as f32);
    // A turned quad: its middle, and how far round (clockwise on the
    // screen, which runs downward).
    let (cx, cy) = ((q.dst[0] + q.dst[2]) / 2.0 * w, (q.dst[1] + q.dst[3]) / 2.0 * h);
    let (sin, cos) = (q.turn * std::f32::consts::TAU).sin_cos();
    let reach = if q.turn != 0.0 {
        let (hw, hh) = ((q.dst[2] - q.dst[0]).abs() * w / 2.0, (q.dst[3] - q.dst[1]).abs() * h / 2.0);
        (hw * hw + hh * hh).sqrt()
    } else {
        0.0
    };
    let bounds = if q.turn != 0.0 {
        [cx - reach, cy - reach, cx + reach, cy + reach]
    } else {
        [q.dst[0] * w, q.dst[1] * h, q.dst[2] * w, q.dst[3] * h]
    };
    let x0 = bounds[0].round().max(0.0) as usize;
    let x1 = (bounds[2].round().max(0.0) as usize).min(out.width);
    let y0 = bounds[1].round().max(0.0) as usize;
    let y1 = (bounds[3].round().max(0.0) as usize).min(out.height);
    let tint = q.color.map(|c| c as f32 / 255.0);
    for py in y0..y1 {
        for px in x0..x1 {
            // Back to where this pixel sat before the turn.
            let (mut sx, mut sy) = (px as f32 + 0.5, py as f32 + 0.5);
            if q.turn != 0.0 {
                let (dx, dy) = (sx - cx, sy - cy);
                sx = cx + dx * cos + dy * sin;
                sy = cy - dx * sin + dy * cos;
            }
            let fx = (sx / w - q.dst[0]) / (q.dst[2] - q.dst[0]);
            let fy = (sy / h - q.dst[1]) / (q.dst[3] - q.dst[1]);
            if !(0.0..1.0).contains(&fx) || !(0.0..1.0).contains(&fy) {
                continue;
            }
            let v = q.src[1] + (q.src[3] - q.src[1]) * fy;
            let u = q.src[0] + (q.src[2] - q.src[0]) * fx;
            let s = sample(tex, u, v);
            let src = [
                s[0] * tint[0],
                s[1] * tint[1],
                s[2] * tint[2],
                s[3] / 255.0 * tint[3],
            ];
            let i = (py * out.width + px) * 4;
            let dst = &mut out.pixels[i..i + 4];
            let d = [dst[0] as f32, dst[1] as f32, dst[2] as f32];
            let blended = match q.render_style {
                STY_NORMAL => [src[0], src[1], src[2]],
                STY_MASKED if src[3] < 0.5 => d,
                STY_MASKED => [src[0], src[1], src[2]],
                STY_TRANSLUCENT | STY_ADDITIVE => [d[0] + src[0], d[1] + src[1], d[2] + src[2]],
                _ => [
                    d[0] + (src[0] - d[0]) * src[3],
                    d[1] + (src[1] - d[1]) * src[3],
                    d[2] + (src[2] - d[2]) * src[3],
                ],
            };
            for k in 0..3 {
                dst[k] = blended[k].round().clamp(0.0, 255.0) as u8;
            }
            dst[3] = 255;
        }
    }
}
