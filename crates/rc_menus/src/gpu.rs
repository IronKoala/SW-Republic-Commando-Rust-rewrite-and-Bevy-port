//! Pictures made ready for a GPU that only knows ordinary alpha blending
//! (Bevy's UI). The original draws each picture with one of several render
//! styles; `render.rs` does them all in software. Here each style becomes a
//! copy of the texture whose alpha gives the same result:
//!
//! - Normal: alpha set to fully opaque.
//! - Masked: alpha cut to fully on or fully off.
//! - Translucent and Additive (adding the picture onto what is behind):
//!   alpha becomes the brightest channel and the colour is divided by it,
//!   so `colour * alpha + behind * (1 - alpha)` puts the full colour on top
//!   of a black background, the way adding does. Over brighter backgrounds
//!   it is a little darker than true adding.
//! - Alpha (and anything else): unchanged.
//!
//! The original multiplied colours and blended in sRGB numbers (no
//! linear light); Bevy tints and blends in linear light, which makes tinted
//! text darker and see-through pictures brighter. So the tint is baked into
//! each copy (one copy per tint colour), and alpha is adjusted so that the
//! GPU's linear-light result over black equals the original's sRGB result.
//! Over brighter backgrounds the result is close but not exact.

use crate::layout::{STY_ADDITIVE, STY_MASKED, STY_NORMAL, STY_TRANSLUCENT};

/// Which copy of a texture a render style needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Variant {
    AsIs,
    Opaque,
    Masked,
    Added,
}

pub fn variant(render_style: u8) -> Variant {
    match render_style {
        STY_NORMAL => Variant::Opaque,
        STY_MASKED => Variant::Masked,
        STY_TRANSLUCENT | STY_ADDITIVE => Variant::Added,
        _ => Variant::AsIs,
    }
}

fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Change RGBA pixels (sRGB colour, straight alpha) in place for `v`,
/// with the widget's `tint` (red green blue alpha, 0..255) baked in. Draw
/// the result with a white tint.
pub fn prepare(pixels: &mut [u8], v: Variant, tint: [u8; 4]) {
    let t = tint.map(|c| c as f32 / 255.0);
    for p in pixels.chunks_exact_mut(4) {
        for k in 0..3 {
            p[k] = (p[k] as f32 * t[k]).round() as u8;
        }
        match v {
            Variant::AsIs => p[3] = (p[3] as f32 * t[3]).round() as u8,
            // As in `render.rs`: Normal ignores alpha, Masked only cuts.
            Variant::Opaque => p[3] = 255,
            Variant::Masked => p[3] = if p[3] as f32 * t[3] >= 128.0 { 255 } else { 0 },
            Variant::Added => {
                let lin = [0, 1, 2].map(|k| to_linear(p[k] as f32 / 255.0));
                let a = lin[0].max(lin[1]).max(lin[2]);
                if a <= 0.0 {
                    p.copy_from_slice(&[0, 0, 0, 0]);
                    continue;
                }
                for k in 0..3 {
                    p[k] = (to_srgb(lin[k] / a) * 255.0).round().clamp(0.0, 255.0) as u8;
                }
                p[3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
                continue;
            }
        }
        see_through_in_srgb(p);
    }
}

/// For a partly see-through pixel, pick the alpha that gives the original's
/// sRGB result over black (`colour * alpha`) when the GPU blends in linear
/// light (`linear(colour) * alpha'`). Exact for the brightest channel.
fn see_through_in_srgb(p: &mut [u8]) {
    if p[3] == 0 || p[3] == 255 {
        return;
    }
    let c = p[0].max(p[1]).max(p[2]) as f32 / 255.0;
    if c <= 0.0 {
        return;
    }
    let a = p[3] as f32 / 255.0;
    let wanted = to_linear(c * a) / to_linear(c);
    p[3] = (wanted * 255.0).round().clamp(1.0, 255.0) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn added_over_black_gives_the_colour() {
        let mut px = [100u8, 50, 200, 17];
        prepare(&mut px, Variant::Added, [255; 4]);
        // Blend over black in linear light, then back to sRGB.
        let a = px[3] as f32 / 255.0;
        for (k, want) in [100.0f32, 50.0, 200.0].iter().enumerate() {
            let got = to_srgb(to_linear(px[k] as f32 / 255.0) * a) * 255.0;
            assert!((got - want).abs() < 2.5, "channel {k}: {got} vs {want}");
        }
    }

    #[test]
    fn black_added_is_invisible() {
        let mut px = [0u8, 0, 0, 255];
        prepare(&mut px, Variant::Added, [255; 4]);
        assert_eq!(px[3], 0);
    }

    #[test]
    fn half_see_through_matches_srgb_over_black() {
        let mut px = [200u8, 200, 200, 128];
        prepare(&mut px, Variant::AsIs, [128, 128, 128, 255]);
        // Original: 200 * 128/255 = 100, then half of it over black = 50.
        let got = to_srgb(to_linear(px[0] as f32 / 255.0) * px[3] as f32 / 255.0) * 255.0;
        assert!((got - 50.0).abs() < 2.5, "{got}");
    }
}
