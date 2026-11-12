//! The original's own attack state for clone troopers and Trandoshans
//! (`StateTrandoshanAttack`, which CloneTrooperBot names in its
//! `RequiredStates`), used when a fighter's file sets
//! `[attack] state = "trandoshan"`.
//!
//! How it works was read from the game's compiled AI code (ctgame.dll)
//! and is written here in our own words; no game code is copied. The
//! numbers come from the class defaults in the game files (the fighter's
//! `[attack]` and `[combat]` settings). docs/DESIGN.md "Clone trooper AI"
//! lists what was found and what is ours.
//!
//! In short: it plans a short list of steps, carries it out, and only
//! plans again once the list is done. Each plan is one of: hit up close,
//! go to a cover point, follow the enemy (to its gun's best distance, or
//! to arm's length), sidestep out and shoot, or stand (or crouch) and
//! shoot a short burst. With nothing to do it waits half a second.

use super::{ally_in_line, angle_diff, clear_line, AiEvent, Fighter, Maneuver, Mode, Target, View};
use rc_movement::{MoveInput, Vec3};
use rc_nav::NodeKind;
use std::collections::VecDeque;

/// The maneuver choices, as bits (the original keeps its last choice to
/// avoid doing some of them twice in a row).
const STAND_SHOOT: u32 = 0x2;
const CROUCH_SHOOT: u32 = 0x4;
const MELEE: u32 = 0x8;
const STEP_LEFT: u32 = 0x10;
const STEP_RIGHT: u32 = 0x20;
const CHASE: u32 = 0x100;
const FAR: u32 = 0x200;
const GOTO_COVER: u32 = 0x80_0000;

/// A reserved cover point farther than this from it is let go, uu
/// (100 uu: the game compares squared distances with 10000).
const KEEP_COVER: f32 = 100.0;
/// How often it checks that its line of fire is clear, s.
const FIRE_CHECK: f32 = 0.5;
/// Aiming further than this off the gun's facing shoots along the gun
/// instead, degrees.
const MAX_AIM_TURN: f32 = 30.0;
/// The aim's random spread: yaw up to ±(1 - Accuracy) × 30°, pitch a
/// fifth of that.
const AIM_SPREAD_YAW: f32 = 60.0;
const AIM_SPREAD_PITCH: f32 = 0.2;
/// A plan with nothing to do waits this long, s.
const IDLE_SLEEP: f32 = 0.5;
/// Facing within this of the enemy ends a turn-to step, degrees. **Ours**:
/// where the original's turn goal says "done" is not in what we read.
const TURN_DONE: f32 = 10.0;
/// A follow re-plans its route this often, s. **Ours** (the enemy moves;
/// how often the original's follow goal re-plans was not found).
const FOLLOW_REPLAN: f32 = 0.5;

/// One step of a plan.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Step {
    /// Turn to face the enemy.
    TurnTo,
    /// Play a one-off animation for `length` s. `hit_at` (0 to 1): a
    /// close-up hit lands that far into it.
    Anim {
        name: String,
        length: f32,
        hit_at: Option<f32>,
    },
    /// Fire up to `shots` shots for `time` s, then stop.
    Shoot { time: f32, shots: i32 },
    /// Start firing without a limit and leave it on.
    FireOn,
    /// Stop firing.
    FireOff,
    /// Sidestep `distance` uu (left or right of the way it faces) over
    /// `length` s, playing `anim`.
    Side {
        left: bool,
        distance: f32,
        length: f32,
        anim: String,
    },
    /// Follow the enemy until within `distance` uu of it.
    Follow { distance: f32 },
    /// Walk to cover point `node`.
    GotoCover(usize),
    /// Wait `time` s.
    Sleep(f32),
    /// Keep its facing fixed (`true`) or turn to the enemy again.
    LockAim(bool),
}

/// The running state of the original attack.
#[derive(Debug, Clone, Default)]
pub(super) struct Original {
    /// In the attack state (the start-of-fight reaction has played).
    active: bool,
    plan: VecDeque<Step>,
    /// The step being carried out and the battle time it started.
    running: Option<(Step, f32)>,
    /// How far the current sidestep has moved, uu.
    side_moved: f32,
    /// The sidestep's direction, fixed when it starts.
    side_dir: Vec3,
    next_replan: f32,
    hit_done: bool,
    /// Its last maneuver choice (bits above).
    last: u32,
    /// No sidestep before this battle time (`NextToggleAttackTime`).
    next_toggle: f32,
    /// The cover point it has reserved.
    pub(super) cover: Option<usize>,
    /// Facing held still (sidestepping out of cover).
    aim_locked: bool,
    // Firing.
    wants_fire: bool,
    /// Shots left before it stops; -1: no limit.
    shots_left: i32,
    next_check: f32,
    last_clear: f32,
    clear: bool,
    next_shot: f32,
    /// The enemy as last known.
    enemy: Option<Target>,
}

impl Fighter {
    /// One frame of the original attack, in place of `decide` and `shoot`.
    /// `saw` is the enemy if it can be seen now.
    pub(super) fn original_tick(
        &mut self,
        dt: f32,
        view: &View,
        saw: Option<Target>,
        events: &mut Vec<AiEvent>,
    ) {
        let now = view.time;
        // Where it thinks the enemy is: where it sees it, else where it
        // last saw it. (The original always knows its enemy's real place;
        // ours only knows what it has seen.)
        if let Some(t) = saw {
            self.orig.enemy = Some(t);
        } else if let (Some(mut t), Some(p)) = (self.orig.enemy, self.last_seen_at) {
            let up = t.eye - t.centre;
            t.centre = p;
            t.eye = p + up;
            self.orig.enemy = Some(t);
        }
        let Some(enemy) = self.orig.enemy.or_else(|| {
            self.last_seen_at.map(|p| Target {
                centre: p,
                eye: p,
                radius: 0.0,
                aim_low: 0.0,
            })
        }) else {
            self.original_stop();
            self.move_and_turn(dt, view, None);
            return;
        };
        if !self.orig.active {
            self.original_activate(now, &enemy, events);
        }
        let can_see = saw.is_some();
        let to = enemy.centre - self.pawn.location;
        let dir = to.normalize_or_zero();
        let dist = to.length() - enemy.radius;

        // Plan again only when the last plan is done.
        if self.orig.running.is_none() && self.orig.plan.is_empty() {
            self.update_self(view, &enemy, dir, dist, can_see, events);
        }
        let side_move = self.run_plan(dt, view, &enemy, dist, events);
        self.move_and_turn(dt, view, side_move.map(|d| (d, enemy.centre)));
        self.update_firing(view, &enemy, can_see, events);
    }

    /// Starting to fight: turn to the enemy, then (with chance
    /// `EmoteProbability`) the spotted gesture.
    fn original_activate(&mut self, now: f32, _enemy: &Target, _events: &mut Vec<AiEvent>) {
        let a = self.params.attack.clone();
        let o = &mut self.orig;
        o.active = true;
        o.plan.clear();
        o.running = None;
        o.plan.push_back(Step::TurnTo);
        // The original plays the gesture with chance EmoteProbability,
        // else its spotted line (troopers have no lines: `[sound]`).
        if self.rng.unit() < a.emote_probability && !a.emote.is_empty() && a.emote_time > 0.0 {
            o.plan.push_back(Step::Anim {
                name: a.emote.clone(),
                length: a.emote_time,
                hit_at: None,
            });
        }
        o.next_toggle = now + self.rng.unit() * a.min_step_interval;
    }

    /// Leaving the attack: let the cover go, stop firing, drop the plan.
    pub(super) fn original_stop(&mut self) {
        let o = &mut self.orig;
        o.active = false;
        o.plan.clear();
        o.running = None;
        o.cover = None;
        o.wants_fire = false;
        o.aim_locked = false;
        self.action = None;
        self.follower = None;
        self.crouch = false;
    }

    /// Choose the next plan (the original's `UpdateSelf`).
    fn update_self(
        &mut self,
        view: &View,
        enemy: &Target,
        dir: Vec3,
        dist: f32,
        can_see: bool,
        events: &mut Vec<AiEvent>,
    ) {
        let now = view.time;
        let a = self.params.attack.clone();
        let c = self.params.combat.clone();
        let m = self.params.movement.clone();
        let mut options = 0u32;
        let mut cover_found = None;

        // Keep its cover if it is still there and still covers it from
        // the enemy; otherwise let it go and look for one.
        if let Some(n) = self.orig.cover {
            let node = &view.nav.nodes[n];
            let keep = (node.location - self.pawn.location).length() <= KEEP_COVER
                && covers_from(&node.kind, node.location, enemy.centre);
            if !keep {
                self.orig.cover = None;
            }
        }
        let anchor = view.nav.nearest_node(self.pawn.location);
        if self.orig.cover.is_none() && c.uses_cover {
            if let Some(anchor) = anchor {
                let point = if dist > a.optimal_dist {
                    enemy.centre + dir * a.optimal_dist
                } else {
                    self.pawn.location
                };
                if let Some(n) = self.best_cover(view, enemy.centre, point, a.cover_range) {
                    if view
                        .nav
                        .route_between(self.pawn.location, anchor, n, view.nav.nodes[n].location)
                        .is_some()
                    {
                        cover_found = Some(n);
                        options |= GOTO_COVER;
                    }
                }
            }
        }
        let in_cover = self.orig.cover;
        // "Crouch cover": low enough to crouch behind, too low to stand.
        let cover_height = in_cover.map_or(0.0, |n| match view.nav.nodes[n].kind {
            NodeKind::Cover { cover_height, .. } => cover_height,
            _ => 0.0,
        });
        let crouch_cover = in_cover.is_some()
            && cover_height >= 1.5 * m.crouch_height
            && cover_height < 2.0 * m.collision_height;

        if c.melee_range > dist && self.orig.last != MELEE {
            options |= MELEE;
        }
        let (fwd, _) = rc_movement::axes(self.facing);
        let facing_dot = fwd.dot(dir.flat().normalize_or_zero());
        if in_cover.is_none() {
            if view.nav.route(self.pawn.location, enemy.centre).is_some() {
                options |= CHASE;
                if dist > a.optimal_dist {
                    options |= FAR;
                }
            }
            if can_see {
                options |= if self.rng.unit() < 0.5 {
                    CROUCH_SHOOT
                } else {
                    STAND_SHOOT
                };
                if now >= self.orig.next_toggle && facing_dot > 0.5 {
                    if self.orig.last != STEP_LEFT && self.can_step(view, true, None, enemy) {
                        options |= STEP_LEFT;
                    }
                    if self.orig.last != STEP_RIGHT && self.can_step(view, false, None, enemy) {
                        options |= STEP_RIGHT;
                    }
                }
            }
        } else {
            if self.orig.last != STAND_SHOOT && a.gun_height > cover_height {
                options |= STAND_SHOOT;
            }
            let stepped = matches!(self.orig.last, STEP_LEFT | STEP_RIGHT);
            if facing_dot > 0.2 && a.can_side_step && !stepped && now >= self.orig.next_toggle {
                if self.can_step(view, false, in_cover, enemy) {
                    options |= STEP_RIGHT;
                }
                if self.can_step(view, true, in_cover, enemy) {
                    options |= STEP_LEFT;
                }
            }
        }

        // Pick, in the original's order. (Leans and grenades come between
        // a far follow and the steps; troopers can't lean and carry no
        // grenades, so they are left out.)
        let pick = if options & MELEE != 0 {
            MELEE
        } else if options & GOTO_COVER != 0 {
            GOTO_COVER
        } else if options & FAR != 0 {
            FAR
        } else if options & (STEP_LEFT | STEP_RIGHT) == STEP_LEFT | STEP_RIGHT {
            if self.rng.unit() < 0.5 {
                STEP_LEFT
            } else {
                STEP_RIGHT
            }
        } else if options & STEP_LEFT != 0 {
            STEP_LEFT
        } else if options & STEP_RIGHT != 0 {
            STEP_RIGHT
        } else if options & STAND_SHOOT != 0 {
            STAND_SHOOT
        } else if options & CROUCH_SHOOT != 0 {
            CROUCH_SHOOT
        } else if options & CHASE != 0 {
            CHASE
        } else {
            0
        };
        self.orig.last = pick;
        let plan = &mut self.orig.plan;
        let shots = (a.weapon_burst_count as f32 * 1.5) as i32;
        let shoot_time = c.shot_delay * a.weapon_burst_count as f32 * 1.5;
        let maneuver = match pick {
            STAND_SHOOT | CROUCH_SHOOT => {
                self.crouch = pick == CROUCH_SHOOT && c.can_crouch;
                plan.push_back(Step::TurnTo);
                plan.push_back(Step::Shoot {
                    time: shoot_time,
                    shots,
                });
                in_cover.map_or(Maneuver::Hold, Maneuver::InCover)
            }
            MELEE => {
                self.follower = None;
                self.crouch = false;
                plan.push_back(Step::TurnTo);
                plan.push_back(Step::Anim {
                    name: a.melee_anim.clone(),
                    length: a.melee_time,
                    hit_at: Some(a.melee_hit_at),
                });
                Maneuver::Hold
            }
            STEP_LEFT | STEP_RIGHT => {
                let left = pick == STEP_LEFT;
                self.crouch = false;
                let locked = in_cover.is_some();
                if locked {
                    if let Some(NodeKind::Cover { facing, .. }) =
                        in_cover.map(|n| &view.nav.nodes[n].kind)
                    {
                        self.facing = *facing;
                    }
                    plan.push_back(Step::LockAim(true));
                }
                let (out_anim, out_time, back_anim, back_time) = if left {
                    ("StepLeft", a.step_left_time, "StepRight", a.step_right_time)
                } else {
                    ("StepRight", a.step_right_time, "StepLeft", a.step_left_time)
                };
                plan.push_back(Step::Side {
                    left,
                    distance: a.step_distance,
                    length: out_time,
                    anim: out_anim.into(),
                });
                // The weapon's own burst, over its burst time × 1.5 plus
                // half a second.
                let burst = a.weapon_burst_count as i32;
                plan.push_back(Step::Shoot {
                    time: burst as f32 * c.shot_delay * 1.5 + 0.5,
                    shots: burst,
                });
                plan.push_back(Step::Side {
                    left: !left,
                    distance: a.step_distance,
                    length: back_time,
                    anim: back_anim.into(),
                });
                if locked {
                    plan.push_back(Step::LockAim(false));
                }
                self.orig.next_toggle = now + a.min_step_interval;
                in_cover.map_or(Maneuver::Hold, Maneuver::InCover)
            }
            CHASE | FAR => {
                self.crouch = false;
                if c.weapon_range >= dist {
                    let distance = if pick == FAR {
                        a.optimal_dist * 0.9
                    } else {
                        c.melee_range
                    };
                    plan.push_back(Step::FireOn);
                    plan.push_back(Step::Follow { distance });
                    plan.push_back(Step::FireOff);
                }
                if pick == FAR {
                    Maneuver::Advance
                } else {
                    Maneuver::Charge
                }
            }
            GOTO_COVER => {
                let n = cover_found.expect("cover chosen");
                self.orig.cover = Some(n);
                self.crouch = false;
                plan.push_back(Step::FireOn);
                plan.push_back(Step::GotoCover(n));
                plan.push_back(Step::FireOff);
                Maneuver::ToCover(n)
            }
            _ => {
                plan.push_back(Step::Sleep(IDLE_SLEEP));
                self.orig.last = 0;
                in_cover.map_or(Maneuver::Hold, Maneuver::InCover)
            }
        };
        // Hiding behind cover for a moment after the plan.
        if pick != GOTO_COVER
            && in_cover.is_some()
            && (crouch_cover || (!can_see && cover_height >= m.collision_height * 1.9))
        {
            let t = self.rng.range(a.cover_hide_min, a.cover_hide_max);
            self.orig.plan.push_back(Step::Sleep(t));
        }
        self.set_maneuver(maneuver, now, events);
    }

    /// The best free cover point (the original's `FindBestCoverpoint`):
    /// one that is free, reachable from here and from the enemy, within
    /// `range` of `point`, faces the enemy (within its field of view),
    /// and is on our side of the enemy. The nearest to `point` wins;
    /// one farther from the enemy than we are, and more than 200 uu from
    /// us, counts four times as far.
    fn best_cover(&self, view: &View, enemy: Vec3, point: Vec3, range: f32) -> Option<usize> {
        let a = &self.params.attack;
        let me = self.pawn.location;
        let mut best: Option<(usize, f32)> = None;
        for (n, node) in view.nav.nodes.iter().enumerate() {
            let NodeKind::Cover {
                strafe_left,
                strafe_right,
                cover_height,
                ..
            } = node.kind
            else {
                continue;
            };
            if view.cover_taken.contains(&n) && self.orig.cover != Some(n) {
                continue;
            }
            if view.nav.blocked.contains(&n) {
                continue;
            }
            let p = node.location;
            // Too tall to shoot over and too narrow to step out of.
            if cover_height > a.gun_height
                && a.step_distance > strafe_left
                && a.step_distance > strafe_right
            {
                continue;
            }
            if (point - p).len2() >= range * range {
                continue;
            }
            if !covers_from(&node.kind, p, enemy) {
                continue;
            }
            if (p - enemy).dot(me - enemy) <= 0.0 {
                continue;
            }
            let mut score = (point - p).len2();
            if (p - enemy).len2() > (me - enemy).len2() && (me - p).len2() > 40000.0 {
                score *= 4.0;
            }
            if best.is_some_and(|(_, b)| score >= b) {
                continue;
            }
            // Reachable from here and from the enemy (checked last: the
            // dearest test).
            if view.nav.route(me, p).is_none() || view.nav.route(enemy, p).is_none() {
                continue;
            }
            best = Some((n, score));
        }
        best.map(|(n, _)| n)
    }

    /// Can it sidestep (left or right) here? Out of `cover`, the cover's
    /// own room to the side must fit the step plus its body, and the
    /// enemy must be in plain view from where it would stand. In the
    /// open, the way must be clear and have floor under it.
    fn can_step(&self, view: &View, left: bool, cover: Option<usize>, enemy: &Target) -> bool {
        let a = &self.params.attack;
        if !a.can_side_step {
            return false;
        }
        let me = self.pawn.location;
        let facing = match cover.map(|n| &view.nav.nodes[n].kind) {
            Some(NodeKind::Cover { facing, .. }) => *facing,
            _ => self.facing,
        };
        let (_, right) = rc_movement::axes(facing);
        let side = if left { right * -1.0 } else { right };
        let dest = me + side * a.step_distance;
        if let Some(n) = cover {
            let NodeKind::Cover {
                strafe_left,
                strafe_right,
                ..
            } = view.nav.nodes[n].kind
            else {
                return false;
            };
            let room = if left { strafe_left } else { strafe_right };
            if room <= self.params.movement.collision_radius + a.step_distance {
                return false;
            }
            return clear_line(view.world, dest, enemy.centre);
        }
        if !clear_line(view.world, me, dest) {
            return false;
        }
        let down = Vec3::new(0.0, 0.0, self.params.movement.collision_height * 1.5);
        matches!(view.world.sweep(Vec3::ZERO, dest, dest - down), Some(h) if h.normal.z > 0.7)
    }

    /// Carry out the plan. Returns the sideways move to make this frame,
    /// if sidestepping.
    fn run_plan(
        &mut self,
        dt: f32,
        view: &View,
        enemy: &Target,
        dist: f32,
        events: &mut Vec<AiEvent>,
    ) -> Option<Vec3> {
        let now = view.time;
        // Steps that finish at once run in the same frame.
        for _ in 0..8 {
            if self.orig.running.is_none() {
                let step = self.orig.plan.pop_front()?;
                self.start_step(&step, view, enemy, events);
                self.orig.running = Some((step, now));
            }
            let (step, started) = self.orig.running.clone().expect("running");
            let elapsed = now - started;
            let (done, side) = match &step {
                Step::TurnTo => {
                    let want = (enemy.centre - self.pawn.location).flat();
                    let off = angle_diff(want.y.atan2(want.x), self.facing);
                    (off.abs().to_degrees() <= TURN_DONE, None)
                }
                Step::Anim { length, hit_at, .. } => {
                    if let Some(at) = hit_at {
                        if !self.orig.hit_done && elapsed >= at * length {
                            self.orig.hit_done = true;
                            self.melee_hit(view, enemy, events);
                        }
                    }
                    (elapsed >= *length, None)
                }
                Step::Shoot { time, .. } => (elapsed >= *time, None),
                Step::FireOn | Step::FireOff | Step::LockAim(_) => (true, None),
                Step::Side {
                    distance, length, ..
                } => {
                    let want =
                        (distance * (elapsed / length).min(1.0) - self.orig.side_moved).max(0.0);
                    self.orig.side_moved += want;
                    let _ = dt;
                    (elapsed >= *length, Some(self.orig.side_dir * want))
                }
                Step::Follow { distance } => {
                    if dist <= *distance {
                        (true, None)
                    } else {
                        if now >= self.orig.next_replan {
                            self.orig.next_replan = now + FOLLOW_REPLAN;
                            if !self.walk_to(view, enemy.centre, events) {
                                self.follower = None;
                            }
                        }
                        (self.follower.is_none(), None)
                    }
                }
                Step::GotoCover(_) => (self.follower.is_none(), None),
                Step::Sleep(t) => (elapsed >= *t, None),
            };
            if !done {
                return side;
            }
            self.end_step(&step);
            self.orig.running = None;
            if side.is_some() {
                return side;
            }
        }
        None
    }

    fn start_step(&mut self, step: &Step, view: &View, _enemy: &Target, events: &mut Vec<AiEvent>) {
        let now = view.time;
        match step {
            Step::Anim { name, length, .. } => {
                self.orig.hit_done = false;
                self.follower = None;
                self.start_action(name, now, *length);
            }
            Step::Shoot { shots, .. } => self.start_shooting(*shots),
            Step::FireOn => self.start_shooting(-1),
            Step::FireOff => self.orig.wants_fire = false,
            Step::LockAim(on) => self.orig.aim_locked = *on,
            Step::Side {
                left, length, anim, ..
            } => {
                self.follower = None;
                let (_, right) = rc_movement::axes(self.facing);
                self.orig.side_dir = if *left { right * -1.0 } else { right };
                self.orig.side_moved = 0.0;
                self.start_action(anim, now, *length);
            }
            Step::Follow { .. } => {
                self.orig.next_replan = now;
            }
            Step::GotoCover(n) => {
                if !self.walk_to_node(view, *n, events) {
                    self.orig.cover = None;
                }
            }
            Step::TurnTo | Step::Sleep(_) => {}
        }
    }

    fn end_step(&mut self, step: &Step) {
        match step {
            // A timed burst stops shooting when its time is up.
            Step::Shoot { .. } => self.orig.wants_fire = false,
            Step::Anim { .. } | Step::Side { .. } => self.action = None,
            Step::Follow { .. } => self.follower = None,
            _ => {}
        }
    }

    fn start_action(&mut self, name: &str, now: f32, length: f32) {
        self.action = Some(super::Action {
            anim: name.to_string(),
            started: now,
            length,
        });
    }

    fn start_shooting(&mut self, shots: i32) {
        self.orig.wants_fire = true;
        self.orig.shots_left = shots;
        // A fresh check of the line of fire before the first shot.
        self.orig.next_check = f32::NEG_INFINITY;
    }

    /// The close-up hit lands (at its animation's hit moment).
    fn melee_hit(&mut self, view: &View, enemy: &Target, events: &mut Vec<AiEvent>) {
        let c = &self.params.combat;
        let origin = self.pawn.location + Vec3::new(0.0, 0.0, 20.0);
        if !clear_line(view.world, origin, enemy.centre) {
            return;
        }
        self.last_melee = view.time;
        events.push(AiEvent::Melee {
            origin,
            direction: (enemy.centre - origin).normalize_or_zero(),
            range: c.melee_range + enemy.radius,
        });
    }

    /// Walk (along a route) and turn: to the enemy, or along the route
    /// when walking (troopers can't walk sideways), or not at all while
    /// its aim is locked. `side` moves it sideways (a sidestep).
    fn move_and_turn(&mut self, dt: f32, view: &View, side: Option<(Vec3, Vec3)>) {
        let now = view.time;
        let mut input = MoveInput::default();
        let mut want_yaw = self.facing;
        if let Some(f) = self.follower.as_mut() {
            match f.update(&self.pawn, dt) {
                Some(mut i) => {
                    want_yaw = i.yaw;
                    let off = angle_diff(want_yaw, self.facing).abs();
                    i.forward = (1.0 - off / std::f32::consts::FRAC_PI_2).clamp(0.0, 1.0);
                    i.yaw = self.facing;
                    input = i;
                    if f.stuck_time > super::STUCK_SECONDS {
                        self.follower = None;
                    }
                }
                None => {
                    self.distance_walked += f.travelled;
                    self.follower = None;
                    if let Maneuver::ToCover(n) = self.maneuver {
                        self.maneuver = Maneuver::InCover(n);
                        self.maneuver_started = now;
                    }
                }
            }
        } else if !self.orig.aim_locked {
            if let Some(p) = self.orig.enemy.map(|t| t.centre).or(self.last_seen_at) {
                want_yaw = rc_nav::steer::yaw_towards(self.pawn.location, p);
            }
        }
        if let Some((delta, _)) = side {
            // The step's own move: slide along, stopped by walls.
            let e = self.pawn.extent();
            let lift = Vec3::new(0.0, 0.0, 2.0);
            let from = self.pawn.location + lift;
            let to = from + delta;
            let got = match view.world.sweep(e, from, to) {
                Some(h) if !h.start_solid => delta * (h.time - 0.01).max(0.0),
                Some(_) => Vec3::ZERO,
                None => delta,
            };
            self.pawn.location += got;
            self.distance_walked += got.length();
        }
        let max_turn = self.params.senses.turn_rate.to_radians() * dt;
        self.facing =
            super::wrap(self.facing + angle_diff(want_yaw, self.facing).clamp(-max_turn, max_turn));
        input.yaw = self.facing;
        input.crouch = self.crouch;
        self.pawn.step(dt, &input, view.world);
    }

    /// The gun: fires every `shot_delay` while it wants to, has shots
    /// left, and its line of fire was clear at the last check (every half
    /// second: no friend in the way, nothing of the level between gun and
    /// enemy).
    fn update_firing(
        &mut self,
        view: &View,
        enemy: &Target,
        can_see: bool,
        events: &mut Vec<AiEvent>,
    ) {
        let now = view.time;
        let c = self.params.combat.clone();
        let a = self.params.attack.clone();
        if !self.orig.wants_fire || self.orig.shots_left == 0 || c.weapon.is_empty() {
            return;
        }
        let (fwd, right) = rc_movement::axes(self.facing);
        let origin = self.eye() - Vec3::new(0.0, 0.0, 10.0) + fwd * 30.0 + right * 10.0;
        if now >= self.orig.next_check {
            self.orig.next_check = now + FIRE_CHECK;
            let max_err = (1.0 - c.accuracy).clamp(0.0, 1.0) * c.max_aim_error;
            let ok = can_see
                && clear_line(view.world, origin, enemy.centre)
                && !ally_in_line(
                    origin,
                    enemy.centre,
                    view.allies,
                    self.pawn.location,
                    max_err,
                );
            if ok {
                self.orig.last_clear = now;
            }
            self.orig.clear = ok || now < self.orig.last_clear + a.post_world_check_fire_duration;
        }
        if !self.orig.clear || now < self.orig.next_shot {
            return;
        }
        // Aim: somewhere between its aim-low point and its eyes.
        let eye_up = enemy.eye.z - enemy.centre.z;
        let h = enemy.aim_low + self.rng.unit() * (eye_up - enemy.aim_low);
        let aim = enemy.centre + Vec3::new(0.0, 0.0, h);
        let to = aim - origin;
        let mut yaw = to.y.atan2(to.x);
        let mut pitch = to.z.atan2(to.length_2d());
        if angle_diff(yaw, self.facing).abs().to_degrees() > MAX_AIM_TURN {
            // Too far off the gun's facing: it shoots along the gun.
            yaw = self.facing;
            pitch = 0.0;
        }
        let miss = (1.0 - c.accuracy).clamp(0.0, 1.0);
        let dyaw = (self.rng.unit() - 0.5) * miss * AIM_SPREAD_YAW;
        let dpitch = (self.rng.unit() - 0.5) * miss * AIM_SPREAD_YAW * AIM_SPREAD_PITCH;
        let perfect = Vec3::new(
            yaw.cos() * pitch.cos(),
            yaw.sin() * pitch.cos(),
            pitch.sin(),
        );
        yaw += dyaw.to_radians();
        pitch += dpitch.to_radians();
        let direction = Vec3::new(
            yaw.cos() * pitch.cos(),
            yaw.sin() * pitch.cos(),
            pitch.sin(),
        );
        let aim_error = perfect.dot(direction).clamp(-1.0, 1.0).acos().to_degrees();
        events.push(AiEvent::Fire {
            origin,
            direction,
            aim_error,
        });
        self.shots_fired += 1;
        self.orig.next_shot = now + c.shot_delay;
        if self.orig.shots_left > 0 {
            self.orig.shots_left -= 1;
        }
    }

    /// Whether this fighter uses the original attack and is in it.
    pub(super) fn original_attacking(&self) -> bool {
        self.params.attack.trandoshan() && self.mode == Mode::Attack
    }
}

/// Does a cover point at `at` cover it from an enemy at `enemy`: is the
/// enemy within the cover's field of view (`FOVdegrees` around the way it
/// faces)?
fn covers_from(kind: &NodeKind, at: Vec3, enemy: Vec3) -> bool {
    let NodeKind::Cover {
        facing,
        fov_degrees,
        ..
    } = *kind
    else {
        return false;
    };
    let (look, _) = rc_movement::axes(facing);
    let to = (enemy - at).normalize_or_zero();
    to.dot(look) > (fov_degrees * 0.5).to_radians().cos()
}

trait Len2 {
    fn len2(self) -> f32;
}

impl Len2 for Vec3 {
    fn len2(self) -> f32 {
        self.dot(self)
    }
}
