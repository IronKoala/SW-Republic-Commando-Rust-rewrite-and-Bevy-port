//! Walking a patrol route: what a character the map gives a route
//! (`PatrolRouteFromFactory` / `PatrolRoute`) does, step by step.
//!
//! The original's patrol is compiled code (CTGame.dll `UStatePatrol`,
//! read in a disassembly for reference; nothing of it is copied here).
//! What it does, in our words:
//! - When the patrol starts it waits 0.2 seconds (`OnActivate`).
//! - Then, each time it has finished what it was doing, it first sends the
//!   `Event` of the point before the one it is on now (so a point's event
//!   goes off once the character has done everything at that point), and
//!   then queues up the next point: walk to the node's anchor (running if
//!   `RunToNode`) unless it has none, turn to the node's facing if
//!   `OrientToNode`, wait `PauseTime` if more than 0, and play
//!   `Animation` `NumAnimLoops` times (at least once) if it names one.
//!   `MakeInvulnerable` makes it unhurtable from that point until a point
//!   without it. While patrolling it fights only if the point says
//!   `ShootWhileMoving`.
//! - Which point comes next: `Loop` goes on to the next and back to the
//!   first after the last; `PingPong` turns round at either end; `Random`
//!   picks one at random from all but the last (as the original's
//!   `rand() % (count - 1)` does); `Once` stops after the last. Ping-pong
//!   and random send the event of the point numbered one less than the
//!   new one, not of the point it came from, as the original does.

use crate::spawns::{PatrolMode, PatrolPoint};
use rc_movement::Vec3;
use std::collections::VecDeque;

/// One thing to do on the way round.
#[derive(Debug, Clone, PartialEq)]
pub enum PatrolStep {
    /// Walk (or run) to a point.
    Move { to: Vec3, run: bool },
    /// Turn to face this way, radians.
    Face(f32),
    /// Stand for this many seconds.
    Wait(f32),
    /// Play this animation once.
    Anim(String),
    /// Send this event into the level.
    Event(String),
    /// From here on it can (`false`) or can't (`true`) be hurt.
    Invulnerable(bool),
    /// From here on it fights while patrolling (`ShootWhileMoving`).
    Fights(bool),
}

/// A character's patrol, as it goes.
#[derive(Debug, Clone)]
pub struct PatrolRun {
    pub points: Vec<PatrolPoint>,
    pub mode: PatrolMode,
    /// The point it goes to next (`GoalIndex`); -1 once it has stopped.
    index: i32,
    /// Which way it goes round in ping-pong (`Step`).
    dir: i32,
    queue: VecDeque<PatrolStep>,
    started: bool,
    /// A point made it unhurtable (`ChangedGodStatus`).
    invulnerable: bool,
}

impl PatrolRun {
    pub fn new(points: Vec<PatrolPoint>, mode: PatrolMode) -> Self {
        PatrolRun {
            points,
            mode,
            index: 0,
            dir: 0,
            queue: VecDeque::new(),
            started: false,
            invulnerable: false,
        }
    }

    /// It has stopped patrolling for good (`Once` after the last point).
    pub fn finished(&self) -> bool {
        self.index == -1 && self.queue.is_empty()
    }

    /// What to do next, once the last step is done; `None` when there is
    /// nothing more. `random(n)` gives a whole number from 0 up to `n`
    /// (not including it), for `Random`.
    pub fn next(&mut self, random: &mut dyn FnMut(usize) -> usize) -> Option<PatrolStep> {
        if !self.started {
            self.started = true;
            return Some(PatrolStep::Wait(0.2));
        }
        if let Some(s) = self.queue.pop_front() {
            return Some(s);
        }
        if self.points.is_empty() {
            return None;
        }
        self.update(random);
        self.queue.pop_front()
    }

    /// `UpdateSelf`: the last point's event, then the next point's steps.
    fn update(&mut self, random: &mut dyn FnMut(usize) -> usize) {
        let n = self.points.len() as i32;
        if self.index > 0 {
            if let Some(e) = self.points[(self.index - 1) as usize].event.clone() {
                self.queue.push_back(PatrolStep::Event(e));
            }
        }
        if self.index >= n || self.index == -1 {
            self.index = -1;
            return;
        }
        let p = self.points[self.index as usize].clone();
        if let Some(to) = p.anchor {
            self.queue.push_back(PatrolStep::Move { to, run: p.run });
        }
        if p.orient && p.node.is_some() {
            self.queue.push_back(PatrolStep::Face(p.node_yaw));
        }
        if p.pause > 0.0 {
            self.queue.push_back(PatrolStep::Wait(p.pause));
        }
        if let Some(a) = &p.animation {
            for _ in 0..p.loops.max(1) {
                self.queue.push_back(PatrolStep::Anim(a.clone()));
            }
        }
        self.queue
            .push_back(PatrolStep::Fights(p.shoot_while_moving));
        if p.invulnerable {
            self.invulnerable = true;
            self.queue.push_back(PatrolStep::Invulnerable(true));
        } else if self.invulnerable {
            self.invulnerable = false;
            self.queue.push_back(PatrolStep::Invulnerable(false));
        }
        match self.mode {
            PatrolMode::Loop => {
                self.index += 1;
                if self.index == n {
                    self.index = 0;
                }
            }
            PatrolMode::PingPong => {
                if self.index == n - 1 {
                    self.dir = -1;
                } else if self.index == 0 {
                    self.dir = 1;
                }
                self.index += self.dir;
            }
            PatrolMode::Random => {
                self.index = if n == 1 {
                    0
                } else {
                    random((n - 1) as usize) as i32
                };
            }
            PatrolMode::Once => self.index += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f32, event: &str) -> PatrolPoint {
        PatrolPoint {
            node: Some(format!("Node{x}")),
            anchor: Some(Vec3::new(x, 0.0, 0.0)),
            event: Some(event.into()),
            run: true,
            ..Default::default()
        }
    }

    fn all(run: &mut PatrolRun, n: usize) -> Vec<PatrolStep> {
        let mut r = |_| 0;
        (0..n).map_while(|_| run.next(&mut r)).collect()
    }

    #[test]
    fn the_geo_03c_gunner_runs_to_the_turret_then_sends_its_event() {
        // PawnFactory1: one point, SonicTurret1 (anchor: its NavPts[0],
        // SquadMarkerSpot21), RunToNode, Event GunnerGetOnTurret, mode
        // Once.
        let to = Vec3::new(3773.8977, -40467.094, 984.5386);
        let mut run = PatrolRun::new(
            vec![PatrolPoint {
                node: Some("SonicTurret1".into()),
                anchor: Some(to),
                event: Some("GunnerGetOnTurret".into()),
                run: true,
                ..Default::default()
            }],
            PatrolMode::Once,
        );
        assert_eq!(
            all(&mut run, 10),
            vec![
                PatrolStep::Wait(0.2),
                PatrolStep::Move { to, run: true },
                PatrolStep::Fights(false),
                PatrolStep::Event("GunnerGetOnTurret".into()),
            ]
        );
        assert!(run.finished());
    }

    #[test]
    fn loop_goes_round_and_ping_pong_turns_at_the_ends() {
        let pts = vec![point(0.0, "A"), point(1.0, "B"), point(2.0, "C")];
        let moves = |steps: &[PatrolStep]| {
            steps
                .iter()
                .filter_map(|s| match s {
                    PatrolStep::Move { to, .. } => Some(to.x as i32),
                    _ => None,
                })
                .take(8)
                .collect::<Vec<_>>()
        };
        let mut lp = PatrolRun::new(pts.clone(), PatrolMode::Loop);
        assert_eq!(moves(&all(&mut lp, 25)), vec![0, 1, 2, 0, 1, 2, 0, 1]);
        let mut pp = PatrolRun::new(pts.clone(), PatrolMode::PingPong);
        assert_eq!(moves(&all(&mut pp, 25)), vec![0, 1, 2, 1, 0, 1, 2, 1]);
        // Once: three points, three events, then nothing.
        let mut once = PatrolRun::new(pts, PatrolMode::Once);
        let steps = all(&mut once, 25);
        let events: Vec<_> = steps
            .iter()
            .filter_map(|s| match s {
                PatrolStep::Event(e) => Some(e.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(events, vec!["A", "B", "C"]);
        assert!(once.finished());
    }

    #[test]
    fn a_point_with_no_anchor_waits_and_animates_in_place() {
        // geo_03c's PawnFactory0: no node, TurretStandBreathe 3 times.
        let mut run = PatrolRun::new(
            vec![PatrolPoint {
                animation: Some("TurretStandBreathe".into()),
                loops: 3,
                ..Default::default()
            }],
            PatrolMode::Once,
        );
        let steps = all(&mut run, 10);
        assert_eq!(
            steps
                .iter()
                .filter(|s| matches!(s, PatrolStep::Anim(_)))
                .count(),
            3
        );
        assert!(!steps.iter().any(|s| matches!(s, PatrolStep::Move { .. })));
    }
}
