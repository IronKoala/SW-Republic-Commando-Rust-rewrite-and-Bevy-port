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
use std::sync::atomic::{AtomicU64, Ordering};

/// Width of one grid column, uu.
const CELL: f32 = 256.0;

static SWEEPS: AtomicU64 = AtomicU64::new(0);
static TESTED: AtomicU64 = AtomicU64::new(0);
static NANOS: AtomicU64 = AtomicU64::new(0);

/// How much collision work was done since the last call, for the speed
/// readout in the log: sweeps, triangles looked at, and time spent (ms,
/// added up over every thread).
pub fn take_counts() -> (u64, u64, f64) {
    (
        SWEEPS.swap(0, Ordering::Relaxed),
        TESTED.swap(0, Ordering::Relaxed),
        NANOS.swap(0, Ordering::Relaxed) as f64 / 1e6,
    )
}

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

    /// The thin solid for one triangle, as planes (the first `n` of them).
    fn planes(t: &[Vec3; 3]) -> ([Plane; 32], usize) {
        let mut planes = [Plane { normal: Vec3::ZERO, dist: 0.0 }; 32];
        let mut n_used = 0;
        let mut add = |n: Vec3| {
            let n = n.normalize_or_zero();
            if n == Vec3::ZERO {
                return;
            }
            let dist = t.iter().map(|p| n.dot(*p)).fold(f32::MIN, f32::max);
            planes[n_used] = Plane { normal: n, dist };
            n_used += 1;
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
        (planes, n_used)
    }
}

impl World for TriangleWorld {
    fn sweep(&self, extent: Vec3, start: Vec3, end: Vec3) -> Option<Hit> {
        if self.triangles.is_empty() {
            return None;
        }
        let t0 = std::time::Instant::now();
        let mut tested = 0u64;
        let hit = self.sweep_inner(extent, start, end, &mut tested);
        SWEEPS.fetch_add(1, Ordering::Relaxed);
        TESTED.fetch_add(tested, Ordering::Relaxed);
        NANOS.fetch_add(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
        hit
    }
}

impl TriangleWorld {
    fn sweep_inner(&self, extent: Vec3, start: Vec3, end: Vec3, tested: &mut u64) -> Option<Hit> {
        // The box the whole move covers, a little larger so rounding can't
        // leave out a triangle the exact test would touch.
        let lo = Vec3::new(
            start.x.min(end.x) - extent.x - MARGIN,
            start.y.min(end.y) - extent.y - MARGIN,
            start.z.min(end.z) - extent.z - MARGIN,
        );
        let hi = Vec3::new(
            start.x.max(end.x) + extent.x + MARGIN,
            start.y.max(end.y) + extent.y + MARGIN,
            start.z.max(end.z) + extent.z + MARGIN,
        );
        // Nowhere near these triangles (a far-away part of the level, a
        // door across the map): nothing to hit.
        if hi.x < self.min.x
            || lo.x > self.max.x
            || hi.y < self.min.y
            || lo.y > self.max.y
            || hi.z < self.min.z
            || lo.z > self.max.z
        {
            return None;
        }
        let (_, r0, _, r1) = self.cell_range(lo, hi);
        let delta = end - start;
        SEEN.with(|seen| {
            let mut seen = seen.borrow_mut();
            let stamp = seen.next_stamp(self.triangles.len());
            let mut best: Option<Hit> = None;
            for r in r0..=r1 {
                // Only the columns of this row the move passes over, so a
                // long diagonal move (a line of sight) doesn't look at the
                // whole square around it.
                let row_lo = self.min.y + r as f32 * CELL - extent.y - MARGIN;
                let row_hi = row_lo + CELL + 2.0 * (extent.y + MARGIN);
                let (x_lo, x_hi) = if delta.y.abs() < 1e-3 {
                    (lo.x, hi.x)
                } else {
                    let ta = ((row_lo - start.y) / delta.y).clamp(0.0, 1.0);
                    let tb = ((row_hi - start.y) / delta.y).clamp(0.0, 1.0);
                    let (xa, xb) = (start.x + delta.x * ta, start.x + delta.x * tb);
                    (xa.min(xb) - extent.x - MARGIN, xa.max(xb) + extent.x + MARGIN)
                };
                let (c0, _, c1, _) = self.cell_range(Vec3::new(x_lo, lo.y, 0.0), Vec3::new(x_hi, hi.y, 0.0));
                for c in c0..=c1 {
                    for &i in &self.cells[r * self.columns + c] {
                        if !seen.first_visit(i, stamp) {
                            continue;
                        }
                        let t = &self.triangles[i as usize];
                        // Quick reject: the triangle's box is outside the
                        // move's box (its box sides are among the solid's
                        // sides, so it can't be hit).
                        let t_lo = Vec3::new(
                            t[0].x.min(t[1].x).min(t[2].x),
                            t[0].y.min(t[1].y).min(t[2].y),
                            t[0].z.min(t[1].z).min(t[2].z),
                        );
                        let t_hi = Vec3::new(
                            t[0].x.max(t[1].x).max(t[2].x),
                            t[0].y.max(t[1].y).max(t[2].y),
                            t[0].z.max(t[1].z).max(t[2].z),
                        );
                        if t_hi.x < lo.x
                            || t_lo.x > hi.x
                            || t_hi.y < lo.y
                            || t_lo.y > hi.y
                            || t_hi.z < lo.z
                            || t_lo.z > hi.z
                        {
                            continue;
                        }
                        *tested += 1;
                        let (planes, n) = Self::planes(t);
                        if let Some(h) = sweep_planes(&planes[..n], extent, start, end) {
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
        })
    }
}

/// Slack added around a move's box before leaving triangles out, uu.
const MARGIN: f32 = 1.0;

/// Which triangles one sweep has already looked at (a triangle can sit in
/// several grid columns). Each sweep gets a new stamp, so nothing needs
/// clearing; one list per thread, shared by every triangle world.
#[derive(Default)]
struct Seen {
    stamp: u32,
    marks: Vec<u32>,
}

impl Seen {
    fn next_stamp(&mut self, triangles: usize) -> u32 {
        if self.marks.len() < triangles {
            self.marks.resize(triangles, 0);
        }
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            self.marks.iter_mut().for_each(|m| *m = 0);
            self.stamp = 1;
        }
        self.stamp
    }

    fn first_visit(&mut self, i: u32, stamp: u32) -> bool {
        let m = &mut self.marks[i as usize];
        if *m == stamp {
            return false;
        }
        *m = stamp;
        true
    }
}

thread_local! {
    static SEEN: std::cell::RefCell<Seen> = std::cell::RefCell::new(Seen::default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room::*;
    use crate::{MoveInput, MoveParams, Pawn};

    /// Every triangle, no grid: what the grid search must agree with.
    fn sweep_all(w: &TriangleWorld, extent: Vec3, start: Vec3, end: Vec3) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        for t in &w.triangles {
            let (planes, n) = TriangleWorld::planes(t);
            if let Some(h) = sweep_planes(&planes[..n], extent, start, end) {
                if best.map_or(true, |b| h.time < b.time || (h.start_solid && !b.start_solid)) {
                    best = Some(h);
                }
            }
        }
        best
    }

    /// The grid search (skipping far-away triangles and columns off the
    /// line) finds the same hits as trying every triangle: random
    /// triangles and random moves, long and short, near and far.
    #[test]
    fn grid_search_matches_trying_every_triangle() {
        let mut seed = 12345u64;
        let mut rnd = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as f32) / (1u64 << 31) as f32
        };
        let mut tris = Vec::new();
        for _ in 0..400 {
            let c = Vec3::new(rnd() * 4000.0 - 2000.0, rnd() * 4000.0 - 2000.0, rnd() * 600.0 - 300.0);
            let mut p = || c + Vec3::new(rnd() * 400.0 - 200.0, rnd() * 400.0 - 200.0, rnd() * 200.0 - 100.0);
            tris.push([p(), p(), p()]);
        }
        let w = TriangleWorld::new(tris);
        let mut hits = 0;
        for k in 0..3000 {
            let start = Vec3::new(rnd() * 6000.0 - 3000.0, rnd() * 6000.0 - 3000.0, rnd() * 800.0 - 400.0);
            let len = if k % 3 == 0 { 3000.0 } else { 200.0 };
            let end = start + Vec3::new(rnd() * 2.0 - 1.0, rnd() * 2.0 - 1.0, rnd() * 0.4 - 0.2) * len;
            let extent = if k % 2 == 0 { Vec3::ZERO } else { Vec3::new(40.0, 40.0, 84.0) };
            let a = w.sweep(extent, start, end);
            let b = sweep_all(&w, extent, start, end);
            assert_eq!(a.is_some(), b.is_some(), "move {k}: {start:?} to {end:?}");
            if let (Some(a), Some(b)) = (a, b) {
                hits += 1;
                assert_eq!(a.start_solid, b.start_solid, "move {k}");
                if !a.start_solid {
                    assert!((a.time - b.time).abs() < 1e-5, "move {k}: {} vs {}", a.time, b.time);
                }
            }
        }
        assert!(hits > 300, "only {hits} hits; the test should hit often");
    }

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
