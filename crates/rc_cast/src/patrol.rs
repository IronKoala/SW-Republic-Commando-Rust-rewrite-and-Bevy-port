//! Following a patrol, as the original's AI does (ctgame.dll's
//! `UStatePatrol`, read from the compiled code; nothing here is copied
//! from it, only how it behaves, described in our own words):
//!
//! - When the patrol starts (`OnActivate`) the character first waits 0.2
//!   seconds.
//! - Each time everything it was doing is done (`UpdateSelf`, called once
//!   the goals it queued have finished) it:
//!   1. sends the `Event` of the stop it just finished (the stop before
//!      the current one; nothing for the first stop),
//!   2. stops patrolling if the current stop is past the end of the route
//!      (or the patrol already ended),
//!   3. otherwise queues the current stop: walk to its `Node` (not done
//!      here: patrols with nodes aren't read), wait `PauseTime` seconds if
//!      above 0, then play `Animation` `NumAnimLoops` times (at least
//!      once) from the start each time, waiting for each to end,
//!   4. and moves to the next stop by `PatrolMode`: `PM_Loop` goes on and
//!      wraps to the first stop after the last (so the last stop's event
//!      is never sent), `PM_PingPong` turns round at either end,
//!      `PM_Random` picks a random stop other than the last (the first
//!      when there is only one), `PM_Once` goes on past the end, which
//!      ends the patrol at the next step, after the last stop's event.
//! - A character that starts switched off (`StartState` `SS_Shutdown`,
//!   ctgame.dll's `UStateShutdown`) stands still with no physics until it
//!   is triggered (an event named by its tag: CTGame.u's `CTBot.Trigger`
//!   posts the "triggered" stimulus, which ends the shutdown); then it
//!   takes its physics back and its patrol starts.
//!
//! The flying gunships and fighters of geo_04a and geo_04d are such
//! patrols: one stop, playing their flight once, whose event the level
//! script waits for to take the ship away.

use crate::read::{Patrol, PatrolMode};
use std::collections::VecDeque;

/// The wait when a patrol starts (`UStatePatrol::OnActivate` queues a
/// 0.2 second sleep).
pub const START_WAIT: f32 = 0.2;

/// One thing the patrol is doing.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Wait(f32),
    /// Play this animation once from the start and wait for it to end.
    Anim(String),
}

/// What [`PatrolRun::advance`] asks of the character.
#[derive(Debug, Default, PartialEq)]
pub struct Next {
    /// Send this event.
    pub event: Option<String>,
    /// The patrol has ended.
    pub ended: bool,
    /// A stop asked to make it invulnerable (not done here).
    pub invulnerable: bool,
}

/// A patrol under way.
#[derive(Debug, Clone)]
pub struct PatrolRun {
    pub route: Patrol,
    /// Switched off, waiting to be triggered.
    pub asleep: bool,
    /// The current stop (`GoalIndex`); -1 once the patrol has ended.
    pub index: i32,
    /// The direction for `PM_PingPong` (`Step`).
    step: i32,
    pub goals: VecDeque<Step>,
    /// Seconds into the first goal (for waits).
    pub waited: f32,
    /// The first goal has started (for animations).
    pub started: bool,
    seed: u32,
}

impl PatrolRun {
    pub fn new(route: Patrol, asleep: bool, seed: u32) -> Self {
        let mut r = PatrolRun {
            route,
            asleep,
            index: 0,
            step: 0,
            goals: VecDeque::new(),
            waited: 0.0,
            started: false,
            seed: seed | 1,
        };
        if !asleep {
            r.start();
        }
        r
    }

    /// The patrol starts (or wakes): wait a moment first.
    pub fn start(&mut self) {
        self.asleep = false;
        self.goals.clear();
        self.goals.push_back(Step::Wait(START_WAIT));
        self.waited = 0.0;
        self.started = false;
    }

    pub fn ended(&self) -> bool {
        self.index == -1
    }

    /// The first goal is done: drop it.
    pub fn goal_done(&mut self) {
        self.goals.pop_front();
        self.waited = 0.0;
        self.started = false;
    }

    fn random(&mut self) -> u32 {
        // xorshift: any spread will do for picking a stop.
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        x
    }

    /// Everything queued is done (`UpdateSelf`): what comes next.
    pub fn advance(&mut self) -> Next {
        let mut out = Next::default();
        let points = &self.route.points;
        let n = points.len() as i32;
        if self.index > 0 {
            if let Some(p) = points.get(self.index as usize - 1) {
                out.event = p.event.clone();
            }
        }
        if self.index >= n || self.index == -1 {
            self.index = -1;
            out.ended = true;
            return out;
        }
        let p = points[self.index as usize].clone();
        if p.pause > 0.0 {
            self.goals.push_back(Step::Wait(p.pause));
        }
        if let Some(a) = &p.anim {
            for _ in 0..p.loops.max(1) {
                self.goals.push_back(Step::Anim(a.clone()));
            }
        }
        out.invulnerable = p.make_invulnerable;
        match self.route.mode {
            PatrolMode::Loop => {
                self.index += 1;
                if self.index >= n {
                    self.index -= n;
                }
            }
            PatrolMode::PingPong => {
                if self.index == n - 1 {
                    self.step = -1;
                } else if self.index == 0 {
                    self.step = 1;
                }
                self.index += self.step;
            }
            PatrolMode::Random => {
                self.index = if n == 1 {
                    0
                } else {
                    (self.random() % (n as u32 - 1)) as i32
                };
            }
            PatrolMode::Once => self.index += 1,
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read::PatrolPoint;

    fn stop(anim: &str, event: &str) -> PatrolPoint {
        PatrolPoint {
            pause: 0.0,
            anim: Some(anim.into()),
            loops: 1,
            event: Some(event.into()),
            make_invulnerable: false,
        }
    }

    /// geo_04d's fighter: one stop, its flight once, then its event and
    /// the end.
    #[test]
    fn a_flyby_flies_once_then_sends_its_event() {
        let route = Patrol {
            points: vec![stop("Walker01FlyBy", "DestroyFighterFlyby01")],
            mode: PatrolMode::Once,
        };
        let mut r = PatrolRun::new(route, false, 7);
        assert_eq!(r.goals, [Step::Wait(START_WAIT)]);
        r.goal_done();
        let a = r.advance();
        assert_eq!(a, Next::default());
        assert_eq!(r.goals, [Step::Anim("Walker01FlyBy".into())]);
        r.goal_done();
        let b = r.advance();
        assert_eq!(b.event.as_deref(), Some("DestroyFighterFlyby01"));
        assert!(b.ended && r.ended() && r.goals.is_empty());
    }

    #[test]
    fn a_shut_down_patrol_waits_to_be_woken() {
        let route = Patrol {
            points: vec![stop("Fly", "Done")],
            mode: PatrolMode::Once,
        };
        let mut r = PatrolRun::new(route, true, 1);
        assert!(r.asleep && r.goals.is_empty());
        r.start();
        assert_eq!(r.goals, [Step::Wait(START_WAIT)]);
    }

    #[test]
    fn loops_skip_the_last_event_and_ping_pong_turns_round() {
        let points = vec![stop("A", "EA"), stop("B", "EB")];
        let mut l = PatrolRun::new(
            Patrol {
                points: points.clone(),
                mode: PatrolMode::Loop,
            },
            false,
            1,
        );
        let mut events = Vec::new();
        for _ in 0..5 {
            l.goals.clear();
            events.push(l.advance().event);
        }
        assert_eq!(
            events,
            [None, Some("EA".into()), None, Some("EA".into()), None]
        );
        let mut p = PatrolRun::new(
            Patrol {
                points: vec![stop("A", "EA"), stop("B", "EB"), stop("C", "EC")],
                mode: PatrolMode::PingPong,
            },
            false,
            1,
        );
        let mut at = Vec::new();
        for _ in 0..6 {
            at.push(p.index);
            p.advance();
        }
        assert_eq!(at, [0, 1, 2, 1, 0, 1]);
    }

    #[test]
    fn loops_count_and_pauses_queue_in_order() {
        let mut s = stop("Spin", "E");
        s.loops = 3;
        s.pause = 1.5;
        let mut r = PatrolRun::new(
            Patrol {
                points: vec![s],
                mode: PatrolMode::Once,
            },
            false,
            1,
        );
        r.goals.clear();
        r.advance();
        assert_eq!(
            r.goals,
            [
                Step::Wait(1.5),
                Step::Anim("Spin".into()),
                Step::Anim("Spin".into()),
                Step::Anim("Spin".into())
            ]
        );
    }
}
