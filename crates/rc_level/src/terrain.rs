//! Outdoor ground ("terrain"): Unreal 2's `TerrainInfo` actor.
//!
//! A terrain is a square grid of points (a heightmap) with a texture
//! painted over it in layers. Republic Commando stores the grid's points
//! already placed in the world, right after the actor's properties: a list
//! of `TerrainSector` objects (the grid cut into pieces for drawing), then
//! the point count and every point as three floats (Unreal units), row by
//! row with X changing fastest. Measured on geo_03a, geo_04a, yyy_01b and
//! yyy_35a: neighbouring points are `TerrainScale` (64 units) apart in X.
//! What follows the points differs from UE2 and isn't needed.
//!
//! Properties used: `QuadVisibilityBitmap` (bit set = that square is drawn;
//! holes are cut where caves and tunnels go), `EdgeTurnBitmap` (bit set =
//! the square is split into triangles along its other diagonal), and
//! `Layers` (each a `TerrainLayer` struct with `Texture`, `AlphaMap`,
//! `UScale`, `VScale`). Layer 0 covers everything; later layers are painted
//! on where their alpha map says.
//!
//! Plain Rust, no Bevy. No code copied from any other project; the layout
//! was found by reading the bytes of the game's own maps.

use crate::actors::{object_bytes, skip_state_frame};
use crate::bsp::Vec3;
use rc_package::properties::{self, read_properties, Property, Value};
use rc_package::{Package, Reader, Result};

/// One painted layer.
#[derive(Debug, Clone)]
pub struct Layer {
    /// Material reference (object index in the map).
    pub texture: i32,
    /// Texture whose alpha says how much of this layer shows (0 = none).
    pub alpha_map: i32,
    pub u_scale: f32,
    pub v_scale: f32,
}

#[derive(Debug, Clone)]
pub struct Terrain {
    /// 0-based export index of the `TerrainInfo`.
    pub export: usize,
    /// Points across (X) and down (Y).
    pub size_x: usize,
    pub size_y: usize,
    /// Distance between neighbouring points in X and Y (Unreal units).
    pub spacing: [f32; 2],
    /// Every point, in Unreal units, row by row (X fastest).
    pub points: Vec<Vec3>,
    /// One bit per square (`x + y * size_x`); true = drawn.
    pub visible: Vec<bool>,
    /// One bit per square; true = split along the other diagonal.
    pub turned: Vec<bool>,
    pub layers: Vec<Layer>,
}

/// A bitmap property: an array count, then that many 32-bit words.
fn bitmap(pkg: &Package, props: &[Property], name: &str, bits: usize, default: bool) -> Vec<bool> {
    let raw = match properties::find(pkg, props, name).map(|p| &p.value) {
        Some(Value::Raw(r)) => r.clone(),
        _ => return vec![default; bits],
    };
    let mut r = Reader::new(&raw);
    let words = match r.compact_index() {
        Ok(n) if n >= 0 => n as usize,
        _ => return vec![default; bits],
    };
    let mut out = vec![default; bits];
    for w in 0..words {
        let Ok(word) = r.u32() else { break };
        for b in 0..32 {
            if let Some(slot) = out.get_mut(w * 32 + b) {
                *slot = word & (1 << b) != 0;
            }
        }
    }
    out
}

fn layer(pkg: &Package, raw: &[u8]) -> Option<Layer> {
    let mut r = Reader::new(raw);
    let props = read_properties(pkg, &mut r).ok()?;
    let obj = |n: &str| match properties::find(pkg, &props, n).map(|p| &p.value) {
        Some(Value::Object(o)) => *o,
        _ => 0,
    };
    let float = |n: &str| match properties::find(pkg, &props, n).map(|p| &p.value) {
        Some(Value::Float(f)) => *f,
        _ => 1.0,
    };
    Some(Layer {
        texture: obj("Texture"),
        alpha_map: obj("AlphaMap"),
        u_scale: float("UScale"),
        v_scale: float("VScale"),
    })
}

/// Read one `TerrainInfo` actor (0-based export index).
pub fn read_terrain(pkg: &Package, data: &[u8], index: usize) -> Result<Terrain> {
    let e = &pkg.exports[index];
    let mut r = Reader::new(object_bytes(data, e)?);
    skip_state_frame(&mut r, e)?;
    let props = read_properties(pkg, &mut r)?;
    let sectors = r.compact_index()?;
    if !(0..=4096).contains(&sectors) {
        return r.invalid(format!("terrain sector count {sectors} looks wrong"));
    }
    for _ in 0..sectors {
        r.compact_index()?;
    }
    let n = r.compact_index()?;
    if n < 4 || n as usize * 12 > r.len() - r.pos() {
        return r.invalid(format!("terrain point count {n} doesn't fit"));
    }
    let n = n as usize;
    let side = (n as f64).sqrt().round() as usize;
    if side * side != n {
        return r.invalid(format!("terrain point count {n} isn't a square grid"));
    }
    let mut points = Vec::with_capacity(n);
    for _ in 0..n {
        points.push([r.f32()?, r.f32()?, r.f32()?]);
    }
    let mut layers: Vec<(u32, Layer)> = props
        .iter()
        .filter(|p| pkg.name(p.name).eq_ignore_ascii_case("Layers"))
        .filter_map(|p| match &p.value {
            Value::Raw(raw) => layer(pkg, raw).map(|l| (p.array_index, l)),
            _ => None,
        })
        .filter(|(_, l)| l.texture != 0)
        .collect();
    layers.sort_by_key(|(i, _)| *i);
    let spacing = [
        (points[1][0] - points[0][0]).abs().max(1.0),
        (points[side][1] - points[0][1]).abs().max(1.0),
    ];
    Ok(Terrain {
        spacing,
        export: index,
        size_x: side,
        size_y: side,
        visible: bitmap(pkg, &props, "QuadVisibilityBitmap", n, true),
        turned: bitmap(pkg, &props, "EdgeTurnBitmap", n, false),
        points,
        layers: layers.into_iter().map(|(_, l)| l).collect(),
    })
}

impl Terrain {
    pub fn point(&self, x: usize, y: usize) -> Vec3 {
        self.points[x + y * self.size_x]
    }

    /// The drawn squares as triangles of point indexes, each wound so its
    /// face points up (+Z) when seen from above in Unreal's axes.
    pub fn triangles(&self) -> Vec<[u32; 3]> {
        let mut out = Vec::new();
        let sx = self.size_x;
        for y in 0..self.size_y.saturating_sub(1) {
            for x in 0..sx.saturating_sub(1) {
                let q = x + y * sx;
                if !self.visible[q] {
                    continue;
                }
                let a = q as u32; // (x, y)
                let b = (q + 1) as u32; // (x+1, y)
                let c = (q + sx) as u32; // (x, y+1)
                let d = (q + sx + 1) as u32; // (x+1, y+1)
                if self.turned[q] {
                    out.push([a, b, c]);
                    out.push([b, d, c]);
                } else {
                    out.push([a, b, d]);
                    out.push([a, d, c]);
                }
            }
        }
        out
    }

    /// Triangles as corner positions (for collision).
    pub fn triangle_points(&self) -> Vec<[Vec3; 3]> {
        self.triangles()
            .into_iter()
            .map(|t| t.map(|i| self.points[i as usize]))
            .collect()
    }

    /// A smooth normal per point (Unreal axes), from the drawn triangles.
    pub fn normals(&self) -> Vec<Vec3> {
        let mut n = vec![[0.0f32; 3]; self.points.len()];
        for t in self.triangles() {
            let [p0, p1, p2] = t.map(|i| self.points[i as usize]);
            let u = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
            let v = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
            let c = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            for i in t {
                for k in 0..3 {
                    n[i as usize][k] += c[k];
                }
            }
        }
        for v in &mut n {
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            *v = if l > 1e-6 {
                // Make them point up, whatever the winding.
                let s = if v[2] < 0.0 { -1.0 } else { 1.0 };
                [s * v[0] / l, s * v[1] / l, s * v[2] / l]
            } else {
                [0.0, 0.0, 1.0]
            };
        }
        n
    }
}

/// Every terrain in the map (`actors` are 0-based export indexes), and the
/// ones that couldn't be read with why.
pub fn read_terrains(
    pkg: &Package,
    data: &[u8],
    actors: impl IntoIterator<Item = usize>,
) -> (Vec<Terrain>, Vec<String>) {
    let mut ok = Vec::new();
    let mut failed = Vec::new();
    for a in actors {
        let Some(e) = pkg.exports.get(a) else { continue };
        if pkg.export_class_name(e) != "TerrainInfo" {
            continue;
        }
        match read_terrain(pkg, data, a) {
            Ok(t) => ok.push(t),
            Err(err) => failed.push(format!("{}: {err}", pkg.name(e.object_name))),
        }
    }
    (ok, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(visible: Vec<bool>, turned: Vec<bool>) -> Terrain {
        let points = (0..9)
            .map(|i| [(i % 3) as f32 * 64.0, (i / 3) as f32 * 64.0, 0.0])
            .collect();
        Terrain {
            export: 0,
            size_x: 3,
            size_y: 3,
            spacing: [64.0, 64.0],
            points,
            visible,
            turned,
            layers: Vec::new(),
        }
    }

    #[test]
    fn squares_become_two_triangles_and_holes_are_left_out() {
        let t = grid(vec![true; 9], vec![false; 9]);
        assert_eq!(t.triangles().len(), 8);
        let mut vis = vec![true; 9];
        vis[0] = false;
        let t = grid(vis, vec![false; 9]);
        assert_eq!(t.triangles().len(), 6);
    }

    #[test]
    fn turned_squares_use_the_other_diagonal() {
        let plain = grid(vec![true; 9], vec![false; 9]).triangles();
        let mut turned = vec![false; 9];
        turned[0] = true;
        let t = grid(vec![true; 9], turned).triangles();
        // The plain split joins corners 0 and 4; the turned one 1 and 3.
        assert!(plain[0].contains(&0) && plain[0].contains(&4));
        assert!(t[0].contains(&1) && t[0].contains(&3) && t[1].contains(&1) && t[1].contains(&3));
    }

    #[test]
    fn flat_ground_normals_point_up() {
        let t = grid(vec![true; 9], vec![false; 9]);
        for n in t.normals() {
            assert!((n[2] - 1.0).abs() < 1e-5, "{n:?}");
        }
    }
}
