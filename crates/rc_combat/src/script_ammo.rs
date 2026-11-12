//! Level scripts giving or taking the player's ammo
//! (`rc_game_core::pickups::AddAmmo`, the original's `ACTION_AddAmmo`).
//!
//! The original's rules, from its own script text (gameplay.u and
//! engine.u), in our words:
//!
//! - The step names a weapon class (`WhichWeapon`) and an amount
//!   (`AmmoAmount`, which may be below 0).
//! - It finds that weapon in the player's inventory. If the player doesn't
//!   carry it, nothing happens.
//! - It calls the weapon's ammo `AddAmmo`: the new count is the old one
//!   plus the amount, kept between 0 and the most carried (`MaxAmmo`).
//!
//! Every use in the campaign's 57 maps names one of the four grenades
//! (geo_01a takes them all away at the start; geo_04a, ras_01a, yyy_01b,
//! yyy_04a and yyy_35a hand out 5 of each), so only grenades are carried
//! out here. A weapon class is logged as not done.

use crate::library::WeaponLibrary;
use crate::Grenades;
use bevy::prelude::*;
use rc_game_core::pickups::AddAmmo;
use rc_game_core::{GameLog, Player};

/// Steps that came before the player had their grenades (the level's
/// start), kept until they do.
#[derive(Resource, Debug, Default)]
pub(crate) struct PendingAddAmmo(Vec<AddAmmo>);

fn say(log: &Option<Res<GameLog>>, s: String) {
    match log {
        Some(l) => l.line(s),
        None => println!("{s}"),
    }
}

/// What one step does to the grenades carried: a line for the log.
pub fn apply(lib: &WeaponLibrary, grenades: &mut Grenades, step: &AddAmmo) -> String {
    let Some(params) = lib.grenades.values().find(|g| {
        g.game_class
            .as_deref()
            .is_some_and(|c| c.eq_ignore_ascii_case(&step.weapon_class))
    }) else {
        return format!(
            "{}: AddAmmo {:+} for {}: no grenade file names that class (weapons not done yet; no map uses them)",
            step.by, step.amount, step.weapon_class
        );
    };
    match grenades.add_ammo(&params.id, step.amount, params.max_count) {
        Some((before, after)) => format!(
            "{}: AddAmmo {:+} for {} ({}): {before} -> {after} of {}",
            step.by, step.amount, step.weapon_class, params.id, params.max_count
        ),
        None => format!(
            "{}: AddAmmo {:+} for {} ({}): not carried, nothing happens",
            step.by, step.amount, step.weapon_class, params.id
        ),
    }
}

/// Carry out the scripts' steps on the player.
pub(crate) fn add_ammo(
    mut steps: MessageReader<AddAmmo>,
    mut pending: ResMut<PendingAddAmmo>,
    lib: Res<WeaponLibrary>,
    mut players: Query<&mut Grenades, With<Player>>,
    log: Option<Res<GameLog>>,
) {
    pending.0.extend(steps.read().cloned());
    if pending.0.is_empty() {
        return;
    }
    let Ok(mut grenades) = players.single_mut() else {
        return;
    };
    for step in std::mem::take(&mut pending.0) {
        let line = apply(&lib, &mut grenades, &step);
        say(&log, format!("grenades: {line}"));
    }
}

/// A new level starts with nothing waiting.
pub(crate) fn forget(mut pending: ResMut<PendingAddAmmo>) {
    pending.0.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grenade::GrenadeParams;

    /// The player's four grenades as the data files describe them.
    fn library() -> WeaponLibrary {
        let mut lib = WeaponLibrary::default();
        for id in [
            "thermal_detonator",
            "sonic_detonator",
            "emp_grenade",
            "flash_grenade",
        ] {
            let path = format!(
                "{}/../../data/grenades/{id}.toml",
                env!("CARGO_MANIFEST_DIR")
            );
            let text = std::fs::read_to_string(path).unwrap();
            lib.grenades
                .insert(id.into(), GrenadeParams::from_text(id, &text).unwrap());
        }
        lib
    }

    fn step(class: &str, amount: i32) -> AddAmmo {
        AddAmmo {
            weapon_class: class.into(),
            amount,
            by: "test".into(),
        }
    }

    #[test]
    fn geo_01a_starts_with_no_grenades() {
        let lib = library();
        let mut g = Grenades {
            carried: [
                "thermal_detonator",
                "sonic_detonator",
                "emp_grenade",
                "flash_grenade",
            ]
            .iter()
            .map(|id| (id.to_string(), lib.grenades[*id].start_count))
            .collect(),
            ..Default::default()
        };
        // ScriptedTrigger52, as the map has it.
        for (class, n) in [
            ("FlashBang", -5),
            ("EMPGrenade", -2),
            ("SonicDetonator", -2),
            ("ThermalDetonator", -2),
        ] {
            let line = apply(&lib, &mut g, &step(class, n));
            assert!(line.contains("-> 0 of 5"), "{line}");
        }
        assert!(g.carried.iter().all(|(_, n)| *n == 0), "{:?}", g.carried);
        // geo_04a hands out 5 of each; the most carried is 5.
        let line = apply(&lib, &mut g, &step("ThermalDetonator", 5));
        assert!(line.contains("0 -> 5 of 5"), "{line}");
        // A weapon class: not carried out.
        let line = apply(&lib, &mut g, &step("DC17mBlaster", 5));
        assert!(line.contains("not done yet"), "{line}");
    }

    /// In a running app: steps sent before the player has grenades (the
    /// level's first frames) wait, then apply once they do.
    #[test]
    fn steps_wait_for_the_player() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<AddAmmo>()
            .init_resource::<PendingAddAmmo>()
            .insert_resource(library())
            .add_systems(Update, add_ammo);
        app.world_mut().write_message(step("ThermalDetonator", -2));
        app.world_mut().write_message(step("FlashBang", -5));
        app.update();
        assert_eq!(app.world().resource::<PendingAddAmmo>().0.len(), 2);
        let player = app
            .world_mut()
            .spawn((
                Player,
                Grenades {
                    carried: vec![("thermal_detonator".into(), 2), ("flash_grenade".into(), 2)],
                    ..Default::default()
                },
            ))
            .id();
        app.update();
        assert!(app.world().resource::<PendingAddAmmo>().0.is_empty());
        let g = app.world().get::<Grenades>(player).unwrap();
        assert_eq!(
            g.carried,
            vec![("thermal_detonator".into(), 0), ("flash_grenade".into(), 0)]
        );
    }
}
