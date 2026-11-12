//! Droid dispensers: machines that make battle droids and super battle
//! droids until they are destroyed.
//!
//! How the original works, from the script source in CTGame.u
//! (`DroidDispenser`, which extends `PawnFactory`, and `Factory`), read
//! only and described in our own words:
//!
//! - It starts **offline**. The event named like its tag starts its intro
//!   animation (`IntroAnim`: dropping in from the sky, being pushed in by
//!   slavers; without one it plays its 2-frame `ActionBreathe`). When that
//!   ends it **warms up** (`WarmUpInfo`: the "warm" skin) for `WarmUpTime`
//!   seconds and comes **online** (`OnlineInfo`: the "on" skin). With an
//!   `OnlineTag` it instead takes that as its tag after the intro and
//!   waits for that event.
//! - Online, it makes a droid at once and then every `SpawnInterval`
//!   seconds (a random time up to `SpawnIntervalMax` when that is larger),
//!   while fewer than `MaxAlive` of its droids are alive and fewer than
//!   `MaxInWorld` of that class are alive anywhere (when set). At the very
//!   start `NumInitDroids` counts instead of `MaxAlive`. With
//!   `bStartTimerAfterDeath` (on for every dispenser) it then **waits**:
//!   it looks once a second for one of its droids having died, then waits
//!   its interval and comes online again (making one at once). It makes
//!   droids for ever (`bGenerateForever`) until it is destroyed.
//! - Each droid appears on one of the machine's slots: it is attached to
//!   the slot's bone (`droidAttach1`..) standing on it, folded up
//!   (`DispBreathe`), can't be hit and doesn't act, while the machine
//!   plays that slot's `Dispense1`.. animation. At the animation's
//!   `AnimNotify_ReleaseDroid` it is let go where the bone has carried it
//!   (and the machine sends `SpawnEvent`); it unfolds (`DispUnfold`),
//!   moved by that animation's root bone, and then walks, can be hit and
//!   starts thinking. Which slot: the spawn point it uses picks its tag,
//!   events and first behaviour, and the slot is the *next* spawn point's
//!   number (it moves on to the next spawn point before attaching), the
//!   last slot when there are more spawn points than slots.
//! - Health is `MaxHealth`. At 66% it shows `SlightlyDamagedInfo`, at 33%
//!   `HeavilyDamagedInfo`, and at 0 it is **destroyed** (`DamagedInfo`:
//!   explosion, the burnt skin, the `Destroyed` animation and usually an
//!   event for the level): the droid on its slot (not yet let go) is
//!   destroyed with it, and once all the droids it made are dead it sends
//!   `ExhaustedEvent`.
//! - The same event while it is online switches it off again; the next one
//!   replays its intro.
//!
//! The animation numbers (how long each animation is, when the droid is
//! let go, where the bones go, how far the unfold moves the droid) come
//! from the game's animation files; the game reads them into a
//! [`DispenserMotion`] for each dispenser. Without one (the headless tools)
//! a droid is let go at once, at the dispenser.

use super::{Battle, BattleEvent, Birth, Try};
use crate::brain::{Fighter, Mode};
use crate::spawns::Transition;
use rc_movement::Vec3;

/// Where a bone is at one moment of an animation, in the dispenser's own
/// space (Unreal units from the model's origin, before the dispenser's
/// turn): its origin and its X and Z axes (unit length).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoneSample {
    pub seconds: f32,
    pub at: Vec3,
    pub x: Vec3,
    pub z: Vec3,
}

/// One slot of a dispenser: its `Dispense<n>` animation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SlotMotion {
    /// The animation's length, seconds.
    pub seconds: f32,
    /// When the droid is let go (`AnimNotify_ReleaseDroid`), 0 to 1
    /// through the animation.
    pub release: f32,
    /// The slot's bone (`droidAttach<n>`) through the animation.
    pub path: Vec<BoneSample>,
}

/// How a droid unfolds once let go (`DispUnfold`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Unfold {
    pub seconds: f32,
    /// How far its root bone has moved the droid from where it was let go,
    /// in the droid's own axes (X forward, Z up), through the animation.
    pub travel: Vec<(f32, Vec3)>,
}

/// A dispenser's animation numbers, read from the game's files.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DispenserMotion {
    /// Its intro animation's length (or `ActionBreathe`'s), seconds.
    pub intro_seconds: f32,
    /// One per slot.
    pub slots: Vec<SlotMotion>,
    /// The unfold of the droid it makes.
    pub unfold: Unfold,
    /// How its intro carries it (its `Move` bone, the skeleton's root):
    /// at each moment (seconds in), its offset from where it was at the
    /// first frame, in its own axes (Unreal units, X forward, Z up).
    /// `DroidDispenser`'s `Physics` is `PHYS_RootMotion`, so the machine
    /// itself moves by this: `entranceSky` drops it 3000 uu, which is why
    /// the maps place those dispensers 3000 uu above the floor.
    pub intro_travel: Vec<(f32, Vec3)>,
}

/// Where a dispenser is in its life.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Phase {
    #[default]
    Offline,
    /// Playing its intro until this battle time.
    Intro {
        until: f32,
    },
    /// Warming up until this battle time.
    WarmUp {
        until: f32,
    },
    /// Waiting for its `OnlineTag` event.
    WaitOnline,
    /// Making droids; next try at the spawner's `next_spawn`.
    Online,
    /// Has `MaxAlive` out: looks for a death at `check`; `ending` once it
    /// saw one and is waiting its interval (until the spawner's
    /// `next_spawn`).
    Waiting {
        check: f32,
        ending: bool,
    },
    /// Nothing left to make (never, for dispensers that make droids for
    /// ever, which all do).
    Shutdown,
    Destroyed,
}

impl Phase {
    pub fn name(&self) -> &'static str {
        match self {
            Phase::Offline => "offline",
            Phase::Intro { .. } => "intro",
            Phase::WarmUp { .. } => "warm-up",
            Phase::WaitOnline => "waiting for its online event",
            Phase::Online => "online",
            Phase::Waiting { .. } => "waiting for a death",
            Phase::Shutdown => "shut down",
            Phase::Destroyed => "destroyed",
        }
    }
}

/// A dispenser's state.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DispState {
    pub phase: Phase,
    /// Health is kept by the game's weapons part; this is what it last
    /// said, for the 66% / 33% steps.
    pub health: f32,
    pub slightly: bool,
    pub heavily: bool,
    /// Its first `NumInitDroids` are out.
    pub init_done: bool,
    /// `MaxAlive` now (`NumInitDroids` at the start).
    pub max_alive: u32,
    /// The droids it made that it still counts (`SpawnedActors`), newest
    /// first.
    pub spawned: Vec<u32>,
    /// The droid on a slot, not yet let go (`bDispensingDroid`).
    pub riding: Option<u32>,
    /// `ExhaustedEvent` still to send once its droids are dead.
    pub check_exhausted: bool,
    /// The tag events switch it by (its `OnlineTag` after the intro).
    pub tag: String,
    /// `RemainingItems`.
    pub remaining: u32,
    /// Its intro has ended: it stands where the intro carried it.
    pub entered: bool,
}

/// A droid on its way out of a dispenser.
#[derive(Debug, Clone, PartialEq)]
pub struct Dispensing {
    pub spawner: usize,
    pub slot: usize,
    /// Battle time it appeared.
    pub started: f32,
    /// Let go at this battle time, from here, facing this way.
    pub released: Option<(f32, Vec3, f32)>,
}

/// What a dispenser did, for the game to show.
#[derive(Debug, Clone, PartialEq)]
pub enum DispenserStep {
    /// Switched on: plays its intro animation (`None`: `ActionBreathe`).
    Intro { anim: Option<String> },
    /// One of its looks: `warm_up`, `online`, `slightly_damaged`,
    /// `heavily_damaged`, `shutdown` or `damaged` (see
    /// [`crate::Dispenser`]).
    Look { which: &'static str },
    /// Switched off by its event again.
    Off,
    /// Droid `id` appeared on slot `slot`: the machine plays
    /// `Dispense<slot+1>`.
    Dispense { id: u32, slot: usize },
    /// Droid `id` was let go and unfolds.
    Released { id: u32 },
    /// Droid `id` finished unfolding and fights.
    Unfolded { id: u32 },
    /// Droid `id` was destroyed on its slot with the machine.
    KilledOnSlot { id: u32 },
}

impl Transition {
    fn looks_name(which: &str) -> &'static str {
        match which {
            "warm_up" => "warm_up",
            "online" => "online",
            "slightly_damaged" => "slightly_damaged",
            "heavily_damaged" => "heavily_damaged",
            "shutdown" => "shutdown",
            _ => "damaged",
        }
    }
}

/// Turn `v` by `yaw` (Unreal convention) about Z.
fn turn(v: Vec3, yaw: f32) -> Vec3 {
    let (s, c) = yaw.sin_cos();
    Vec3::new(v.x * c - v.y * s, v.x * s + v.y * c, v.z)
}

/// How a dispenser's base has moved it: the engine's `SetBase` from the
/// map's `AttachTag` (`UGameEngine::LoadMap` bases the actor on the first
/// actor whose tag or name is its `AttachTag`). A based actor is carried
/// rigidly (`ULevel::MoveActor`: its place relative to the base is kept,
/// and it turns as the base turns).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Carry {
    /// The base's turn since the start, as rows (`apply` = rows x point).
    pub turn: [Vec3; 3],
    /// Where the base's turn and move take the level's origin.
    pub shift: Vec3,
    /// The base's change of yaw, radians.
    pub yaw: f32,
}

impl Carry {
    /// Where a point that rode the base from the start is now.
    pub fn apply(&self, p: Vec3) -> Vec3 {
        Vec3::new(
            self.turn[0].dot(p),
            self.turn[1].dot(p),
            self.turn[2].dot(p),
        ) + self.shift
    }
}

fn lerp(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    a + (b - a) * t
}

impl SlotMotion {
    /// The bone at `t` seconds into the animation.
    fn at(&self, t: f32) -> Option<BoneSample> {
        let first = *self.path.first()?;
        if t <= first.seconds {
            return Some(first);
        }
        for w in self.path.windows(2) {
            if t <= w[1].seconds {
                let k = (t - w[0].seconds) / (w[1].seconds - w[0].seconds).max(1e-6);
                return Some(BoneSample {
                    seconds: t,
                    at: lerp(w[0].at, w[1].at, k),
                    x: lerp(w[0].x, w[1].x, k),
                    z: lerp(w[0].z, w[1].z, k),
                });
            }
        }
        self.path.last().copied()
    }

    fn release_seconds(&self) -> f32 {
        self.release * self.seconds
    }
}

impl Unfold {
    fn travel_at(&self, t: f32) -> Vec3 {
        travel_at(&self.travel, t)
    }
}

/// Where a list of (seconds, offset) samples is at `t`, between samples
/// in a straight line; the last offset after the end.
fn travel_at(travel: &[(f32, Vec3)], t: f32) -> Vec3 {
    let Some(&(t0, first)) = travel.first() else {
        return Vec3::ZERO;
    };
    if t <= t0 {
        return first;
    }
    for w in travel.windows(2) {
        if t <= w[1].0 {
            let k = (t - w[0].0) / (w[1].0 - w[0].0).max(1e-6);
            return lerp(w[0].1, w[1].1, k);
        }
    }
    travel.last().map_or(Vec3::ZERO, |p| p.1)
}

impl Battle {
    /// Where dispenser `spawner`'s collision centre is now (Unreal units):
    /// where the map put it, plus how far its intro has carried it.
    pub fn dispenser_location(&self, spawner: usize) -> Vec3 {
        let Some(sp) = self.spawners.get(spawner) else {
            return Vec3::ZERO;
        };
        let f = &sp.factory;
        let (Some(st), Some(m)) = (&sp.disp, &sp.motion) else {
            return f.location;
        };
        let along = match st.phase {
            Phase::Intro { until } => {
                travel_at(&m.intro_travel, m.intro_seconds - (until - self.time))
            }
            _ if st.entered => m.intro_travel.last().map_or(Vec3::ZERO, |p| p.1),
            _ => Vec3::ZERO,
        };
        let at = f.location + turn(along, f.yaw);
        match &sp.carry {
            Some(c) => c.apply(at),
            None => at,
        }
    }

    /// Which way dispenser `spawner` faces now (radians): the map's yaw
    /// plus its base's turn.
    pub fn dispenser_yaw(&self, spawner: usize) -> f32 {
        self.spawners.get(spawner).map_or(0.0, |sp| {
            sp.factory.yaw + sp.carry.as_ref().map_or(0.0, |c| c.yaw)
        })
    }

    /// Set how dispenser `spawner`'s base has carried it (see [`Carry`]).
    pub fn set_dispenser_carry(&mut self, spawner: usize, carry: Option<Carry>) {
        if let Some(sp) = self.spawners.get_mut(spawner) {
            sp.carry = carry;
        }
    }

    /// Give dispenser `spawner` its animation numbers.
    pub fn set_dispenser_motion(&mut self, spawner: usize, motion: DispenserMotion) {
        if let Some(sp) = self.spawners.get_mut(spawner) {
            sp.motion = Some(motion);
        }
    }

    /// Whether droid `id` is still coming out of a dispenser: on its slot
    /// or unfolding. It can't be hit and doesn't act.
    pub fn is_dispensing(&self, id: u32) -> bool {
        self.dispensing.contains_key(&id)
    }

    /// Droid `id` on its way out of a dispenser, if it is.
    pub fn dispensing(&self, id: u32) -> Option<&Dispensing> {
        self.dispensing.get(&id)
    }

    /// The game's weapons part says dispenser `spawner` now has `health`
    /// left (after a hit): its damage steps, or its destruction, as in
    /// `DroidDispenser.TakeDamage`.
    pub fn dispenser_hurt(&mut self, spawner: usize, health: f32) -> Vec<BattleEvent> {
        let mut out = Vec::new();
        let Some(sp) = self.spawners.get_mut(spawner) else {
            return out;
        };
        let Some(d) = sp.factory.dispenser.clone() else {
            return out;
        };
        let Some(st) = sp.disp.as_mut() else {
            return out;
        };
        if st.phase == Phase::Destroyed {
            return out;
        }
        st.health = health;
        let slight = d.max_health * 0.66;
        let heavy = d.max_health * 0.33;
        if !st.slightly && health <= slight && health > heavy {
            st.slightly = true;
            self.look(spawner, "slightly_damaged", &mut out);
        } else if !st.heavily && health <= heavy && health > 0.0 {
            st.heavily = true;
            self.look(spawner, "heavily_damaged", &mut out);
        } else if health <= 0.0 {
            self.destroy_dispenser(spawner, &mut out);
        }
        out
    }

    fn destroy_dispenser(&mut self, i: usize, out: &mut Vec<BattleEvent>) {
        let sp = &mut self.spawners[i];
        let Some(st) = sp.disp.as_mut() else { return };
        st.phase = Phase::Destroyed;
        sp.active = false;
        let riding = st.riding.take();
        out.push(BattleEvent::SpawnerOff {
            spawner: i,
            why: "destroyed",
        });
        self.look(i, "damaged", out);
        // The droid still on its slot goes with it (100000 explosion
        // damage in the original).
        if let Some(id) = riding {
            self.dispensing.remove(&id);
            out.push(BattleEvent::Dispenser {
                spawner: i,
                step: DispenserStep::KilledOnSlot { id },
            });
            out.extend(self.killed(id));
        }
    }

    /// Show one of dispenser `i`'s looks, and send its event.
    fn look(&mut self, i: usize, which: &'static str, out: &mut Vec<BattleEvent>) {
        let sp = &self.spawners[i];
        let Some(d) = &sp.factory.dispenser else {
            return;
        };
        let t = match Transition::looks_name(which) {
            "warm_up" => &d.warm_up,
            "online" => &d.online,
            "slightly_damaged" => &d.slightly_damaged,
            "heavily_damaged" => &d.heavily_damaged,
            "shutdown" => &d.shutdown,
            _ => &d.damaged,
        };
        let event = t.event.clone();
        let name = sp.factory.name.clone();
        out.push(BattleEvent::Dispenser {
            spawner: i,
            step: DispenserStep::Look { which },
        });
        if let Some(e) = event {
            out.push(BattleEvent::Event {
                name: e,
                by: format!("dispenser {name} ({which})"),
            });
        }
    }

    /// An event reached dispenser `i` (named like its tag now).
    pub(super) fn trigger_dispenser(&mut self, i: usize, event: &str, out: &mut Vec<BattleEvent>) {
        let time = self.time;
        let sp = &mut self.spawners[i];
        let Some(st) = sp.disp.as_mut() else { return };
        match st.phase {
            Phase::Offline => {
                // `Offline.Trigger`: play the intro; what comes next waits
                // for its end. (Its spawn points are checked then.)
                let d = sp.factory.dispenser.as_ref().unwrap();
                let seconds = sp.motion.as_ref().map_or(0.0, |m| m.intro_seconds);
                st.phase = Phase::Intro {
                    until: time + seconds,
                };
                let anim = d.intro_anim.clone();
                out.push(BattleEvent::SpawnerOn {
                    spawner: i,
                    by: super::SwitchedOn::Event(event.to_string()),
                });
                out.push(BattleEvent::Dispenser {
                    spawner: i,
                    step: DispenserStep::Intro { anim },
                });
            }
            Phase::WaitOnline => self.go_online(i, out),
            Phase::Online => {
                // `Factory.Online.Trigger`: back offline.
                st.phase = Phase::Offline;
                sp.active = false;
                out.push(BattleEvent::SpawnerOff {
                    spawner: i,
                    why: "event",
                });
                out.push(BattleEvent::Dispenser {
                    spawner: i,
                    step: DispenserStep::Off,
                });
            }
            // Warming up, waiting for a death, shut down or destroyed:
            // those states don't take events.
            _ => {}
        }
    }

    fn go_online(&mut self, i: usize, out: &mut Vec<BattleEvent>) {
        let time = self.time;
        let sp = &mut self.spawners[i];
        if let Some(st) = sp.disp.as_mut() {
            st.phase = Phase::Online;
        }
        sp.active = true;
        // `Factory.Online` begins by making one at once.
        sp.next_spawn = time;
        if sp.factory.spawn_mode == 1 && !sp.factory.spots.is_empty() {
            sp.next_spot = self.rng.next_u32() as usize % sp.factory.spots.len();
        }
        self.look(i, "online", out);
    }

    /// `SetFactoryTimer`: when the next try is.
    fn factory_wait(&mut self, i: usize) -> f32 {
        let f = &self.spawners[i].factory;
        if f.spawn_interval < f.spawn_interval_max {
            let (a, b) = (f.spawn_interval, f.spawn_interval_max);
            self.rng.range(a, b)
        } else {
            f.spawn_interval
        }
    }

    /// Dead or gone droids leave its list (`RemoveDeadActor`); whether any
    /// did.
    fn remove_dead(&mut self, i: usize) -> bool {
        let alive: Vec<u32> = self.alive().map(|f| f.id).collect();
        let Some(st) = self.spawners[i].disp.as_mut() else {
            return false;
        };
        let before = st.spawned.len();
        st.spawned.retain(|id| alive.contains(id));
        st.spawned.len() != before
    }

    /// Run dispenser `i`'s clock: intro, warm-up, waiting, making droids.
    pub(super) fn step_dispenser(&mut self, i: usize, out: &mut Vec<BattleEvent>) {
        let time = self.time;
        let phase = match self.spawners[i].disp.as_ref() {
            Some(st) => st.phase.clone(),
            None => return,
        };
        match phase {
            Phase::Intro { until } if time >= until => {
                // `Offline.AnimEnd`.
                let sp = &mut self.spawners[i];
                sp.disp.as_mut().unwrap().entered = true;
                if !sp.factory.can_spawn() {
                    sp.disp.as_mut().unwrap().phase = Phase::Offline;
                    out.push(BattleEvent::SpawnerRefused { spawner: i });
                    return;
                }
                let d = sp.factory.dispenser.clone().unwrap();
                match &d.online_tag {
                    None if d.warm_up_time > 0.0 => {
                        // Timed from when the intro ended, not from this step.
                        sp.disp.as_mut().unwrap().phase = Phase::WarmUp {
                            until: until + d.warm_up_time,
                        };
                        self.look(i, "warm_up", out);
                    }
                    None => self.go_online(i, out),
                    Some(tag) => {
                        let st = sp.disp.as_mut().unwrap();
                        st.tag = tag.clone();
                        st.phase = Phase::WaitOnline;
                        self.look(i, "warm_up", out);
                    }
                }
            }
            Phase::WarmUp { until } if time >= until => {
                self.go_online(i, out);
                self.spawners[i].next_spawn = until;
            }
            Phase::Waiting { check, ending } if time >= check => {
                // `Factory.Waiting.Timer`.
                // Each wait is timed from when the last one ended.
                if ending {
                    self.go_online(i, out);
                    self.spawners[i].next_spawn = check;
                } else if self.remove_dead(i) {
                    let wait = self.factory_wait(i);
                    if let Some(st) = self.spawners[i].disp.as_mut() {
                        st.phase = Phase::Waiting {
                            check: check + wait,
                            ending: true,
                        };
                    }
                } else if let Some(st) = self.spawners[i].disp.as_mut() {
                    st.phase = Phase::Waiting {
                        check: check + 1.0,
                        ending: false,
                    };
                }
            }
            Phase::Online if time >= self.spawners[i].next_spawn => {
                // `DroidDispenser.Online.Timer`: set the timer, then try.
                let wait = self.factory_wait(i);
                let due = self.spawners[i].next_spawn;
                self.spawners[i].next_spawn = due.max(time - 0.1) + wait.max(0.1);
                self.dispense(i, out);
            }
            Phase::Destroyed => {
                // `Damaged.Tick`: once all it made are dead, its
                // `ExhaustedEvent`.
                let checking = self.spawners[i]
                    .disp
                    .as_ref()
                    .is_some_and(|s| s.check_exhausted);
                if checking {
                    self.remove_dead(i);
                    let sp = &mut self.spawners[i];
                    let st = sp.disp.as_mut().unwrap();
                    if st.spawned.is_empty() {
                        st.check_exhausted = false;
                        if let Some(name) = sp.factory.exhausted_event.clone() {
                            out.push(BattleEvent::Event {
                                name,
                                by: format!("dispenser {} (all its droids gone)", sp.factory.name),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// `DroidDispenser.GenerateActor`.
    fn dispense(&mut self, i: usize, out: &mut Vec<BattleEvent>) -> Try {
        let num_init = self.spawners[i]
            .factory
            .dispenser
            .as_ref()
            .map_or(0, |d| d.num_init_droids);
        let orig_max = self.spawners[i].factory.max_alive;
        {
            let st = self.spawners[i].disp.as_mut().unwrap();
            if !st.init_done && num_init > 0 {
                st.max_alive = num_init;
                if st.spawned.len() as u32 >= st.max_alive {
                    st.max_alive = orig_max;
                    st.init_done = true;
                }
            }
        }
        self.remove_dead(i);
        let sp = &self.spawners[i];
        let f = &sp.factory;
        let st = sp.disp.as_ref().unwrap();
        if (st.spawned.len() as u32) < st.max_alive && (st.remaining > 0 || f.generate_forever) {
            if f.max_in_world > 0.0 {
                let same = |m: usize| {
                    self.spawners[m]
                        .factory
                        .prototype
                        .eq_ignore_ascii_case(&f.prototype)
                };
                let in_world = self
                    .fighters
                    .iter()
                    .zip(&self.made_by)
                    .filter(|(d, &m)| d.mode != Mode::Dead && same(m))
                    .count() as f32;
                if in_world >= f.max_in_world {
                    return Try::NotNow;
                }
            }
            // (A new droid while one is still on a slot can't happen with
            // the maps' numbers: every dispenser's interval is longer than
            // its slots' time to let go.)
            self.make_on_slot(i, out);
            return Try::Made;
        }
        if st.remaining == 0 && !f.generate_forever {
            if let Some(st) = self.spawners[i].disp.as_mut() {
                st.phase = Phase::Shutdown;
            }
            self.spawners[i].active = false;
            self.look(i, "shutdown", out);
            return Try::Spent;
        }
        Try::NotNow
    }

    /// Make a droid on its slot (the middle of `GenerateActor`, then
    /// `SetupNewActor` and `Dispense`).
    fn make_on_slot(&mut self, i: usize, out: &mut Vec<BattleEvent>) {
        let n_spots = self.spawners[i].factory.spots.len();
        let spot_index = self.spawners[i].next_spot % n_spots.max(1);
        // The spawn point gives its tag, events and first behaviour.
        let spot = self.spawners[i].factory.spot(spot_index);
        let params = self.spawners[i].params.clone();
        let id = self.next_id;
        self.next_id += 1;
        // `DetermineNextSpawnPoint` comes before `Dispense`, so the slot is
        // the next spawn point's number.
        let next = match self.spawners[i].factory.spawn_mode {
            1 if n_spots > 1 => {
                let mut k = self.rng.next_u32() as usize % n_spots;
                if k == spot_index {
                    k = (k + 1) % n_spots;
                }
                k
            }
            1 => 0,
            _ => (spot_index + 1) % n_spots.max(1),
        };
        let sp = &mut self.spawners[i];
        sp.next_spot = next;
        sp.made += 1;
        let d = sp.factory.dispenser.as_ref().unwrap();
        let slot = next.min(d.num_dispensers.saturating_sub(1) as usize);
        let (centre, facing) = self.slot_place(i, slot, 0.0, params.movement.collision_height);
        let feet = centre - Vec3::new(0.0, 0.0, params.movement.collision_height);
        let mut fighter =
            Fighter::new(id, params, feet, facing, spot.offspring.start_state.clone());
        fighter.pawn.location = centre;
        fighter.pawn.velocity = Vec3::ZERO;
        fighter.facing = facing;
        fighter.team = crate::battle::team_of(spot.offspring.team)
            .or(crate::battle::team_of(self.spawners[i].factory.team));
        self.fighters.push(fighter);
        self.made_by.push(i);
        self.births.insert(
            id,
            Birth {
                spot: (n_spots > 0).then_some(spot_index),
                hurt: false,
            },
        );
        let sp = &mut self.spawners[i];
        let st = sp.disp.as_mut().unwrap();
        st.spawned.insert(0, id);
        st.riding = Some(id);
        let full = st.spawned.len() as u32 >= st.max_alive;
        let orig = sp.factory.max_alive;
        if sp.factory.start_timer_after_death && full {
            st.max_alive = orig;
            st.init_done = true;
            st.phase = Phase::Waiting {
                check: self.time + 1.0,
                ending: false,
            };
        }
        self.dispensing.insert(
            id,
            Dispensing {
                spawner: i,
                slot,
                started: self.time,
                released: None,
            },
        );
        out.push(BattleEvent::Spawned {
            id,
            spawner: i,
            feet,
            yaw: facing,
        });
        out.push(BattleEvent::Dispenser {
            spawner: i,
            step: DispenserStep::Dispense { id, slot },
        });
    }

    /// Where a droid of half-height `half` stands on slot `slot` of
    /// dispenser `i`, `t` seconds into the slot's animation: its centre
    /// (the actor is attached to the bone and raised by its own collision
    /// height along the bone's Z, as `DroidDispenser.Dispense` does) and
    /// its facing (the bone's X axis).
    fn slot_place(&self, i: usize, slot: usize, t: f32, half: f32) -> (Vec3, f32) {
        let sp = &self.spawners[i];
        let f = &sp.factory;
        let d = f.dispenser.as_ref().unwrap();
        // `bAlignBottom`: the model's origin is the bottom of its
        // collision cylinder.
        let origin = self.dispenser_location(i) - Vec3::new(0.0, 0.0, d.collision_height);
        let yaw = self.dispenser_yaw(i);
        let sample = sp
            .motion
            .as_ref()
            .and_then(|m| m.slots.get(slot))
            .and_then(|s| s.at(t));
        match sample {
            Some(b) => {
                let at = origin + turn(b.at, yaw) + turn(b.z, yaw) * half;
                let x = turn(b.x, yaw);
                (at, x.y.atan2(x.x))
            }
            None => (origin + Vec3::new(0.0, 0.0, half), yaw),
        }
    }

    /// Move droids on their slots and unfolding; let them go and finish
    /// them at the animations' moments.
    pub(super) fn step_dispensing(&mut self, out: &mut Vec<BattleEvent>) {
        let time = self.time;
        let ids: Vec<u32> = self.dispensing.keys().copied().collect();
        for id in ids {
            let Some(dp) = self.dispensing.get(&id).cloned() else {
                continue;
            };
            let Some(k) = self.fighters.iter().position(|f| f.id == id) else {
                self.dispensing.remove(&id);
                continue;
            };
            if self.fighters[k].mode == Mode::Dead {
                self.dispensing.remove(&id);
                continue;
            }
            let half = self.fighters[k].params.movement.collision_height;
            let motion = self.spawners[dp.spawner].motion.clone();
            match dp.released {
                None => {
                    let t = time - dp.started;
                    let release = motion
                        .as_ref()
                        .and_then(|m| m.slots.get(dp.slot))
                        .map_or(0.0, |s| s.release_seconds());
                    let (centre, facing) =
                        self.slot_place(dp.spawner, dp.slot, t.min(release), half);
                    let f = &mut self.fighters[k];
                    f.pawn.location = centre;
                    f.pawn.velocity = Vec3::ZERO;
                    f.facing = facing;
                    if t >= release {
                        // `ReleaseDroid`.
                        let at = dp.started + release;
                        self.dispensing.get_mut(&id).unwrap().released = Some((at, centre, facing));
                        let sp = &mut self.spawners[dp.spawner];
                        if let Some(st) = sp.disp.as_mut() {
                            if st.riding == Some(id) {
                                st.riding = None;
                            }
                        }
                        out.push(BattleEvent::Dispenser {
                            spawner: dp.spawner,
                            step: DispenserStep::Released { id },
                        });
                        if let Some(name) = sp.factory.spawn_event.clone() {
                            out.push(BattleEvent::Event {
                                name,
                                by: format!("dispenser {}", sp.factory.name),
                            });
                        }
                    }
                }
                Some((at, from, facing)) => {
                    let unfold = motion.map(|m| m.unfold).unwrap_or_default();
                    let t = (time - at).min(unfold.seconds);
                    let f = &mut self.fighters[k];
                    f.pawn.location = from + turn(unfold.travel_at(t), facing);
                    f.pawn.velocity = Vec3::ZERO;
                    if time - at >= unfold.seconds {
                        // `Dispense.AnimEnd`: walks, can be hit, thinks.
                        f.home = f.pawn.location;
                        self.dispensing.remove(&id);
                        out.push(BattleEvent::Dispenser {
                            spawner: dp.spawner,
                            step: DispenserStep::Unfolded { id },
                        });
                    }
                }
            }
        }
    }
}
