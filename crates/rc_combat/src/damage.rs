//! Applying damage, recharging shields, and the player's health readout.

use crate::health::{take_damage, Damage, Dead, Died, Difficulty, Health, Shield};
use bevy::prelude::*;
use rc_game_core::{GameLog, Player, PlayerVitals};

/// Apply every `Damage` sent this frame, shield first. Sends `Died` once
/// when health reaches 0 and marks the entity `Dead`.
pub(crate) fn apply_damage(
    mut commands: Commands,
    mut hurt: MessageReader<Damage>,
    mut targets: Query<(&mut Health, Option<&mut Shield>, Has<Player>, Has<Dead>)>,
    names: Query<&Name>,
    difficulty: Res<Difficulty>,
    mut died: MessageWriter<Died>,
    log: Option<Res<GameLog>>,
) {
    for d in hurt.read() {
        let Ok((mut health, shield, is_player, dead)) = targets.get_mut(d.target) else {
            continue;
        };
        if dead {
            continue;
        }
        let amount = if is_player {
            d.amount * difficulty.damage_to_player
        } else {
            d.amount
        };
        let mut shield = shield;
        let taken = take_damage(&mut health, shield.as_deref_mut(), amount);
        let who = names
            .get(d.target)
            .map_or_else(|_| format!("{:?}", d.target), |n| n.to_string());
        // Who did it, so a health drop can be traced from the log alone.
        // No instigator means a level script, a fall or the like.
        let from = match d.instigator {
            Some(e) if e == d.target => " from itself".to_string(),
            Some(e) => format!(
                " from {}",
                names
                    .get(e)
                    .map_or_else(|_| format!("{e:?}"), |n| n.to_string())
            ),
            None => " from no one (script, fall or level)".to_string(),
        };
        let line =
            format!(
            "combat: {who} took {amount} {:?} damage{from} (shield {}, health {}), health now {}/{}{}",
            d.kind,
            taken.to_shield,
            taken.to_health,
            health.current.max(0.0),
            health.max,
            shield.as_ref().map_or(String::new(), |s| format!(", shield {}/{}", s.current, s.max)),
        );
        match &log {
            Some(l) => l.line(line),
            None => println!("{line}"),
        }
        if taken.killed {
            commands.entity(d.target).insert(Dead);
            died.write(Died {
                entity: d.target,
                killer: d.instigator,
            });
            let line = format!("combat: {who} is down");
            match &log {
                Some(l) => l.line(line),
                None => println!("{line}"),
            }
        }
    }
}

pub(crate) fn recharge_shields(time: Res<Time>, mut shields: Query<&mut Shield, Without<Dead>>) {
    let dt = time.delta_secs();
    for mut s in &mut shields {
        s.tick(dt);
    }
}

/// The player's health and shield for the on-screen display.
pub(crate) fn player_vitals(
    players: Query<(&Health, Option<&Shield>, Has<Dead>), With<Player>>,
    mut out: ResMut<PlayerVitals>,
) {
    let Ok((h, s, dead)) = players.single() else {
        return;
    };
    let new = PlayerVitals {
        health: h.current.max(0.0),
        max_health: h.max,
        shield: s.map_or(0.0, |s| s.current),
        max_shield: s.map_or(0.0, |s| s.max),
        incapacitated: dead,
    };
    if *out != new {
        *out = new;
    }
}
