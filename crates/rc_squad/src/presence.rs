//! Squad mates leaving and rejoining the squad.
//!
//! In the original, the squad is whoever the map puts in it: squad mates
//! placed in the map join the player's squad when the level starts, and
//! the level's scripts take them out and put them back with
//! `ACTION_ChangeSquadMember` (CTGame.u: it calls the squad's
//! `RemoveFromSquad` / `AddToSquad` for every pawn with the `MemberTag`).
//! While out of the squad a mate no longer follows the player or takes
//! orders; the level's scripts walk them about instead (Fixer slicing the
//! final door on ras_02b, Sev waiting to be found on geo_01c). Mates made
//! by a spawner start out of the squad until a script adds them.
//!
//! Each [`Member`](crate::squad::Member) carries a [`Presence`]. Away and
//! gone members are skipped by everything the squad does (orders, marked
//! jobs, cover, fighting, reviving). An away member stands where they are
//! (gravity still applies) unless a script sent them somewhere with
//! [`Squad::script_move`], which they walk to with the squad's own
//! path-finding and steering; [`SquadEvent::ScriptArrived`] says when they
//! got there.

use crate::nav::Navigator;
use crate::squad::{Leader, MarkerState, MemberState, Squad, SquadEvent};
use rc_movement::{MoveInput, Pawn, Vec3, World};
use std::f32::consts::{PI, TAU};

/// Where a squad mate is, as far as the squad goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// With the squad: follows the player, takes orders.
    InSquad,
    /// In the level but out of the squad: the level's scripts have them.
    Away,
    /// Not in the level (not arrived yet, or taken out).
    Gone,
}

impl Presence {
    pub fn name(self) -> &'static str {
        match self {
            Presence::InSquad => "in_squad",
            Presence::Away => "away",
            Presence::Gone => "gone",
        }
    }

    pub fn from_name(s: &str) -> Option<Presence> {
        Some(match s {
            "in_squad" => Presence::InSquad,
            "away" => Presence::Away,
            "gone" => Presence::Gone,
            _ => return None,
        })
    }
}

/// A level script's walk for a mate away from the squad.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScriptMove {
    /// Where to stand (feet), Unreal units.
    pub to: Vec3,
    /// Walk rather than run.
    pub walk: bool,
    /// Turn to this facing on arrival.
    pub face: Option<f32>,
    /// The script action that sent them, handed back on arrival.
    pub index: usize,
}

fn angle_diff(a: f32, b: f32) -> f32 {
    (a - b + PI).rem_euclid(TAU) - PI
}

impl Squad {
    /// Bring member `i` into the level standing at `feet` facing `yaw`,
    /// in the squad or not.
    pub fn arrive(&mut self, i: usize, feet: Vec3, yaw: f32, in_squad: bool) -> Vec<SquadEvent> {
        let Some(m) = self.members.get_mut(i) else {
            return Vec::new();
        };
        m.pawn = Pawn::new(m.pawn.params.clone(), feet);
        m.yaw = yaw;
        m.goal = feet;
        m.hold_point = feet;
        m.script_move = None;
        m.path.clear();
        m.planned_for = None;
        m.state = MemberState::Idle;
        m.presence = if in_squad {
            Presence::InSquad
        } else {
            Presence::Away
        };
        m.think_in = 0.0;
        vec![SquadEvent::CameIn {
            member: m.settings.name.clone(),
            at: feet,
            in_squad,
        }]
    }

    /// Put member `i` in the squad (`join`) or take them out of it. Only
    /// a member in the level can join; nothing happens when they're
    /// already where they're asked to be.
    pub fn set_in_squad(&mut self, i: usize, join: bool) -> Vec<SquadEvent> {
        let mut events = Vec::new();
        let Some(m) = self.members.get(i) else {
            return events;
        };
        match (m.presence, join) {
            (Presence::Away, true) => {
                let m = &mut self.members[i];
                m.presence = Presence::InSquad;
                m.script_move = None;
                m.state = MemberState::Idle;
                m.think_in = 0.0;
                m.hold_point = m.pawn.feet();
                events.push(SquadEvent::JoinedSquad {
                    member: m.settings.name.clone(),
                    at: m.pawn.feet(),
                });
            }
            (Presence::InSquad, false) => {
                self.leave_cover(i, "left the squad", &mut events);
                let m = &mut self.members[i];
                m.presence = Presence::Away;
                m.engaging = None;
                m.revive = None;
                m.fight.target = None;
                m.state = MemberState::Idle;
                m.path.clear();
                m.planned_for = None;
                m.goal = m.pawn.feet();
                if let Some(k) = m.marker.take() {
                    self.markers[k].1 = MarkerState::Open;
                    events.push(SquadEvent::MarkerCancelled {
                        member: self.members[i].settings.name.clone(),
                        marker: format!("{} (left the squad)", self.markers[k].0.name),
                    });
                }
                if self.player_revive.is_some_and(|(j, _)| j == i) {
                    self.player_revive = None;
                }
                let m = &self.members[i];
                events.push(SquadEvent::LeftSquad {
                    member: m.settings.name.clone(),
                    at: m.pawn.feet(),
                });
            }
            _ => {}
        }
        events
    }

    /// Take member `i` out of the level (a script's `DestroyPawn`).
    pub fn remove(&mut self, i: usize) -> Vec<SquadEvent> {
        let mut events = self.set_in_squad(i, false);
        if let Some(m) = self.members.get_mut(i) {
            if m.presence != Presence::Gone {
                m.presence = Presence::Gone;
                m.script_move = None;
                events.push(SquadEvent::Removed {
                    member: m.settings.name.clone(),
                });
            }
        }
        events
    }

    /// A level script sends member `i` (away from the squad) to `mv.to`.
    /// Returns false when they aren't away (in the squad, the squad leads
    /// them; not in the level).
    pub fn script_move(&mut self, i: usize, mv: ScriptMove) -> bool {
        let Some(m) = self.members.get_mut(i) else {
            return false;
        };
        if m.presence != Presence::Away || m.down {
            return false;
        }
        m.script_move = Some(mv);
        m.goal = mv.to;
        m.path.clear();
        m.planned_for = None;
        m.slow_time = 0.0;
        m.tried_jump = false;
        m.stuck_reported = false;
        m.state = MemberState::Moving {
            urgent: !mv.walk,
            by_order: true,
        };
        true
    }

    /// A level script puts member `i` at `feet` at once (`TeleportToPoint`).
    pub fn script_teleport(&mut self, i: usize, feet: Vec3, yaw: f32) {
        if let Some(m) = self.members.get_mut(i) {
            m.pawn = Pawn::new(m.pawn.params.clone(), feet);
            m.yaw = yaw;
            m.goal = feet;
            m.hold_point = feet;
            m.path.clear();
            m.planned_for = None;
        }
    }

    /// A level script turns member `i` (away from the squad) to `yaw`.
    pub fn script_face(&mut self, i: usize, yaw: f32) {
        if let Some(m) = self.members.get_mut(i) {
            if m.presence == Presence::Away {
                m.yaw = yaw.rem_euclid(TAU);
            }
        }
    }

    /// One frame for a member away from the squad or out of the level.
    pub(crate) fn step_away(
        &mut self,
        i: usize,
        dt: f32,
        _leader: Leader,
        world: &dyn World,
        nav: &dyn Navigator,
        events: &mut Vec<SquadEvent>,
    ) {
        if self.members[i].presence == Presence::Gone {
            return;
        }
        if self.members[i].down {
            let m = &mut self.members[i];
            m.input = MoveInput::default();
            m.pawn.step(dt, &m.input, world);
            return;
        }
        // Arrived where the script sent them?
        if let Some(mv) = self.members[i].script_move {
            let m = &self.members[i];
            let feet = m.pawn.feet();
            let d = Vec3::new(feet.x - mv.to.x, feet.y - mv.to.y, 0.0).length();
            // Within reach, or stuck long enough that the squad would have
            // given up a marked job: done either way, so the script
            // carries on.
            let stuck = m.slow_time > crate::squad::MARKER_GIVE_UP;
            if d <= self
                .settings
                .arrive_radius
                .max(self.settings.waypoint_radius)
                || stuck
            {
                let m = &mut self.members[i];
                m.script_move = None;
                m.state = MemberState::Idle;
                m.goal = feet;
                if let Some(face) = mv.face {
                    m.yaw = face.rem_euclid(TAU);
                }
                events.push(SquadEvent::ScriptArrived {
                    member: m.settings.name.clone(),
                    index: mv.index,
                    at: feet,
                });
            }
        }
        let facing = self.members[i].yaw;
        self.steer(i, dt, facing, false, world, nav, events);
        if self.members[i].script_move.is_none() {
            // Stand still where they are, facing the way they were turned.
            let m = &mut self.members[i];
            let d = angle_diff(facing, m.yaw);
            m.yaw = (m.yaw + d).rem_euclid(TAU);
        }
        let m = &mut self.members[i];
        m.pawn.step(dt, &m.input, world);
    }
}
