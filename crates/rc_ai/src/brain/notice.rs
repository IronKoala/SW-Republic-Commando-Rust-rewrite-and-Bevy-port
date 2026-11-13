//! How a fighter with the original's noticing (`[senses] noticing =
//! "original"`, the clone troopers) notices enemies, picks the one to
//! fight, and decides when to fight.
//!
//! Read from the game's compiled AI code: the bot's decision loop and its
//! "local situation" check (ctgame.dll) and the engine's sight test
//! (engine.dll). Written here in our own words; docs/DESIGN.md "How the
//! troopers notice the enemy" lists the rules and what is not ported yet.

use super::{clear_line, AiEvent, Fighter, Mode, Target, View};
use rc_movement::{Vec3, World};

/// What it knows about its enemies, between frames.
#[derive(Debug, Clone)]
pub(super) struct Notice {
    /// Everyone it could fight this frame, with an id for each.
    around: Vec<(Target, u32)>,
    /// When it last looked over everyone (`LastUpdateTime`), if ever.
    assessed_at: Option<f32>,
    /// Its next decision (`NextDecisionTime`), battle time.
    decide_at: f32,
    /// The nearest enemy it could see or hear at the last look
    /// (`BestTarget`).
    best: Option<u32>,
    /// The enemy it is fighting (`Enemy`).
    enemy: Option<u32>,
    /// When it last saw its enemy (`LastTimeEnemySeen`). Kept when the
    /// enemy changes, as the original does.
    enemy_seen_at: f32,
    /// How many enemies it could see or hear at the last look.
    detected: u32,
    /// Who last hurt it, and when.
    last_hit_by: Option<(u32, f32)>,
    /// The last decision was to fight.
    pub(super) attack: bool,
    /// Its attack's "can I see my enemy" answer, from its last plan.
    pub(super) can_see_enemy: bool,
    /// Events to hand out with the next frame.
    pending: Vec<AiEvent>,
}

impl Default for Notice {
    fn default() -> Self {
        Notice {
            around: Vec::new(),
            assessed_at: None,
            decide_at: 0.0,
            best: None,
            enemy: None,
            enemy_seen_at: f32::NEG_INFINITY,
            detected: 0,
            last_hit_by: None,
            attack: false,
            can_see_enemy: false,
            pending: Vec::new(),
        }
    }
}

/// When an enemy's radius squared over its distance squared is below this
/// (it is more than 100 radii away), only the line to its eyes counts and
/// the corner checks are skipped. Game: engine `LineOfSightTo`.
const CORNER_CHECK_MIN: f32 = 0.0001;

impl Fighter {
    /// Uses the original's noticing (see the module notes).
    pub fn original_noticing(&self) -> bool {
        self.params.senses.original_noticing()
    }

    /// One frame of noticing. `around` is everyone it could fight and
    /// `ids` names each one (the same id must mean the same enemy every
    /// frame). Returns the enemy it is fighting, where it really is (the
    /// original's AI always knows its enemy's real place), or `None` when
    /// it is not fighting.
    pub fn notice(
        &mut self,
        world: &dyn World,
        around: &[Target],
        ids: &[u32],
        now: f32,
    ) -> Option<Target> {
        self.nt.around = around.iter().copied().zip(ids.iter().copied()).collect();
        if self.mode == Mode::Dead {
            return None;
        }
        // The decision loop: every DecisionInterval, times 0.8 to 1.2 at
        // random, it looks over everyone and picks its state. Fighting is
        // worth 0.6 when it has a target, standing about 0.1; looking into
        // a noise (not ported yet) at most the noise's priority, which
        // never reaches 0.6. A level script's order pauses the loop.
        if self.script.is_none() && now >= self.nt.decide_at {
            self.assess(world, now);
            self.nt.attack = self.nt.best.is_some();
            if self.nt.attack && self.nt.enemy.is_none() {
                self.nt.enemy = self.nt.best;
            }
            let k = self.rng.range(0.8, 1.2);
            self.nt.decide_at = now + k * self.params.senses.decision_interval;
        }
        if !self.nt.attack {
            return None;
        }
        self.enemy_target()
    }

    /// Its attack is about to plan its next steps: like the original's
    /// attack, it picks its enemy afresh (looking over everyone again if
    /// the last look is older than `situation_interval`) and asks whether
    /// it can see it. Returns the enemy, if it still has one.
    pub(super) fn notice_replan(&mut self, world: &dyn World, now: f32) -> Option<Target> {
        let stale = self
            .nt
            .assessed_at
            .is_none_or(|t| now - t > self.params.senses.situation_interval);
        if stale {
            self.assess(world, now);
        }
        self.nt.enemy = self.nt.best;
        let enemy = self.enemy_target();
        self.nt.can_see_enemy = match (enemy, self.nt.enemy) {
            (Some(t), Some(id)) => self.can_see_enemy(world, &t, id, now),
            _ => false,
        };
        enemy
    }

    /// Something hurt it; `by` is the id of whoever did. For the next few
    /// seconds it knows where that one is, seen or not.
    pub(super) fn notice_hit(&mut self, by: u32, now: f32) {
        self.nt.last_hit_by = Some((by, now));
    }

    /// `tick` for a fighter with the original's noticing. `view.target` is
    /// what `notice` returned this frame.
    pub(super) fn tick_original_noticing(
        &mut self,
        dt: f32,
        view: &View,
        mut events: Vec<AiEvent>,
    ) -> Vec<AiEvent> {
        events.extend(self.notice_events());
        // A level script's order comes first.
        if let Some(goal) = self.script.clone() {
            self.original_stop();
            self.script_tick(dt, view, goal, &mut events);
            return events;
        }
        let seen = view.target.is_some() && self.nt.can_see_enemy;
        if seen != self.enemy_seen_now {
            self.enemy_seen_now = seen;
            if !seen {
                events.push(AiEvent::LostSight);
            }
        }
        if let Some(t) = view.target {
            self.last_seen_at = Some(t.centre);
            if seen {
                self.last_seen_time = view.time;
            }
        }
        if self.nt.attack {
            self.set_mode(Mode::Attack, &mut events);
        } else if self.mode == Mode::Attack {
            self.original_stop();
            self.set_mode(Mode::Idle, &mut events);
        }
        if self.original_attacking() {
            self.original_tick(dt, view, view.target, &mut events);
        } else {
            // Its standing-about state isn't ported: it stands and waits.
            self.move_and_turn(dt, view, None);
        }
        events
    }

    /// Events noticing produced since the last frame.
    pub(super) fn notice_events(&mut self) -> Vec<AiEvent> {
        std::mem::take(&mut self.nt.pending)
    }

    fn enemy_target(&self) -> Option<Target> {
        let id = self.nt.enemy?;
        self.nt
            .around
            .iter()
            .find(|(_, i)| *i == id)
            .map(|(t, _)| *t)
    }

    /// Look over everyone within sight radius (`ULocalSituation::Assess`):
    /// count the ones it can see or hear and take the nearest of those as
    /// its best target.
    fn assess(&mut self, world: &dyn World, now: f32) {
        let radius = self.params.senses.sight_radius;
        let centre = self.pawn.location;
        let before = self.nt.detected;
        let mut closest = radius * radius;
        let mut best = None;
        let mut detected = 0;
        let around = self.nt.around.clone();
        for (t, id) in &around {
            let d2 = len2(t.centre - centre);
            if d2 > radius * radius {
                continue;
            }
            if !self.can_see_or_hear(world, t, *id, now) {
                continue;
            }
            detected += 1;
            if d2 < closest {
                closest = d2;
                best = Some(*id);
            }
        }
        self.nt.detected = detected;
        self.nt.best = best;
        self.nt.assessed_at = Some(now);
        // The original's spotted moment (it tells the world "enemy
        // spotted" here, which only other characters' noise handling uses).
        if before == 0 && detected > 0 {
            if let Some(t) = best.and_then(|b| around.iter().find(|(_, i)| *i == b)) {
                self.nt.pending.push(AiEvent::Spotted {
                    distance: (t.0.centre - centre).length(),
                });
            }
        }
    }

    /// `CanSeeOrHear`: it knows where `t` is if `t` hurt it within the last
    /// `remember_attacker` seconds, or is within `AutoDetectRadius` with
    /// nothing solid between, or it can see it (`can_see_enemy`).
    fn can_see_or_hear(&mut self, world: &dyn World, t: &Target, id: u32, now: f32) -> bool {
        let s = &self.params.senses;
        if let Some((by, at)) = self.nt.last_hit_by {
            if by == id && now - at < s.remember_attacker {
                return true;
            }
        }
        let near = s.auto_detect_radius;
        if len2(t.centre - self.pawn.location) <= near * near
            && clear_line(world, t.centre, self.pawn.location)
        {
            return true;
        }
        self.can_see_enemy(world, t, id, now)
    }

    /// `CanSeeEnemy`: can it see `t`? For its current enemy, seeing it
    /// notes the time, and an enemy out of sight still counts as seen for
    /// `EnemyMemoryTime` seconds after that.
    fn can_see_enemy(&mut self, world: &dyn World, t: &Target, id: u32, now: f32) -> bool {
        let seen = self.see_actor(world, t);
        if self.nt.enemy == Some(id) {
            if seen {
                self.nt.enemy_seen_at = now;
                return true;
            }
            return now - self.nt.enemy_seen_at < self.params.senses.enemy_memory_time;
        }
        seen
    }

    /// The engine's `SeeActor`: within `SightRadius`, inside the view cone
    /// (`PeripheralVision`, the cosine of its half-angle, against the way
    /// it faces), and a clear line of sight.
    fn see_actor(&self, world: &dyn World, t: &Target) -> bool {
        let s = &self.params.senses;
        let to = t.centre - self.pawn.location;
        if len2(to) > s.sight_radius * s.sight_radius {
            return false;
        }
        let (fwd, _) = rc_movement::axes(self.facing);
        if fwd.dot(to.normalize_or_zero()) < s.peripheral_vision {
            return false;
        }
        self.line_of_sight(world, t)
    }

    /// The engine's `LineOfSightTo`: a clear line from its eyes to `t`'s
    /// eyes; failing that, unless `t` is far away for its size, clear
    /// lines to the corners of a square around `t`'s middle. Of the four
    /// corners only the first three are tried, and of those the ones that
    /// lie farthest from and nearest to the level's origin are skipped (so
    /// one corner is tried in all). That is what the original does.
    fn line_of_sight(&self, world: &dyn World, t: &Target) -> bool {
        let eye = self.eye();
        if clear_line(world, eye, t.eye) {
            return true;
        }
        let r = t.radius;
        let d2 = len2(t.centre - eye);
        if d2 <= 0.0 || r * r / d2 < CORNER_CHECK_MIN {
            return false;
        }
        let c = t.centre;
        let corners = [
            Vec3::new(c.x - r, c.y + r, c.z),
            Vec3::new(c.x + r, c.y + r, c.z),
            Vec3::new(c.x - r, c.y - r, c.z),
            Vec3::new(c.x + r, c.y - r, c.z),
        ];
        let (mut far, mut near) = (0, 0);
        let (mut far_d, mut near_d) = (len2(corners[0]), len2(corners[0]));
        for (i, p) in corners.iter().enumerate().skip(1) {
            let d = len2(*p);
            if d > far_d {
                far = i;
                far_d = d;
            } else if d < near_d {
                near = i;
                near_d = d;
            }
        }
        (0..3)
            .filter(|&i| i != far && i != near)
            .any(|i| clear_line(world, corners[i], eye))
    }
}

fn len2(v: Vec3) -> f32 {
    v.dot(v)
}
