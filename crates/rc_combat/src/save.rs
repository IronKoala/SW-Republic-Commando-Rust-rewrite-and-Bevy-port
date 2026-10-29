//! The player's `[combat]` section in a save file: health, shield and the
//! weapons carried with their ammo (see `rc_game_core::save`).

use crate::health::{Dead, Health, Shield, Team};
use crate::library::WeaponLibrary;
use crate::weapon::WeaponState;
use crate::{Arsenal, Hittable};
use bevy::prelude::*;
use rc_game_core::save::SavePart;
use rc_game_core::{GameLog, Player};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CombatSave {
    pub health: f32,
    /// None if the player has no shield.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shield: Option<f32>,
    /// The weapon in hand, by id.
    pub current: String,
    pub weapons: Vec<WeaponSave>,
}

/// One weapon carried.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeaponSave {
    /// The weapon file's name without `.toml`, like `dc17m_blaster`.
    pub id: String,
    /// Shots in the magazine (or the charge, for recharging weapons).
    pub clip: f32,
    /// Ammo outside the magazine.
    pub reserve: u32,
    /// Seconds of reload left, if it was reloading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reloading: Option<f32>,
}

impl SavePart for CombatSave {
    const KEY: &'static str = "combat";

    fn save(world: &mut World) -> Option<Self> {
        let mut q = world
            .query_filtered::<(&Health, Option<&Shield>, &Arsenal), With<Player>>();
        let (h, s, a) = q.iter(world).next()?;
        Some(CombatSave {
            health: h.current,
            shield: s.map(|s| s.current),
            current: a.current().map(|w| w.0.clone()).unwrap_or_default(),
            weapons: a
                .weapons
                .iter()
                .map(|(id, w)| WeaponSave {
                    id: id.clone(),
                    clip: w.clip,
                    reserve: w.reserve,
                    reloading: w.reloading,
                })
                .collect(),
        })
    }

    /// The player was just spawned and may not be equipped yet; this
    /// equips them from the save, so the usual starting kit isn't given.
    fn load(world: &mut World, d: Self) -> Result<(), String> {
        let lib = world
            .get_resource::<WeaponLibrary>()
            .ok_or("the weapon files aren't loaded")?;
        let setup = lib.player.clone();
        let mut weapons = Vec::new();
        let mut unknown = Vec::new();
        for w in &d.weapons {
            match lib.weapons.get(&w.id) {
                Some(p) => {
                    let mut state = WeaponState::new(p);
                    state.clip = w.clip;
                    state.reserve = w.reserve;
                    state.reloading = w.reloading;
                    weapons.push((w.id.clone(), state));
                }
                None => unknown.push(w.id.clone()),
            }
        }
        let current = weapons.iter().position(|w| w.0 == d.current).unwrap_or(0);
        let mut q = world.query_filtered::<Entity, With<Player>>();
        let player = q.iter(world).next().ok_or("no player in the level")?;
        let mut shield = Shield::new(
            setup.shield,
            setup.shield_recharge_rate,
            setup.shield_recharge_delay,
        );
        if let Some(s) = d.shield {
            shield.current = s.min(shield.max);
        }
        let mut health = Health::new(setup.health);
        health.current = d.health.min(health.max);
        let mut e = world.entity_mut(player);
        e.insert((
            health,
            shield,
            Team::Republic,
            Hittable::COMMANDO,
            Arsenal { weapons, current },
        ));
        if health.is_dead() {
            e.insert(Dead);
        } else {
            e.remove::<Dead>();
        }
        let log = world.resource::<GameLog>();
        log.line(format!(
            "weapons: player put back: health {:.0}, shield {:?}, holding {}, {} weapons",
            d.health,
            d.shield,
            d.current,
            d.weapons.len()
        ));
        if !unknown.is_empty() {
            return Err(format!("unknown weapons left out: {unknown:?}"));
        }
        Ok(())
    }
}
