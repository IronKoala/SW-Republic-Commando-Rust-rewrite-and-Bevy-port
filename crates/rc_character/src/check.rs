//! The on-screen check: compares the character's outline in a capture of
//! the window with the outline we expect from our own skinned points.
//!
//! Expected outline: every triangle of the posed mesh, projected with the
//! camera and filled in on a CPU-side "mask" (a yes/no grid, one cell per
//! pixel). Actual outline: every captured pixel that isn't the background
//! colour. The score is "intersection over union": the pixels both agree
//! on, divided by the pixels either one marks. 1.0 is a perfect match.

use crate::{Loaded, RunLog, BACKGROUND};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::ScreenshotCaptured;

pub(crate) const CHECK_ATTEMPTS: u32 = 5;
/// Outline match needed to pass.
const PASS_SCORE: f64 = 0.95;
/// A pixel counts as "character" if any channel differs from the
/// background by more than this (0-255).
const BACKGROUND_TOLERANCE: i32 = 12;
/// Largest allowed difference in colour balance (each channel's share of
/// the total, 0 to 1) between the character on screen and its skin picture.
/// A grey skin drawn magenta differs by about 0.3.
const MAX_HUE_DIFFERENCE: f64 = 0.06;

pub(crate) struct Expected {
    /// Each point's position in window pixels (`None` if behind the camera).
    pub(crate) projected: Vec<Option<Vec2>>,
    /// Window size in the same units.
    pub(crate) window: Vec2,
}

#[derive(Resource, Default)]
pub(crate) struct ScreenCheck {
    pub(crate) attempts: u32,
    pub(crate) next_frame: Option<u32>,
    pub(crate) passed: Option<bool>,
    pub(crate) expected: Option<Expected>,
}

/// Fill one triangle into the mask (cells whose centre is inside).
fn fill(mask: &mut [bool], w: usize, h: usize, a: Vec2, b: Vec2, c: Vec2) {
    let lo = a.min(b).min(c).max(Vec2::ZERO);
    let hi = a
        .max(b)
        .max(c)
        .min(Vec2::new(w as f32 - 1.0, h as f32 - 1.0));
    let area = (b - a).perp_dot(c - a);
    if area.abs() < 1e-6 {
        return;
    }
    for y in lo.y.floor() as usize..=hi.y.ceil().max(0.0) as usize {
        for x in lo.x.floor() as usize..=hi.x.ceil().max(0.0) as usize {
            if x >= w || y >= h {
                continue;
            }
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let w0 = (b - p).perp_dot(c - p) / area;
            let w1 = (c - p).perp_dot(a - p) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                mask[y * w + x] = true;
            }
        }
    }
}

pub(crate) fn check_screen(
    captured: On<ScreenshotCaptured>,
    loaded: Res<Loaded>,
    log: Res<RunLog>,
    mut check: ResMut<ScreenCheck>,
) {
    let shot = &captured.image;
    let (w, h) = (shot.width() as usize, shot.height() as usize);
    let format = shot.texture_descriptor.format;
    let Some(data) = shot.data.as_ref() else {
        log.line("on-screen check FAIL: the capture had no pixel data");
        check.passed = Some(false);
        return;
    };
    let bgra = matches!(
        format,
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
    );
    if !bgra
        && !matches!(
            format,
            TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb
        )
    {
        log.line(format!(
            "on-screen check FAIL: capture format {format:?} not handled"
        ));
        check.passed = Some(false);
        return;
    }
    if data.as_chunks::<4>().0.iter().all(|p| p[..3] == [0, 0, 0]) {
        log.line("on-screen check: the capture is completely black (window not drawn yet)");
        if check.attempts < CHECK_ATTEMPTS {
            // Try again a little later (frame counter is bumped by the caller).
            check.next_frame = Some(u32::MAX);
        } else {
            log.line(format!(
                "on-screen check FAIL: still black after {CHECK_ATTEMPTS} captures"
            ));
            check.passed = Some(false);
        }
        return;
    }
    let Some(expected) = check.expected.take() else {
        log.line("on-screen check FAIL: no expected outline was recorded");
        check.passed = Some(false);
        return;
    };
    // The capture is in real screen pixels; with display scaling (common
    // on Windows) that differs from window units.
    let sx = w as f32 / expected.window.x;
    let sy = h as f32 / expected.window.y;
    let c = &loaded.0;
    let mut want = vec![false; w * h];
    let mut skipped = 0usize;
    for t in &c.mesh.triangles {
        let pts = t
            .wedges
            .map(|wi| expected.projected[c.mesh.wedges[wi as usize].point as usize]);
        match pts {
            [Some(a), Some(b), Some(d)] => {
                let s = Vec2::new(sx, sy);
                fill(&mut want, w, h, a * s, b * s, d * s);
            }
            _ => skipped += 1,
        }
    }
    let pixel = |i: usize| -> [u8; 3] {
        let p = &data[i * 4..];
        if bgra {
            [p[2], p[1], p[0]]
        } else {
            [p[0], p[1], p[2]]
        }
    };
    let (mut both, mut either, mut n_want, mut n_got) = (0usize, 0usize, 0usize, 0usize);
    let mut mean = [0f64; 3];
    for (i, &wanted) in want.iter().enumerate() {
        let px = pixel(i);
        let got =
            (0..3).any(|k| (px[k] as i32 - BACKGROUND[k] as i32).abs() > BACKGROUND_TOLERANCE);
        if got {
            n_got += 1;
            for k in 0..3 {
                mean[k] += px[k] as f64;
            }
        }
        n_want += wanted as usize;
        both += (got && wanted) as usize;
        either += (got || wanted) as usize;
    }
    if n_got == 0 {
        // Seen in the cloud test machine: the window was drawn, but the
        // character's mesh hadn't reached the graphics card yet. Same retry
        // as for an all-black capture.
        log.line("on-screen check: only background in the capture (character not drawn yet)");
        if check.attempts < CHECK_ATTEMPTS {
            check.next_frame = Some(u32::MAX);
        } else {
            log.line(format!(
                "on-screen check FAIL: no character after {CHECK_ATTEMPTS} captures"
            ));
            check.passed = Some(false);
        }
        return;
    }
    let score = both as f64 / either.max(1) as f64;
    let mean = mean.map(|m| m / n_got.max(1) as f64);
    log.line(format!(
        "on-screen check: capture {w}x{h} {format:?}; expected character pixels {n_want}, found {n_got}, in both {both}; {skipped} triangles behind the camera"
    ));
    log.line(format!(
        "on-screen check: character pixels' average colour R {:.0} G {:.0} B {:.0}; corner pixel {:?} (background {:?})",
        mean[0],
        mean[1],
        mean[2],
        pixel(2 * w + 2),
        BACKGROUND
    ));
    // Colour: lighting makes the character brighter or darker than its skin
    // picture, but shouldn't change its hue. Compare each channel's share of
    // the total ("chromaticity"). Without a skin the material is plain grey.
    let skin_mean = c.skin.as_ref().map_or([153.0; 3], |(_, img)| {
        let m = img.mean();
        [m[0], m[1], m[2]]
    });
    let share = |v: [f64; 3]| {
        let t = (v[0] + v[1] + v[2]).max(1.0);
        v.map(|x| x / t)
    };
    let (a, b) = (share(mean), share(skin_mean));
    let hue_diff = (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0, f64::max);
    let colour_ok = hue_diff <= MAX_HUE_DIFFERENCE;
    log.line(format!(
        "on-screen check: colour balance differs from the skin's by {hue_diff:.3} ({MAX_HUE_DIFFERENCE} or less passes; skin average R {:.0} G {:.0} B {:.0})",
        skin_mean[0], skin_mean[1], skin_mean[2]
    ));
    let pass = score >= PASS_SCORE && n_want > 1000 && colour_ok;
    check.passed = Some(pass);
    log.line(format!(
        "on-screen check {}: outline match {score:.3} (1.0 is perfect, {PASS_SCORE} or more passes), colour {}",
        if pass { "PASS" } else { "FAIL" },
        if colour_ok { "ok" } else { "WRONG" }
    ));
}
