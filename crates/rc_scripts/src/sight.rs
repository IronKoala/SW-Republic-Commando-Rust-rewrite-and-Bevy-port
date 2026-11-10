//! The line check for line-of-sight triggers ([`crate::Director::look`]):
//! is anything in the level between the player's eye and what a trigger
//! watches for?

use rc_movement::{Vec3, World};

/// How long a piece of a long line check is, uu (the level's collision
/// looks at everything inside the box around a check, so long diagonal
/// lines are cut up, as the droids' sight checks are).
const PIECE: f32 = 256.0;

/// How near the watched thing the check stops, uu: the thing itself may
/// be solid (a prop, a turret), and that mustn't hide it.
const STOP_SHORT: f32 = 32.0;

/// Whether a straight line from `a` to `b` is free of level geometry
/// (walls, floors, solid props, closed doors), stopping just short of `b`.
pub fn clear_line(world: &dyn World, a: Vec3, b: Vec3) -> bool {
    let len = (b - a).length();
    if len < 1.0 {
        return true;
    }
    let end = 1.0 - (STOP_SHORT / len).min(0.5);
    let pieces = ((len * end) / PIECE).ceil().max(1.0) as usize;
    for i in 0..pieces {
        let t0 = end * i as f32 / pieces as f32;
        let t1 = end * (i + 1) as f32 / pieces as f32;
        if let Some(h) = world.sweep(Vec3::ZERO, a + (b - a) * t0, a + (b - a) * t1) {
            if h.time < 0.999 {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_movement::TriangleWorld;

    #[test]
    fn a_wall_blocks_the_view() {
        // A wall across X = 500, from Y -1000 to 1000, Z -1000 to 1000.
        let p = |y: f32, z: f32| Vec3::new(500.0, y, z);
        let w = TriangleWorld::new(vec![
            [p(-1000.0, -1000.0), p(1000.0, -1000.0), p(1000.0, 1000.0)],
            [p(-1000.0, -1000.0), p(1000.0, 1000.0), p(-1000.0, 1000.0)],
        ]);
        let eye = Vec3::new(0.0, 0.0, 0.0);
        assert!(!clear_line(&w, eye, Vec3::new(1000.0, 0.0, 0.0)));
        assert!(clear_line(&w, eye, Vec3::new(400.0, 0.0, 0.0)));
        // Something standing right against the wall's near side is seen.
        assert!(clear_line(&w, eye, Vec3::new(510.0, 0.0, 0.0)));
        assert!(clear_line(&w, eye, Vec3::new(0.0, 3000.0, 0.0)));
    }
}
