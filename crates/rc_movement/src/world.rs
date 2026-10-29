//! Collision with the world.
//!
//! Like Unreal Engine 2, the player collides with level geometry as an
//! upright box: `radius` wide on X and Y, `height` tall above and below its
//! centre. The movement code only ever asks one question: "if this box slides
//! from here to there, where does it first touch something?". Anything that
//! answers it can be a world: today a set of test boxes and ramps, later the
//! level's own polygons.

use crate::Vec3;

/// The first contact of a moving box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// How far along the move the touch happens, 0 (start) to 1 (end).
    pub time: f32,
    /// Direction the touched surface faces, length 1.
    pub normal: Vec3,
    /// The box already overlapped something at the start.
    pub start_solid: bool,
}

pub trait World {
    /// Slide a box with half-size `extent` from `start` to `end`. `None` if
    /// nothing is in the way.
    fn sweep(&self, extent: Vec3, start: Vec3, end: Vec3) -> Option<Hit>;
}

/// One flat side of a solid: points `p` with `normal.dot(p) <= dist` are on
/// its inner side.
#[derive(Debug, Clone, Copy)]
pub struct Plane {
    pub normal: Vec3,
    pub dist: f32,
}

/// A convex solid: the space inside all of its planes.
#[derive(Debug, Clone)]
pub struct Brush {
    pub planes: Vec<Plane>,
    pub min: Vec3,
    pub max: Vec3,
}

impl Brush {
    /// An axis-aligned box from its lowest to its highest corner.
    pub fn cuboid(min: Vec3, max: Vec3) -> Self {
        Brush::from_planes(min, max, vec![])
    }

    /// A ramp inside the box `min`..`max`, rising along +X from height
    /// `min.z` at `min.x` to `max.z` at `max.x`.
    pub fn ramp_x(min: Vec3, max: Vec3) -> Self {
        let run = max.x - min.x;
        let rise = max.z - min.z;
        let normal = Vec3::new(-rise, 0.0, run).normalize_or_zero();
        let slope = Plane {
            normal,
            dist: normal.dot(min),
        };
        Brush::from_planes(min, max, vec![slope])
    }

    /// The box's six sides plus any extra cutting planes. The six sides also
    /// act as the "bevel" planes that keep sweeps of a box exact at edges.
    fn from_planes(min: Vec3, max: Vec3, mut extra: Vec<Plane>) -> Self {
        let mut planes = vec![
            Plane { normal: Vec3::new(1.0, 0.0, 0.0), dist: max.x },
            Plane { normal: Vec3::new(-1.0, 0.0, 0.0), dist: -min.x },
            Plane { normal: Vec3::new(0.0, 1.0, 0.0), dist: max.y },
            Plane { normal: Vec3::new(0.0, -1.0, 0.0), dist: -min.y },
            Plane { normal: Vec3::new(0.0, 0.0, 1.0), dist: max.z },
            Plane { normal: Vec3::new(0.0, 0.0, -1.0), dist: -min.z },
        ];
        planes.append(&mut extra);
        Brush { planes, min, max }
    }

    fn sweep(&self, extent: Vec3, start: Vec3, end: Vec3) -> Option<Hit> {
        sweep_planes(&self.planes, extent, start, end)
    }
}

/// Sweep a box against the convex solid inside `planes`. The solid is grown
/// by the box's size on every plane, so the box becomes a point and the sweep
/// a line.
pub(crate) fn sweep_planes(planes: &[Plane], extent: Vec3, start: Vec3, end: Vec3) -> Option<Hit> {
    {
        let delta = end - start;
        let mut enter = -1.0f32;
        let mut exit = 1.0f32;
        let mut enter_normal = Vec3::UP;
        let mut started_outside = false;
        // For a start inside the solid: the side it is least deep behind.
        let mut nearest_out = f32::MIN;
        let mut nearest_normal = Vec3::UP;
        for p in planes {
            let n = p.normal;
            let grow = n.x.abs() * extent.x + n.y.abs() * extent.y + n.z.abs() * extent.z;
            let d_start = n.dot(start) - p.dist - grow;
            let d_end = n.dot(end) - p.dist - grow;
            if d_start > nearest_out {
                nearest_out = d_start;
                nearest_normal = n;
            }
            if d_start > 0.0 {
                started_outside = true;
                if d_end >= 0.0 {
                    // Both ends outside this side: the line misses the solid.
                    return None;
                }
                let t = d_start / (d_start - d_end);
                if t > enter {
                    enter = t;
                    enter_normal = n;
                }
            } else if d_end > 0.0 {
                let t = d_start / (d_start - d_end);
                exit = exit.min(t);
            }
        }
        if !started_outside {
            // An overlap of a hair's width comes from rounding: count it as
            // touching the surface, not as being stuck inside.
            if nearest_out > -0.01 {
                if delta.dot(nearest_normal) >= 0.0 {
                    return None;
                }
                return Some(Hit {
                    time: 0.0,
                    normal: nearest_normal,
                    start_solid: false,
                });
            }
            return Some(Hit {
                time: 0.0,
                normal: nearest_normal,
                start_solid: true,
            });
        }
        if enter <= exit && (0.0..=1.0).contains(&enter) {
            Some(Hit {
                time: enter,
                normal: enter_normal,
                start_solid: false,
            })
        } else {
            None
        }
    }
}

/// A world made of convex solids, used for the test room.
#[derive(Debug, Clone, Default)]
pub struct BrushWorld {
    pub brushes: Vec<Brush>,
}

impl World for BrushWorld {
    fn sweep(&self, extent: Vec3, start: Vec3, end: Vec3) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        for b in &self.brushes {
            if let Some(h) = b.sweep(extent, start, end) {
                if h.start_solid {
                    return Some(h);
                }
                if best.map_or(true, |b| h.time < b.time) {
                    best = Some(h);
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor() -> BrushWorld {
        BrushWorld {
            brushes: vec![Brush::cuboid(
                Vec3::new(-1000.0, -1000.0, -10.0),
                Vec3::new(1000.0, 1000.0, 0.0),
            )],
        }
    }

    #[test]
    fn box_lands_on_floor() {
        let w = floor();
        let e = Vec3::new(40.0, 40.0, 84.0);
        let h = w
            .sweep(e, Vec3::new(0.0, 0.0, 184.0), Vec3::new(0.0, 0.0, -16.0))
            .unwrap();
        // Bottom of the box touches z = 0 when the centre is at 84: 100 of
        // the 200 units down.
        assert!((h.time - 0.5).abs() < 1e-5, "{h:?}");
        assert_eq!(h.normal, Vec3::UP);
        assert!(!h.start_solid);
    }

    #[test]
    fn sliding_along_floor_is_free() {
        let w = floor();
        let e = Vec3::new(40.0, 40.0, 84.0);
        let h = w.sweep(e, Vec3::new(0.0, 0.0, 84.1), Vec3::new(500.0, 0.0, 84.1));
        assert!(h.is_none());
    }

    #[test]
    fn ramp_normal_and_hit() {
        // 100 long, 50 high: about 26.6 degrees.
        let r = Brush::ramp_x(Vec3::new(0.0, -100.0, 0.0), Vec3::new(100.0, 100.0, 50.0));
        let n = r.planes[6].normal;
        assert!((n.z - 0.894).abs() < 1e-3, "{n:?}");
        // A point dropped onto the middle of the ramp hits at z = 25.
        let h = r
            .sweep(Vec3::ZERO, Vec3::new(50.0, 0.0, 100.0), Vec3::new(50.0, 0.0, 0.0))
            .unwrap();
        assert!((h.time - 0.75).abs() < 1e-4, "{h:?}");
    }
}
