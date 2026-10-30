//! rc_panini: the optional Panini view for very wide screens.
//!
//! A normal 3D view is "rectilinear": straight lines stay straight, but
//! things near the left and right edges are stretched, and the wider the
//! view the worse it gets. At 32:9 the view is 135 degrees across and a
//! droid at the edge looks twice as wide as in the middle.
//!
//! The Panini view (Sharpless, 2010) first wraps the scene around a
//! cylinder, then looks at the cylinder from a point `d` behind its axis.
//! `d = 0` is the plain view; `d = 1` is full Panini. Upright lines stay
//! straight and upright, and the sides are no longer stretched.
//!
//! How it is done here: the 3D view is drawn as usual but a little wider,
//! then a full-screen pass moves each pixel to its Panini place. The width
//! is chosen so the middle of the screen keeps exactly its normal size, so
//! aiming feels the same; the bend only shows towards the sides, where you
//! now also see a little more. The cost: the middle is softer, because the
//! wider view spends fewer of its pixels on it ([`Fit::middle_detail`]:
//! about 80% at strength 0.25 on 21:9, 65% on 32:9). If the wanted width
//! would pass [`MAX_SOURCE_HALF_ANGLE`], the middle is enlarged a little
//! instead.
//!
//! The maths, for a direction at angle `a` across and height `h` on a
//! cylinder of radius 1 (`h` is up over the distance across the floor):
//!
//! ```text
//! panini x = (d + 1) sin a / (d + cos a)
//! panini y = (d + 1) h     / (d + cos a)
//! ```
//!
//! and back, from a screen position to the direction, with `k = x / (d+1)`:
//! `cos a = (-k² d + sqrt(1 + k² (1 - d²))) / (1 + k²)`, `sin a = k (d + cos a)`,
//! `h = y (d + cos a) / (d + 1)`. [`Fit::source`] does this and the shader
//! (`panini.wgsl`) does the same for every pixel.

use bevy::asset::embedded_asset;
use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
use bevy::math::Vec2;
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::render::RenderApp;
use bevy::shader::ShaderRef;

/// The widest the 3D view itself is drawn, half-angle across, degrees.
/// Wider than this a straight-line view wastes nearly all its pixels on the
/// edges.
pub const MAX_SOURCE_HALF_ANGLE: f32 = 75.0;

/// Put this on a 3D camera to bend its view. Use [`Fit`] to fill it in and
/// to set the camera's field of view to match.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default, Debug, PartialEq)]
#[extract_app(RenderApp)]
pub struct PaniniView {
    pub strength: f32,
    pub x_edge: f32,
    pub src_tan: f32,
    pub aspect: f32,
}

impl FullscreenMaterial for PaniniView {
    fn fragment_shader() -> ShaderRef {
        "embedded://rc_panini/panini.wgsl".into()
    }
}

pub struct PaniniPlugin;

impl Plugin for PaniniPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "panini.wgsl");
        app.add_plugins(FullscreenMaterialPlugin::<PaniniView>::default());
    }
}

/// The numbers for one screen shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// What the camera should draw, up and down, radians (Bevy's fov).
    pub source_vertical_fov: f32,
    /// What the player sees across the screen, degrees.
    pub seen_across_degrees: f32,
    /// How much of the plain view's detail the middle of the screen keeps
    /// (1 is all of it).
    pub middle_detail: f32,
    pub view: PaniniView,
}

impl Fit {
    /// `vertical_fov` is the view's normal up-and-down angle (radians),
    /// `aspect` the screen's width over height, `strength` 0 to 1.
    pub fn new(vertical_fov: f32, aspect: f32, strength: f32) -> Fit {
        let d = strength.clamp(0.0, 1.0);
        // The plain view's tan of half the angle across.
        let tan_h = (vertical_fov / 2.0).tan() * aspect;
        let x_at = |a: f32| (d + 1.0) * a.sin() / (d + a.cos());
        // Find the drawn half-angle whose Panini edge is the plain view's
        // edge: then the middle keeps its size.
        let max = MAX_SOURCE_HALF_ANGLE.to_radians();
        let half = if x_at(max) <= tan_h {
            max
        } else {
            let (mut lo, mut hi) = (0.0f32, max);
            for _ in 0..40 {
                let mid = (lo + hi) / 2.0;
                if x_at(mid) < tan_h {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            (lo + hi) / 2.0
        };
        let src_tan = half.tan();
        Fit {
            source_vertical_fov: 2.0 * (src_tan / aspect).atan(),
            seen_across_degrees: 2.0 * half.to_degrees(),
            middle_detail: (tan_h / src_tan).min(1.0),
            view: PaniniView {
                strength: d,
                x_edge: x_at(half),
                src_tan,
                aspect,
            },
        }
    }

    /// Where a screen position (-1..1 across, up positive) comes from in
    /// the drawn view (same units). The shader does the same.
    pub fn source(&self, s: Vec2) -> Vec2 {
        let v = self.view;
        let d = v.strength;
        let x = s.x * v.x_edge;
        let y = s.y * v.x_edge / v.aspect;
        let k = x / (d + 1.0);
        let kk = k * k;
        let c = (-kk * d + (1.0 + kk * (1.0 - d * d)).sqrt()) / (1.0 + kk);
        let sn = k * (d + c);
        let h = y * (d + c) / (d + 1.0);
        Vec2::new(sn / c, h / c) / Vec2::new(v.src_tan, v.src_tan / v.aspect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 69 degrees up and down: the game's 85 across 4:3.
    const V: f32 = 1.2043;

    #[test]
    fn strength_zero_changes_nothing() {
        let f = Fit::new(V, 32.0 / 9.0, 0.0);
        assert!((f.source_vertical_fov - V).abs() < 1e-4);
        for s in [
            Vec2::new(0.3, 0.2),
            Vec2::new(-1.0, 1.0),
            Vec2::new(0.9, -0.5),
        ] {
            assert!((f.source(s) - s).length() < 1e-4, "{s} -> {}", f.source(s));
        }
    }

    #[test]
    fn middle_keeps_its_size_and_every_pixel_has_a_source() {
        for aspect in [16.0 / 9.0, 21.0 / 9.0, 32.0 / 9.0] {
            for d in [0.25, 0.5, 1.0] {
                let f = Fit::new(V, aspect, d);
                // Near the middle, one step on screen is one step in the
                // plain view: drawn wider by src_tan / tan_h, so the source
                // step is smaller by the same.
                let tan_h = (V / 2.0).tan() * aspect;
                let step = f.source(Vec2::new(0.001, 0.0)).x / 0.001;
                let capped = f.seen_across_degrees >= 2.0 * MAX_SOURCE_HALF_ANGLE - 0.01;
                if !capped {
                    assert!(
                        (step - tan_h / f.view.src_tan).abs() < 1e-3,
                        "aspect {aspect} d {d}: {step}"
                    );
                }
                // The corners and edges land inside the drawn view.
                for s in [
                    Vec2::new(1.0, 1.0),
                    Vec2::new(-1.0, -1.0),
                    Vec2::new(1.0, 0.0),
                ] {
                    let src = f.source(s);
                    assert!(
                        src.x.abs() <= 1.0001 && src.y.abs() <= 1.0001,
                        "{s} -> {src}"
                    );
                }
                println!(
                    "aspect {aspect:.2} strength {d}: drawn {:.1} deg up and down, seen {:.1} across, middle detail {:.0}%{}",
                    f.source_vertical_fov.to_degrees(),
                    f.seen_across_degrees,
                    f.middle_detail * 100.0,
                    if capped { " (widest allowed)" } else { "" }
                );
            }
        }
    }

    #[test]
    fn edges_are_less_stretched() {
        // How much wider a small thing at the screen's edge looks than in
        // the middle: plain view versus Panini.
        let plain = Fit::new(V, 32.0 / 9.0, 0.0);
        let bent = Fit::new(V, 32.0 / 9.0, 0.5);
        let angle_step = |f: &Fit, x: f32| {
            let a = |x: f32| (f.source(Vec2::new(x, 0.0)).x * f.view.src_tan).atan();
            (a(x + 0.001) - a(x)) / 0.001
        };
        // Stretch = screen per angle at the edge over the middle.
        let stretch = |f: &Fit| angle_step(f, 0.0) / angle_step(f, 0.95);
        println!(
            "edge stretch: plain {:.2}, Panini 0.5 {:.2}",
            stretch(&plain),
            stretch(&bent)
        );
        assert!(stretch(&plain) > 4.0);
        assert!(stretch(&bent) < stretch(&plain) * 0.6);
    }
}
