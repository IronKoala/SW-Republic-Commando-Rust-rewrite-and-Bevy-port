//! The `[enemies]` section of a save file: the battle's spawners and each
//! living droid's place, health and whether it was fighting (see
//! `rc_game_core::save`). Droids that were destroyed stay destroyed, since
//! their spawner remembers how many it made.

use crate::Fight;
use bevy::prelude::*;
use rc_ai::{BattleSnapshot, Birth, FighterSnapshot, Mode, SpawnerSnapshot};
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
    pub next_spawn: f32,
    /// The spawn point it uses next.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub next_spot: usize,
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
                    next_spawn: s.next_spawn,
                    next_spot: s.next_spot,
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
                    next_spawn: s.next_spawn,
                    next_spot: s.next_spot,
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
                    birth: Birth {
                        spot: f.spot,
                        hurt: f.hurt,
                    },
                })
                .collect(),
        };
        let events = fight.battle.restore(&snap)?;
        fight.restored = events;
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
