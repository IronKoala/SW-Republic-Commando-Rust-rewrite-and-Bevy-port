//! The original's ambient creatures (Kreetles, Phidna): they don't fight,
//! they wander about and walk patrol routes.
//!
//! How the original does it (ctcharacters.u `Ambient` and `AmbientBot`,
//! read only; ctgame.dll, decompiled locally for reference only, never
//! copied; described in our own words):
//! - `AmbientBot` has two states, `StateIdle` and `StateWander`, and gets
//!   `StatePatrol` too when its pawn has a patrol route (`CTBot.Possess`).
//! - About every `DecisionInterval` (0.5 s, times a random 0.8 to 1.2)
//!   it asks each state how much it wants to run (`EstimateUtility`) and
//!   switches to the one that wants it most (the first on a tie). Idle
//!   wants 0.1, wander 0.2, patrol the pawn's `PatrolPriority` while the
//!   route isn't finished (0 after).
//! - A state runs a queue of goals (walk to a point, wait). When the
//!   queue is empty it adds more (`UpdateSelf`); a goal that fails clears
//!   the queue.
//! - Wander: pick a random navigation point within 2048 uu that it can
//!   reach along the paths, walk there, wait 0 to 5 s, again.
//! - Patrol: walk to the route's next point, wait its `PauseTime`, choose
//!   the next point by the patrol mode ([`crate::patrol::Patrol::next`]).
//! - `Ambient.Tick`: one not drawn on screen for `DeactivateTime` seconds
//!   (1) while walking stops (no collision, no movement) until it is
//!   drawn again. The game tells us when it was last on screen
//!   ([`Fighter::last_on_screen`]).

use super::{AiEvent, Fighter, View};
use crate::patrol::Patrol;
use rc_movement::{MoveInput, Vec3};
use rc_nav::NodeKind;
use std::collections::VecDeque;

/// Which of its states an ambient creature is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmbientState {
    Idle,
    Wander,
    Patrol,
}

impl AmbientState {
    pub fn name(self) -> &'static str {
        match self {
            AmbientState::Idle => "idle",
            AmbientState::Wander => "wander",
            AmbientState::Patrol => "patrol",
        }
    }
}

/// One goal in a state's queue.
#[derive(Debug, Clone, PartialEq)]
enum Goal {
    /// Walk to this navigation point.
    Goto(usize),
    /// Wait this many seconds (counting down).
    Sleep(f32),
}

/// An ambient creature's thinking.
#[derive(Debug, Clone)]
pub struct AmbientMind {
    pub state: Option<AmbientState>,
    /// Each state's goal queue (idle, wander, patrol).
    queues: [VecDeque<Goal>; 3],
    /// The first goal has been started (its route planned).
    started: bool,
    next_decision: f32,
    /// The patrol point it goes to next; -1 when the route is done.
    pub patrol_index: i32,
    patrol_step: i32,
    /// The pawn's `PatrolPriority` (a point can change it).
    pub patrol_priority: f32,
}

impl Default for AmbientMind {
    fn default() -> Self {
        AmbientMind {
            state: None,
            queues: Default::default(),
            started: false,
            next_decision: 0.0,
            patrol_index: 0,
            patrol_step: 1,
            patrol_priority: crate::patrol::DEFAULT_PATROL_PRIORITY,
        }
    }
}

fn slot(s: AmbientState) -> usize {
    match s {
        AmbientState::Idle => 0,
        AmbientState::Wander => 1,
        AmbientState::Patrol => 2,
    }
}

impl Fighter {
    /// Give it a patrol route (from its spawner, spawn point or map
    /// placement).
    pub fn set_patrol(&mut self, patrol: Option<Patrol>) {
        if let Some(p) = &patrol {
            self.ambient.patrol_priority = p.priority;
        }
        self.patrol = patrol;
    }

    /// `Pawn.SetWalking` through the bot's `Run`: whether it walks or runs
    /// from now on (`bIsWalking`). Without `can_run` it always walks once
    /// told anything.
    fn run(&mut self, run: bool) {
        let a = &self.params.ambient;
        let walk = !(a.can_run && run);
        self.walking = !a.can_run || walk;
    }

    fn utility(&self, s: AmbientState) -> f32 {
        let a = &self.params.ambient;
        match s {
            AmbientState::Idle => a.idle_utility,
            AmbientState::Wander => a.wander_utility,
            AmbientState::Patrol => {
                let p = self.patrol.as_ref().map_or(0, |p| p.points.len());
                if self.ambient.patrol_index != -1 && p > 0 {
                    self.ambient.patrol_priority
                } else {
                    0.0
                }
            }
        }
    }

    fn ambient_decide(&mut self, events: &mut Vec<AiEvent>) {
        let mut states = vec![AmbientState::Idle, AmbientState::Wander];
        if self.patrol.as_ref().is_some_and(|p| !p.points.is_empty()) {
            states.push(AmbientState::Patrol);
        }
        let mut best: Option<(AmbientState, f32)> = None;
        for s in states {
            let u = self.utility(s);
            if best.is_none_or(|(_, b)| u > b) {
                best = Some((s, u));
            }
        }
        let Some((to, _)) = best else { return };
        if self.ambient.state == Some(to) {
            return;
        }
        // The old state's shutdown.
        if self.ambient.state == Some(AmbientState::Patrol) {
            if self
                .patrol
                .as_ref()
                .is_some_and(|p| p.mode == crate::patrol::PatrolMode::Once)
            {
                self.ambient.patrol_index = -1;
            }
            self.invulnerable = false;
        }
        self.follower = None;
        self.ambient.started = false;
        events.push(AiEvent::ManeuverChanged {
            from: self.ambient.state.map_or("none", |s| s.name()),
            to: to.name(),
        });
        self.ambient.state = Some(to);
        // The new state's start.
        match to {
            AmbientState::Wander => {
                if self.params.ambient.can_walk {
                    self.run(false);
                }
            }
            AmbientState::Patrol => self.ambient.queues[slot(to)].push_back(Goal::Sleep(0.2)),
            AmbientState::Idle => {}
        }
    }

    /// The state's own step once its goals are done: queue more.
    fn ambient_update_self(&mut self, view: &View, events: &mut Vec<AiEvent>) {
        match self.ambient.state {
            Some(AmbientState::Wander) => self.wander_next(view),
            Some(AmbientState::Patrol) => self.patrol_next(view, events),
            _ => {}
        }
    }

    /// `StateWander`: a random point it can reach within the radius,
    /// walk there and wait.
    fn wander_next(&mut self, view: &View) {
        // The original needs the pawn's anchor (the point it stands at);
        // ours is the nearest point.
        let Some(anchor) = view.nav.nearest_node(self.pawn.location) else {
            return;
        };
        let reach = reachable(view.nav, anchor);
        let r = self.params.ambient.wander_radius;
        let here = self.pawn.location;
        let choices: Vec<usize> = (0..view.nav.nodes.len())
            .filter(|&i| {
                let n = &view.nav.nodes[i];
                reach[i]
                    && !matches!(n.kind, NodeKind::Fly)
                    && !view.nav.blocked.contains(&i)
                    && (n.location - here).length() < r
            })
            .collect();
        if choices.is_empty() {
            return;
        }
        let node = choices[self.rng.next_u32() as usize % choices.len()];
        let wait = self.rng.unit() * self.params.ambient.wander_pause_max;
        let q = &mut self.ambient.queues[slot(AmbientState::Wander)];
        q.push_back(Goal::Goto(node));
        q.push_back(Goal::Sleep(wait));
    }

    /// `StatePatrol`: the route's next point.
    fn patrol_next(&mut self, view: &View, events: &mut Vec<AiEvent>) {
        let Some(p) = self.patrol.clone() else { return };
        let idx = self.ambient.patrol_index;
        if idx > 0 {
            if let Some(e) = p
                .points
                .get(idx as usize - 1)
                .and_then(|pt| pt.event.clone())
            {
                events.push(AiEvent::SendEvent { name: e });
            }
        }
        if idx < 0 || idx as usize >= p.points.len() {
            self.ambient.patrol_index = -1;
            return;
        }
        let pt = &p.points[idx as usize];
        let q = slot(AmbientState::Patrol);
        if let Some(n) = pt.node.as_deref().and_then(|n| view.nav.find(n)) {
            // Already standing at it: no walk.
            if view.nav.nearest_node(self.pawn.location) != Some(n) {
                self.run(pt.run_to_node);
                self.ambient.queues[q].push_back(Goal::Goto(n));
            }
        }
        if pt.pause_time > 0.0 {
            self.ambient.queues[q].push_back(Goal::Sleep(pt.pause_time));
        }
        if pt.priority_override > 0.0 {
            self.ambient.patrol_priority = pt.priority_override;
        }
        if pt.make_invulnerable {
            self.invulnerable = true;
        }
        let r = self.rng.next_u32() & 0x7fff;
        let mut step = self.ambient.patrol_step;
        self.ambient.patrol_index = p.next(idx as usize, &mut step, r) as i32;
        self.ambient.patrol_step = step;
    }

    /// One frame of an ambient creature.
    pub(super) fn ambient_tick(&mut self, dt: f32, view: &View) -> Vec<AiEvent> {
        let mut events = Vec::new();
        let now = view.time;
        // Not seen for a while: stop until seen again.
        let a = self.params.ambient.clone();
        let walking_floor = self.pawn.mode == rc_movement::Mode::Walking;
        let unseen = now - self.last_on_screen;
        if a.deactivate_time != -1.0 && unseen > a.deactivate_time && walking_floor {
            self.stasis = true;
        } else if unseen <= a.deactivate_time || a.deactivate_time == -1.0 {
            self.stasis = false;
        }
        if self.stasis {
            self.pawn.velocity = Vec3::ZERO;
            return events;
        }
        if now >= self.ambient.next_decision {
            let k = 0.8 + 0.4 * self.rng.unit();
            self.ambient.next_decision = now + k * self.params.senses.decision_interval;
            self.ambient_decide(&mut events);
        }
        let mut input = MoveInput::default();
        let mut want_yaw = self.facing;
        if let Some(s) = self.ambient.state {
            let q = slot(s);
            match self.ambient.queues[q].front().cloned() {
                Some(Goal::Sleep(t)) => {
                    if t - dt <= 0.0 {
                        self.ambient.queues[q].pop_front();
                    } else if let Some(Goal::Sleep(left)) = self.ambient.queues[q].front_mut() {
                        *left = t - dt;
                    }
                }
                Some(Goal::Goto(n)) => {
                    if !self.ambient.started {
                        self.ambient.started = true;
                        if !self.walk_to_node(view, n, &mut events) {
                            // Failed: the whole queue goes.
                            self.ambient.queues[q].clear();
                            self.ambient.started = false;
                        }
                    }
                    let walk = self.walking;
                    if let Some(f) = self.follower.as_mut() {
                        f.walk = walk;
                        match f.update(&self.pawn, dt) {
                            Some(mut i) => {
                                want_yaw = i.yaw;
                                let off = super::angle_diff(want_yaw, self.facing).abs();
                                i.forward =
                                    (1.0 - off / std::f32::consts::FRAC_PI_2).clamp(0.0, 1.0);
                                input = i;
                                if f.stuck_time > super::STUCK_SECONDS {
                                    events.push(AiEvent::Stuck);
                                    self.follower = None;
                                    self.ambient.queues[q].clear();
                                    self.ambient.started = false;
                                }
                            }
                            None => {
                                self.distance_walked += f.travelled;
                                self.follower = None;
                                self.ambient.queues[q].pop_front();
                                self.ambient.started = false;
                            }
                        }
                    } else if self.ambient.started {
                        // Planned an empty route: already there.
                        self.ambient.queues[q].pop_front();
                        self.ambient.started = false;
                    }
                }
                None => {}
            }
            if self.ambient.queues[q].is_empty() {
                self.ambient_update_self(view, &mut events);
            }
        }
        let max_turn = self.params.senses.turn_rate.to_radians() * dt;
        self.facing = super::wrap(
            self.facing + super::angle_diff(want_yaw, self.facing).clamp(-max_turn, max_turn),
        );
        input.yaw = self.facing;
        input.walk = self.walking;
        self.pawn.step(dt, &input, view.world);
        events
    }
}

/// Which nodes can be walked to from `from` along walkable, unblocked
/// links (the original's `Reachable` check along the path network).
fn reachable(nav: &rc_nav::NavGraph, from: usize) -> Vec<bool> {
    let mut seen = vec![false; nav.nodes.len()];
    let mut stack = vec![from];
    seen[from] = true;
    while let Some(a) = stack.pop() {
        for l in &nav.links[a] {
            if rc_nav::reach::walkable(l.flags) && !seen[l.to] && !nav.blocked.contains(&l.to) {
                seen[l.to] = true;
                stack.push(l.to);
            }
        }
    }
    seen
}
