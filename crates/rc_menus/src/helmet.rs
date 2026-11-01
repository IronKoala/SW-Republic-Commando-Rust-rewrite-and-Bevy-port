//! The commando's helmet: the frame round the edge of the first-person view.
//!
//! The original draws it as a 3D model, not a picture: the HUD class
//! `CTGame.CTHUD` has `StaticMesh = Helmet` (`HUDMeshes.HelmetSurface.Helmet`, in
//! `StaticMeshes/hudmeshes.usx`) and draws it at the eye with its own view
//! angle, `DisplayFOV` (84 degrees across a 4:3 screen). The model has two
//! parts: the metal frame (`HUDTextures.Helmet.HelmetMetal_Shader`, picture
//! `Helmet_HUD`) and a sheet of glass (`HelmetFinalBlend`, whose shader has
//! no picture at all, so there is nothing of it to draw).
//!
//! The original was made for 4:3 screens. On a wider screen we keep the
//! frame's sides their own shape and move them apart to the screen's edges,
//! stretching only the thin bars across the top and bottom (see [`fit`]);
//! on a narrower one they move in. Nothing of the frame is shrunk.
//!
//! This module knows nothing about Bevy: it reads the model, places it for a
//! screen shape and can paint it in software for checks (`rc_probe
//! --helmet`). `rc_game` draws it with the GPU.

use crate::library::Library;
use crate::value::V;
use rc_image::Rgba;
use rc_package::class::object_path;
use rc_package::static_mesh;
use serde::Deserialize;

/// `data/ui/helmet.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelmetPlan {
    /// Draw it at all (the GAME page's HELMET row can still turn it off).
    #[serde(default = "yes")]
    pub enabled: bool,
    /// The HUD class whose `StaticMesh` and `DisplayFOV` we follow.
    #[serde(default = "hud_class")]
    pub hud_class: String,
    /// Where the model is, as `Package.Name`.
    #[serde(default = "mesh")]
    pub mesh: String,
    /// Used if the HUD class can't be read: degrees across a 4:3 screen.
    #[serde(default = "display_fov")]
    pub display_fov: f32,
    /// How bright the frame is, 0 to 1. The original lights it with
    /// `AmbientGlow` 20 and lights inside the helmet, which we don't copy
    /// yet; this is a guess to be matched against the real game.
    #[serde(default = "brightness")]
    pub brightness: f32,
    /// Where the frame's sides begin, as a fraction of the half width of a
    /// 4:3 screen from the middle. Beyond it the model keeps its shape and
    /// moves to the screen edge; inside it, it stretches.
    #[serde(default = "sides_from")]
    pub sides_from: f32,
}

fn yes() -> bool {
    true
}
fn hud_class() -> String {
    "CTGame.CTHUD".into()
}
fn mesh() -> String {
    "HUDMeshes.HelmetSurface.Helmet".into()
}
fn display_fov() -> f32 {
    84.0
}
fn brightness() -> f32 {
    0.5
}
fn sides_from() -> f32 {
    0.5
}

impl Default for HelmetPlan {
    fn default() -> Self {
        toml::from_str("").expect("defaults")
    }
}

impl HelmetPlan {
    pub fn parse(text: &str) -> Result<HelmetPlan, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}

/// One drawable part of the model.
#[derive(Debug, Clone)]
pub struct HelmetPart {
    /// The material's path, for the log.
    pub material: String,
    pub picture: Rgba,
    /// Triangles, three corner numbers each, into the model's points.
    pub indices: Vec<u32>,
}

/// The model, in Unreal units and axes (X forward, Y right, Z up, the eye
/// at the origin).
#[derive(Debug, Clone)]
pub struct HelmetModel {
    pub points: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub parts: Vec<HelmetPart>,
    /// Degrees across a 4:3 screen.
    pub display_fov: f32,
    /// What was read and what was left out, for the log.
    pub notes: Vec<String>,
}

/// Read the model and its pictures from the install.
pub fn load(lib: &mut Library, plan: &HelmetPlan) -> Result<HelmetModel, String> {
    let mut notes = vec![];
    let mut display_fov = plan.display_fov;
    match lib.class(&plan.hud_class) {
        Ok(c) => {
            if let Some(V::Object { path: m, .. }) = c.get("StaticMesh") {
                let wanted = plan.mesh.rsplit('.').next().unwrap_or_default();
                if !m
                    .rsplit('.')
                    .next()
                    .unwrap_or_default()
                    .eq_ignore_ascii_case(wanted)
                {
                    notes.push(format!(
                        "{} names StaticMesh {m}, the plan says {}",
                        plan.hud_class, plan.mesh
                    ));
                }
            }
            match c.get("DisplayFOV") {
                Some(&V::Float(f)) if f > 1.0 && f < 179.0 => display_fov = f,
                _ => notes.push(format!(
                    "{} has no DisplayFOV; using {display_fov}",
                    plan.hud_class
                )),
            }
        }
        Err(e) => notes.push(format!(
            "{}: {e}; using DisplayFOV {display_fov}",
            plan.hud_class
        )),
    }
    let (loaded, i) = lib.find(&plan.mesh)?;
    let (pkg, data) = (&loaded.pkg, &loaded.data);
    let e = &pkg.exports[i];
    if pkg.export_class_name(e) != "StaticMesh" {
        return Err(format!("{} is not a StaticMesh", plan.mesh));
    }
    let m = static_mesh::read_static_mesh(pkg, data, e).map_err(|e| e.to_string())?;
    let uvs = m.uv_sets.first().cloned().unwrap_or_default();
    if uvs.len() != m.vertices.len() {
        return Err(format!("{}: no texture coordinates", plan.mesh));
    }
    let package = plan.mesh.split('.').next().unwrap_or_default();
    let mut parts = vec![];
    for (k, s) in m.sections.iter().enumerate() {
        let reference = m.materials.get(k).copied().unwrap_or(0);
        let material = match rc_package::ObjectRef::from_index(reference) {
            rc_package::ObjectRef::Import(_) => object_path(pkg, reference),
            rc_package::ObjectRef::Export(_) => {
                format!("{package}.{}", object_path(pkg, reference))
            }
            rc_package::ObjectRef::None => {
                notes.push(format!("part {k}: no material, left out"));
                continue;
            }
        };
        let picture = match lib.texture(&material) {
            Ok(p) => p,
            Err(e) => {
                // The glass: its shader names no picture.
                notes.push(format!("part {k} ({material}): nothing to draw ({e})"));
                continue;
            }
        };
        let first = s.first_index as usize;
        let end = (first + s.num_faces as usize * 3).min(m.indices.len());
        let indices: Vec<u32> = m.indices[first..end].iter().map(|&j| j as u32).collect();
        notes.push(format!(
            "part {k} ({material}): {} triangles, picture {}x{}",
            indices.len() / 3,
            picture.width,
            picture.height
        ));
        parts.push(HelmetPart {
            material,
            picture,
            indices,
        });
    }
    if parts.is_empty() {
        return Err(format!("{}: no part could be drawn", plan.mesh));
    }
    Ok(HelmetModel {
        points: m.vertices.iter().map(|v| v.position).collect(),
        normals: m.vertices.iter().map(|v| v.normal).collect(),
        uvs,
        parts,
        display_fov,
        notes,
    })
}

/// Tangent of half the up-and-down view angle for a view angle given in
/// degrees across a 4:3 screen (the same for every screen shape).
pub fn half_height(display_fov: f32) -> f32 {
    (display_fov.to_radians() / 2.0).tan() / (4.0 / 3.0)
}

/// The model's points moved for a screen `aspect` (width / height).
///
/// Seen from the eye, a point's place across the screen is `y / x`. The
/// screen's half width there is `half_height * aspect`, which is wider than
/// the 4:3 the model was made for by `extra`. Points beyond `sides_from` of
/// the 4:3 half width (the frame's sides) move out by `extra`, keeping their
/// shape; points nearer the middle (the bars across the top and bottom)
/// spread out evenly so nothing tears. Up and down is never changed.
pub fn fit(model: &HelmetModel, plan: &HelmetPlan, aspect: f32) -> Vec<[f32; 3]> {
    let t = half_height(model.display_fov);
    let edge_4_3 = t * 4.0 / 3.0;
    let extra = t * aspect - edge_4_3;
    let u0 = (plan.sides_from * edge_4_3).max(1e-3);
    // Keep the middle from folding over on very narrow screens.
    let stretch = ((u0 + extra) / u0).max(0.05);
    model
        .points
        .iter()
        .map(|&[x, y, z]| {
            if x <= 1e-3 {
                return [x, y, z];
            }
            let u = y / x;
            let moved = if u.abs() <= u0 {
                u * stretch
            } else {
                u + u.signum() * (u0 * stretch - u0)
            };
            [x, moved * x, z]
        })
        .collect()
}

/// Paint the fitted helmet (`points` from [`fit`]) on a `width` x `height`
/// picture, see-through where there is no helmet. Flat lit at the plan's
/// brightness, nearest texel, the nearest triangle wins. For checks only.
pub fn paint(
    model: &HelmetModel,
    points: &[[f32; 3]],
    brightness: f32,
    width: usize,
    height: usize,
) -> Rgba {
    let mut out = Rgba {
        width,
        height,
        pixels: vec![0; width * height * 4],
    };
    let mut depth = vec![f32::INFINITY; width * height];
    let t = half_height(model.display_fov);
    let aspect = width as f32 / height as f32;
    let (w, h) = (width as f32, height as f32);
    let screen = |p: [f32; 3]| -> Option<[f32; 3]> {
        let [x, y, z] = p;
        if x <= 1e-3 {
            return None;
        }
        let sx = (y / x) / (t * aspect);
        let sy = (z / x) / t;
        Some([(sx + 1.0) * 0.5 * w, (1.0 - sy) * 0.5 * h, x])
    };
    let b = brightness.clamp(0.0, 1.0);
    for part in &model.parts {
        let pic = &part.picture;
        for tri in part.indices.chunks_exact(3) {
            let idx = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
            let (Some(a), Some(c1), Some(c2)) = (
                screen(points[idx[0]]),
                screen(points[idx[1]]),
                screen(points[idx[2]]),
            ) else {
                continue;
            };
            let v = [a, c1, c2];
            let area = (v[1][0] - v[0][0]) * (v[2][1] - v[0][1])
                - (v[2][0] - v[0][0]) * (v[1][1] - v[0][1]);
            if area.abs() < 1e-9 {
                continue;
            }
            let min_x = v
                .iter()
                .map(|p| p[0])
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(0.0) as usize;
            let max_x = v
                .iter()
                .map(|p| p[0])
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil()
                .min(w - 1.0);
            let min_y = v
                .iter()
                .map(|p| p[1])
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(0.0) as usize;
            let max_y = v
                .iter()
                .map(|p| p[1])
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil()
                .min(h - 1.0);
            if max_x < 0.0 || max_y < 0.0 {
                continue;
            }
            for py in min_y..=max_y as usize {
                for px in min_x..=max_x as usize {
                    let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                    let w0 =
                        ((v[1][0] - fx) * (v[2][1] - fy) - (v[2][0] - fx) * (v[1][1] - fy)) / area;
                    let w1 =
                        ((v[2][0] - fx) * (v[0][1] - fy) - (v[0][0] - fx) * (v[2][1] - fy)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    // Perspective-correct texture place.
                    let iz = [1.0 / v[0][2], 1.0 / v[1][2], 1.0 / v[2][2]];
                    let one = w0 * iz[0] + w1 * iz[1] + w2 * iz[2];
                    let d = 1.0 / one;
                    let k = py * width + px;
                    if d >= depth[k] {
                        continue;
                    }
                    let uv = |j: usize| model.uvs[idx[j]];
                    let tu =
                        (w0 * iz[0] * uv(0)[0] + w1 * iz[1] * uv(1)[0] + w2 * iz[2] * uv(2)[0]) * d;
                    let tv =
                        (w0 * iz[0] * uv(0)[1] + w1 * iz[1] * uv(1)[1] + w2 * iz[2] * uv(2)[1]) * d;
                    let sx = ((tu.rem_euclid(1.0)) * pic.width as f32) as usize % pic.width.max(1);
                    let sy =
                        ((tv.rem_euclid(1.0)) * pic.height as f32) as usize % pic.height.max(1);
                    let s = (sy * pic.width + sx) * 4;
                    depth[k] = d;
                    for c in 0..3 {
                        out.pixels[k * 4 + c] = (pic.pixels[s + c] as f32 * b) as u8;
                    }
                    out.pixels[k * 4 + 3] = 255;
                }
            }
        }
    }
    out
}

/// How much of the screen the helmet covers, measured on a painted picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coverage {
    /// Share of the left and right edge columns covered.
    pub left_edge: f32,
    pub right_edge: f32,
    /// Share of the top and bottom rows covered.
    pub top_edge: f32,
    pub bottom_edge: f32,
    /// Share of the middle (half the width, half the height) covered: the
    /// opening you look through.
    pub middle: f32,
    /// Share of the whole screen covered.
    pub all: f32,
}

pub fn coverage(p: &Rgba) -> Coverage {
    let (w, h) = (p.width, p.height);
    let on = |x: usize, y: usize| p.pixels[(y * w + x) * 4 + 3] > 0;
    let share = |n: usize, of: usize| n as f32 / of.max(1) as f32;
    let col = |x: usize| share((0..h).filter(|&y| on(x, y)).count(), h);
    let row = |y: usize| share((0..w).filter(|&x| on(x, y)).count(), w);
    let mut mid = 0;
    let mut all = 0;
    for y in 0..h {
        for x in 0..w {
            if on(x, y) {
                all += 1;
                if x >= w / 4 && x < w * 3 / 4 && y >= h / 4 && y < h * 3 / 4 {
                    mid += 1;
                }
            }
        }
    }
    Coverage {
        left_edge: col(0),
        right_edge: col(w - 1),
        top_edge: row(0),
        bottom_edge: row(h - 1),
        middle: share(mid, (w / 2) * (h / 2)),
        all: share(all, w * h),
    }
}

/// Landscape screen shapes we promise to fit: phones on their side, flip
/// and fold phones, TVs, monitors and ultrawides (width / height).
pub const SHAPES: &[(&str, f32)] = &[
    ("5:4 monitor", 5.0 / 4.0),
    ("4:3", 4.0 / 3.0),
    ("3:2 laptop", 3.0 / 2.0),
    ("8:5 (16:10)", 16.0 / 10.0),
    ("Steam Deck 16:10", 1280.0 / 800.0),
    ("16:9 TV", 16.0 / 9.0),
    ("foldable open 4.2:3.7", 2208.0 / 1768.0),
    ("phone 19.5:9", 19.5 / 9.0),
    ("phone 20:9", 20.0 / 9.0),
    ("flip phone 22:9", 22.0 / 9.0),
    ("21:9 ultrawide", 21.0 / 9.0),
    ("3440x1440", 3440.0 / 1440.0),
    ("maximized 3440x1369", 3440.0 / 1369.0),
    ("32:9 super ultrawide", 32.0 / 9.0),
];

/// Check the fitted frame at every shape against the 4:3 original: it
/// should reach every screen edge about as much as it does at 4:3 and
/// cover the middle no more. Returns one line per shape and the problems.
pub fn check_shapes(model: &HelmetModel, plan: &HelmetPlan) -> (Vec<String>, Vec<String>) {
    let height = 240;
    let at = |aspect: f32| {
        let w = (height as f32 * aspect).round() as usize;
        coverage(&paint(model, &fit(model, plan, aspect), 1.0, w, height))
    };
    let base = at(4.0 / 3.0);
    let mut lines = vec![];
    let mut problems = vec![];
    for (name, aspect) in SHAPES {
        let c = at(*aspect);
        lines.push(format!(
            "{name:>24}: edges L {:.2} R {:.2} T {:.2} B {:.2}, middle {:.2}, all {:.2}",
            c.left_edge, c.right_edge, c.top_edge, c.bottom_edge, c.middle, c.all
        ));
        let near = |a: f32, b: f32| (a - b).abs() <= 0.1;
        if !near(c.left_edge, base.left_edge) || !near(c.right_edge, base.right_edge) {
            problems.push(format!(
                "{name}: the sides reach the edge {:.2}/{:.2}, at 4:3 {:.2}/{:.2}",
                c.left_edge, c.right_edge, base.left_edge, base.right_edge
            ));
        }
        // Along the top and bottom: a bar that spans the whole 4:3 screen
        // must span every screen; pieces that don't (the bottom corners)
        // must keep their size, measured in screen heights.
        let along = [
            ("top", c.top_edge, base.top_edge),
            ("bottom", c.bottom_edge, base.bottom_edge),
        ];
        for (side, now, was) in along {
            let ok = if was >= 0.99 {
                now >= 0.99
            } else {
                (now * aspect - was * 4.0 / 3.0).abs() <= 0.15
            };
            if !ok {
                problems.push(format!(
                    "{name}: along the {side} {:.2} screen heights covered, at 4:3 {:.2}",
                    now * aspect,
                    was * 4.0 / 3.0
                ));
            }
        }
        if c.middle > base.middle + 0.05 {
            problems.push(format!(
                "{name}: covers {:.2} of the middle, at 4:3 {:.2}",
                c.middle, base.middle
            ));
        }
    }
    (lines, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square_model() -> HelmetModel {
        HelmetModel {
            points: vec![
                [10.0, -12.0, 0.0],
                [10.0, 0.0, 0.0],
                [10.0, 3.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            normals: vec![[0.0; 3]; 4],
            uvs: vec![[0.0; 2]; 4],
            parts: vec![],
            display_fov: 84.0,
            notes: vec![],
        }
    }

    #[test]
    fn four_three_leaves_it_alone() {
        let m = square_model();
        let p = fit(&m, &HelmetPlan::default(), 4.0 / 3.0);
        for (a, b) in p.iter().zip(&m.points) {
            for k in 0..3 {
                assert!((a[k] - b[k]).abs() < 1e-4, "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn sides_move_out_without_changing_shape() {
        let m = square_model();
        let plan = HelmetPlan::default();
        let t = half_height(84.0);
        let p = fit(&m, &plan, 21.0 / 9.0);
        // A side point moves out by exactly the extra screen width.
        let extra = t * (21.0 / 9.0 - 4.0 / 3.0) * 10.0;
        assert!((p[0][1] - (-12.0 - extra)).abs() < 1e-3, "{:?}", p[0]);
        // The middle stays in the middle; up and down never change.
        assert!(p[1][1].abs() < 1e-6);
        assert!(p
            .iter()
            .zip(&m.points)
            .all(|(a, b)| a[2] == b[2] && a[0] == b[0]));
        // The eye point (x = 0) is left alone.
        assert_eq!(p[3], m.points[3]);
    }

    #[test]
    fn plan_defaults_and_unknown_keys() {
        let p = HelmetPlan::default();
        assert!(p.enabled);
        assert_eq!(p.mesh, "HUDMeshes.HelmetSurface.Helmet");
        assert!(HelmetPlan::parse("bogus = 1").is_err());
    }
}
