//! How much light reaches a point in the level: a "light probe" built from
//! the light the game baked into the level's surfaces.
//!
//! The level's own surfaces carry their light (a colour per prop corner, the
//! terrain's corner colours and the BSP's lightmaps), but things that move
//! (squad mates, droids, the first-person gun, the helmet) have none. The
//! original lights those by where they stand: dark in a dark tunnel, bright
//! in the sun. This gives them the same: the light on the surfaces around a
//! point that face it, nearer ones counting more.
//!
//! It is built once per level from `scene::Scene` (no Bevy), and asked a few
//! times a frame, so it is cheap on phones and in VR: the samples sit in a
//! grid of cells, and a question looks only at the cells around the point.
//!
//! Units: positions are Bevy's (see `scene::to_bevy`). The light is on the
//! game's own scale, the one the level is drawn with: 1.0 shows a texture
//! as stored, 0.5 at half, up to 2.0 (the baked light's double strength).
//! It is the stored 0-255 value x the drawing gain / 255 (see
//! `LightingSettings`).

use crate::scene::Scene;
use std::collections::HashMap;

/// One surface point and the light on it, as it is built.
#[derive(Debug, Clone, Copy)]
struct Sample {
    pos: [f32; 3],
    normal: [f32; 3],
    light: [f32; 3],
}

/// The same, packed small (a level holds about a million): the normal in
/// steps of 1/127 and the light in steps of 2/255.
#[derive(Debug, Clone, Copy)]
struct Packed {
    pos: [f32; 3],
    normal: [i8; 3],
    light: [u8; 3],
}

impl Packed {
    fn pack(s: Sample) -> Packed {
        Packed {
            pos: s.pos,
            normal: s.normal.map(|v| (v.clamp(-1.0, 1.0) * 127.0).round() as i8),
            light: s.light.map(|v| (v.clamp(0.0, 2.0) * 127.5).round() as u8),
        }
    }
}

/// The finished probe.
pub struct LightProbe {
    cell: f32,
    radius: f32,
    cells: HashMap<(i32, i32, i32), Vec<Packed>>,
    samples: usize,
}

/// Which surfaces gave samples, for the log.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProbeStats {
    pub prop_samples: usize,
    pub terrain_samples: usize,
    pub bsp_samples: usize,
}

impl LightProbe {
    /// Build the probe from a level. `prop_gain` and `lightmap_gain` are the
    /// drawing gains (`LightingSettings::vertex_colour_gain` and
    /// `lightmap_gain`); `radius` is how far (Bevy units) a point looks for
    /// lit surfaces.
    pub fn build(
        scene: &Scene,
        prop_gain: f32,
        lightmap_gain: f32,
        radius: f32,
    ) -> (LightProbe, ProbeStats) {
        let mut probe = LightProbe {
            cell: radius.max(0.5),
            radius: radius.max(0.5),
            cells: HashMap::new(),
            samples: 0,
        };
        let mut stats = ProbeStats::default();
        for b in &scene.batches {
            // The sky box is far away from the playing area; see-through
            // surfaces (glass, glows) don't say what light is there.
            if b.sky || b.translucent || b.mover.is_some() {
                continue;
            }
            if b.terrain_layer.is_some_and(|l| l > 0) {
                continue;
            }
            let corner_light: Box<dyn Fn(usize) -> [f32; 3]> =
                if !b.colours.is_empty() && b.colours.len() == b.positions.len() {
                    let k = prop_gain / 255.0;
                    Box::new(move |i| {
                        let c = b.colours[i];
                        [c[0] as f32 * k, c[1] as f32 * k, c[2] as f32 * k]
                    })
                } else {
                    let page = b.lightmap_page.and_then(|p| scene.lightmap_pages.get(p));
                    if page.is_none() || b.lightmap_uvs.len() != b.positions.len() {
                        continue;
                    }
                    // The page is looked up per sample below, at its own
                    // spot; the corners only carry the gain.
                    let k = lightmap_gain / 255.0;
                    Box::new(move |_| [k, k, k])
                };
            let page = if b.colours.is_empty() {
                b.lightmap_page.and_then(|p| scene.lightmap_pages.get(p))
            } else {
                None
            };
            for t in b.indices.chunks_exact(3) {
                let i = [t[0] as usize, t[1] as usize, t[2] as usize];
                if i.iter().any(|&v| v >= b.positions.len()) {
                    continue;
                }
                let pos = i.map(|v| b.positions[v]);
                let Some(normal) = face_normal(
                    pos,
                    i.map(|v| b.normals.get(v).copied().unwrap_or([0.0; 3])),
                ) else {
                    continue;
                };
                let light = i.map(|v| corner_light(v));
                // Big props (canyon walls) have corners metres apart, so
                // spread samples over each triangle about every `SPACING`.
                let longest = (0..3)
                    .map(|e| dist(pos[e], pos[(e + 1) % 3]))
                    .fold(0.0f32, f32::max);
                // A triangle no longer than `SPACING` gives one sample, in
                // its middle (its corners are its neighbours' too); bigger
                // ones a grid of n steps along each side.
                let n = (longest / SPACING).floor() as usize;
                let grid: Vec<[f32; 3]> = if n == 0 {
                    vec![[1.0 / 3.0; 3]]
                } else {
                    let n = n.min(MAX_STEPS);
                    (0..=n)
                        .flat_map(|a| (0..=(n - a)).map(move |c| (a, c)))
                        .map(|(a, c)| {
                            [
                                a as f32 / n as f32,
                                c as f32 / n as f32,
                                (n - a - c) as f32 / n as f32,
                            ]
                        })
                        .collect()
                };
                for w in grid {
                    let mut l = blend3(light, w);
                    if let Some(page) = page {
                        let tx = texel(page, blend2(i.map(|v| b.lightmap_uvs[v]), w));
                        l = [l[0] * tx[0], l[1] * tx[1], l[2] * tx[2]];
                    }
                    probe.add(Sample {
                        pos: blend3(pos, w),
                        normal,
                        light: l,
                    });
                    count(&mut stats, b, page.is_some());
                }
            }
        }
        (probe, stats)
    }

    fn key(&self, p: [f32; 3]) -> (i32, i32, i32) {
        (
            (p[0] / self.cell).floor() as i32,
            (p[1] / self.cell).floor() as i32,
            (p[2] / self.cell).floor() as i32,
        )
    }

    fn add(&mut self, s: Sample) {
        let k = self.key(s.pos);
        self.cells.entry(k).or_default().push(Packed::pack(s));
        self.samples += 1;
    }

    pub fn samples(&self) -> usize {
        self.samples
    }

    /// The light at `p` (Bevy position), on the game's scale (see the
    /// module notes), or `None` when no lit surface faces it nearby (out in
    /// the open air, or outside the level).
    ///
    /// Each surface point within `radius` that faces `p` counts, by how
    /// squarely it faces it and by 1 / (1 + distance²), so the floor under
    /// your feet and the wall beside you count most.
    pub fn light_at(&self, p: [f32; 3]) -> Option<[f32; 3]> {
        // Out in the open the nearest surface can be further away (high
        // above a canyon floor): look twice and four times as far.
        [1, 2, 4].into_iter().find_map(|k| self.light_within(p, k))
    }

    fn light_within(&self, p: [f32; 3], reach: i32) -> Option<[f32; 3]> {
        let r = self.radius * reach as f32;
        let r2 = r * r;
        let (cx, cy, cz) = self.key(p);
        let mut sum = [0.0f32; 3];
        let mut weight = 0.0f32;
        for dx in -reach..=reach {
            for dy in -reach..=reach {
                for dz in -reach..=reach {
                    let Some(list) = self.cells.get(&(cx + dx, cy + dy, cz + dz)) else {
                        continue;
                    };
                    for s in list {
                        let d = [p[0] - s.pos[0], p[1] - s.pos[1], p[2] - s.pos[2]];
                        let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                        if d2 > r2 {
                            continue;
                        }
                        let len = d2.sqrt().max(1e-3);
                        let n = s.normal.map(|v| v as f32 / 127.0);
                        let facing = (n[0] * d[0] + n[1] * d[1] + n[2] * d[2]) / len;
                        if facing <= 0.05 {
                            continue;
                        }
                        let w = facing / (1.0 + d2);
                        for i in 0..3 {
                            sum[i] += s.light[i] as f32 / 127.5 * w;
                        }
                        weight += w;
                    }
                }
            }
        }
        (weight > 0.0).then(|| sum.map(|v| v / weight))
    }
}

/// About how far apart (Bevy units) samples are spread over big triangles,
/// and the most steps along one triangle's longest side.
const SPACING: f32 = 3.0;
const MAX_STEPS: usize = 10;

fn count(stats: &mut ProbeStats, b: &crate::scene::Batch, lightmap: bool) {
    if lightmap {
        stats.bsp_samples += 1;
    } else if b.terrain_layer.is_some() {
        stats.terrain_samples += 1;
    } else {
        stats.prop_samples += 1;
    }
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn blend3(v: [[f32; 3]; 3], w: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|k| v[0][k] * w[0] + v[1][k] * w[1] + v[2][k] * w[2])
}

fn blend2(v: [[f32; 2]; 3], w: [f32; 3]) -> [f32; 2] {
    [0, 1].map(|k| v[0][k] * w[0] + v[1][k] * w[1] + v[2][k] * w[2])
}

/// The way a triangle faces: its corners' normals when it has them (two-
/// sided and odd windings stay right), else its winding. `None` for a
/// triangle with no area.
fn face_normal(p: [[f32; 3]; 3], corner: [[f32; 3]; 3]) -> Option<[f32; 3]> {
    let s = blend3(corner, [1.0; 3]);
    let len = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt();
    if len > 1e-3 {
        return Some(s.map(|v| v / len));
    }
    let u = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
    let v = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    (len > 1e-6).then(|| n.map(|v| v / len))
}

/// The lightmap's colour at `uv` (0..1), 0-255 per channel.
fn texel(page: &rc_image::Rgba, uv: [f32; 2]) -> [f32; 3] {
    if page.width == 0 || page.height == 0 {
        return [0.0; 3];
    }
    let x = ((uv[0].rem_euclid(1.0)) * page.width as f32) as usize;
    let y = ((uv[1].rem_euclid(1.0)) * page.height as f32) as usize;
    let i = (y.min(page.height - 1) * page.width + x.min(page.width - 1)) * 4;
    match page.pixels.get(i..i + 3) {
        Some(p) => [p[0] as f32, p[1] as f32, p[2] as f32],
        None => [0.0; 3],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(samples: &[Sample]) -> LightProbe {
        let mut p = LightProbe {
            cell: 4.0,
            radius: 4.0,
            cells: HashMap::new(),
            samples: 0,
        };
        for s in samples {
            p.add(*s);
        }
        p
    }

    #[test]
    fn only_surfaces_facing_the_point_count() {
        // A bright floor below and a dark ceiling facing away (its back
        // turned to the point, like the far side of a wall).
        let p = probe(&[
            Sample {
                pos: [0.0, 0.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                light: [1.0, 1.0, 1.0],
            },
            Sample {
                pos: [0.0, 2.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                light: [0.0, 0.0, 0.0],
            },
        ]);
        let l = p.light_at([0.0, 1.0, 0.0]).unwrap();
        assert!((l[0] - 1.0).abs() < 0.01, "{l:?}");
    }

    #[test]
    fn nearer_surfaces_count_more() {
        let p = probe(&[
            Sample {
                pos: [0.0, 0.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                light: [1.0, 1.0, 1.0],
            },
            Sample {
                pos: [0.0, 4.0, 0.0],
                normal: [0.0, -1.0, 0.0],
                light: [0.0, 0.0, 0.0],
            },
        ]);
        let l = p.light_at([0.0, 1.0, 0.0]).unwrap();
        // Weights 1/2 and 1/10: (0.5 x 1) / 0.6.
        assert!((l[0] - 0.5 / 0.6).abs() < 0.01, "{l:?}");
    }

    #[test]
    fn nothing_nearby_is_none() {
        let p = probe(&[Sample {
            pos: [0.0, 0.0, 0.0],
            normal: [0.0, 1.0, 0.0],
            light: [1.0; 3],
        }]);
        assert!(p.light_at([90.0, 90.0, 90.0]).is_none());
    }
}
