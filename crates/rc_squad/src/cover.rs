//! Taking cover in a fight.
//!
//! The original's squad hides behind the map's cover points (the level
//! designers' `CoverPoint` actors, which the droids use too) and crouches
//! there between bursts. How it picks one is compiled code we don't read,
//! so the rules here are our own. The numbers they use are the game's
//! where it stores them:
//!
//! - when: a member who has been fighting for the order's
//!   `TimeBeforeSeekingCover` (SquadStance 6 s, StanceFormUp 2 s,
//!   StanceSecurePosition and StanceSearchAndDestroy 0 s) looks for cover;
//! - how far: only cover within the order's `PreventRange` of what the
//!   order ties them to (the player for form up, the spot for secure area,
//!   where they stood for hold), and, for orders with
//!   `FavorCombatNearestTether` (form up, secure area), the cover nearest
//!   that tether rather than nearest themselves;
//! - which way: a cover point shields from enemies inside its
//!   `FOVdegrees` (140, CoverPoint) around the way it faces;
//! - how long: after `CoverBoredomTime` (15 s, CTBot) at one point they
//!   look for another.
//!
//! Every number is in `[cover]` and the `[stance.*]` blocks of
//! `data/squad/squad.toml`. See docs/DESIGN.md, "Taking cover".

use crate::nav::{CoverSpot, Navigator};
use crate::squad::{Leader, MemberState, Squad, SquadEvent};
use rc_movement::{Pawn, Vec3, World};

/// A cover point a squad mate holds or is going to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TakenCover {
    pub spot: CoverSpot,
    /// Where their feet go: on the floor under the point.
    pub feet: Vec3,
}

/// Seconds before looking again after finding no cover. Our own.
const COVER_RETRY: f32 = 1.0;
/// At most this many cover points get a line-of-sight test per look. Our
/// own, to keep each think cheap.
const MAX_SIGHT_TESTS: usize = 12;

/// Where the eyes are when standing, even while crouched: in cover they
/// stand up to shoot, so that's where they see from.
pub(crate) fn standing_eye(pawn: &Pawn) -> Vec3 {
    pawn.feet()
        + Vec3::new(
            0.0,
            0.0,
            pawn.params.collision_height + pawn.params.base_eye_height,
        )
}

/// The floor under a map point placed at a body's middle.
fn floor_under(world: &dyn World, pawn: &Pawn, centre: Vec3) -> Vec3 {
    let e = Vec3::new(
        pawn.params.collision_radius,
        pawn.params.collision_radius,
        pawn.params.collision_height,
    );
    let start = centre + Vec3::new(0.0, 0.0, 20.0);
    let down = centre - Vec3::new(0.0, 0.0, 400.0);
    match world.sweep(e, start, down) {
        Some(h) if !h.start_solid => start + (down - start) * h.time - Vec3::new(0.0, 0.0, e.z),
        _ => centre - Vec3::new(0.0, 0.0, e.z),
    }
}

impl Squad {
    /// Member `i` is at their cover point (standing still on it).
    pub fn in_cover(&self, i: usize) -> bool {
        let m = &self.members[i];
        match m.cover {
            Some(c) => {
                m.state == MemberState::Idle
                    && (m.pawn.feet() - c.feet).length_2d() <= self.settings.arrive_radius * 2.0
            }
            None => false,
        }
    }

    /// Give up member `i`'s cover point, if they have one.
    pub(crate) fn leave_cover(&mut self, i: usize, why: &str, events: &mut Vec<SquadEvent>) {
        let m = &mut self.members[i];
        m.crouch = false;
        if m.cover.take().is_some() {
            m.cover_time = 0.0;
            m.planned_for = None;
            if m.state == MemberState::Idle {
                // Back to their place in the order at the next think.
                m.think_in = 0.0;
            }
            events.push(SquadEvent::CoverLeft {
                member: m.settings.name.clone(),
                why: why.into(),
            });
        }
    }

    /// Every frame: the fight and cover clocks.
    pub(crate) fn tick_cover(&mut self, i: usize, dt: f32) {
        let in_cover = self.in_cover(i);
        let calm_time = self.settings.cover.calm_time;
        let m = &mut self.members[i];
        if m.fight.target.is_some() {
            m.fight_time += dt;
            m.calm_time = 0.0;
        } else {
            m.calm_time += dt;
            if m.calm_time >= calm_time {
                m.fight_time = 0.0;
            }
        }
        if in_cover {
            m.cover_time += dt;
        }
        m.cover_retry_in -= dt;
    }

    /// Every think: take, keep, change or leave cover.
    pub(crate) fn think_cover(
        &mut self,
        i: usize,
        leader: Leader,
        world: &dyn World,
        nav: &dyn Navigator,
        events: &mut Vec<SquadEvent>,
    ) {
        let c = self.settings.cover.clone();
        let st = self.stance_settings().clone();
        let m = &self.members[i];
        if !c.use_cover || m.down || m.revive.is_some() || m.marker.is_some() {
            let why = if m.down {
                "down"
            } else if m.revive.is_some() {
                "going to revive someone"
            } else {
                "on a marked job"
            };
            self.leave_cover(i, why, events);
            return;
        }
        let tether = self.tether_point(i, leader);
        if let Some(have) = m.cover {
            if m.fight.target.is_none() && m.calm_time >= c.calm_time {
                self.leave_cover(i, "no enemy in sight", events);
                return;
            }
            let from_tether = (have.feet - tether).length_2d();
            if from_tether > st.prevent_range {
                let why = format!(
                    "{from_tether:.0} uu from what the order ties them to (allowed {:.0})",
                    st.prevent_range
                );
                self.leave_cover(i, &why, events);
                return;
            }
            if self.in_cover(i) && m.cover_time >= c.boredom_time {
                // Bored: another point if there is one, else stay.
                self.members[i].cover_time = 0.0;
                if let Some(target) = self.target_of(i).copied() {
                    if let Some((next, enemy_d)) =
                        self.pick_cover(i, target.eye, tether, world, nav, Some(have.spot.id))
                    {
                        self.take_cover(i, next, enemy_d, "bored at the last one", events);
                    }
                }
            }
            return;
        }
        let Some(target) = self.target_of(i).copied() else {
            return;
        };
        if m.fight_time < st.seek_cover_after || m.cover_retry_in > 0.0 {
            return;
        }
        match self.pick_cover(i, target.eye, tether, world, nav, None) {
            Some((next, enemy_d)) => self.take_cover(i, next, enemy_d, "under fire", events),
            None => self.members[i].cover_retry_in = COVER_RETRY,
        }
    }

    fn take_cover(
        &mut self,
        i: usize,
        cover: TakenCover,
        enemy_distance: f32,
        why: &str,
        events: &mut Vec<SquadEvent>,
    ) {
        let m = &mut self.members[i];
        let distance = (cover.feet - m.pawn.feet()).length();
        m.cover = Some(cover);
        m.cover_time = 0.0;
        m.planned_for = None;
        events.push(SquadEvent::CoverTaken {
            member: m.settings.name.clone(),
            at: cover.feet,
            distance,
            enemy_distance,
            why: why.into(),
        });
    }

    /// The best free cover point for member `i` against an enemy whose
    /// eyes are at `enemy`: near enough what they're tied to, facing the
    /// enemy, and with a clear shot from there when standing. Returns it
    /// with the enemy's distance from it.
    fn pick_cover(
        &self,
        i: usize,
        enemy: Vec3,
        tether: Vec3,
        world: &dyn World,
        nav: &dyn Navigator,
        not: Option<usize>,
    ) -> Option<(TakenCover, f32)> {
        let c = &self.settings.cover;
        let st = self.stance_settings();
        let m = &self.members[i];
        let feet = m.pawn.feet();
        let sight = self.fight_for(i).sight_radius;
        let half_fov_cos = (c.fov * 0.5).cos();
        let eye_up = m.pawn.params.base_eye_height;
        let mut tests = 0;
        let mut best: Option<(TakenCover, f32, f32)> = None;
        for spot in nav.cover_near(feet, c.search_radius) {
            if Some(spot.id) == not {
                continue;
            }
            let taken = self
                .members
                .iter()
                .enumerate()
                .any(|(j, o)| j != i && o.cover.is_some_and(|t| t.spot.id == spot.id));
            if taken {
                continue;
            }
            if (spot.centre - tether).length_2d() > st.prevent_range {
                continue;
            }
            let to_enemy = enemy - spot.centre;
            let d = to_enemy.length();
            if d < c.min_enemy_distance || d > sight {
                continue;
            }
            let flat = Vec3::new(to_enemy.x, to_enemy.y, 0.0).normalize_or_zero();
            let fwd = Vec3::new(spot.facing.cos(), spot.facing.sin(), 0.0);
            if flat.dot(fwd) < half_fov_cos {
                continue;
            }
            let cost = if st.cover_near_tether {
                (spot.centre - tether).length()
            } else {
                (spot.centre - feet).length()
            };
            if best.as_ref().is_some_and(|b| b.1 <= cost) {
                continue;
            }
            tests += 1;
            if tests > MAX_SIGHT_TESTS {
                break;
            }
            // A clear shot when standing up from it.
            let eye = spot.centre + Vec3::new(0.0, 0.0, eye_up);
            if world.sweep(Vec3::ZERO, eye, enemy).is_some() {
                continue;
            }
            let feet = floor_under(world, &m.pawn, spot.centre);
            best = Some((TakenCover { spot, feet }, cost, d));
        }
        best.map(|(t, _, d)| (t, d))
    }
}
