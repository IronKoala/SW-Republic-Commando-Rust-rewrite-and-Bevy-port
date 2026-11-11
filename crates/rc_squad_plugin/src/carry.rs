//! The squad going on with the player to the next map: the `[squad_carry]`
//! section that `data/levels/travel.toml` lists.
//!
//! What the original keeps (CTCharacters' clone player, read from the
//! game's own scripts, not copied): each squad mate's health, matched by
//! who they are, and the last order the squad was given. A mate who was
//! down when the player left gets up on the next map with a quarter of
//! their full health. Shields come back full. "Secure position" isn't kept,
//! since the spot was on the old map. Squad mates' weapons and ammo aren't
//! kept: in the original they have no ammo to run out of.
//!
//! The section is only written when leaving a map ([`CarryingOver`]), so
//! ordinary save files keep their full `[squad]` section instead. The one
//! exception: if a save is made after arriving but before the squad has
//! appeared (the new map's start checkpoint can be), the carried health
//! still waiting to be put back is written, so that checkpoint has it too.

use crate::{SquadMate, SquadState};
use bevy::prelude::*;
use rc_combat::{Dead, Health, Shield};
use rc_game_core::save::{CarryingOver, SavePart};
use rc_game_core::GameLog;
use rc_squad::Stance;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SquadCarry {
    /// form_up, search_and_destroy or hold. Missing: the next map's own
    /// start (form up).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stance: Option<String>,
    pub members: Vec<CarriedMate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CarriedMate {
    /// Sev, Fixer or Scorch: the mate of that name on the next map gets it.
    pub name: String,
    pub health: f32,
}

/// A carried section waiting for the next map's squad to appear.
#[derive(Resource, Debug, Clone)]
pub struct PendingSquadCarry(pub SquadCarry);

/// The original's rule for a mate who is down when the player leaves.
pub const DOWN_HEALTH_SHARE: f32 = 0.25;

fn carried_stance(s: Stance) -> Option<&'static str> {
    match s {
        Stance::FormUp => Some("form_up"),
        Stance::SearchAndDestroy => Some("search_and_destroy"),
        Stance::Hold => Some("hold"),
        // The original doesn't carry it either: the spot is on the old map.
        Stance::SecurePosition => None,
    }
}

fn stance(name: &str) -> Result<Stance, String> {
    Ok(match name {
        "form_up" => Stance::FormUp,
        "search_and_destroy" => Stance::SearchAndDestroy,
        "hold" => Stance::Hold,
        other => return Err(format!("unknown stance {other:?}")),
    })
}

impl SavePart for SquadCarry {
    const KEY: &'static str = "squad_carry";

    fn save(world: &mut World) -> Option<Self> {
        if world.get_resource::<CarryingOver>().is_none() {
            // Not leaving: only a carry that hasn't been put back yet.
            return world
                .get_resource::<PendingSquadCarry>()
                .map(|p| p.0.clone());
        }
        if let Some(p) = world.get_resource::<PendingSquadCarry>() {
            // Leaving again before the squad appeared: pass it on as is.
            return Some(p.0.clone());
        }
        let squad = world.get_resource::<SquadState>()?.squad.as_ref()?;
        let stance = carried_stance(squad.stance).map(String::from);
        let marked_down: Vec<bool> = squad.members.iter().map(|m| m.down).collect();
        let mut q = world.query::<(&SquadMate, &Health, Has<Dead>)>();
        let mut bodies: Vec<(usize, CarriedMate)> = q
            .iter(world)
            .map(|(m, h, dead)| {
                // Down by the body (no health) or by the squad (still
                // waiting for a revive) both count.
                let down = dead || h.current <= 0.0 || marked_down.get(m.index) == Some(&true);
                let health = if down {
                    h.max * DOWN_HEALTH_SHARE
                } else {
                    h.current
                };
                (
                    m.index,
                    CarriedMate {
                        name: m.name.clone(),
                        health,
                    },
                )
            })
            .collect();
        bodies.sort_by_key(|b| b.0);
        Some(SquadCarry {
            stance,
            members: bodies.into_iter().map(|b| b.1).collect(),
        })
    }

    fn load(world: &mut World, data: Self) -> Result<(), String> {
        if let Some(s) = &data.stance {
            stance(s)?;
        }
        world.insert_resource(PendingSquadCarry(data));
        Ok(())
    }

    fn load_missing(world: &mut World) {
        // A save file without it: nothing carried is waiting any more.
        world.remove_resource::<PendingSquadCarry>();
    }
}

/// Put the carried health and order on the new map's squad once it exists.
pub(crate) fn apply_pending(world: &mut World) {
    // Only the mates in the level now get a body; one who comes in later
    // (a map's spawner) starts fresh, as in the original.
    let Some(want) = world
        .get_resource::<SquadState>()
        .and_then(|s| s.squad.as_ref())
        .map(|s| {
            s.members
                .iter()
                .filter(|m| m.presence != rc_squad::Presence::Gone)
                .count()
        })
    else {
        return;
    };
    let mut q = world.query::<&SquadMate>();
    if q.iter(world).count() < want {
        return;
    }
    let Some(PendingSquadCarry(d)) = world.remove_resource::<PendingSquadCarry>() else {
        return;
    };
    let mut lines = Vec::new();
    let mut q = world.query::<(Entity, &SquadMate)>();
    let bodies: Vec<(Entity, String)> = q.iter(world).map(|(e, m)| (e, m.name.clone())).collect();
    for (e, name) in &bodies {
        let Some(c) = d.members.iter().find(|c| &c.name == name) else {
            lines.push(format!("{name} new on this map (full health)"));
            continue;
        };
        let mut ent = world.entity_mut(*e);
        let Some(mut h) = ent.get_mut::<Health>() else {
            continue;
        };
        // Never put a mate down on arrival: at least 1 health.
        h.current = c.health.clamp(1.0, h.max);
        let (now, max) = (h.current, h.max);
        if let Some(mut s) = ent.get_mut::<Shield>() {
            s.current = s.max;
        }
        ent.remove::<Dead>();
        lines.push(format!("{name} {now:.0}/{max:.0}"));
    }
    for c in &d.members {
        if !bodies.iter().any(|(_, n)| n == &c.name) {
            lines.push(format!("{} not on this map", c.name));
        }
    }
    let stance = d.stance.as_deref().and_then(|s| stance(s).ok());
    if let Some(s) = stance {
        let mut state = world.resource_mut::<SquadState>();
        let squad = state.squad.as_mut().expect("checked above");
        squad.stance = s;
        if s == Stance::Hold {
            for m in &mut squad.members {
                m.hold_point = m.pawn.feet();
            }
        }
    }
    world.resource::<GameLog>().line(format!(
        "squad: carried over: {}; order {}",
        lines.join(", "),
        d.stance.as_deref().unwrap_or("form_up (new map)")
    ));
}

/// Back in the main menu: a carry that never reached a squad is dropped.
pub(crate) fn forget_pending(mut commands: Commands) {
    commands.remove_resource::<PendingSquadCarry>();
}
