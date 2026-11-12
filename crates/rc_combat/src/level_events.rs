//! The level events the player's weapons, pickups and body send, the way
//! the original's scripts (engine.u, ctinventory.u, ctcharacters.u) send
//! them. Level scripts wait for several of them (geo_03a's `GotSniper`,
//! geo_04d's `AAEvent`, geo_01b's `BlasterOutOfAmmoEvent`, geo_03c's
//! `EliteBeamEvent`, the `PlayerIncapacitated` / `PlayerRevived` pairs of
//! geo_01b to geo_05c). Each goes out as a [`RaiseEvent`], which the level
//! scripts pass on.
//!
//! - A weapon pickup's `Event` (from the map): when the player takes the
//!   pickup itself (`Pickup.ValidTouch`). Walking into a weapon you
//!   already carry only takes its ammo and sends nothing, because the
//!   game's `Weapon.HandlePickupQuery` handles it. The Elite Beam's pickup
//!   sends it whenever it is swapped in (`EliteBeamPickup.ValidTouch`).
//! - A weapon's `first_draw` (`FirstTimeSwitchEvent`): the first time the
//!   player brings it up after getting it (`Weapon.BringUp`, with the
//!   player controller's `FirstTimeSwitch` list; kept in [`WeaponsDrawn`]
//!   and in saves). Grenades are never brought up in the original (they
//!   aren't in the weapon cycle), so the sonic detonator's
//!   `SonicDetEvent` is never sent there, and isn't here.
//! - A weapon's `out_of_ammo` (`OutOfAmmoEvent`): when the weapon in hand
//!   has no ammo left at all and doesn't recharge
//!   (`Weapon.SwitchToWeaponWithAmmo`, which the original calls when such
//!   a weapon is in hand, fired out or brought up empty). The original
//!   then switches to another weapon; the port doesn't switch yet.
//! - The player's `down` / `revived` (`PlayerCommando`'s `Event` and
//!   `ReviveEvent`): on going down (`Pawn.OnIncapacitated`, or `Died`)
//!   and when a revive has brought them back (state `Reviving` ends,
//!   `CloneCommando.PlayReviveCue`).

use crate::health::Dead;
use crate::library::WeaponLibrary;
use crate::pickup::WeaponTaken;
use crate::Arsenal;
use bevy::prelude::*;
use rc_game_core::{GameLog, Player, RaiseEvent};

/// The weapons the player has brought up at least once (by weapon id):
/// the original's `PlayerController.FirstTimeSwitch` (0 = brought up).
/// Put on the player; saved with the `[combat]` section.
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct WeaponsDrawn(pub Vec<String>);

fn say(log: &Option<Res<GameLog>>, s: String) {
    match log {
        Some(l) => l.line(s),
        None => println!("{s}"),
    }
}

/// A placed weapon's own event, when the player took it.
pub(crate) fn pickup_events(
    mut taken: MessageReader<WeaponTaken>,
    mut raise: MessageWriter<RaiseEvent>,
    log: Option<Res<GameLog>>,
) {
    for t in taken.read() {
        let (Some(event), true) = (&t.event, t.new_weapon) else {
            continue;
        };
        let by = t
            .placed
            .as_ref()
            .map_or_else(|| t.weapon.clone(), |p| p.1.clone());
        say(&log, format!("events: {by} was picked up: sends {event}"));
        raise.write(RaiseEvent {
            name: event.clone(),
            by,
        });
    }
}

/// What the weapon events saw last frame.
#[derive(Default)]
pub(crate) struct InHand {
    player: Option<Entity>,
    weapon: Option<String>,
    empty: bool,
}

/// `first_draw` and `out_of_ammo` for the weapon in the player's hand.
pub(crate) fn weapon_events(
    mut commands: Commands,
    lib: Res<WeaponLibrary>,
    mut players: Query<
        (Entity, &Arsenal, Option<&mut WeaponsDrawn>),
        (With<Player>, Without<Dead>),
    >,
    mut seen: Local<InHand>,
    mut raise: MessageWriter<RaiseEvent>,
    log: Option<Res<GameLog>>,
) {
    let Ok((me, arsenal, drawn)) = players.single_mut() else {
        return;
    };
    let Some((id, state)) = arsenal.current() else {
        return;
    };
    let Some(p) = lib.weapons.get(id) else {
        return;
    };
    let brought_up = seen.player != Some(me) || seen.weapon.as_deref() != Some(id.as_str());
    if brought_up {
        let first = match drawn {
            Some(mut d) => {
                let first = !d.0.contains(id);
                if first {
                    d.0.push(id.clone());
                }
                first
            }
            None => {
                commands.entity(me).insert(WeaponsDrawn(vec![id.clone()]));
                true
            }
        };
        if let (true, Some(event)) = (first, &p.first_draw_event) {
            say(
                &log,
                format!("events: {id} brought up for the first time: sends {event}"),
            );
            raise.write(RaiseEvent {
                name: event.clone(),
                by: id.clone(),
            });
        }
    }
    let empty = !p.recharges() && state.shots_in_clip() == 0 && state.reserve == 0;
    if empty && (brought_up || !seen.empty) {
        if let Some(event) = &p.out_of_ammo_event {
            say(
                &log,
                format!("events: {id} in hand is out of ammo: sends {event}"),
            );
            raise.write(RaiseEvent {
                name: event.clone(),
                by: id.clone(),
            });
        }
    }
    *seen = InHand {
        player: Some(me),
        weapon: Some(id.clone()),
        empty,
    };
}

/// The player going down and being revived.
pub(crate) fn player_down_events(
    lib: Res<WeaponLibrary>,
    players: Query<(Entity, Has<Dead>), With<Player>>,
    mut seen: Local<Option<(Entity, bool)>>,
    mut raise: MessageWriter<RaiseEvent>,
    log: Option<Res<GameLog>>,
) {
    let Ok((me, down)) = players.single() else {
        return;
    };
    let was = match *seen {
        Some((e, was)) if e == me => was,
        // A new player (level start, a loaded save): nothing changed.
        _ => down,
    };
    *seen = Some((me, down));
    if was == down {
        return;
    }
    let event = if down {
        &lib.player.down_event
    } else {
        &lib.player.revived_event
    };
    if let Some(event) = event {
        say(
            &log,
            format!(
                "events: the player {}: sends {event}",
                if down { "went down" } else { "was revived" }
            ),
        );
        raise.write(RaiseEvent {
            name: event.clone(),
            by: "the player".into(),
        });
    }
}
