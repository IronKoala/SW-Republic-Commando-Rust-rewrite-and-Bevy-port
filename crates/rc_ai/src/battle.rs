//! A whole fight on one map: the spawners, the droids they make, and the
//! shots droids hear. The Bevy plugin (`rc_enemies`) and the headless test
//! (`rc_droidsim`) both run this, so they behave the same.
//!
//! Spawners follow the original's rules (see [`crate::spawns`]). In the
//! game the level's scripts switch them on and off by event
//! ([`Battle::trigger`], [`Battle::stop`], [`Battle::reset`]), and the
//! events spawners and their droids send come back as
//! [`BattleEvent::Event`] for the scripts. Without scripts (`scripted`
//! false: the headless tools) a spawner switches itself on when the player
//! walks into its trigger or, if it has none, comes near.
//!
//! Health is not kept here: in the game it belongs to the weapons and
//! combat part, which tells a droid it was hurt (`Fighter::on_damaged`) or
//! destroyed (`Fighter::kill`).

use crate::brain::{AiEvent, Fighter, Mode, Rng, Target, View};
use crate::params::AiParams;
use crate::spawns::{Factory, Offspring};
use rc_movement::{Vec3, World};
use rc_nav::NavGraph;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// One spawner and what it has done so far.
#[derive(Debug, Clone)]
pub struct Spawner {
    pub factory: Factory,
    /// The settings of what it makes (its enemy type).
    pub params: Arc<AiParams>,
    /// Switched on: making characters.
    pub active: bool,
    /// Switched off for good by a script: events no longer switch it on.
    pub disabled: bool,
    /// Characters made that count against its capacity (a script's
    /// `ResetFactory` sets it back to 0).
    pub made: u32,
    next_spawn: f32,
    /// The spawn point it uses next (spawn mode 0 and 1).
    next_spot: usize,
}

/// What switched a spawner on.
#[derive(Debug, Clone, PartialEq)]
pub enum SwitchedOn {
    /// An event named like its tag (from the level's scripts).
    Event(String),
    /// No scripts: the player walked into its trigger.
    WalkInTrigger,
    /// No scripts: the player came near one only scripts could start.
    PlayerNear,
}

/// What happened during a step, for the log and for the game to act on.
#[derive(Debug, Clone, PartialEq)]
pub enum BattleEvent {
    /// A spawner switched on.
    SpawnerOn { spawner: usize, by: SwitchedOn },
    /// A spawner switched off: `why` says how (`"event"` sent again,
    /// `"exhausted"`, `"stopped"`, `"stopped for good"`).
    SpawnerOff { spawner: usize, why: &'static str },
    /// An event switched a spawner on that has nowhere to put characters
    /// (the original refuses too).
    SpawnerRefused { spawner: usize },
    /// A new droid. `feet` in Unreal units.
    Spawned {
        id: u32,
        spawner: usize,
        feet: Vec3,
        yaw: f32,
    },
    /// A droid removed without dying (a script stopped its spawner and
    /// destroyed what it made): the game removes its body.
    Removed { id: u32 },
    /// An event for the level's scripts: a spawner's `SpawnEvent` or
    /// `ExhaustedEvent`, or a droid's death or first-hurt event. `by` says
    /// what sent it.
    Event { name: String, by: String },
    /// Something a droid decided or did.
    Ai { id: u32, event: AiEvent },
}

/// What a battle needs to carry on where it was, for save files.
#[derive(Debug, Clone, PartialEq)]
pub struct BattleSnapshot {
    pub time: f32,
    pub next_id: u32,
    /// Each spawner, in order.
    pub spawners: Vec<SpawnerSnapshot>,
    /// The living fighters.
    pub fighters: Vec<FighterSnapshot>,
}

/// One spawner, for save files.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpawnerSnapshot {
    pub active: bool,
    pub disabled: bool,
    pub made: u32,
    /// Battle time of its next try.
    pub next_spawn: f32,
    pub next_spot: usize,
}

/// Where a fighter was made, so the battle knows its tag and events.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Birth {
    /// The spawn point it was made at; `None` at the factory itself.
    pub spot: Option<usize>,
    /// Already hurt once (its first-hurt event was sent).
    pub hurt: bool,
}

/// One living fighter, for save files.
#[derive(Debug, Clone, PartialEq)]
pub struct FighterSnapshot {
    pub id: u32,
    pub spawner: usize,
    pub feet: Vec3,
    pub facing: f32,
    pub mode: Mode,
    /// Where it last saw (or heard) the enemy.
    pub last_seen_at: Option<Vec3>,
    /// Grenades it still carries; `None` for its type's full count (types
    /// without grenades, and saves from before enemies threw them).
    pub grenades_left: Option<u32>,
    pub birth: Birth,
}

pub struct Battle {
    pub params: Arc<AiParams>,
    pub nav: NavGraph,
    pub spawners: Vec<Spawner>,
    pub fighters: Vec<Fighter>,
    /// Which spawner made each fighter (same order as `fighters`).
    pub made_by: Vec<usize>,
    /// Seconds since the battle began.
    pub time: f32,
    /// The level's scripts switch spawners on and off (see the module
    /// notes). Set by the game when it runs them.
    pub scripted: bool,
    next_id: u32,
    births: HashMap<u32, Birth>,
    rng: Rng,
}

/// What came of a spawner's try at making a character.
enum Try {
    Made,
    /// Not now (enough alive, the player out of range, the level full).
    NotNow,
    /// Its capacity is used up.
    Spent,
}

impl Battle {
    /// `factories` are the spawners this battle runs (the caller picks,
    /// for example only those making battle droids).
    pub fn new(params: Arc<AiParams>, nav: NavGraph, factories: Vec<Factory>) -> Self {
        let typed = factories.into_iter().map(|f| (f, params.clone())).collect();
        Battle::with_types(params, nav, typed)
    }

    /// Spawners making different enemy types, each with its own settings.
    /// `params` gives the whole battle's limits (`[spawning]`).
    pub fn with_types(
        params: Arc<AiParams>,
        nav: NavGraph,
        factories: Vec<(Factory, Arc<AiParams>)>,
    ) -> Self {
        Battle {
            params,
            nav,
            spawners: factories
                .into_iter()
                .map(|(factory, params)| Spawner {
                    factory,
                    params,
                    active: false,
                    disabled: false,
                    made: 0,
                    next_spawn: 0.0,
                    next_spot: 0,
                })
                .collect(),
            fighters: Vec::new(),
            made_by: Vec::new(),
            time: 0.0,
            scripted: false,
            next_id: 1,
            births: HashMap::new(),
            rng: Rng::new(7),
        }
    }

    /// The spawner that made a fighter.
    pub fn spawner_of(&self, id: u32) -> Option<usize> {
        self.fighters
            .iter()
            .position(|f| f.id == id)
            .map(|i| self.made_by[i])
    }

    /// What a fighter was given by its spawner: its tag and events.
    pub fn offspring(&self, id: u32) -> Option<Offspring> {
        let sp = &self.spawners[self.spawner_of(id)?];
        let birth = self.births.get(&id).copied().unwrap_or_default();
        let mut o = match birth.spot.and_then(|i| sp.factory.spots.get(i)) {
            Some(spot) => spot.offspring.clone(),
            None => sp.factory.offspring.clone(),
        };
        if birth.hurt {
            o.first_damaged_event = None;
        }
        Some(o)
    }

    /// The living fighters whose tag (given by their spawner) is `tag`.
    pub fn tagged(&self, tag: &str) -> Vec<u32> {
        self.alive()
            .filter(|f| {
                self.offspring(f.id)
                    .and_then(|o| o.tag)
                    .is_some_and(|t| t.eq_ignore_ascii_case(tag))
            })
            .map(|f| f.id)
            .collect()
    }

    /// The living fighters that follow the level script `script` (a
    /// `ScriptedSequence`'s tag, given by their spawner).
    pub fn following(&self, script: &str) -> Vec<u32> {
        self.alive()
            .filter(|f| {
                self.offspring(f.id)
                    .and_then(|o| o.script)
                    .is_some_and(|t| t.eq_ignore_ascii_case(script))
            })
            .map(|f| f.id)
            .collect()
    }

    /// A script removed a fighter (`ACTION_DestroyPawn`): gone without
    /// dying, so no death event. Returns the [`BattleEvent::Removed`] for
    /// the game to remove its body, if it was alive.
    pub fn remove(&mut self, id: u32) -> Option<BattleEvent> {
        let f = self.fighter_mut(id).filter(|f| f.mode != Mode::Dead)?;
        let mut ev = Vec::new();
        f.kill(&mut ev);
        Some(BattleEvent::Removed { id })
    }

    /// An event was sent in the level: every spawner whose tag it names
    /// switches on, or off if it was on (as in the original). Spawners a
    /// script switched off for good ignore it.
    pub fn trigger(&mut self, event: &str) -> Vec<BattleEvent> {
        let mut out = Vec::new();
        for i in 0..self.spawners.len() {
            let sp = &mut self.spawners[i];
            if !sp.factory.tag.eq_ignore_ascii_case(event) || sp.disabled {
                continue;
            }
            if sp.active {
                sp.active = false;
                out.push(BattleEvent::SpawnerOff {
                    spawner: i,
                    why: "event",
                });
            } else if sp.factory.can_spawn() {
                self.switch_on(i, SwitchedOn::Event(event.to_string()), &mut out);
            } else {
                out.push(BattleEvent::SpawnerRefused { spawner: i });
            }
        }
        out
    }

    /// A script stopped the spawners tagged `tag` (`StopFactorySpawning`):
    /// switched off (for good when `for_good`), and what they made removed
    /// when `remove_offspring` (removed, not killed: no death events).
    pub fn stop(&mut self, tag: &str, for_good: bool, remove_offspring: bool) -> Vec<BattleEvent> {
        let mut out = Vec::new();
        for i in 0..self.spawners.len() {
            if !self.spawners[i].factory.tag.eq_ignore_ascii_case(tag) {
                continue;
            }
            if remove_offspring {
                for (f, &m) in self.fighters.iter_mut().zip(&self.made_by) {
                    if m == i && f.mode != Mode::Dead {
                        let mut ev = Vec::new();
                        f.kill(&mut ev);
                        out.push(BattleEvent::Removed { id: f.id });
                    }
                }
            }
            let sp = &mut self.spawners[i];
            let was = sp.active;
            sp.active = false;
            sp.disabled |= for_good;
            if was || for_good {
                out.push(BattleEvent::SpawnerOff {
                    spawner: i,
                    why: if for_good {
                        "stopped for good"
                    } else {
                        "stopped"
                    },
                });
            }
        }
        out
    }

    /// A script refilled the spawners tagged `tag` (`ResetFactory`): their
    /// whole capacity is available again. Returns how many.
    pub fn reset(&mut self, tag: &str) -> usize {
        let mut n = 0;
        for sp in &mut self.spawners {
            if sp.factory.tag.eq_ignore_ascii_case(tag) {
                sp.made = 0;
                n += 1;
            }
        }
        n
    }

    pub fn alive(&self) -> impl Iterator<Item = &Fighter> {
        self.fighters.iter().filter(|f| f.mode != Mode::Dead)
    }

    pub fn fighter_mut(&mut self, id: u32) -> Option<&mut Fighter> {
        self.fighters.iter_mut().find(|f| f.id == id)
    }

    /// Whether any living droid is fighting (has seen or heard the enemy).
    pub fn in_fight(&self) -> bool {
        self.alive()
            .any(|f| matches!(f.mode, Mode::Alerted | Mode::Attack))
    }

    /// Advance by `dt` seconds. `player` is where the player is (for
    /// spawners and for droids to look at); `None` while there's no player.
    pub fn step(&mut self, dt: f32, world: &dyn World, player: Option<Target>) -> Vec<BattleEvent> {
        self.step_with(dt, world, player, &[])
    }

    /// Like [`Battle::step`], with other enemies of the droids besides the
    /// player (squad mates). Each droid fights the nearest one it can see,
    /// and the player when it sees none (so it still hunts the player).
    /// Spawners only react to the player.
    pub fn step_with(
        &mut self,
        dt: f32,
        world: &dyn World,
        player: Option<Target>,
        others: &[Target],
    ) -> Vec<BattleEvent> {
        let mut out = Vec::new();
        self.spawn(player.map(|p| p.centre), &mut out);
        let taken: HashSet<usize> = self.alive().filter_map(|f| f.cover_node()).collect();
        let allies: Vec<Vec3> = self.alive().map(|f| f.pawn.location).collect();
        let mut heard = Vec::new();
        let mut targets: Vec<Target> = player.into_iter().collect();
        targets.extend_from_slice(others);
        for f in self.fighters.iter_mut() {
            if f.mode == Mode::Dead {
                continue;
            }
            // Stunned (a flash or EMP grenade): stands still, does nothing.
            if self.time < f.stunned_until {
                f.pawn.velocity = Vec3::ZERO;
                continue;
            }
            let target = f.choose_target(world, &targets).or(player);
            let view = View {
                world,
                nav: &self.nav,
                time: self.time,
                target,
                cover_taken: &taken,
                allies: &allies,
            };
            for e in f.tick(dt, &view) {
                if let AiEvent::Fire { origin, .. } = e {
                    heard.push(origin);
                }
                out.push(BattleEvent::Ai { id: f.id, event: e });
            }
        }
        self.keep_apart(world);
        for at in heard {
            out.extend(self.hear(at));
        }
        self.time += dt;
        out
    }

    /// A shot (or other loud noise) at `at`: every droid in earshot reacts.
    pub fn hear(&mut self, at: Vec3) -> Vec<BattleEvent> {
        let mut out = Vec::new();
        for f in self.fighters.iter_mut() {
            let mut ev = Vec::new();
            f.hear(at, self.time, &mut ev);
            out.extend(
                ev.into_iter()
                    .map(|event| BattleEvent::Ai { id: f.id, event }),
            );
        }
        out
    }

    /// The droid was hurt; `from` is where the shot came from, if known.
    /// The first time, its first-hurt event (if it has one) is sent.
    pub fn damaged(&mut self, id: u32, from: Option<Vec3>) -> Vec<BattleEvent> {
        let time = self.time;
        let mut ev = Vec::new();
        let mut alive = false;
        if let Some(f) = self.fighter_mut(id) {
            alive = f.mode != Mode::Dead;
            f.on_damaged(from, time, &mut ev);
        }
        let mut out: Vec<BattleEvent> = ev
            .into_iter()
            .map(|event| BattleEvent::Ai { id, event })
            .collect();
        if alive {
            if let Some(name) = self.offspring(id).and_then(|o| o.first_damaged_event) {
                out.push(BattleEvent::Event {
                    name,
                    by: format!("droid {id} (first hurt)"),
                });
            }
            self.births.entry(id).or_default().hurt = true;
        }
        out
    }

    /// The droid can't act for `seconds` (a flash or EMP blast).
    pub fn stun(&mut self, id: u32, seconds: f32) {
        let until = self.time + seconds;
        if let Some(f) = self.fighter_mut(id) {
            f.stunned_until = f.stunned_until.max(until);
        }
    }

    /// The droid was destroyed. Its death event (if it has one) is sent.
    pub fn killed(&mut self, id: u32) -> Vec<BattleEvent> {
        let mut ev = Vec::new();
        let mut was_alive = false;
        if let Some(f) = self.fighter_mut(id) {
            was_alive = f.mode != Mode::Dead;
            f.kill(&mut ev);
        }
        let mut out: Vec<BattleEvent> = ev
            .into_iter()
            .map(|event| BattleEvent::Ai { id, event })
            .collect();
        if was_alive {
            if let Some(name) = self.offspring(id).and_then(|o| o.event) {
                out.push(BattleEvent::Event {
                    name,
                    by: format!("droid {id} (destroyed)"),
                });
            }
        }
        out
    }

    /// For tests and demos with a stand-in player: a path node 2500-4500 uu
    /// along the paths from some spawner's walk-in trigger, and that
    /// trigger's centre. Returns (node, trigger centre, spawner).
    pub fn approach(&self) -> Option<(usize, Vec3, usize)> {
        for (k, sp) in self.spawners.iter().enumerate() {
            for &(c, _) in &sp.factory.triggers {
                for (i, n) in self.nav.nodes.iter().enumerate() {
                    if !matches!(n.kind, rc_nav::NodeKind::Path) || self.nav.blocked.contains(&i) {
                        continue;
                    }
                    if !(2000.0..4500.0).contains(&(n.location - c).length()) {
                        continue;
                    }
                    if let Some(r) = self.nav.route(n.location, c) {
                        if (2500.0..4500.0).contains(&r.length) {
                            return Some((i, c, k));
                        }
                    }
                }
            }
        }
        None
    }

    /// Characters don't collide with each other in `rc_movement` (only
    /// with the level), so two droids could walk to the same spot and
    /// stand inside each other. Push any overlapping pair apart, each by
    /// half the overlap, where the level allows.
    fn keep_apart(&mut self, world: &dyn World) {
        let n = self.fighters.len();
        for i in 0..n {
            for j in i + 1..n {
                let (a, b) = (&self.fighters[i], &self.fighters[j]);
                if a.mode == Mode::Dead || b.mode == Mode::Dead {
                    continue;
                }
                let need = a.pawn.extent().x + b.pawn.extent().x;
                let d = (b.pawn.location - a.pawn.location).flat();
                let dist = d.length();
                if dist >= need
                    || (a.pawn.location.z - b.pawn.location.z).abs()
                        > a.pawn.extent().z + b.pawn.extent().z
                {
                    continue;
                }
                // Exactly on top of each other: split along a direction
                // that depends on the pair, so it is the same every run.
                let dir = if dist > 1.0 {
                    d * (1.0 / dist)
                } else {
                    let k = (a.id * 7 + b.id) as f32;
                    Vec3::new(k.cos(), k.sin(), 0.0)
                };
                let push = dir * ((need - dist) / 2.0 + 1.0);
                for (k, delta) in [(i, push * -1.0), (j, push)] {
                    let p = &mut self.fighters[k].pawn;
                    if world
                        .sweep(p.extent(), p.location, p.location + delta)
                        .is_none()
                    {
                        p.location = p.location + delta;
                    }
                }
            }
        }
    }

    /// Everything needed to carry on later (see [`Battle::restore`]).
    /// Dead fighters are left out: only their spawner's count matters.
    pub fn snapshot(&self) -> BattleSnapshot {
        BattleSnapshot {
            time: self.time,
            next_id: self.next_id,
            spawners: self
                .spawners
                .iter()
                .map(|s| SpawnerSnapshot {
                    active: s.active,
                    disabled: s.disabled,
                    made: s.made,
                    next_spawn: s.next_spawn,
                    next_spot: s.next_spot,
                })
                .collect(),
            fighters: self
                .fighters
                .iter()
                .zip(&self.made_by)
                .filter(|(f, _)| f.mode != Mode::Dead)
                .map(|(f, &spawner)| FighterSnapshot {
                    id: f.id,
                    spawner,
                    feet: f.pawn.feet(),
                    facing: f.facing,
                    mode: f.mode,
                    last_seen_at: f.last_seen_at,
                    grenades_left: (!f.params.grenades.grenade.is_empty())
                        .then_some(f.grenades_left),
                    birth: self.births.get(&f.id).copied().unwrap_or_default(),
                })
                .collect(),
        }
    }

    /// Carry on from a snapshot, on a battle freshly made for the same map.
    /// Returns a `Spawned` event for each fighter put back, so the game
    /// makes their bodies the usual way. Fighters that were in a fight
    /// start alerted toward where they last saw the enemy; the rest of
    /// their plans start over.
    pub fn restore(&mut self, s: &BattleSnapshot) -> Result<Vec<BattleEvent>, String> {
        if s.spawners.len() != self.spawners.len() {
            return Err(format!(
                "the save has {} spawners, this map has {}",
                s.spawners.len(),
                self.spawners.len()
            ));
        }
        for (sp, saved) in self.spawners.iter_mut().zip(&s.spawners) {
            sp.active = saved.active;
            sp.disabled = saved.disabled;
            sp.made = saved.made;
            sp.next_spawn = saved.next_spawn;
            sp.next_spot = saved.next_spot;
        }
        self.time = s.time;
        self.next_id = s
            .next_id
            .max(s.fighters.iter().map(|f| f.id + 1).max().unwrap_or(1));
        self.fighters.clear();
        self.made_by.clear();
        self.births.clear();
        let mut out = Vec::new();
        for f in &s.fighters {
            let sp = self.spawners.get(f.spawner).ok_or(format!(
                "fighter {} names spawner {} of {}",
                f.id,
                f.spawner,
                self.spawners.len()
            ))?;
            // Its spawner's enemy type, not the battle's default, and the
            // first behaviour its spawn point (or factory) gave it.
            let start = match f.birth.spot.and_then(|i| sp.factory.spots.get(i)) {
                Some(spot) => spot.offspring.start_state.clone(),
                None => sp.factory.offspring.start_state.clone(),
            };
            let params = sp.params.clone();
            let mut fighter = Fighter::new(f.id, params, f.feet, f.facing, start);
            if let Some(n) = f.grenades_left {
                fighter.grenades_left = n.min(fighter.grenades_left);
            }
            if matches!(f.mode, Mode::Alerted | Mode::Attack | Mode::Investigate) {
                let at = f.last_seen_at.unwrap_or(fighter.pawn.location);
                let mut ev = Vec::new();
                fighter.alert(at, self.time, &mut ev);
            }
            self.fighters.push(fighter);
            self.made_by.push(f.spawner);
            self.births.insert(f.id, f.birth);
            out.push(BattleEvent::Spawned {
                id: f.id,
                spawner: f.spawner,
                feet: f.feet,
                yaw: f.facing,
            });
        }
        Ok(out)
    }

    fn switch_on(&mut self, i: usize, by: SwitchedOn, out: &mut Vec<BattleEvent>) {
        let n = self.spawners[i].factory.spots.len();
        let sp = &mut self.spawners[i];
        sp.active = true;
        // The original makes one straight away.
        sp.next_spawn = self.time;
        if sp.factory.spawn_mode == 1 && n > 0 {
            sp.next_spot = self.rng.next_u32() as usize % n;
        }
        out.push(BattleEvent::SpawnerOn { spawner: i, by });
    }

    /// Without scripts: switch spawners on when the player walks into
    /// their trigger, or comes near one that has none.
    fn switch_on_unscripted(&mut self, player: Vec3, out: &mut Vec<BattleEvent>) {
        let within = self.params.spawning.activate_unscripted_within;
        for i in 0..self.spawners.len() {
            let sp = &self.spawners[i];
            if sp.active || sp.disabled || !sp.factory.can_spawn() {
                continue;
            }
            let f = &sp.factory;
            let radius = sp.params.movement.collision_radius;
            let by_trigger = f
                .triggers
                .iter()
                .any(|&(c, r)| (player - c).length_2d() <= r + radius);
            let near = f.triggers.is_empty() && (f.spot(0).location - player).length() <= within;
            if by_trigger {
                self.switch_on(i, SwitchedOn::WalkInTrigger, out);
            } else if near {
                self.switch_on(i, SwitchedOn::PlayerNear, out);
            }
        }
    }

    /// Spawners that are on try to make a character when their timer is
    /// up. At most one new droid per step, to spread the work.
    fn spawn(&mut self, player: Option<Vec3>, out: &mut Vec<BattleEvent>) {
        if !self.scripted {
            if let Some(p) = player {
                self.switch_on_unscripted(p, out);
            }
        }
        let mut made_one = false;
        for i in 0..self.spawners.len() {
            let sp = &self.spawners[i];
            if !sp.active || self.time < sp.next_spawn || made_one {
                continue;
            }
            let f = &sp.factory;
            let wait = if f.spawn_interval_max > f.spawn_interval {
                self.rng.range(f.spawn_interval, f.spawn_interval_max)
            } else {
                f.spawn_interval
            };
            self.spawners[i].next_spawn = self.time + wait.max(0.1);
            match self.try_make(i, player, out) {
                Try::Made => made_one = true,
                Try::NotNow => {}
                Try::Spent => {
                    let alive_here = self.alive_from(i);
                    let sp = &mut self.spawners[i];
                    if alive_here == 0 && !sp.factory.generate_forever {
                        sp.active = false;
                        if let Some(name) = sp.factory.exhausted_event.clone() {
                            out.push(BattleEvent::Event {
                                name,
                                by: format!("spawner {} (exhausted)", sp.factory.name),
                            });
                        }
                        out.push(BattleEvent::SpawnerOff {
                            spawner: i,
                            why: "exhausted",
                        });
                    }
                }
            }
        }
    }

    fn alive_from(&self, spawner: usize) -> u32 {
        self.fighters
            .iter()
            .zip(&self.made_by)
            .filter(|(d, &m)| m == spawner && d.mode != Mode::Dead)
            .count() as u32
    }

    fn try_make(&mut self, i: usize, player: Option<Vec3>, out: &mut Vec<BattleEvent>) -> Try {
        let alive_here = self.alive_from(i);
        let alive_total = self.alive().count() as u32;
        let sp = &self.spawners[i];
        let f = &sp.factory;
        if !f.generate_forever && sp.made >= f.capacity {
            return Try::Spent;
        }
        if alive_here >= f.max_alive || alive_total >= self.params.spawning.max_alive_total {
            return Try::NotNow;
        }
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
        // Where: the factory, the spawn point nearest the player, or the
        // next spawn point (in turn or random).
        let spot_index = if f.at_factory() || f.spots.is_empty() {
            None
        } else if f.spawn_mode == 3 {
            let p = player.unwrap_or(f.location);
            (0..f.spots.len()).min_by(|&a, &b| {
                let da = (f.spots[a].location - p).length();
                let db = (f.spots[b].location - p).length();
                da.total_cmp(&db)
            })
        } else {
            Some(sp.next_spot % f.spots.len())
        };
        let spot = match spot_index {
            Some(k) => f.spots[k].clone(),
            None => f.spot(0),
        };
        if f.min_player_range < f.max_player_range {
            if let Some(p) = player {
                let d = (spot.location - p).length();
                if d < f.min_player_range || d > f.max_player_range {
                    return Try::NotNow;
                }
            }
        }
        let params = sp.params.clone();
        let spawn_event = f.spawn_event.clone();
        let name = f.name.clone();
        let (mode, spots) = (f.spawn_mode, f.spots.len());
        let feet = spot.location - Vec3::new(0.0, 0.0, params.movement.collision_height);
        let id = self.next_id;
        self.next_id += 1;
        let next_spot = match mode {
            1 if spots > 1 => {
                let now = self.spawners[i].next_spot;
                let mut k = self.rng.next_u32() as usize % spots;
                if k == now {
                    k = (k + 1) % spots;
                }
                k
            }
            _ => self.spawners[i].next_spot + 1,
        };
        let sp = &mut self.spawners[i];
        sp.made += 1;
        sp.next_spot = next_spot;
        self.fighters.push(Fighter::new(
            id,
            params,
            feet,
            spot.yaw,
            spot.offspring.start_state.clone(),
        ));
        self.made_by.push(i);
        self.births.insert(
            id,
            Birth {
                spot: spot_index,
                hurt: false,
            },
        );
        out.push(BattleEvent::Spawned {
            id,
            spawner: i,
            feet,
            yaw: spot.yaw,
        });
        if let Some(name_) = spawn_event {
            out.push(BattleEvent::Event {
                name: name_,
                by: format!("spawner {name}"),
            });
        }
        Try::Made
    }
}
