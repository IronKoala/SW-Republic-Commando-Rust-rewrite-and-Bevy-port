//! The `[enemies]` section of a save file: the battle's spawners and each
//! living droid's place, health and whether it was fighting (see
//! `rc_game_core::save`). Droids that were destroyed stay destroyed, since
//! their spawner remembers how many it made.

use crate::Fight;
use bevy::prelude::*;
use rc_ai::{BattleSnapshot, Birth, DispenserSnapshot, FighterSnapshot, Mode, SpawnerSnapshot};
use rc_combat::Health;
use rc_game_core::save::SavePart;
use rc_game_core::GameLog;
use rc_movement::Vec3 as UVec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnemiesSave {
    /// Seconds since the fight on this map began.
    pub time: f32,
    pub next_id: u32,
    /// One per spawner on the map, in the map's order.
    pub spawners: Vec<SpawnerSave>,
    pub droids: Vec<DroidSave>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpawnerSave {
    pub on: bool,
    /// Switched off for good by a script.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
    pub made: u32,
    /// Has been switched on at some time (counts for battle music
    /// volumes). Saves from before this field: on now, or made any.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub was_on: bool,
    pub next_spawn: f32,
    /// The spawn point it uses next.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub next_spot: usize,
    /// A droid dispenser's own state (see `rc_ai::battle::dispense`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispenser: Option<DispenserSave>,
}

/// A droid dispenser: where it is in its steps (offline, intro, warm_up,
/// wait_online, online, waiting, shutdown, destroyed), its health and
/// damage steps, and the droids it made that are still about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DispenserSave {
    pub phase: String,
    /// When its intro, warm-up or wait ends (seconds into the fight).
    pub until: f32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ending: bool,
    pub health: f32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub slightly: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub heavily: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub init_done: bool,
    pub max_alive: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spawned: Vec<u32>,
    /// The event it answers now (its tag, or its online tag).
    pub tag: String,
    pub remaining: u32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub check_exhausted: bool,
    /// Its intro has ended: it stands where the intro carried it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub entered: bool,
}

impl From<&DispenserSnapshot> for DispenserSave {
    fn from(d: &DispenserSnapshot) -> Self {
        DispenserSave {
            phase: d.phase.clone(),
            until: d.until,
            ending: d.ending,
            health: d.health,
            slightly: d.slightly,
            heavily: d.heavily,
            init_done: d.init_done,
            max_alive: d.max_alive,
            spawned: d.spawned.clone(),
            tag: d.tag.clone(),
            remaining: d.remaining,
            check_exhausted: d.check_exhausted,
            entered: d.entered,
        }
    }
}

impl From<&DispenserSave> for DispenserSnapshot {
    fn from(d: &DispenserSave) -> Self {
        DispenserSnapshot {
            phase: d.phase.clone(),
            until: d.until,
            ending: d.ending,
            health: d.health,
            slightly: d.slightly,
            heavily: d.heavily,
            init_done: d.init_done,
            max_alive: d.max_alive,
            spawned: d.spawned.clone(),
            tag: d.tag.clone(),
            remaining: d.remaining,
            check_exhausted: d.check_exhausted,
            entered: d.entered,
        }
    }
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DroidSave {
    pub id: u32,
    pub spawner: usize,
    /// Feet, Unreal units.
    pub feet: [f32; 3],
    pub facing: f32,
    pub health: f32,
    /// idle, alerted, attack or investigate.
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<[f32; 3]>,
    /// Grenades it still carries (only enemies that carry them).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grenades_left: Option<u32>,
    /// The spawn point it was made at (its tag and events come from
    /// there); none: at its spawner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot: Option<usize>,
    /// Already hurt once (its first-hurt event was sent).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hurt: bool,
}

fn arr(v: UVec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn vec([x, y, z]: [f32; 3]) -> UVec3 {
    UVec3::new(x, y, z)
}

fn mode(name: &str) -> Mode {
    match name {
        "alerted" => Mode::Alerted,
        "attack" => Mode::Attack,
        "investigate" => Mode::Investigate,
        _ => Mode::Idle,
    }
}

impl SavePart for EnemiesSave {
    const KEY: &'static str = "enemies";

    fn save(world: &mut World) -> Option<Self> {
        let fight = world.get_resource::<Fight>()?;
        let snap = fight.battle.snapshot();
        let entities = fight.entities.clone();
        let droids = snap
            .fighters
            .iter()
            .map(|f| DroidSave {
                id: f.id,
                spawner: f.spawner,
                feet: arr(f.feet),
                facing: f.facing,
                health: entities
                    .get(&f.id)
                    .and_then(|&e| world.get::<Health>(e))
                    .map_or(0.0, |h| h.current),
                mode: f.mode.name().into(),
                last_seen_at: f.last_seen_at.map(arr),
                grenades_left: f.grenades_left,
                spot: f.birth.spot,
                hurt: f.birth.hurt,
            })
            .collect();
        Some(EnemiesSave {
            time: snap.time,
            next_id: snap.next_id,
            spawners: snap
                .spawners
                .iter()
                .map(|s| SpawnerSave {
                    on: s.active,
                    disabled: s.disabled,
                    made: s.made,
                    was_on: s.was_on,
                    next_spawn: s.next_spawn,
                    next_spot: s.next_spot,
                    dispenser: s.dispenser.as_ref().map(DispenserSave::from),
                })
                .collect(),
            droids,
        })
    }

    fn load(world: &mut World, d: Self) -> Result<(), String> {
        let mut fight = world
            .get_resource_mut::<Fight>()
            .ok_or("no fight on this map")?;
        let snap = BattleSnapshot {
            time: d.time,
            next_id: d.next_id,
            spawners: d
                .spawners
                .iter()
                .map(|s| SpawnerSnapshot {
                    active: s.on,
                    disabled: s.disabled,
                    made: s.made,
                    was_on: s.was_on || s.on || s.made > 0,
                    next_spawn: s.next_spawn,
                    next_spot: s.next_spot,
                    dispenser: s.dispenser.as_ref().map(DispenserSnapshot::from),
                })
                .collect(),
            fighters: d
                .droids
                .iter()
                .map(|f| FighterSnapshot {
                    id: f.id,
                    spawner: f.spawner,
                    feet: vec(f.feet),
                    facing: f.facing,
                    mode: mode(&f.mode),
                    last_seen_at: f.last_seen_at.map(vec),
                    grenades_left: f.grenades_left,
                    birth: Birth {
                        spot: f.spot,
                        hurt: f.hurt,
                    },
                })
                .collect(),
        };
        let events = fight.battle.restore(&snap)?;
        fight.restored = events;
        fight.dispensers_restored = true;
        fight.restored_health = d.droids.iter().map(|f| (f.id, f.health)).collect();
        let on = d.spawners.iter().filter(|s| s.on).count();
        let fighting = d.droids.iter().filter(|f| f.mode != "idle").count();
        world.resource::<GameLog>().line(format!(
            "enemies: put back t={:.1}s: {} droids ({fighting} fighting), {on} of {} spawners on",
            d.time,
            d.droids.len(),
            d.spawners.len()
        ));
        Ok(())
    }
}
