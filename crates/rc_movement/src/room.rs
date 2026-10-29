//! A test room for trying the movement before real levels load.
//!
//! Unreal units, Z up, floor top at z = 0. The player starts at the origin
//! looking along +X. Ahead, at x = 1000, are side-by-side "lanes", each
//! testing one thing (y is the lane's centre line):
//!
//! | lane y | what                                   | expected            |
//! |-------:|----------------------------------------|---------------------|
//! | -1600  | ceiling 130 above the floor            | only crouched fits  |
//! |  -800  | step 30 high                           | walks up            |
//! |  -200  | step 40 high                           | blocked; jump it    |
//! |   400  | stairs: 8 steps, 16 high, 32 deep      | walks up            |
//! |  1000  | ramp at 30 degrees                     | walks up            |
//! |  1600  | ramp at 50 degrees                     | too steep           |
//!
//! Behind the start, at x = -1200, is a 700-high tower with a stair-free
//! top, for fall tests (spawn on it with the viewer's `--start` option).

use crate::{Brush, BrushWorld, Vec3};

pub const LANE_CEILING: f32 = -1600.0;
pub const LANE_STEP_30: f32 = -800.0;
pub const LANE_STEP_40: f32 = -200.0;
pub const LANE_STAIRS: f32 = 400.0;
pub const LANE_RAMP_30: f32 = 1000.0;
pub const LANE_RAMP_50: f32 = 1600.0;
/// Where the lanes' obstacles start along X.
pub const LANES_X: f32 = 1000.0;
/// Top of the fall-test tower.
pub const TOWER_TOP: Vec3 = Vec3::new(-1200.0, 0.0, 700.0);

fn cuboid(min: (f32, f32, f32), max: (f32, f32, f32)) -> Brush {
    Brush::cuboid(Vec3::new(min.0, min.1, min.2), Vec3::new(max.0, max.1, max.2))
}

/// A ramp rising `angle_deg` along +X from `x`, `rise` high, then a flat top
/// 300 long, in the lane centred on `y` (300 wide).
fn ramp(x: f32, y: f32, angle_deg: f32, rise: f32) -> Vec<Brush> {
    let run = rise / angle_deg.to_radians().tan();
    vec![
        Brush::ramp_x(Vec3::new(x, y - 150.0, 0.0), Vec3::new(x + run, y + 150.0, rise)),
        cuboid((x + run, y - 150.0, 0.0), (x + run + 300.0, y + 150.0, rise)),
    ]
}

pub fn test_room() -> BrushWorld {
    let mut b = vec![
        // Floor and four walls.
        cuboid((-2000.0, -2000.0, -16.0), (4000.0, 2000.0, 0.0)),
        cuboid((-2016.0, -2016.0, -16.0), (-2000.0, 2016.0, 600.0)),
        cuboid((4000.0, -2016.0, -16.0), (4016.0, 2016.0, 600.0)),
        cuboid((-2016.0, -2016.0, -16.0), (4016.0, -2000.0, 600.0)),
        cuboid((-2016.0, 2000.0, -16.0), (4016.0, 2016.0, 600.0)),
    ];
    let x = LANES_X;
    // Low ceiling: 130 above the floor, 400 long.
    let y = LANE_CEILING;
    b.push(cuboid((x, y - 150.0, 130.0), (x + 400.0, y + 150.0, 200.0)));
    // Steps 30 and 40 high.
    for (y, h) in [(LANE_STEP_30, 30.0), (LANE_STEP_40, 40.0)] {
        b.push(cuboid((x, y - 150.0, 0.0), (x + 400.0, y + 150.0, h)));
    }
    // Stairs, then a landing.
    let y = LANE_STAIRS;
    for i in 0..8 {
        let x0 = x + 32.0 * i as f32;
        let top = 16.0 * (i + 1) as f32;
        b.push(cuboid((x0, y - 150.0, 0.0), (x0 + 32.0, y + 150.0, top)));
    }
    b.push(cuboid((x + 256.0, y - 150.0, 0.0), (x + 556.0, y + 150.0, 128.0)));
    b.extend(ramp(x, LANE_RAMP_30, 30.0, 200.0));
    b.extend(ramp(x, LANE_RAMP_50, 50.0, 200.0));
    // Fall tower.
    let t = TOWER_TOP;
    b.push(cuboid((t.x - 200.0, t.y - 200.0, 0.0), (t.x + 200.0, t.y + 200.0, t.z)));
    BrushWorld { brushes: b }
}
