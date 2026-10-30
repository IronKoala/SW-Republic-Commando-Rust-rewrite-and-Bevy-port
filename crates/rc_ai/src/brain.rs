//! One computer-controlled fighter: its body, what it knows about its
//! enemy, and what it decides to do each frame.
//!
//! The original's decision making lives in its compiled engine
//! (`CTBot` and the `State...Attack` classes are "native"), so the rules
//! below are our own, shaped by what the script packages do tell us: the
//! names and numbers of the settings (sight radius, peripheral vision,
//! enemy memory, decision interval, ready and safe distance, burst count,
//! cover boredom time), the battle droid's attack state ("move a set
//! distance at a time when closing in", "a time limit per maneuver") and
//! the spawners' "stationary attack" start state. See docs/DESIGN.md
//! "Enemies" for the rules in plain language.
//!
//! Nothing here knows about Bevy. Each frame the caller passes a `View`
//! (the level, the path network, where the enemy is) and gets back
//! `AiEvent`s (shots fired, state changes) to act on and log.

use crate::params::AiParams;
use crate::spawns::StartState;
use rc_movement::{MoveInput, Pawn, Vec3, World};
use rc_nav::{NavGraph, NodeKind, PathFollower};
use std::collections::HashSet;
use std::sync::Arc;

/// A small, fast random number source (xorshift), so runs repeat exactly
/// for the same seed and no extra library is needed.
#[derive(Debug, Clone)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Rng(seed.wrapping_mul(2_654_435_761).max(1))
    }
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// 0 to 1.
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
}

/// What the AI is doing, broadly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Standing where it was put, looking out.
    Idle,
    /// Has just noticed the enemy, about to react.
    Alerted,
    /// Fighting.
    Attack,
    /// Lost the enemy: walking to where it was last seen.
    Investigate,
    Dead,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Idle => "idle",
            Mode::Alerted => "alerted",
            Mode::Attack => "attack",
            Mode::Investigate => "investigate",
            Mode::Dead => "dead",
        }
    }
}

/// What it is doing within a fight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Maneuver {
    /// Stand and shoot.
    Hold,
    /// Walk closer, `move_interval` at a time.
    Advance,
    /// Walk away to get back to `ready_distance`.
    BackOff,
    /// Walk to a spot with a clear shot.
    Reposition,
    /// Walk to cover point `node`.
    ToCover(usize),
    /// At cover point `node`: crouch, stand up to fire.
    InCover(usize),
    /// Rushing at the enemy to hit it up close.
    Charge,
}

impl Maneuver {
    pub fn name(self) -> &'static str {
        match self {
            Maneuver::Hold => "hold",
            Maneuver::Advance => "advance",
            Maneuver::BackOff => "back off",
            Maneuver::Reposition => "reposition",
            Maneuver::ToCover(_) => "to cover",
            Maneuver::InCover(_) => "in cover",
            Maneuver::Charge => "charge",
        }
    }
}

/// The enemy, as the caller sees it this frame (Unreal units).
#[derive(Debug, Clone, Copy)]
pub struct Target {
    /// Centre of its collision box (what we aim at).
    pub centre: Vec3,
    /// Its eyes (what has to be visible for us to see it, and it us).
    pub eye: Vec3,
}

/// Everything the AI may look at this frame.
pub struct View<'a> {
    pub world: &'a dyn World,
    pub nav: &'a NavGraph,
    /// Game time, seconds.
    pub time: f32,
    pub target: Option<Target>,
    /// Cover points other fighters hold or are walking to.
    pub cover_taken: &'a HashSet<usize>,
    /// Centres of friendly fighters (may include this one), so it doesn't
    /// shoot through them.
    pub allies: &'a [Vec3],
}

/// Something the caller should act on or log.
#[derive(Debug, Clone, PartialEq)]
pub enum AiEvent {
    ModeChanged {
        from: Mode,
        to: Mode,
    },
    ManeuverChanged {
        from: &'static str,
        to: &'static str,
    },
    /// Saw the enemy (first time, or again after losing it).
    Spotted {
        distance: f32,
    },
    LostSight,
    /// Started walking a route: number of nodes and length, uu.
    Route {
        nodes: usize,
        length: f32,
    },
    NoRoute,
    /// A shot. `origin` is the muzzle, `direction` has length 1,
    /// `aim_error` is how far off a perfect aim it is, degrees.
    Fire {
        origin: Vec3,
        direction: Vec3,
        aim_error: f32,
    },
    /// Couldn't make progress along a route for 2 s; gave up on it.
    Stuck,
    /// A close-up hit (a force pike): along `direction` (length 1) from
    /// `origin`, reaching `range` uu.
    Melee {
        origin: Vec3,
        direction: Vec3,
        range: f32,
    },
    /// Threw a grenade (`grenade` is its file id): it leaves the hand at
    /// `origin` with `velocity` (uu/s), aimed to land near `aim_at`.
    /// `left` is how many it still carries.
    Throw {
        grenade: String,
        origin: Vec3,
        velocity: Vec3,
        aim_at: Vec3,
        left: u32,
    },
}

/// One fighter.
#[derive(Debug, Clone)]
pub struct Fighter {
    pub id: u32,
    pub params: Arc<AiParams>,
    pub pawn: Pawn,
    pub start_state: StartState,
    /// Where it was placed (stationary fighters come back here).
    pub home: Vec3,
    pub mode: Mode,
    pub maneuver: Maneuver,
    /// Facing, radians (Unreal convention).
    pub facing: f32,
    /// Crouching this frame (in cover between bursts).
    pub crouch: bool,
    // What it knows about the enemy.
    pub enemy_seen_now: bool,
    pub last_seen_at: Option<Vec3>,
    pub last_seen_time: f32,
    alerted_at: f32,
    // Plans.
    follower: Option<PathFollower>,
    next_decision: f32,
    maneuver_started: f32,
    /// Was shot while out in the open; look for cover at the next decision.
    wants_cover: bool,
    /// No searching for a new spot before this time.
    next_search: f32,
    // Shooting.
    burst_left: u32,
    next_shot: f32,
    // Counters, for the log.
    pub shots_fired: u32,
    pub distance_walked: f32,
    /// Since when a friend has been standing in its line of fire.
    ally_in_way_since: Option<f32>,
    /// Can't act before this battle time (stunned by a grenade).
    pub stunned_until: f32,
    /// Which of the battle's targets it fought last (0 is the player).
    pub target_index: Option<usize>,
    /// Grenades it still carries.
    pub grenades_left: u32,
    /// No grenade before this battle time.
    next_throw: f32,
    /// Battle time of its last throw (for the animation).
    pub last_throw: f32,
    /// No close-up hit before this battle time.
    next_melee: f32,
    /// Battle time of its last close-up hit (for the animation).
    pub last_melee: f32,
    rng: Rng,
}

const STUCK_SECONDS: f32 = 2.0;
/// How far off the floor a charge's straight-line check starts, uu (about
/// a stair step).
const STEP_LIFT: f32 = 24.0;
/// A new spot closer than this isn't worth walking to, uu.
const MIN_MOVE: f32 = 150.0;
/// After failing to find a route or spot, wait this long before trying
/// again, s (keeps the log and the CPU quiet).
const RETRY_AFTER: f32 = 3.0;

impl Fighter {
    /// A fighter standing with its feet at `feet`, facing `yaw`.
    pub fn new(
        id: u32,
        params: Arc<AiParams>,
        feet: Vec3,
        yaw: f32,
        start_state: StartState,
    ) -> Self {
        let pawn = Pawn::new(params.movement.clone(), feet);
        let home = pawn.location;
        let grenades_left = if params.grenades.grenade.is_empty() {
            0
        } else {
            params.grenades.count
        };
        Fighter {
            id,
            burst_left: params.combat.burst_count,
            params,
            pawn,
            start_state,
            home,
            mode: Mode::Idle,
            maneuver: Maneuver::Hold,
            facing: yaw,
            crouch: false,
            enemy_seen_now: false,
            last_seen_at: None,
            last_seen_time: f32::NEG_INFINITY,
            alerted_at: 0.0,
            follower: None,
            next_decision: 0.0,
            maneuver_started: 0.0,
            wants_cover: false,
            next_search: 0.0,
            next_shot: 0.0,
            shots_fired: 0,
            distance_walked: 0.0,
            ally_in_way_since: None,
            stunned_until: f32::NEG_INFINITY,
            target_index: None,
            grenades_left,
            next_throw: 0.0,
            last_throw: f32::NEG_INFINITY,
            next_melee: 0.0,
            last_melee: f32::NEG_INFINITY,
            rng: Rng::new(id.wrapping_add(17)),
        }
    }

    pub fn eye(&self) -> Vec3 {
        self.pawn.eye()
    }

    pub fn is_moving(&self) -> bool {
        self.pawn.velocity.length_2d() > 20.0
    }

    /// Where it is walking to, if anywhere.
    pub fn destination(&self) -> Option<Vec3> {
        self.follower
            .as_ref()
            .and_then(|f| f.points.last().copied())
    }

    /// The cover point it holds or is walking to.
    pub fn cover_node(&self) -> Option<usize> {
        match self.maneuver {
            Maneuver::ToCover(n) | Maneuver::InCover(n) => Some(n),
            _ => None,
        }
    }

    /// Something hurt it. `from` is where the shot came from, if known.
    /// It turns to fight, and if out in the open, looks for cover.
    pub fn on_damaged(&mut self, from: Option<Vec3>, time: f32, events: &mut Vec<AiEvent>) {
        if self.mode == Mode::Dead {
            return;
        }
        if let Some(p) = from {
            if self.last_seen_at.is_none() || time - self.last_seen_time > 1.0 {
                self.last_seen_at = Some(p);
                self.last_seen_time = time;
            }
        }
        if matches!(self.mode, Mode::Idle | Mode::Investigate) {
            self.set_mode(Mode::Alerted, events);
            self.alerted_at = time - self.params.senses.reaction_time * 0.5;
        }
        // Being shot in the open is a reason to find cover now.
        if !matches!(self.maneuver, Maneuver::ToCover(_) | Maneuver::InCover(_)) {
            self.next_decision = time;
            self.wants_cover = true;
        }
    }

    /// It heard fighting (a shot) at `at`. If it wasn't fighting already,
    /// it turns that way and gets ready, as if it had glimpsed the enemy
    /// there. Shots farther than its hearing radius are ignored.
    pub fn hear(&mut self, at: Vec3, time: f32, events: &mut Vec<AiEvent>) {
        if !matches!(self.mode, Mode::Idle | Mode::Investigate) {
            return;
        }
        if (at - self.pawn.location).length() > self.params.senses.hearing_radius {
            return;
        }
        self.last_seen_at = Some(at);
        self.last_seen_time = time;
        self.alerted_at = time;
        self.follower = None;
        self.set_mode(Mode::Alerted, events);
    }

    /// Put it straight into the alerted state toward `at`, however far
    /// away (a saved fight being put back).
    pub fn alert(&mut self, at: Vec3, time: f32, events: &mut Vec<AiEvent>) {
        self.last_seen_at = Some(at);
        self.last_seen_time = time;
        self.alerted_at = time;
        self.follower = None;
        self.set_mode(Mode::Alerted, events);
    }

    pub fn kill(&mut self, events: &mut Vec<AiEvent>) {
        self.set_mode(Mode::Dead, events);
        self.follower = None;
    }

    fn set_mode(&mut self, to: Mode, events: &mut Vec<AiEvent>) {
        if self.mode != to {
            events.push(AiEvent::ModeChanged {
                from: self.mode,
                to,
            });
            self.mode = to;
        }
    }

    fn set_maneuver(&mut self, to: Maneuver, time: f32, events: &mut Vec<AiEvent>) {
        if self.maneuver != to {
            events.push(AiEvent::ManeuverChanged {
                from: self.maneuver.name(),
                to: to.name(),
            });
            self.maneuver = to;
            self.maneuver_started = time;
        }
    }

    /// Of several enemies, the one to fight: the one it fought last while
    /// it can still see it (so it doesn't flick between targets every
    /// frame), else the nearest it can see. `None` if it sees none.
    /// **Our choice:** the original's target picking is in its compiled
    /// engine.
    pub fn choose_target(&mut self, world: &dyn World, targets: &[Target]) -> Option<Target> {
        let s = &self.params.senses;
        let eye = self.eye();
        let fighting = matches!(self.mode, Mode::Alerted | Mode::Attack);
        let (fwd, _) = rc_movement::axes(self.facing);
        let visible = |t: &Target| {
            let to = t.eye - eye;
            let dist = to.length();
            if dist > s.sight_radius {
                return false;
            }
            if dist > s.auto_detect_radius
                && !fighting
                && to.flat().normalize_or_zero().dot(fwd) < s.peripheral_vision
            {
                return false;
            }
            clear_line(world, eye, t.eye)
        };
        if let Some(i) = self.target_index {
            if let Some(t) = targets.get(i).filter(|t| visible(t)) {
                return Some(*t);
            }
        }
        let best = targets
            .iter()
            .enumerate()
            .filter(|(_, t)| visible(t))
            .min_by(|a, b| {
                let da = (a.1.centre - self.pawn.location).length();
                let db = (b.1.centre - self.pawn.location).length();
                da.total_cmp(&db)
            });
        self.target_index = best.map(|(i, _)| i);
        best.map(|(_, t)| *t)
    }

    /// Can it see the target right now?
    fn sees(&self, view: &View, t: &Target) -> bool {
        let s = &self.params.senses;
        let eye = self.eye();
        let to = t.eye - eye;
        let dist = to.length();
        if dist > s.sight_radius {
            return false;
        }
        // The view cone only limits noticing someone new. Once fighting, it
        // keeps track of its enemy whenever a straight line is clear (the
        // way Unreal Engine 2 bots treat their current enemy).
        let fighting = matches!(self.mode, Mode::Alerted | Mode::Attack);
        if dist > s.auto_detect_radius && !fighting {
            let (fwd, _) = rc_movement::axes(self.facing);
            let dir = to.flat().normalize_or_zero();
            if dir.dot(fwd) < s.peripheral_vision {
                return false;
            }
        }
        clear_line(view.world, eye, t.eye)
    }

    /// One frame: look, decide, move, shoot. Returns what happened.
    pub fn tick(&mut self, dt: f32, view: &View) -> Vec<AiEvent> {
        let mut events = Vec::new();
        if self.mode == Mode::Dead {
            return events;
        }
        let now = view.time;
        let senses = self.params.senses.clone();

        // 1. Look.
        let saw = view.target.filter(|t| self.sees(view, t));
        if let Some(t) = saw {
            if !self.enemy_seen_now {
                events.push(AiEvent::Spotted {
                    distance: (t.centre - self.pawn.location).length(),
                });
            }
            self.enemy_seen_now = true;
            self.last_seen_at = Some(t.centre);
            self.last_seen_time = now;
            if matches!(self.mode, Mode::Idle | Mode::Investigate) {
                self.alerted_at = now;
                self.set_mode(Mode::Alerted, &mut events);
            }
        } else if self.enemy_seen_now {
            self.enemy_seen_now = false;
            events.push(AiEvent::LostSight);
        }

        // 2. Decide.
        match self.mode {
            Mode::Alerted if now - self.alerted_at >= senses.reaction_time => {
                self.set_mode(Mode::Attack, &mut events);
                self.next_decision = now;
            }
            Mode::Attack if now - self.last_seen_time > senses.enemy_memory_time => {
                self.set_mode(Mode::Investigate, &mut events);
                self.set_maneuver(Maneuver::Hold, now, &mut events);
                self.follower = None;
                if let Some(p) = self.last_seen_at {
                    self.walk_to(view, p, &mut events);
                }
            }
            Mode::Attack if now >= self.next_decision => {
                self.next_decision = now + senses.decision_interval;
                self.decide(view, &mut events);
            }
            Mode::Investigate if self.follower.is_none() => {
                self.set_mode(Mode::Idle, &mut events);
            }
            _ => {}
        }

        // Charging and now within reach: stop and hit.
        if self.maneuver == Maneuver::Charge {
            if let Some(t) = saw {
                if (t.centre - self.pawn.location).length_2d()
                    <= self.params.combat.melee_range * 0.7
                {
                    self.follower = None;
                    self.set_maneuver(Maneuver::Hold, now, &mut events);
                }
            }
        }

        // 3. Turn and move.
        let mut input = MoveInput::default();
        let enemy_dir = self
            .last_seen_at
            .map(|p| rc_nav::steer::yaw_towards(self.pawn.location, p));
        let mut want_yaw = self.facing;
        if let Some(f) = self.follower.as_mut() {
            match f.update(&self.pawn, dt) {
                Some(mut i) => {
                    want_yaw = i.yaw;
                    // Droids can't strafe: turn first, then walk. Walk at
                    // full speed only when facing the way it's going.
                    let off = angle_diff(want_yaw, self.facing).abs();
                    i.forward = (1.0 - off / std::f32::consts::FRAC_PI_2).clamp(0.0, 1.0);
                    i.yaw = self.facing;
                    input = i;
                    if f.stuck_time > STUCK_SECONDS {
                        events.push(AiEvent::Stuck);
                        self.follower = None;
                    }
                }
                None => {
                    self.distance_walked += f.travelled;
                    self.follower = None;
                    self.arrived(now, &mut events);
                }
            }
        } else if self.mode == Mode::Attack || self.mode == Mode::Alerted {
            if let Some(y) = enemy_dir {
                want_yaw = y;
            }
        }
        let max_turn = senses.turn_rate.to_radians() * dt;
        self.facing =
            wrap(self.facing + angle_diff(want_yaw, self.facing).clamp(-max_turn, max_turn));
        input.yaw = self.facing;
        input.crouch = self.crouch;
        self.pawn.step(dt, &input, view.world);

        // 4. Shoot, or hit up close.
        if self.mode == Mode::Attack {
            if let (Some(t), true) = (saw, self.enemy_seen_now) {
                if !self.melee(view.world, now, &t, &mut events)
                    && !self.params.combat.weapon.is_empty()
                {
                    self.shoot(view.world, view.allies, now, &t, &mut events);
                }
            }
        }
        events
    }

    /// Reached the end of a route.
    fn arrived(&mut self, now: f32, events: &mut Vec<AiEvent>) {
        match self.maneuver {
            Maneuver::ToCover(n) => self.set_maneuver(Maneuver::InCover(n), now, events),
            Maneuver::Advance | Maneuver::BackOff | Maneuver::Reposition => {
                self.set_maneuver(Maneuver::Hold, now, events)
            }
            _ => {}
        }
    }

    fn walk_to(&mut self, view: &View, to: Vec3, events: &mut Vec<AiEvent>) -> bool {
        match view.nav.route(self.pawn.location, to) {
            Some(r) => {
                events.push(AiEvent::Route {
                    nodes: r.nodes.len(),
                    length: r.length,
                });
                self.follower = Some(PathFollower::new(r.points));
                true
            }
            None => {
                events.push(AiEvent::NoRoute);
                false
            }
        }
    }

    fn walk_to_node(&mut self, view: &View, node: usize, events: &mut Vec<AiEvent>) -> bool {
        let to = view.nav.nodes[node].location;
        let Some(a) = view.nav.nearest_node(self.pawn.location) else {
            return false;
        };
        match view.nav.route_between(self.pawn.location, a, node, to) {
            Some(mut r) => {
                // The node itself is the goal; don't list it twice.
                r.points.pop();
                events.push(AiEvent::Route {
                    nodes: r.nodes.len(),
                    length: r.length,
                });
                self.follower = Some(PathFollower::new(r.points));
                true
            }
            None => {
                events.push(AiEvent::NoRoute);
                false
            }
        }
    }

    /// The fight: choose what to do next. Runs every `decision_interval`.
    fn decide(&mut self, view: &View, events: &mut Vec<AiEvent>) {
        let now = view.time;
        let c = self.params.combat.clone();
        let Some(enemy) = self.last_seen_at else {
            return;
        };
        let dist = (enemy - self.pawn.location).length();
        let busy = self.follower.is_some();
        let elapsed = now - self.maneuver_started;

        // A maneuver that runs too long is dropped.
        if busy && elapsed > c.max_maneuver_time {
            self.follower = None;
            self.set_maneuver(Maneuver::Hold, now, events);
            return;
        }
        // A grenade, now and then, at an enemy it knows the place of.
        if self.try_grenade(view, enemy, events) {
            return;
        }
        // A melee fighter rushes in when the enemy is close enough (always,
        // if it has no gun), following it as it moves.
        if c.melee_range > 0.0
            && now - self.last_seen_time < 1.0
            && (c.weapon.is_empty() || dist <= c.lunge_distance)
        {
            if dist <= c.melee_range * 0.7 {
                self.follower = None;
                self.set_maneuver(Maneuver::Hold, now, events);
            } else if self.charge(view, enemy, events) {
                self.set_maneuver(Maneuver::Charge, now, events);
            }
            return;
        }
        if self.maneuver == Maneuver::Charge {
            // Lost it: stop rushing.
            self.follower = None;
            self.set_maneuver(Maneuver::Hold, now, events);
        }
        // Walking somewhere for a clear shot, and now it has one: stop and
        // shoot.
        if busy
            && self.enemy_seen_now
            && matches!(self.maneuver, Maneuver::Reposition | Maneuver::Advance)
            && dist <= c.safe_distance.max(c.weapon_range * 0.5)
        {
            self.follower = None;
            self.set_maneuver(Maneuver::Hold, now, events);
            return;
        }
        if busy {
            return;
        }
        let stationary = self.start_state == StartState::StationaryAttack;
        let unseen_for = now - self.last_seen_time;

        // In cover: stay until bored, crouching between bursts.
        if let Maneuver::InCover(_) = self.maneuver {
            if elapsed < c.cover_boredom_time && unseen_for < c.lost_sight_reposition * 2.0 {
                return;
            }
            self.set_maneuver(Maneuver::Hold, now, events);
        }

        // Shot at in the open, or bored: look for cover facing the enemy.
        if std::mem::take(&mut self.wants_cover)
            || (self.maneuver == Maneuver::Hold && elapsed > c.cover_boredom_time)
        {
            if let Some(n) = self.pick_cover(view, enemy) {
                if self.walk_to_node(view, n, events) {
                    self.set_maneuver(Maneuver::ToCover(n), now, events);
                    return;
                }
            }
        }

        // No clear shot for a while: move to a spot that has one, or else
        // a step towards where the enemy was last seen.
        // A friend in the line of fire counts as no clear shot.
        let blocked_for = self.ally_in_way_since.map_or(0.0, |t| now - t);
        if unseen_for.max(blocked_for) > c.lost_sight_reposition && now >= self.next_search {
            if let Some(n) = self.pick_firing_spot(view, enemy) {
                if self.walk_to_node(view, n, events) {
                    self.set_maneuver(Maneuver::Reposition, now, events);
                    return;
                }
            }
            let dir = (enemy - self.pawn.location).normalize_or_zero();
            let goal = self.pawn.location + dir * c.move_interval.min(dist);
            if self.walk_to(view, goal, events) && self.destination_far_enough() {
                self.set_maneuver(Maneuver::Advance, now, events);
                return;
            }
            self.follower = None;
            self.next_search = now + RETRY_AFTER;
        }

        if stationary {
            self.set_maneuver(Maneuver::Hold, now, events);
            return;
        }

        // Keep between ready and safe distance: close in a step at a time,
        // or back away if the enemy is too close.
        if dist > c.safe_distance {
            let dir = (enemy - self.pawn.location).normalize_or_zero();
            let step = (dist - c.ready_distance).min(c.move_interval);
            let goal = self.pawn.location + dir * step;
            if self.walk_to(view, goal, events) {
                self.set_maneuver(Maneuver::Advance, now, events);
                return;
            }
        } else if dist < c.ready_distance * 0.5 {
            let dir = (self.pawn.location - enemy).normalize_or_zero();
            let goal = self.pawn.location + dir * c.move_interval;
            if self.walk_to(view, goal, events) {
                self.set_maneuver(Maneuver::BackOff, now, events);
                return;
            }
        }
        self.set_maneuver(Maneuver::Hold, now, events);
    }

    /// Maybe throw a grenade at the enemy last seen at `enemy`. **Our
    /// rules** (the original decides in its compiled engine; the numbers
    /// are its `GrenadeProbability`, `MinGrenadeTime` and the grenade's
    /// `ValidRange`): it must carry one, its last throw must be
    /// `min_time` ago, it must have seen the enemy within `seen_within`
    /// seconds, between `min_range` and `max_range` away, and no friend
    /// may stand where the grenade will land. Then it throws with chance
    /// `probability`, if it finds an arc the level doesn't block.
    fn try_grenade(&mut self, view: &View, enemy: Vec3, events: &mut Vec<AiEvent>) -> bool {
        let g = self.params.grenades.clone();
        let now = view.time;
        if self.grenades_left == 0 || g.grenade.is_empty() || now < self.next_throw {
            return false;
        }
        if now - self.last_seen_time > g.seen_within {
            return false;
        }
        let dist = (enemy - self.pawn.location).length_2d();
        if dist < g.min_range || dist > g.max_range {
            return false;
        }
        // Aim at the floor by the enemy: `enemy` is the middle of its body.
        let aim_at = enemy - Vec3::new(0.0, 0.0, 60.0);
        let me = self.pawn.location;
        if view
            .allies
            .iter()
            .any(|&p| (p - me).length() >= 1.0 && (p - aim_at).length_2d() < g.radius + 50.0)
        {
            return false;
        }
        if self.rng.unit() >= g.probability {
            return false;
        }
        let (fwd, right) = rc_movement::axes(self.facing);
        let origin = self.eye() - Vec3::new(0.0, 0.0, 10.0) + fwd * 30.0 + right * 10.0;
        // A little aim error, as with its shots.
        let c = &self.params.combat;
        let miss = (1.0 - c.accuracy).clamp(0.0, 1.0) * c.max_aim_error;
        let spread = dist * miss.to_radians().tan();
        let spin = self.rng.unit() * std::f32::consts::TAU;
        let r = spread * self.rng.unit().sqrt();
        let aim_at = aim_at + Vec3::new(r * spin.cos(), r * spin.sin(), 0.0);
        let Some(velocity) = throw_arc(view.world, origin, aim_at, g.speed, g.gravity) else {
            // Blocked or too far: try again later rather than every decision.
            self.next_throw = now + RETRY_AFTER;
            return false;
        };
        self.grenades_left -= 1;
        self.next_throw = now + g.min_time;
        self.last_throw = now;
        // Busy throwing: no shots for a moment.
        self.next_shot = self.next_shot.max(now + g.throw_time);
        self.burst_left = c.burst_count;
        events.push(AiEvent::Throw {
            grenade: g.grenade.clone(),
            origin,
            velocity,
            aim_at,
            left: self.grenades_left,
        });
        true
    }

    /// Head straight at the enemy if nothing is in the way, else along
    /// the paths. Re-planned at every decision, since the enemy moves.
    fn charge(&mut self, view: &View, enemy: Vec3, events: &mut Vec<AiEvent>) -> bool {
        let here = self.pawn.location;
        let goal = Vec3::new(enemy.x, enemy.y, here.z);
        // The body, lifted off the floor by a step's height (so standing on
        // the floor, or a small step, doesn't count as in the way).
        let lift = Vec3::new(0.0, 0.0, STEP_LIFT);
        let e = self.pawn.extent();
        let body = Vec3::new(e.x, e.y, (e.z - STEP_LIFT / 2.0).max(1.0));
        let up = lift * 0.5;
        if view.world.sweep(body, here + up, goal + up).is_none() {
            self.follower = Some(PathFollower::new(vec![goal]));
            return true;
        }
        self.walk_to(view, enemy, events)
    }

    /// A close-up hit, if it has a melee weapon, the target is within
    /// reach and in front of it, nothing is in the way, and the last hit
    /// was `melee_delay` ago. **Our rule** (the original's is compiled):
    /// the reach is measured between the two bodies' centres.
    fn melee(
        &mut self,
        world: &dyn World,
        now: f32,
        t: &Target,
        events: &mut Vec<AiEvent>,
    ) -> bool {
        let c = &self.params.combat;
        if c.melee_range <= 0.0 {
            return false;
        }
        let to = t.centre - self.pawn.location;
        if to.length_2d() > c.melee_range {
            return false;
        }
        let off = angle_diff(to.y.atan2(to.x), self.facing).abs().to_degrees();
        if off > 30.0 {
            return true; // turning to face it: no shooting meanwhile
        }
        if now < self.next_melee {
            return true;
        }
        let origin = self.pawn.location + Vec3::new(0.0, 0.0, 20.0);
        if !clear_line(world, origin, t.centre) {
            return false;
        }
        self.next_melee = now + c.melee_delay;
        self.last_melee = now;
        events.push(AiEvent::Melee {
            origin,
            direction: (t.centre - origin).normalize_or_zero(),
            range: c.melee_range,
        });
        true
    }

    fn destination_far_enough(&self) -> bool {
        self.destination()
            .is_some_and(|d| (d - self.pawn.location).length_2d() >= MIN_MOVE)
    }

    /// The best free cover point near us that faces the enemy and still
    /// lets us shoot at it when standing.
    fn pick_cover(&self, view: &View, enemy: Vec3) -> Option<usize> {
        let c = &self.params.combat;
        if !c.uses_cover {
            return None;
        }
        let mut best: Option<(usize, f32)> = None;
        for n in view
            .nav
            .nodes_near(self.pawn.location, c.cover_search_radius)
        {
            let node = &view.nav.nodes[n];
            let NodeKind::Cover { facing, .. } = node.kind else {
                continue;
            };
            if view.cover_taken.contains(&n) {
                continue;
            }
            let to_enemy = enemy - node.location;
            let d = to_enemy.length();
            if d < c.ready_distance * 0.5 || d > self.params.senses.sight_radius {
                continue;
            }
            let (fwd, _) = rc_movement::axes(facing);
            if to_enemy.flat().normalize_or_zero().dot(fwd) < 0.5 {
                continue;
            }
            let cost = (node.location - self.pawn.location).length();
            if best.is_none_or(|(_, b)| cost < b) {
                best = Some((n, cost));
            }
        }
        best.map(|(n, _)| n)
    }

    /// The nearest path node within the fighting band that has a clear line
    /// to the enemy.
    fn pick_firing_spot(&self, view: &View, enemy: Vec3) -> Option<usize> {
        let c = &self.params.combat;
        let eye_up = Vec3::new(0.0, 0.0, self.params.movement.base_eye_height);
        let mut tested = 0;
        for n in view
            .nav
            .nodes_near(self.pawn.location, c.cover_search_radius)
        {
            let node = &view.nav.nodes[n];
            if matches!(node.kind, NodeKind::Fly) || view.cover_taken.contains(&n) {
                continue;
            }
            // Where it already stands has no clear shot, or it wouldn't be
            // looking.
            if (node.location - self.pawn.location).length_2d() < MIN_MOVE {
                continue;
            }
            let d = (enemy - node.location).length();
            if d < c.ready_distance * 0.5 || d > c.weapon_range.min(self.params.senses.sight_radius)
            {
                continue;
            }
            tested += 1;
            if tested > 12 {
                break;
            }
            if clear_line(view.world, node.location + eye_up, enemy + eye_up) {
                return Some(n);
            }
        }
        None
    }

    /// Fire in bursts while it has a clear shot within its aim limits.
    fn shoot(
        &mut self,
        world: &dyn World,
        allies: &[Vec3],
        now: f32,
        t: &Target,
        events: &mut Vec<AiEvent>,
    ) {
        let c = self.params.combat.clone();
        let to = t.centre - self.pawn.location;
        let dist = to.length();
        let off = angle_diff(to.y.atan2(to.x), self.facing).abs().to_degrees();
        let in_cover = matches!(self.maneuver, Maneuver::InCover(_));
        if dist > c.weapon_range || off > c.max_aim_yaw {
            self.crouch = in_cover;
            return;
        }
        if now < self.next_shot {
            // Resting between bursts: hide if in cover.
            self.crouch = in_cover && self.burst_left == c.burst_count;
            return;
        }
        self.crouch = false;
        if in_cover && self.pawn.crouched {
            return; // stand up first
        }
        let (fwd, right) = rc_movement::axes(self.facing);
        let origin = self.eye() - Vec3::new(0.0, 0.0, 10.0) + fwd * 30.0 + right * 10.0;
        // Aim at the middle of the body; if that's hidden (behind a wall
        // or a trench lip) but the head isn't, aim at the head.
        let aim = if clear_line(world, origin, t.centre) {
            t.centre
        } else {
            t.eye
        };
        if ally_in_line(origin, aim, allies, self.pawn.location) {
            // Hold fire rather than shoot a friend; `decide` moves it if
            // this lasts.
            self.ally_in_way_since.get_or_insert(now);
            return;
        }
        self.ally_in_way_since = None;
        let perfect = (aim - origin).normalize_or_zero();
        let max_err = (1.0 - c.accuracy).clamp(0.0, 1.0) * c.max_aim_error;
        let (direction, aim_error) = scatter(perfect, max_err, &mut self.rng);
        events.push(AiEvent::Fire {
            origin,
            direction,
            aim_error,
        });
        self.shots_fired += 1;
        self.burst_left -= 1;
        if self.burst_left == 0 {
            self.burst_left = c.burst_count;
            self.next_shot = now + self.rng.range(c.burst_pause_min, c.burst_pause_max);
        } else {
            self.next_shot = now + c.shot_delay;
        }
    }
}

/// How close (sideways, uu) a friend's centre may be to the line of fire:
/// a droid's collision radius plus a margin for aim error.
const ALLY_CLEARANCE: f32 = 50.0;

/// Does a friend (other than the one at `me`) stand in the line from `a`
/// to `b`?
pub fn ally_in_line(a: Vec3, b: Vec3, allies: &[Vec3], me: Vec3) -> bool {
    let d = b - a;
    let len2 = d.dot(d);
    if len2 < 1.0 {
        return false;
    }
    allies.iter().any(|&p| {
        if (p - me).length() < 1.0 {
            return false;
        }
        let t = (p - a).dot(d) / len2;
        if !(0.0..1.0).contains(&t) {
            return false;
        }
        let closest = a + d * t;
        let off = p - closest;
        (off.x * off.x + off.y * off.y).sqrt() < ALLY_CLEARANCE && off.z.abs() < 90.0
    })
}

/// The starting velocity for a throw from `from` that lands at `to`, with
/// `gravity` pulling down (uu/s²) and at most `max_speed` (uu/s), whose
/// path the level doesn't block (`None` if there is none). **Our choice**:
/// it tries the gentlest lob first (the slowest throw that reaches,
/// which comes down steeply and rolls on little), then the flattest arc at
/// full speed (for under a low ceiling).
pub fn throw_arc(
    world: &dyn World,
    from: Vec3,
    to: Vec3,
    max_speed: f32,
    gravity: f32,
) -> Option<Vec3> {
    let d = to - from;
    let flat = d.length_2d();
    if flat < 1.0 || gravity <= 0.0 {
        return None;
    }
    let dir = d.flat() * (1.0 / flat);
    let h = d.z;
    let mut arcs = Vec::new();
    // Slowest throw: speed² = g (h + r), at tan(angle) = (h + r) / flat.
    let r = d.length();
    let v = (gravity * (h + r)).sqrt();
    if v <= max_speed {
        arcs.push((v, ((h + r) / flat).atan()));
    }
    // Flattest arc at full speed.
    let v = max_speed;
    let disc = v.powi(4) - gravity * (gravity * flat * flat + 2.0 * h * v * v);
    if disc >= 0.0 {
        arcs.push((v, ((v * v - disc.sqrt()) / (gravity * flat)).atan()));
    }
    arcs.into_iter().find_map(|(speed, angle)| {
        let vel = dir * (speed * angle.cos()) + Vec3::UP * (speed * angle.sin());
        arc_clear(world, from, vel, gravity, flat / (speed * angle.cos())).then_some(vel)
    })
}

/// Does the level leave the arc from `from` at `vel` clear for the first
/// 90% of its `flight_time` (the end lands on the floor, so isn't checked)?
fn arc_clear(world: &dyn World, from: Vec3, vel: Vec3, gravity: f32, flight_time: f32) -> bool {
    const PIECES: usize = 12;
    let at = |t: f32| from + vel * t - Vec3::new(0.0, 0.0, 0.5 * gravity * t * t);
    let end = flight_time * 0.9;
    (0..PIECES).all(|i| {
        let t0 = end * i as f32 / PIECES as f32;
        let t1 = end * (i + 1) as f32 / PIECES as f32;
        clear_line(world, at(t0), at(t1))
    })
}

/// Is the straight line from `a` to `b` free of level geometry?
pub fn clear_line(world: &dyn World, a: Vec3, b: Vec3) -> bool {
    trace(world, a, b).is_none()
}

/// Length of the pieces a long trace is cut into, uu. The level's collision
/// lookup checks everything inside the box around a trace, so a long
/// diagonal trace in one piece looks at far more triangles than needed.
const TRACE_PIECE: f32 = 256.0;

/// Where the straight line from `a` to `b` first touches level geometry,
/// as a fraction of the way (0 to 1), or `None` if it is clear.
pub fn trace(world: &dyn World, a: Vec3, b: Vec3) -> Option<f32> {
    let len = (b - a).length();
    let pieces = (len / TRACE_PIECE).ceil().max(1.0) as usize;
    for i in 0..pieces {
        let t0 = i as f32 / pieces as f32;
        let t1 = (i + 1) as f32 / pieces as f32;
        let p0 = a + (b - a) * t0;
        let p1 = a + (b - a) * t1;
        if let Some(h) = world.sweep(Vec3::ZERO, p0, p1) {
            let t = t0 + (t1 - t0) * h.time;
            if t < 0.999 {
                return Some(t);
            }
        }
    }
    None
}

/// `dir` turned by a random angle up to `max_deg` degrees in a random
/// direction (even spread over the cone's area). Returns the new direction
/// and the angle used.
pub fn scatter(dir: Vec3, max_deg: f32, rng: &mut Rng) -> (Vec3, f32) {
    if max_deg <= 0.0 {
        return (dir, 0.0);
    }
    let angle = max_deg.to_radians() * rng.unit().sqrt();
    let spin = rng.unit() * std::f32::consts::TAU;
    // Two directions at right angles to `dir`.
    let helper = if dir.z.abs() < 0.9 {
        Vec3::UP
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };
    let a = cross(dir, helper).normalize_or_zero();
    let b = cross(dir, a);
    let side = a * spin.cos() + b * spin.sin();
    let out = (dir * angle.cos() + side * angle.sin()).normalize_or_zero();
    (out, angle.to_degrees())
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

/// `a - b` as an angle in -pi..pi.
pub fn angle_diff(a: f32, b: f32) -> f32 {
    wrap(a - b)
}

fn wrap(a: f32) -> f32 {
    let t = std::f32::consts::TAU;
    let mut x = a % t;
    if x > std::f32::consts::PI {
        x -= t;
    } else if x < -std::f32::consts::PI {
        x += t;
    }
    x
}
