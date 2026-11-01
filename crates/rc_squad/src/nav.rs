//! Finding a way from one spot to another, and cover to hide behind.
//!
//! The squad doesn't do its own pathfinding: the enemies' crate `rc_nav`
//! (the map's path network, shared by every AI) does. Anything that can
//! answer "how do I get from here to there?" is a `Navigator`: a map's
//! `rc_nav::NavGraph`, or `StraightLine` (straight for the spot, sliding
//! along walls) when a map has no path network, and in tests.
//!
//! The squad only asks when it can't see a clear way: a short move with
//! nothing in the way goes straight (see `Squad::update`).
//!
//! A navigator also lists the map's cover points (the level designers'
//! `CoverPoint` actors, the same ones the droids use), which the squad
//! takes in a fight (`crate::cover`).

use rc_movement::Vec3;

/// One of the map's cover points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoverSpot {
    /// Which point it is (the path network's node number), so two squad
    /// mates don't take the same one.
    pub id: usize,
    /// Where a body's middle goes (as placed in the map).
    pub centre: Vec3,
    /// The way it faces: towards the danger it shields from, radians.
    pub facing: f32,
}

pub trait Navigator {
    /// Points to walk through, ending at `to`, not including `from`. `None`
    /// when there is no way there.
    fn find_path(&self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>>;

    /// Cover points within `radius` of `at`, nearest first. None by
    /// default (a map without a path network has none).
    fn cover_near(&self, _at: Vec3, _radius: f32) -> Vec<CoverSpot> {
        Vec::new()
    }
}

/// Straight to the spot.
pub struct StraightLine;

impl Navigator for StraightLine {
    fn find_path(&self, _from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        Some(vec![to])
    }
}

/// Straight to the spot, with some cover points (for tests and test rooms).
pub struct StraightWithCover(pub Vec<CoverSpot>);

impl Navigator for StraightWithCover {
    fn find_path(&self, _from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        Some(vec![to])
    }

    fn cover_near(&self, at: Vec3, radius: f32) -> Vec<CoverSpot> {
        let mut v: Vec<CoverSpot> = self
            .0
            .iter()
            .filter(|c| (c.centre - at).length() <= radius)
            .copied()
            .collect();
        v.sort_by(|a, b| {
            (a.centre - at)
                .length()
                .total_cmp(&(b.centre - at).length())
        });
        v
    }
}

impl Navigator for rc_nav::NavGraph {
    fn find_path(&self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        rc_nav::NavGraph::find_path(self, from, to)
    }

    fn cover_near(&self, at: Vec3, radius: f32) -> Vec<CoverSpot> {
        self.nodes_near(at, radius)
            .into_iter()
            .filter_map(|n| match self.nodes[n].kind {
                rc_nav::NodeKind::Cover { facing, .. } => Some(CoverSpot {
                    id: n,
                    centre: self.nodes[n].location,
                    facing,
                }),
                _ => None,
            })
            .collect()
    }
}
