//! Weapons lying on the ground (an enemy's weapon after it falls) that the
//! player picks up by walking over them.

use crate::health::Dead;
use crate::library::WeaponLibrary;
use crate::weapon::WeaponState;
use crate::Arsenal;
use bevy::prelude::*;
use rc_game_core::units::UU_PER_UNIT;
use rc_game_core::{GameLog, GameState, Player};

/// How close the player's feet must come, uu. **Our choice** (the
/// original uses each pickup's collision size, which we don't read yet).
pub const PICKUP_REACH: f32 = 80.0;

/// A weapon on the ground. Spawn one with a `Transform` (Bevy space, at
/// the ground) and the player collects it on touching it.
#[derive(Component, Debug, Clone)]
pub struct WeaponPickup {
    /// A weapon file name without `.toml`.
    pub weapon: String,
    /// Shots it holds. `None`: the weapon file's `start_ammo`.
    pub ammo: Option<u32>,
}

/// Spawn a pickup (with the bolt look of its weapon, if the game draws).
pub fn spawn_pickup(commands: &mut Commands, weapon: &str, at: Vec3) -> Entity {
    commands
        .spawn((
            WeaponPickup {
                weapon: weapon.to_string(),
                ammo: None,
            },
            Transform::from_translation(at + Vec3::Y * (10.0 / UU_PER_UNIT)),
            Visibility::default(),
            DespawnOnExit(GameState::InGame),
        ))
        .id()
}

/// Give the pickup's weapon (or its ammo) to a player standing on it.
pub(crate) fn collect_pickups(
    mut commands: Commands,
    lib: Res<WeaponLibrary>,
    mut players: Query<(&Transform, &mut Arsenal), (With<Player>, Without<Dead>)>,
    pickups: Query<(Entity, &Transform, &WeaponPickup), Without<Player>>,
    looks: Option<Res<crate::view::BoltLooks>>,
    shown: Query<(), With<Mesh3d>>,
    log: Option<Res<GameLog>>,
) {
    let Ok((me, mut arsenal)) = players.single_mut() else {
        return;
    };
    let say = |s: String| match &log {
        Some(l) => l.line(s),
        None => println!("{s}"),
    };
    for (e, t, p) in &pickups {
        if let (Some(looks), false) = (&looks, shown.contains(e)) {
            looks.attach(&mut commands.entity(e), &p.weapon);
        }
        let gap = (t.translation - me.translation) * UU_PER_UNIT;
        if Vec2::new(gap.x, gap.z).length() > PICKUP_REACH || gap.y.abs() > 120.0 {
            continue;
        }
        let Some(params) = lib.weapons.get(&p.weapon) else {
            say(format!("weapons: pickup of unknown weapon `{}`", p.weapon));
            commands.entity(e).despawn();
            continue;
        };
        let ammo = p.ammo.unwrap_or(params.start_ammo).min(params.max_ammo);
        match arsenal.weapons.iter_mut().find(|(id, _)| *id == p.weapon) {
            Some((_, state)) => {
                let room = params
                    .max_ammo
                    .saturating_sub(state.shots_in_clip() + state.reserve);
                let got = ammo.min(room);
                if got == 0 {
                    continue; // full: leave it lying
                }
                state.reserve += got;
                say(format!(
                    "weapons: picked up {got} shots for {} (spare now {})",
                    p.weapon, state.reserve
                ));
            }
            None => {
                let mut state = WeaponState::new(params);
                if !params.recharges() {
                    let clip = params.clip_size.min(ammo);
                    state.clip = clip as f32;
                    state.reserve = ammo - clip;
                }
                say(format!(
                    "weapons: picked up {} ({}) with {ammo} shots",
                    p.weapon, params.name
                ));
                arsenal.weapons.push((p.weapon.clone(), state));
            }
        }
        commands.entity(e).despawn();
    }
}
