//! Finding a way from one spot to another.
//!
//! The squad doesn't do its own pathfinding: the enemies' crate `rc_nav`
//! (the map's path network, shared by every AI) does. Anything that can
//! answer "how do I get from here to there?" is a `Navigator`: a map's
//! `rc_nav::NavGraph`, or `StraightLine` (straight for the spot, sliding
//! along walls) when a map has no path network, and in tests.
//!
//! The squad only asks when it can't see a clear way: a short move with
//! nothing in the way goes straight (see `Squad::update`).

use rc_movement::Vec3;

pub trait Navigator {
    /// Points to walk through, ending at `to`, not including `from`. `None`
    /// when there is no way there.
    fn find_path(&self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>>;
}

/// Straight to the spot.
pub struct StraightLine;

impl Navigator for StraightLine {
    fn find_path(&self, _from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        Some(vec![to])
    }
}

impl Navigator for rc_nav::NavGraph {
    fn find_path(&self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        rc_nav::NavGraph::find_path(self, from, to)
    }
}
