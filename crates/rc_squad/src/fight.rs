//! Squad mates fighting and reviving.
//!
//! The original decides all of this in compiled code (`ctgame.dll`), which
//! we don't read, so the rules here are our own. Where a number comes from
//! the game's data it says so in `squad.toml`. The rules, in short:
//!
//! - **Picking a target.** Every think (`think_interval`), each member who
//!   is up and not busy with a job looks for enemies it can see: within
//!   `sight_radius`, with nothing solid between its eyes and the enemy's.
//!   It takes the nearest. Seeing nothing, it drops its target.
//! - **Shooting.** After `reaction_time` seconds on a new target it fires
//!   bursts: `burst_count` shots `shot_interval` apart, then a pause
//!   between `burst_pause_min` and `burst_pause_max`. Each shot is aimed
//!   at the enemy's middle, off by up to `(1 - accuracy) x max_aim_error`.
//!   Standing members turn to face their target and only fire within
//!   `max_aim_yaw` of where they face; moving members fire on the move.
//!   Nobody fires when the player or another squad mate is within
//!   `friendly_clearance` of the line of fire, nearer than the target.
//! - **Reviving.** A downed squad mate (or the player, with
//!   `revive_player`) gets the nearest member who is up, not on a marked
//!   job and not already reviving someone, if one is within `range`. That
//!   member runs over, and once within `reach` of the body kneels for
//!   `time` seconds; then the patient is back up with `health` of their
//!   maximum. The player revives a squad mate by holding Use within
//!   `reach` of them for `player_time` seconds.
//!
//! Everything is in Unreal units with Z up, like the rest of `rc_squad`.

use crate::squad::{Leader, MemberState, Squad, SquadEvent};
use rc_movement::{Vec3, World};
use std::fmt;

/// An enemy the squad knows about this frame. `id` is whatever the game
/// uses to name it (the plugin passes the Bevy entity's bits), so shots
/// and kills can be matched up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Threat {
    pub id: u64,
    pub feet: Vec3,
    /// The middle of its body: what members aim at.
    pub centre: Vec3,
    /// Its eyes: what has to be visible to see it.
    pub eye: Vec3,
}

/// Someone down and waiting to be revived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Patient {
    /// A squad mate, by index in `Squad::members`.
    Mate(usize),
    /// The squad leader (the player).
    Leader,
}

/// A small random number source (xorshift), so runs repeat exactly.
#[derive(Debug, Clone)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Rng(seed.wrapping_mul(2_654_435_761).max(1))
    }

    /// 0 up to (not including) 1.
    pub fn next_f32(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Between `a` and `b`.
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next_f32()
    }
}

/// One member's fighting state.
#[derive(Debug, Clone, Default)]
pub struct FightState {
    /// The enemy they're shooting at, by `Threat::id`.
    pub target: Option<u64>,
    /// Seconds since they picked this target.
    pub seen_for: f32,
    /// Shots left in this burst.
    pub burst_left: u32,
    /// Squad time of the next allowed shot.
    pub next_shot: f32,
    /// Shots fired, all game.
    pub shots: u32,
    /// Times they held fire because a friend was in the way (for the log).
    pub held_fire: u32,
}

impl fmt::Display for Patient {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Patient::Mate(i) => write!(f, "squad mate {i}"),
            Patient::Leader => write!(f, "the player"),
        }
    }
}

fn dist(a: Vec3, b: Vec3) -> f32 {
    (a - b).length()
}

fn dist_2d(a: Vec3, b: Vec3) -> f32 {
    Vec3::new(a.x - b.x, a.y - b.y, 0.0).length()
}

/// How far `p` is from the segment `a`..`b`, and how far along it (0..1).
fn point_segment(p: Vec3, a: Vec3, b: Vec3) -> (f32, f32) {
    let ab = b - a;
    let len2 = ab.dot(ab);
    let t = if len2 > 0.0 {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (dist(p, a + ab * t), t)
}

/// Height of a standing commando's middle above its feet, and of a downed
/// one lying on the floor, uu.
const STANDING_MIDDLE: f32 = 84.0;
const LYING_MIDDLE: f32 = 30.0;

impl Squad {
    /// The name the log uses for a patient.
    pub fn patient_name(&self, p: Patient) -> String {
        match p {
            Patient::Mate(i) => self
                .members
                .get(i)
                .map_or_else(|| format!("member {i}"), |m| m.settings.name.clone()),
            Patient::Leader => "the player".into(),
        }
    }

    fn patient_down(&self, p: Patient) -> bool {
        match p {
            Patient::Mate(i) => self.members.get(i).is_some_and(|m| m.down),
            Patient::Leader => self.leader_down,
        }
    }

    pub(crate) fn patient_feet(&self, p: Patient, leader: Leader) -> Vec3 {
        match p {
            Patient::Mate(i) => self.members[i].pawn.feet(),
            Patient::Leader => leader.feet,
        }
    }

    /// Where reviver `i` kneels: beside the patient, on the reviver's side.
    pub(crate) fn revive_spot(&self, i: usize, p: Patient, leader: Leader) -> Vec3 {
        let body = self.patient_feet(p, leader);
        let from = self.members[i].pawn.feet() - body;
        let d = from.length_2d();
        let keep = self.settings.revive.reach * 0.6;
        if d <= keep {
            return self.members[i].pawn.feet();
        }
        body + Vec3::new(from.x / d * keep, from.y / d * keep, 0.0)
    }

    /// Drop revive jobs whose patient is up again, and send the nearest
    /// free member to each downed body nobody is seeing to.
    pub(crate) fn assign_revivers(&mut self, leader: Leader, events: &mut Vec<SquadEvent>) {
        for i in 0..self.members.len() {
            let Some(p) = self.members[i].revive else {
                continue;
            };
            if !self.patient_down(p) || self.members[i].down {
                let name = self.patient_name(p);
                let m = &mut self.members[i];
                m.revive = None;
                if matches!(
                    m.state,
                    MemberState::Reviving { .. } | MemberState::Moving { .. }
                ) {
                    m.state = MemberState::Idle;
                }
                m.think_in = 0.0;
                events.push(SquadEvent::ReviveCancelled {
                    member: m.settings.name.clone(),
                    patient: name,
                    why: if m.down {
                        "went down".into()
                    } else {
                        "already back up".into()
                    },
                });
            }
        }
        let mut patients: Vec<Patient> = Vec::new();
        if self.leader_down && self.settings.revive.revive_player {
            patients.push(Patient::Leader);
        }
        patients.extend(
            (0..self.members.len())
                .filter(|&i| self.members[i].down)
                .map(Patient::Mate),
        );
        for p in patients {
            if self.members.iter().any(|m| m.revive == Some(p)) {
                continue;
            }
            let body = self.patient_feet(p, leader);
            let best = self
                .members
                .iter()
                .enumerate()
                .filter(|(j, m)| {
                    !m.down
                        && m.marker.is_none()
                        && m.revive.is_none()
                        && Patient::Mate(*j) != p
                        && dist(m.pawn.feet(), body) <= self.settings.revive.range
                })
                .min_by(|a, b| dist(a.1.pawn.feet(), body).total_cmp(&dist(b.1.pawn.feet(), body)))
                .map(|(j, m)| (j, dist(m.pawn.feet(), body)));
            let Some((j, distance)) = best else {
                continue;
            };
            let patient = self.patient_name(p);
            let m = &mut self.members[j];
            m.revive = Some(p);
            m.fight.target = None;
            m.state = MemberState::Moving {
                urgent: true,
                by_order: true,
            };
            m.planned_for = None;
            m.think_in = 0.0;
            events.push(SquadEvent::ReviveSetOff {
                member: m.settings.name.clone(),
                patient,
                distance,
            });
        }
    }

    /// Reviver `i` is close enough to the body: start kneeling. Called by
    /// `think` while moving.
    pub(crate) fn reached_patient(&self, i: usize, leader: Leader) -> bool {
        let m = &self.members[i];
        let Some(p) = m.revive else { return false };
        let body = self.patient_feet(p, leader);
        dist_2d(m.pawn.feet(), body) <= self.settings.revive.reach
            && (m.pawn.feet().z - body.z).abs() < 150.0
    }

    /// Count down a revive; at the end the patient is up (the game applies
    /// the health when it sees `ReviveDone`).
    pub(crate) fn tick_revive(&mut self, i: usize, dt: f32, events: &mut Vec<SquadEvent>) {
        let MemberState::Reviving { left } = self.members[i].state else {
            return;
        };
        let left = left - dt;
        if left > 0.0 {
            self.members[i].state = MemberState::Reviving { left };
            return;
        }
        let p = self.members[i]
            .revive
            .take()
            .expect("reviving members have a patient");
        let patient_name = self.patient_name(p);
        let m = &mut self.members[i];
        m.state = MemberState::Idle;
        m.think_in = 0.0;
        events.push(SquadEvent::ReviveDone {
            member: m.settings.name.clone(),
            patient: p,
            patient_name,
            seconds: self.settings.revive.time,
        });
    }

    /// The player holding (or not) the Use button this frame. While held
    /// within `reach` of a downed squad mate, it counts up to
    /// `player_time`, then that mate is revived.
    pub fn player_use(&mut self, dt: f32, leader: Leader, holding: bool) -> Vec<SquadEvent> {
        let mut events = Vec::new();
        if self.leader_down {
            holding_stopped(self, &mut events, "the player is down");
            return events;
        }
        let near = (0..self.members.len())
            .filter(|&i| self.members[i].down)
            .map(|i| (i, dist_2d(self.members[i].pawn.feet(), leader.feet)))
            .filter(|(i, d)| {
                *d <= self.settings.revive.reach
                    && (self.members[*i].pawn.feet().z - leader.feet.z).abs() < 150.0
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i);
        match (holding, near) {
            (true, Some(i)) => {
                let progress = match self.player_revive {
                    Some((k, t)) if k == i => t + dt,
                    _ => {
                        events.push(SquadEvent::PlayerReviveStarted {
                            patient: self.members[i].settings.name.clone(),
                        });
                        dt
                    }
                };
                if progress >= self.settings.revive.player_time {
                    self.player_revive = None;
                    events.push(SquadEvent::ReviveDone {
                        member: "the player".into(),
                        patient: Patient::Mate(i),
                        patient_name: self.members[i].settings.name.clone(),
                        seconds: progress,
                    });
                } else {
                    self.player_revive = Some((i, progress));
                }
            }
            (true, None) => holding_stopped(self, &mut events, "moved away"),
            (false, _) => holding_stopped(self, &mut events, "let go of Use"),
        }
        events
    }

    /// The enemy member `i` is fighting, if it's still about.
    pub(crate) fn target_of(&self, i: usize) -> Option<&Threat> {
        let id = self.members[i].fight.target?;
        self.threats.iter().find(|t| t.id == id)
    }

    /// Pick (or drop) member `i`'s target. Called every think.
    pub(crate) fn pick_target(
        &mut self,
        i: usize,
        world: &dyn World,
        events: &mut Vec<SquadEvent>,
    ) {
        let m = &self.members[i];
        let busy = m.down
            || m.revive.is_some()
            || matches!(
                m.state,
                MemberState::Working { .. } | MemberState::Reviving { .. }
            );
        let eye = m.pawn.eye();
        let sight = self.settings.fight.sight_radius;
        let best = if busy {
            None
        } else {
            self.threats
                .iter()
                .filter(|t| dist(t.centre, eye) <= sight)
                .filter(|t| world.sweep(Vec3::ZERO, eye, t.eye).is_none())
                .min_by(|a, b| dist(a.centre, eye).total_cmp(&dist(b.centre, eye)))
                .map(|t| (t.id, dist(t.centre, eye)))
        };
        let m = &mut self.members[i];
        match (m.fight.target, best) {
            (Some(old), Some((id, _))) if old == id => {}
            (_, Some((id, distance))) => {
                m.fight.target = Some(id);
                m.fight.seen_for = 0.0;
                m.fight.burst_left = self.settings.fight.burst_count;
                events.push(SquadEvent::TargetPicked {
                    member: m.settings.name.clone(),
                    target: id,
                    distance,
                });
            }
            (Some(_), None) => {
                m.fight.target = None;
                if !busy {
                    events.push(SquadEvent::TargetLost {
                        member: m.settings.name.clone(),
                    });
                }
            }
            (None, None) => {}
        }
    }

    /// Member `i` shoots at its target if it's time and the line is clear.
    pub(crate) fn shoot(
        &mut self,
        i: usize,
        dt: f32,
        leader: Leader,
        events: &mut Vec<SquadEvent>,
    ) {
        let f = self.settings.fight.clone();
        let Some(target) = self.target_of(i).copied() else {
            self.members[i].fight.target = None;
            return;
        };
        let now = self.time;
        let m = &self.members[i];
        if m.down
            || matches!(
                m.state,
                MemberState::Working { .. } | MemberState::Reviving { .. }
            )
        {
            return;
        }
        let origin = m.pawn.eye() - Vec3::new(0.0, 0.0, 12.0);
        let to = target.centre - origin;
        let moving = matches!(m.state, MemberState::Moving { .. });
        let off_facing = {
            let want = to.y.atan2(to.x);
            let d = (want - m.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            d.abs()
        };
        self.members[i].fight.seen_for += dt;
        let m = &self.members[i];
        if m.fight.seen_for < f.reaction_time || now < m.fight.next_shot {
            return;
        }
        if !moving && off_facing > f.max_aim_yaw {
            return;
        }
        // Friends in the way: the leader and the other squad mates.
        let range = to.length();
        let mut friends: Vec<(String, Vec3)> = vec![(
            "the player".into(),
            leader.feet
                + Vec3::new(
                    0.0,
                    0.0,
                    if self.leader_down {
                        LYING_MIDDLE
                    } else {
                        STANDING_MIDDLE
                    },
                ),
        )];
        for (j, o) in self.members.iter().enumerate() {
            if j != i {
                let up = if o.down {
                    LYING_MIDDLE
                } else {
                    STANDING_MIDDLE
                };
                friends.push((
                    o.settings.name.clone(),
                    o.pawn.feet() + Vec3::new(0.0, 0.0, up),
                ));
            }
        }
        let blocker = friends.into_iter().find(|(_, p)| {
            let (d, t) = point_segment(*p, origin, target.centre);
            d < f.friendly_clearance && t * range < range - 1.0 && t > 0.0
        });
        if let Some((friend, _)) = blocker {
            let m = &mut self.members[i];
            m.fight.held_fire += 1;
            m.fight.next_shot = now + f.shot_interval;
            if m.fight.held_fire % 20 == 1 {
                events.push(SquadEvent::HeldFire {
                    member: m.settings.name.clone(),
                    friend,
                });
            }
            return;
        }
        // Aim error, then fire.
        let err = (1.0 - f.accuracy).clamp(0.0, 1.0) * f.max_aim_error;
        let yaw = to.y.atan2(to.x) + self.rng.range(-err, err);
        let pitch = to.z.atan2(to.length_2d()) + self.rng.range(-err, err) * 0.5;
        let direction = Vec3::new(
            pitch.cos() * yaw.cos(),
            pitch.cos() * yaw.sin(),
            pitch.sin(),
        );
        let pause = self.rng.range(f.burst_pause_min, f.burst_pause_max);
        let m = &mut self.members[i];
        let burst_start = m.fight.burst_left == f.burst_count;
        m.fight.shots += 1;
        m.fight.burst_left = m.fight.burst_left.saturating_sub(1);
        if m.fight.burst_left == 0 {
            m.fight.burst_left = f.burst_count;
            m.fight.next_shot = now + pause;
        } else {
            m.fight.next_shot = now + f.shot_interval;
        }
        events.push(SquadEvent::Fire {
            member: m.settings.name.clone(),
            index: i,
            origin,
            direction,
            target: target.id,
            distance: range,
            burst_start,
        });
    }
}

fn holding_stopped(squad: &mut Squad, events: &mut Vec<SquadEvent>, why: &str) {
    if let Some((i, t)) = squad.player_revive.take() {
        events.push(SquadEvent::PlayerReviveStopped {
            patient: squad.members[i].settings.name.clone(),
            after: t,
            why: why.into(),
        });
    }
}
