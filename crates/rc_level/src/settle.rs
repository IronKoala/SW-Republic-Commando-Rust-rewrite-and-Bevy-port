//! Physics props (`KarmaProp`: loose rocks, crates) resting on the ground.
//!
//! On geo_04a the map leaves its falling rocks (`K_GEO4_FallingRock`, by
//! the start) 40 to 125 units above the ground under them (checked against
//! the terrain's own heightmap, `TerrainMap`, which matches the points we
//! draw to 0.1 unit). Their Karma settings say `KStartEnabled`: physics is
//! on from the start, so in the game they drop onto the ground as the level
//! begins. This port has no physics for them yet, so instead each such prop
//! is moved straight down until it touches what is below it, once, when the
//! level loads (`starts_awake` says which). Turning to rest on a slope isn't
//! done.
//!
//! Props whose physics starts asleep stay where the map puts them, as they
//! would in the game until something hits them: geo_04a's other rocks
//! (`Physics` set to none in the map), and the 47 cans on ras_01a hung near
//! the ceiling, which have a start speed for when something sets them off.
//!
//! Plain Rust, no Bevy. Unreal units, Z up.

use crate::bsp::Vec3;
use std::collections::HashMap;

/// Size of a cell of the ground grid, in Unreal units.
const CELL: f32 = 256.0;

/// The static ground (BSP, props, terrain) sorted into a grid of columns so
/// "what is straight below this point" is quick to answer.
pub struct Ground<'a> {
    triangles: &'a [[Vec3; 3]],
    cells: HashMap<(i32, i32), Vec<u32>>,
}

fn cell(v: f32) -> i32 {
    (v / CELL).floor() as i32
}

impl<'a> Ground<'a> {
    pub fn new(triangles: &'a [[Vec3; 3]]) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, t) in triangles.iter().enumerate() {
            let (x0, x1) = (
                t.iter().map(|p| p[0]).fold(f32::MAX, f32::min),
                t.iter().map(|p| p[0]).fold(f32::MIN, f32::max),
            );
            let (y0, y1) = (
                t.iter().map(|p| p[1]).fold(f32::MAX, f32::min),
                t.iter().map(|p| p[1]).fold(f32::MIN, f32::max),
            );
            // Triangles covering more than 4096 cells (a 16,000-unit
            // square: sky boxes, backdrops) are left out to keep this small.
            let (cx0, cx1, cy0, cy1) = (cell(x0), cell(x1), cell(y0), cell(y1));
            if (cx1 - cx0 + 1) as i64 * (cy1 - cy0 + 1) as i64 > 4096 {
                continue;
            }
            for cx in cx0..=cx1 {
                for cy in cy0..=cy1 {
                    cells.entry((cx, cy)).or_default().push(i as u32);
                }
            }
        }
        Ground { triangles, cells }
    }

    /// The height of the highest ground surface straight below (or at)
    /// `p`, if any.
    pub fn below(&self, p: Vec3) -> Option<f32> {
        let list = self.cells.get(&(cell(p[0]), cell(p[1])))?;
        let mut best: Option<f32> = None;
        for &i in list {
            let t = &self.triangles[i as usize];
            if let Some(z) = height_at(t, p[0], p[1]) {
                if z <= p[2] + 0.5 && best.is_none_or(|b| z > b) {
                    best = Some(z);
                }
            }
        }
        best
    }
}

/// Height of triangle `t` above the point (x, y), if (x, y) is inside it
/// seen from above. Vertical triangles have none.
fn height_at(t: &[Vec3; 3], x: f32, y: f32) -> Option<f32> {
    let [a, b, c] = *t;
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if d.abs() < 1e-6 {
        return None;
    }
    let l1 = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / d;
    let l2 = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / d;
    let l3 = 1.0 - l1 - l2;
    let e = -1e-4;
    (l1 >= e && l2 >= e && l3 >= e).then(|| l1 * a[2] + l2 * b[2] + l3 * c[2])
}

/// How far a prop made of `points` (its corners, placed in the world) can
/// move straight down before one of them touches the ground: the smallest
/// gap under any corner. `None` if nothing is below it, or if it would
/// have to drop more than `max` (a hole, or a prop meant to hang).
pub fn drop_distance(points: &[Vec3], ground: &Ground, max: f32) -> Option<f32> {
    let mut gap: Option<f32> = None;
    for p in points {
        if let Some(z) = ground.below(*p) {
            let g = (p[2] - z).max(0.0);
            gap = Some(gap.map_or(g, |v: f32| v.min(g)));
        }
    }
    gap.filter(|g| *g <= max)
}

/// Move a physics prop straight down onto the ground (see the module
/// notes). Returns how far it moved, or `None` if it was left in place.
pub fn settle(
    place: &mut crate::actors::Placement,
    mesh: &rc_package::static_mesh::StaticMesh,
    ground: &Ground,
) -> Option<f32> {
    let points: Vec<Vec3> = mesh
        .vertices
        .iter()
        .map(|v| place.transform_point(v.position))
        .collect();
    let d = drop_distance(&points, ground, MAX_DROP)?;
    place.location[2] -= d;
    Some(d)
}

/// Whether a placed physics prop's physics runs from the moment the level
/// starts: its `KParams` (a `KarmaParams` object in the map) says
/// `KStartEnabled`, and the map doesn't switch its `Physics` off. Only what
/// the map itself stores is read (class defaults aren't), so a prop that
/// relies on its class for this counts as asleep.
pub fn starts_awake(pkg: &rc_package::Package, data: &[u8], actor: usize) -> bool {
    use rc_package::properties::{self, read_properties, Value};
    let Ok(props) = crate::actors::read_actor_properties(pkg, data, actor) else {
        return false;
    };
    if matches!(
        properties::find(pkg, &props, "Physics").map(|p| &p.value),
        Some(Value::Byte(0))
    ) {
        return false;
    }
    let Some(Value::Object(k)) = properties::find(pkg, &props, "KParams").map(|p| &p.value) else {
        return false;
    };
    let Some(e) = usize::try_from(*k - 1)
        .ok()
        .and_then(|i| pkg.exports.get(i))
    else {
        return false;
    };
    let Ok(bytes) = crate::actors::object_bytes(data, e) else {
        return false;
    };
    let mut r = rc_package::Reader::new(bytes);
    let Ok(kp) = read_properties(pkg, &mut r) else {
        return false;
    };
    matches!(
        properties::find(pkg, &kp, "KStartEnabled").map(|p| &p.value),
        Some(Value::Bool(true))
    )
}

/// Props further above the ground than this are left where the map puts
/// them (geo_04a's highest rock is 182 above).
pub const MAX_DROP: f32 = 512.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rock_drops_onto_floor() {
        // A floor at height 10 made of two triangles, 0..1000 square.
        let floor = [
            [
                [0.0, 0.0, 10.0],
                [1000.0, 0.0, 10.0],
                [1000.0, 1000.0, 10.0],
            ],
            [
                [0.0, 0.0, 10.0],
                [1000.0, 1000.0, 10.0],
                [0.0, 1000.0, 10.0],
            ],
        ];
        let ground = Ground::new(&floor);
        let rock = [
            [500.0, 500.0, 150.0],
            [520.0, 500.0, 140.0],
            [500.0, 530.0, 200.0],
        ];
        assert_eq!(drop_distance(&rock, &ground, MAX_DROP), Some(130.0));
        // Already touching: no drop.
        let resting = [[100.0, 100.0, 10.0], [120.0, 100.0, 40.0]];
        assert_eq!(drop_distance(&resting, &ground, MAX_DROP), Some(0.0));
        // Nothing below.
        assert_eq!(
            drop_distance(&[[5000.0, 5000.0, 50.0]], &ground, MAX_DROP),
            None
        );
        // Too far above.
        assert_eq!(
            drop_distance(&[[500.0, 500.0, 900.0]], &ground, MAX_DROP),
            None
        );
    }
}
