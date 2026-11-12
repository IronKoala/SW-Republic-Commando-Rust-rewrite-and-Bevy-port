//! Applying damage, recharging shields, and the player's health readout.

use crate::health::{
    take_damage, CannotDie, Damage, Dead, Died, Difficulty, Health, Invulnerable, Killed, Shield,
};
use bevy::prelude::*;
use rc_game_core::{GameLog, Player, PlayerVitals};

/// Apply every `Damage` sent this frame, shield first. Sends `Died` once
/// when health reaches 0 and marks the entity `Dead`.
///
/// A hit whose damage class kills outright (`DamageRules`, as the
/// original's `Pawn.Died` decides: `bKillsNotIncapacitatesPlayer` for the
/// player, `bKillsNotIncapacitates` for everyone else) also marks it
/// `Killed`, even through `CannotDie`, and kills a commando who is already
/// down (the original's `Incapacitated.TakeDamage`). A hit that bypasses
/// shields goes straight to health and leaves the shield alone.
pub(crate) fn apply_damage(
    mut commands: Commands,
    mut hurt: MessageReader<Damage>,
    mut targets: Query<(
        &mut Health,
        Option<&mut Shield>,
        Has<Player>,
        Has<Dead>,
        Has<Invulnerable>,
        Has<CannotDie>,
        Has<Killed>,
    )>,
    names: Query<&Name>,
    difficulty: Res<Difficulty>,
    mut died: MessageWriter<Died>,
    log: Option<Res<GameLog>>,
) {
    for d in hurt.read() {
        let Ok((mut health, shield, is_player, dead, invulnerable, cannot_die, killed)) =
            targets.get_mut(d.target)
        else {
            continue;
        };
        if invulnerable {
            continue;
        }
        let kills = if is_player {
            d.rules.kills_player
        } else {
            d.rules.kills
        };
        let who = || {
            names
                .get(d.target)
                .map_or_else(|_| format!("{:?}", d.target), |n| n.to_string())
        };
        if dead {
            // Down already: only damage that kills outright does anything.
            if kills && !killed && d.amount > 0.0 {
                commands.entity(d.target).insert(Killed);
                let line = format!(
                    "combat: {} is dead for good ({:?} damage while down)",
                    who(),
                    d.kind
                );
                match &log {
                    Some(l) => l.line(line),
                    None => println!("{line}"),
                }
            }
            continue;
        }
        let amount = if is_player {
            d.amount * difficulty.damage_to_player
        } else {
            d.amount
        };
        let mut shield = shield;
        let to_shield = if d.rules.bypass_shields {
            None
        } else {
            shield.as_deref_mut()
        };
        let mut taken = take_damage(&mut health, to_shield, amount);
        if cannot_die && taken.killed && !kills {
            health.current = 1.0;
            taken.killed = false;
        }
        let who = who();
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
            if kills {
                commands.entity(d.target).insert(Killed);
            }
            died.write(Died {
                entity: d.target,
                killer: d.instigator,
            });
            let line = if kills {
                format!("combat: {who} is dead for good")
            } else {
                format!("combat: {who} is down")
            };
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
    players: Query<(&Health, Option<&Shield>, Has<Dead>, Has<Killed>), With<Player>>,
    mut out: ResMut<PlayerVitals>,
) {
    let Ok((h, s, dead, killed)) = players.single() else {
        return;
    };
    let new = PlayerVitals {
        health: h.current.max(0.0),
        max_health: h.max,
        shield: s.map_or(0.0, |s| s.current),
        max_shield: s.map_or(0.0, |s| s.max),
        // Down and waiting for a revive; dead for good isn't.
        incapacitated: dead && !killed,
    };
    if *out != new {
        *out = new;
    }
}
