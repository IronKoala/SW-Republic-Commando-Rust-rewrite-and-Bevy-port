//! Collision with a soup of triangles, for real levels: the level's BSP
//! polygons and its static meshes, cut into triangles by the level reader.
//!
//! Each triangle is treated as a very thin solid and swept against with the
//! same "grow the solid by the box, then trace a line" method as the test
//! room. For a box against a triangle the solid needs these sides to be
//! exact: the triangle's face (both ways), one side per edge, the six sides
//! of its bounding box, and one per edge-and-axis pair (the edge crossed
//! with X, Y or Z, both ways).
//!
//! Triangles are filed in a grid of square columns on the ground plane, so
//! a sweep only looks at triangles near it.

use crate::world::{sweep_planes, Hit, Plane, World};
use crate::Vec3;
use std::collections::HashSet;

/// Width of one grid column, uu.
const CELL: f32 = 256.0;

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}

#[derive(Debug, Clone)]
pub struct TriangleWorld {
    pub triangles: Vec<[Vec3; 3]>,
    min: Vec3,
    max: Vec3,
    columns: usize,
    rows: usize,
    /// Triangle numbers in each grid column.
    cells: Vec<Vec<u32>>,
}

impl TriangleWorld {
    pub fn new(triangles: Vec<[Vec3; 3]>) -> Self {
        let mut min = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
        let mut max = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
        for t in &triangles {
            for p in t {
                min = Vec3::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z));
                max = Vec3::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z));
            }
        }
        if triangles.is_empty() {
            min = Vec3::ZERO;
            max = Vec3::ZERO;
        }
        let columns = (((max.x - min.x) / CELL).floor() as usize + 1).max(1);
        let rows = (((max.y - min.y) / CELL).floor() as usize + 1).max(1);
        let mut w = TriangleWorld {
            triangles,
            min,
            max,
            columns,
            rows,
            cells: vec![Vec::new(); columns * rows],
        };
        for (i, t) in w.triangles.iter().enumerate() {
            let lo = Vec3::new(t[0].x.min(t[1].x).min(t[2].x), t[0].y.min(t[1].y).min(t[2].y), 0.0);
            let hi = Vec3::new(t[0].x.max(t[1].x).max(t[2].x), t[0].y.max(t[1].y).max(t[2].y), 0.0);
            let (c0, r0, c1, r1) = w.cell_range(lo, hi);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    w.cells[r * columns + c].push(i as u32);
                }
            }
        }
        w
    }

    /// Lowest and highest corners of all triangles.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        (self.min, self.max)
    }

    fn cell_range(&self, lo: Vec3, hi: Vec3) -> (usize, usize, usize, usize) {
        let clamp_c = |x: f32| (((x - self.min.x) / CELL).floor().max(0.0) as usize).min(self.columns - 1);
        let clamp_r = |y: f32| (((y - self.min.y) / CELL).floor().max(0.0) as usize).min(self.rows - 1);
        (clamp_c(lo.x), clamp_r(lo.y), clamp_c(hi.x), clamp_r(hi.y))
    }

    /// The thin solid for one triangle, as planes.
    fn planes(t: &[Vec3; 3]) -> Vec<Plane> {
        let mut planes = Vec::with_capacity(29);
        let mut add = |n: Vec3| {
            let n = n.normalize_or_zero();
            if n == Vec3::ZERO {
                return;
            }
            let dist = t.iter().map(|p| n.dot(*p)).fold(f32::MIN, f32::max);
            planes.push(Plane { normal: n, dist });
        };
        let edges = [t[1] - t[0], t[2] - t[1], t[0] - t[2]];
        let face = cross(edges[0], t[2] - t[0]);
        // The face goes first, so a hit on it reports the face's normal.
        add(face);
        add(-face);
        for e in edges {
            add(cross(e, face));
            add(-cross(e, face));
        }
        let axes = [Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::UP];
        for a in axes {
            add(a);
            add(-a);
        }
        for e in edges {
            for a in axes {
                let c = cross(e, a);
                add(c);
                add(-c);
            }
        }
        planes
    }
}

impl World for TriangleWorld {
    fn sweep(&self, extent: Vec3, start: Vec3, end: Vec3) -> Option<Hit> {
        if self.triangles.is_empty() {
            return None;
        }
        let lo = Vec3::new(start.x.min(end.x) - extent.x, start.y.min(end.y) - extent.y, 0.0);
        let hi = Vec3::new(start.x.max(end.x) + extent.x, start.y.max(end.y) + extent.y, 0.0);
        let z_lo = start.z.min(end.z) - extent.z;
        let z_hi = start.z.max(end.z) + extent.z;
        let (c0, r0, c1, r1) = self.cell_range(lo, hi);
        let mut seen = HashSet::new();
        let mut best: Option<Hit> = None;
        for r in r0..=r1 {
            for c in c0..=c1 {
                for &i in &self.cells[r * self.columns + c] {
                    if !seen.insert(i) {
                        continue;
                    }
                    let t = &self.triangles[i as usize];
                    // Quick reject on height.
                    let tz_lo = t[0].z.min(t[1].z).min(t[2].z);
                    let tz_hi = t[0].z.max(t[1].z).max(t[2].z);
                    if tz_hi < z_lo || tz_lo > z_hi {
                        continue;
                    }
                    if let Some(h) = sweep_planes(&Self::planes(t), extent, start, end) {
                        if h.start_solid {
                            return Some(h);
                        }
                        if best.map_or(true, |b| h.time < b.time) {
                            best = Some(h);
                        }
                    }
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room::*;
    use crate::{MoveInput, MoveParams, Pawn};

    /// The test room's solids, cut into triangles.
    fn room_as_triangles() -> TriangleWorld {
        let mut tris = Vec::new();
        for b in &test_room().brushes {
            let (lo, hi) = (b.min, b.max);
            let ramp = b.planes.len() > 6;
            let low_top = if ramp { lo.z } else { hi.z };
            let p = [
                Vec3::new(lo.x, lo.y, lo.z),
                Vec3::new(hi.x, lo.y, lo.z),
                Vec3::new(hi.x, hi.y, lo.z),
                Vec3::new(lo.x, hi.y, lo.z),
                Vec3::new(lo.x, lo.y, low_top),
                Vec3::new(hi.x, lo.y, hi.z),
                Vec3::new(hi.x, hi.y, hi.z),
                Vec3::new(lo.x, hi.y, low_top),
            ];
            for f in [[0, 1, 2, 3], [4, 7, 6, 5], [0, 4, 5, 1], [2, 6, 7, 3], [1, 5, 6, 2], [0, 3, 7, 4]] {
                let q = [p[f[0]], p[f[1]], p[f[2]], p[f[3]]];
                if (q[0] - q[1]).length() < 1e-3 && (q[2] - q[3]).length() < 1e-3 {
                    continue;
                }
                tris.push([q[0], q[1], q[2]]);
                tris.push([q[0], q[2], q[3]]);
            }
        }
        TriangleWorld::new(tris)
    }

    #[test]
    fn box_lands_on_a_triangle_floor() {
        let w = TriangleWorld::new(vec![[
            Vec3::new(-500.0, -500.0, 0.0),
            Vec3::new(500.0, -500.0, 0.0),
            Vec3::new(0.0, 500.0, 0.0),
        ]]);
        let e = Vec3::new(40.0, 40.0, 84.0);
        let h = w.sweep(e, Vec3::new(0.0, 0.0, 184.0), Vec3::new(0.0, 0.0, -16.0)).unwrap();
        assert!((h.time - 0.5).abs() < 1e-4, "{h:?}");
        assert!((h.normal.z.abs() - 1.0).abs() < 1e-5);
        // Past the triangle's edge: no hit.
        assert!(w.sweep(e, Vec3::new(400.0, 400.0, 184.0), Vec3::new(400.0, 400.0, -16.0)).is_none());
    }

    #[test]
    fn walking_matches_the_brush_room() {
        // The same run up the 30-high step, in both worlds.
        let tw = room_as_triangles();
        let bw = test_room();
        let input = MoveInput { forward: 1.0, ..Default::default() };
        let feet = Vec3::new(700.0, LANE_STEP_30, 0.0);
        let mut a = Pawn::new(MoveParams::player_commando(), feet);
        let mut b = Pawn::new(MoveParams::player_commando(), feet);
        for _ in 0..90 {
            a.step(1.0 / 60.0, &input, &tw);
            b.step(1.0 / 60.0, &input, &bw);
        }
        assert!((a.feet().z - 30.0).abs() < 0.5, "triangles: feet {}", a.feet().z);
        assert!((a.location - b.location).length() < 1.0, "{:?} vs {:?}", a.location, b.location);
    }

    #[test]
    fn every_lane_matches_the_brush_room() {
        let tw = room_as_triangles();
        let bw = test_room();
        let run = MoveInput { forward: 1.0, ..Default::default() };
        let crouch = MoveInput { forward: 1.0, crouch: true, ..Default::default() };
        for (lane, input, secs) in [
            (LANE_STEP_40, run, 1.5),
            (LANE_STAIRS, run, 2.0),
            (LANE_RAMP_30, run, 1.6),
            (LANE_RAMP_50, run, 3.0),
            (LANE_CEILING, run, 1.5),
            (LANE_CEILING, crouch, 1.8),
        ] {
            let feet = Vec3::new(700.0, lane, 0.0);
            let mut a = Pawn::new(MoveParams::player_commando(), feet);
            let mut b = Pawn::new(MoveParams::player_commando(), feet);
            for _ in 0..(secs * 60.0) as usize {
                a.step(1.0 / 60.0, &input, &tw);
                b.step(1.0 / 60.0, &input, &bw);
            }
            let gap = (a.location - b.location).length();
            assert!(gap < 1.0, "lane {lane}: {:?} vs {:?}", a.location, b.location);
        }
    }
}
