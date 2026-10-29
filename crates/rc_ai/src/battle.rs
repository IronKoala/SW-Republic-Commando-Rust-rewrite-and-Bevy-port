//! A whole fight on one map: the spawners, the droids they make, and the
//! shots droids hear. The Bevy plugin (`rc_enemies`) and the headless test
//! (`rc_droidsim`) both run this, so they behave the same.
//!
//! Health is not kept here: in the game it belongs to the weapons and
//! combat part, which tells a droid it was hurt (`Fighter::on_damaged`) or
//! destroyed (`Fighter::kill`).

use crate::brain::{AiEvent, Fighter, Mode, Target, View};
use crate::params::AiParams;
use crate::spawns::Factory;
use rc_movement::{Vec3, World};
use rc_nav::NavGraph;
use std::collections::HashSet;
use std::sync::Arc;

/// One spawner and what it has done so far.
#[derive(Debug, Clone)]
pub struct Spawner {
    pub factory: Factory,
    pub active: bool,
    /// Droids made so far.
    pub made: u32,
    next_spawn: f32,
}

/// What happened during a step, for the log and for the game to act on.
#[derive(Debug, Clone, PartialEq)]
pub enum BattleEvent {
    /// A spawner switched on: by its walk-in trigger, or (`by_trigger`
    /// false) by the player coming near one only scripts could start.
    SpawnerOn { spawner: usize, by_trigger: bool },
    /// A new droid. `feet` in Unreal units.
    Spawned {
        id: u32,
        spawner: usize,
        feet: Vec3,
        yaw: f32,
    },
    /// Something a droid decided or did.
    Ai { id: u32, event: AiEvent },
}

/// What a battle needs to carry on where it was, for save files.
#[derive(Debug, Clone, PartialEq)]
pub struct BattleSnapshot {
    pub time: f32,
    pub next_id: u32,
    /// Each spawner, in order: switched on, droids made, battle time of
    /// its next droid.
    pub spawners: Vec<(bool, u32, f32)>,
    /// The living fighters.
    pub fighters: Vec<FighterSnapshot>,
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
    next_id: u32,
}

impl Battle {
    /// `factories` are the spawners this battle runs (the caller picks,
    /// for example only those making battle droids).
    pub fn new(params: Arc<AiParams>, nav: NavGraph, factories: Vec<Factory>) -> Self {
        Battle {
            params,
            nav,
            spawners: factories
                .into_iter()
                .map(|factory| Spawner {
                    factory,
                    active: false,
                    made: 0,
                    next_spawn: 0.0,
                })
                .collect(),
            fighters: Vec::new(),
            made_by: Vec::new(),
            time: 0.0,
            next_id: 1,
        }
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
        let mut out = Vec::new();
        if let Some(p) = player {
            self.spawn(p.centre, &mut out);
        }
        let taken: HashSet<usize> = self.alive().filter_map(|f| f.cover_node()).collect();
        let allies: Vec<Vec3> = self.alive().map(|f| f.pawn.location).collect();
        let mut heard = Vec::new();
        let view = View {
            world,
            nav: &self.nav,
            time: self.time,
            target: player,
            cover_taken: &taken,
            allies: &allies,
        };
        for f in self.fighters.iter_mut() {
            if f.mode == Mode::Dead {
                continue;
            }
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
    pub fn damaged(&mut self, id: u32, from: Option<Vec3>) -> Vec<BattleEvent> {
        let time = self.time;
        let mut ev = Vec::new();
        if let Some(f) = self.fighter_mut(id) {
            f.on_damaged(from, time, &mut ev);
        }
        ev.into_iter()
            .map(|event| BattleEvent::Ai { id, event })
            .collect()
    }

    pub fn killed(&mut self, id: u32) -> Vec<BattleEvent> {
        let mut ev = Vec::new();
        if let Some(f) = self.fighter_mut(id) {
            f.kill(&mut ev);
        }
        ev.into_iter()
            .map(|event| BattleEvent::Ai { id, event })
            .collect()
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
                .map(|s| (s.active, s.made, s.next_spawn))
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
        for (sp, &(active, made, next)) in self.spawners.iter_mut().zip(&s.spawners) {
            sp.active = active;
            sp.made = made;
            sp.next_spawn = next;
        }
        self.time = s.time;
        self.next_id = s.next_id.max(s.fighters.iter().map(|f| f.id + 1).max().unwrap_or(1));
        self.fighters.clear();
        self.made_by.clear();
        let mut out = Vec::new();
        for f in &s.fighters {
            let start = self
                .spawners
                .get(f.spawner)
                .ok_or(format!("fighter {} names spawner {} of {}", f.id, f.spawner, self.spawners.len()))?
                .factory
                .start_state
                .clone();
            let mut fighter = Fighter::new(f.id, self.params.clone(), f.feet, f.facing, start);
            if matches!(f.mode, Mode::Alerted | Mode::Attack | Mode::Investigate) {
                let at = f.last_seen_at.unwrap_or(fighter.pawn.location);
                let mut ev = Vec::new();
                fighter.alert(at, self.time, &mut ev);
            }
            self.fighters.push(fighter);
            self.made_by.push(f.spawner);
            out.push(BattleEvent::Spawned {
                id: f.id,
                spawner: f.spawner,
                feet: f.feet,
                yaw: f.facing,
            });
        }
        Ok(out)
    }

    /// Switch spawners on, and make at most one droid per step.
    fn spawn(&mut self, player: Vec3, out: &mut Vec<BattleEvent>) {
        let s = &self.params.spawning;
        let radius = self.params.movement.collision_radius;
        let alive_total = self.alive().count() as u32;
        for i in 0..self.spawners.len() {
            let sp = &mut self.spawners[i];
            let f = &sp.factory;
            if !sp.active {
                let by_trigger = f
                    .triggers
                    .iter()
                    .any(|&(c, r)| (player - c).length_2d() <= r + radius);
                let near = f.triggers.is_empty()
                    && (f.spot(0).location - player).length() <= s.activate_unscripted_within;
                if by_trigger || near {
                    sp.active = true;
                    out.push(BattleEvent::SpawnerOn {
                        spawner: i,
                        by_trigger,
                    });
                }
                continue;
            }
            let alive_here = self
                .fighters
                .iter()
                .zip(&self.made_by)
                .filter(|(d, &m)| m == i && d.mode != Mode::Dead)
                .count() as u32;
            let more = f.generate_forever || sp.made < f.capacity.max(1);
            if more
                && alive_here < f.max_alive
                && alive_total < s.max_alive_total
                && self.time >= sp.next_spawn
            {
                let spot = f.spot(sp.made as usize);
                let feet =
                    spot.location - Vec3::new(0.0, 0.0, self.params.movement.collision_height);
                let id = self.next_id;
                self.next_id += 1;
                sp.made += 1;
                sp.next_spawn = self.time + f.spawn_interval.max(0.1);
                let start_state = f.start_state.clone();
                self.fighters.push(Fighter::new(
                    id,
                    self.params.clone(),
                    feet,
                    spot.yaw,
                    start_state,
                ));
                self.made_by.push(i);
                out.push(BattleEvent::Spawned {
                    id,
                    spawner: i,
                    feet,
                    yaw: spot.yaw,
                });
                return; // one per step
            }
        }
    }
}
