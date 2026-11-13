//! What the level's own walls and rooms do to a sound, as the original
//! game works it out (read from its `Engine.dll`, 2026-11-13; the
//! decompiled code stays on the PC and is never committed).
//!
//! Three things, all from the level's BSP `Model`:
//!
//! - **Is there a wall in between?** The game's `ULevel::IsAudibleAt` draws a
//!   straight line from the sound to the listener through the BSP (the
//!   level's own walls, floors and ceilings; placed meshes don't count for
//!   the usual `SoundOcclusion=OCCLUSION_Default`) with `UModel::FastLineCheck`.
//!   [`Hearing::line_clear`] is that same walk.
//! - **Which room ("zone") is a point in?** `UModel::PointRegion`, the same
//!   walk as `Bsp::point_zone`.
//! - **How many rooms apart are two zones?** When a level loads,
//!   `UGameEngine::LoadMap` fills a 64 x 64 table from each zone's
//!   `Connectivity` bits (stored in the map). The engine then scales a
//!   sound's reach by `0.85 ^ (steps * steps)` (`ULevel::CalculateRadiusMultiplier`).
//!   [`Hearing::radius_multiplier`] does the same sums.
//!
//! How the sound code uses these is in `rc_scripts_plugin::sounds`.

use crate::bsp::{count, Bsp, Vec3};
use rc_package::{Reader, Result};

/// The game's zone table size (`FZoneProperties Zones[64]`).
pub const MAX_ZONES: usize = 64;

/// `NF_NotCsg` in a node's flags: the node doesn't change "inside/outside".
const NOT_CSG: u8 = 0x01;

#[derive(Debug, Clone)]
struct Node {
    plane: [f32; 4],
    /// Back and front child, -1 for none.
    child: [i32; 2],
    not_csg: bool,
}

/// A level's walls and rooms, for sound.
#[derive(Debug, Clone)]
pub struct Hearing {
    nodes: Vec<Node>,
    /// The tree's `RootOutside`: whether the space at the root is open.
    root_outside: bool,
    /// How many zones the level has (`NumZones`); 0 means "one zone, 0".
    zones: usize,
    /// Each zone's `Connectivity` bits, as stored.
    connectivity: Vec<u64>,
    /// The engine's `ZoneDist` table: `[from][to]`.
    zone_dist: Vec<[u8; MAX_ZONES]>,
    bsp_zones: Vec<[u8; 2]>,
}

impl Hearing {
    /// Read from the level's `Model`. `object` is the model object's bytes
    /// (the same bytes `bsp::read_model` read `bsp` from).
    pub fn read(bsp: &Bsp, object: &[u8]) -> Result<Hearing> {
        let (connectivity, root_outside) = read_zones(bsp, object)?;
        let nodes = bsp
            .nodes
            .iter()
            .map(|n| Node {
                plane: n.plane,
                child: [n.back, n.front],
                not_csg: n.node_flags & NOT_CSG != 0,
            })
            .collect();
        let zones = connectivity.len();
        Ok(Hearing {
            nodes,
            root_outside,
            zones,
            zone_dist: zone_distances(&connectivity),
            connectivity,
            bsp_zones: bsp.nodes.iter().map(|n| n.zone).collect(),
        })
    }

    /// A level with no walls and one zone: everything is heard.
    pub fn open() -> Hearing {
        Hearing {
            nodes: Vec::new(),
            root_outside: true,
            zones: 0,
            connectivity: Vec::new(),
            zone_dist: zone_distances(&[]),
            bsp_zones: Vec::new(),
        }
    }

    /// How many zones the level has.
    pub fn zones(&self) -> usize {
        self.zones
    }

    /// A zone's stored `Connectivity` bits.
    pub fn connectivity(&self, zone: usize) -> Option<u64> {
        self.connectivity.get(zone).copied()
    }

    /// The zone a point (Unreal units) is in: `UModel::PointRegion`. 0
    /// when the level has no zones.
    pub fn zone_at(&self, p: Vec3) -> u8 {
        if self.nodes.is_empty() {
            return 0;
        }
        let mut i = 0usize;
        let mut front = false;
        let mut last = 0usize;
        for _ in 0..=self.nodes.len() {
            let Some(n) = self.nodes.get(i) else { break };
            last = i;
            front = plane_dot(&n.plane, p) >= 0.0;
            let c = n.child[front as usize];
            if c < 0 {
                break;
            }
            i = c as usize;
        }
        if self.zones == 0 {
            return 0;
        }
        self.bsp_zones.get(last).map_or(0, |z| z[front as usize])
    }

    /// True when nothing solid of the level's BSP lies on the straight line
    /// from `start` to `end` (Unreal units): `UModel::FastLineCheck(end,
    /// start)`. The game calls it with the sound's spot as `end` and the
    /// listener's as `start`.
    pub fn line_clear(&self, end: Vec3, start: Vec3) -> bool {
        if self.nodes.is_empty() {
            return true;
        }
        self.check(0, end, start, self.root_outside)
    }

    fn check(&self, mut node: i32, mut end: Vec3, start: Vec3, mut outside: bool) -> bool {
        while node >= 0 {
            let Some(n) = self.nodes.get(node as usize) else {
                return outside;
            };
            let d_start = plane_dot(&n.plane, start);
            let d_end = plane_dot(&n.plane, end);
            // The game tests the sign bit: +0 counts as in front, -0 not.
            let g_start = !d_start.is_sign_negative();
            let g_end = !d_end.is_sign_negative();
            if g_start != g_end {
                let t = d_start / (d_start - d_end);
                let middle = [
                    (end[0] - start[0]) * t + start[0],
                    (end[1] - start[1]) * t + start[1],
                    (end[2] - start[2]) * t + start[2],
                ];
                let out = side_outside(outside, g_end, n.not_csg);
                if !self.check(n.child[g_end as usize], middle, end, out) {
                    return false;
                }
                end = middle;
            }
            outside = side_outside(outside, g_start, n.not_csg);
            node = n.child[g_start as usize];
        }
        outside
    }

    /// How far a sound carries between two zones, as a share of its reach:
    /// `0.85 ^ (steps * steps)`, worked out in double precision as the
    /// game does.
    pub fn radius_multiplier(&self, from: u8, to: u8) -> f32 {
        let d = self
            .zone_dist
            .get(from as usize)
            .and_then(|row| row.get(to as usize))
            .copied()
            .unwrap_or(255) as i32;
        0.85f64.powf((d * d) as f64) as f32
    }

    /// The `ZoneDist` entry from one zone to another.
    pub fn zone_steps(&self, from: u8, to: u8) -> u8 {
        self.zone_dist
            .get(from as usize)
            .and_then(|row| row.get(to as usize))
            .copied()
            .unwrap_or(255)
    }
}

/// `n . p - w`, in the game's order of sums.
fn plane_dot(n: &[f32; 4], p: Vec3) -> f32 {
    ((n[1] * p[1] + n[2] * p[2]) + n[0] * p[0]) - n[3]
}

/// "Outside" on the far side of a node: a `NotCsg` node keeps it, any
/// other takes the side the point is on.
fn side_outside(outside: bool, front: bool, not_csg: bool) -> bool {
    if not_csg {
        outside
    } else {
        front
    }
}

/// The engine's zone-to-zone table, filled as `UGameEngine::LoadMap`
/// does: 0 to itself, 255 for "not reached", and for each round `d` from
/// 1 to 63 every zone reached so far gets `d`; between rounds each zone's
/// reach grows by the reach of every zone it already reaches.
fn zone_distances(connectivity: &[u64]) -> Vec<[u8; MAX_ZONES]> {
    let mut dist = vec![[255u8; MAX_ZONES]; MAX_ZONES];
    for (i, row) in dist.iter_mut().enumerate() {
        row[i] = 0;
    }
    let mut reach = [0u64; MAX_ZONES];
    for (i, r) in reach.iter_mut().enumerate() {
        *r = connectivity.get(i).copied().unwrap_or(0);
    }
    for d in 1..MAX_ZONES as u8 {
        for i in 0..MAX_ZONES {
            for j in 0..MAX_ZONES {
                if dist[i][j] > d && reach[i] & (1u64 << j) != 0 {
                    dist[i][j] = d;
                }
            }
        }
        let mut next = [0u64; MAX_ZONES];
        for i in 0..MAX_ZONES {
            for j in 0..MAX_ZONES {
                if reach[i] & (1u64 << j) != 0 {
                    next[i] |= reach[j];
                }
            }
        }
        reach = next;
    }
    dist
}

/// Each zone's `Connectivity`, and `RootOutside`, from the part of the
/// model after the vertex pool (the layout `lightmap.rs` reads).
/// A zone is stored as its actor, then `Connectivity` (8 bytes),
/// `Visibility` (8 bytes) and 4 more bytes.
fn read_zones(bsp: &Bsp, object: &[u8]) -> Result<(Vec<u64>, bool)> {
    let mut r = Reader::new(object);
    r.seek(bsp.read_end)?;
    let _num_shared_sides = r.i32()?;
    let zones = r.i32()?;
    if !(0..=MAX_ZONES as i32).contains(&zones) {
        return r.invalid(format!("zone count {zones}"));
    }
    let mut connectivity = Vec::with_capacity(zones as usize);
    for _ in 0..zones {
        r.compact_index()?;
        let b = r.bytes(20)?;
        connectivity.push(u64::from_le_bytes(b[0..8].try_into().unwrap()));
    }
    r.compact_index()?; // Polys
    let n = count(&mut r, "bounds", 25)?;
    r.bytes(n * 25)?;
    let n = count(&mut r, "leaf hulls", 4)?;
    r.bytes(n * 4)?;
    let n = count(&mut r, "leaves", 10)?;
    for _ in 0..n {
        r.compact_index()?;
        r.compact_index()?;
        r.bytes(8)?;
    }
    let n = count(&mut r, "lights", 1)?;
    for _ in 0..n {
        r.compact_index()?;
    }
    let root_outside = r.i32()? != 0;
    Ok((connectivity, root_outside))
}

/// Each zone's raw 20 bytes after its actor, for checking the layout.
pub fn zone_records(bsp: &Bsp, object: &[u8]) -> Result<Vec<[u8; 20]>> {
    let mut r = Reader::new(object);
    r.seek(bsp.read_end)?;
    r.i32()?;
    let zones = r.i32()?;
    if !(0..=MAX_ZONES as i32).contains(&zones) {
        return r.invalid(format!("zone count {zones}"));
    }
    (0..zones)
        .map(|_| {
            r.compact_index()?;
            Ok(r.bytes(20)?.try_into().unwrap())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_table_counts_rounds() {
        // 0 - 1 - 2 in a row, each connected to itself and its neighbours.
        let c = [0b011, 0b111, 0b110];
        let d = zone_distances(&c);
        assert_eq!(d[0][0], 0);
        assert_eq!(d[0][1], 1);
        assert_eq!(d[1][2], 1);
        // Reached after one round of growing.
        assert_eq!(d[0][2], 2);
        assert_eq!(d[0][3], 255);
    }

    #[test]
    fn multiplier_shrinks_with_steps() {
        let h = Hearing {
            zone_dist: zone_distances(&[0b011, 0b111, 0b110]),
            ..Hearing::open()
        };
        assert_eq!(h.radius_multiplier(0, 0), 1.0);
        assert!((h.radius_multiplier(0, 1) - 0.85).abs() < 1e-6);
        assert!((h.radius_multiplier(0, 2) - 0.85f32.powi(4)).abs() < 1e-6);
        assert!(h.radius_multiplier(0, 3) < 1e-30);
    }

    #[test]
    fn a_wall_blocks_the_line() {
        // One plane x = 0 facing +x; in front is open, behind is solid.
        let h = Hearing {
            nodes: vec![Node {
                plane: [1.0, 0.0, 0.0, 0.0],
                child: [-1, -1],
                not_csg: false,
            }],
            root_outside: false,
            bsp_zones: vec![[0, 0]],
            ..Hearing::open()
        };
        assert!(h.line_clear([10.0, 0.0, 0.0], [20.0, 5.0, 0.0]));
        assert!(!h.line_clear([10.0, 0.0, 0.0], [-20.0, 5.0, 0.0]));
    }
}
