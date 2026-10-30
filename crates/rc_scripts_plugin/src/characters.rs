//! Script actions on characters: who an action means, and what it asks of
//! them.
//!
//! An action names its characters by tag (`Tag`, `PawnTag`,
//! `ActorToTeleportTag`, ...), or not at all: then it means the characters
//! running the `ScriptedSequence` it belongs to (the sequence's tag is
//! their `AIScriptTag` / `AIScriptFromFactory`). Characters carry
//! [`ScriptTags`] so they can be found; the player is found by `Player`
//! and the tags in `scripts.toml`.
//!
//! - Done here, for any character: `MakeInvulnerable` (rc_combat
//!   `Invulnerable` on or off), `SetCannotDie` (`CannotDie`), `HurtActor`
//!   on characters (`Damage`), `ChangeSquadStance` (a
//!   [`SquadStanceOrder`] for the squad part).
//! - Sent as a [`CharacterOrder`] to the part that owns the character
//!   (only characters whose `ScriptTags::follows_orders` is set; enemies
//!   for now): `MoveToPoint`, `MoveToPlayer`, `TurnTowardPlayer`,
//!   `TeleportToPoint`, `PlayAnim`, `WaitForAnimEnd`, `WaitForPlayer`,
//!   `DestroyPawn`, and `DestroyActor` / `SetHidden` on characters. The
//!   script waits for a [`CharacterOrderDone`] where the action takes
//!   time.
//!
//! A character a factory makes appears a frame after the event that
//! switched the factory on, while its script may already be running: an
//! action that finds nobody waits up to `character_grace_seconds` for
//! them (later actions of the same script wait behind it). After that it
//! is logged as "no character" and the script carries on, as it does for
//! characters the game doesn't have yet (placed clone troopers, gunships).

use crate::{LevelScript, PART};
use bevy::prelude::*;
use rc_combat::{CannotDie, Damage, DamageKind, Invulnerable};
use rc_game_core::units::to_unreal;
use rc_game_core::{CharacterAct, CharacterOrder, GameLog, Player, ScriptTags, SquadStanceOrder};
use rc_level::scripts::{Place, ScriptObject};
use std::collections::{HashMap, HashSet};

/// An action on characters waiting to be carried out.
#[derive(Debug, Clone)]
pub struct Call {
    pub script: String,
    pub index: usize,
    /// Lower case, without `ACTION_`.
    pub kind: String,
    pub action: ScriptObject,
    /// The sequence's tag, for actions that name nobody.
    pub owner: Option<String>,
    pub waits: bool,
    /// Give up looking for the characters after this (scripts' clock).
    pub deadline: f32,
}

/// The action kinds this module takes (others reach it only when they
/// name no level part: `HurtActor`, `DestroyActor`, `SetHidden`).
pub const KINDS: &[&str] = &[
    "movetopoint",
    "movetoplayer",
    "turntowardplayer",
    "teleporttopoint",
    "playanim",
    "waitforanimend",
    "waitforplayer",
    "makeinvulnerable",
    "setcannotdie",
    "destroypawn",
    "changesquadstance",
];

/// Where scripts can send characters, by lower-case tag and by name.
#[derive(Debug, Default)]
pub struct Places {
    by_tag: HashMap<String, Vec<usize>>,
    by_name: HashMap<String, usize>,
    list: Vec<Place>,
}

impl Places {
    pub fn new(list: Vec<Place>) -> Self {
        let mut p = Places::default();
        for (i, pl) in list.iter().enumerate() {
            p.by_tag
                .entry(pl.tag.to_ascii_lowercase())
                .or_default()
                .push(i);
            p.by_name.insert(pl.name.to_ascii_lowercase(), i);
        }
        p.list = list;
        p
    }

    /// The actor a `DestinationTag` names: the first with that tag, else
    /// the one with that name.
    pub fn find(&self, tag: &str) -> Option<&Place> {
        let k = tag.to_ascii_lowercase();
        self.by_tag
            .get(&k)
            .and_then(|v| v.first())
            .or_else(|| self.by_name.get(&k))
            .map(|&i| &self.list[i])
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

/// The value naming an action's characters, if it has one.
fn tag_field(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "makeinvulnerable" => "Tag",
        "setcannotdie" => "PawnTag",
        "teleporttopoint" => "ActorToTeleportTag",
        "hurtactor" => "TargetTag",
        "destroyactor" => "DestroyTag",
        "sethidden" => "HideActorTag",
        _ => return None,
    })
}

fn named(a: &ScriptObject, key: &str) -> Option<String> {
    a.text(key)
        .filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("None"))
        .map(str::to_string)
}

/// Where a destination tag is: an actor in the map, or the player.
/// Unreal units and yaw.
fn find_place(
    script: &LevelScript,
    tag: &str,
    player: &Query<(Entity, &Transform), With<Player>>,
) -> Option<([f32; 3], f32)> {
    if script.director.settings.is_player(tag) {
        let (_, t) = player.single().ok()?;
        let feet = to_unreal(t.translation);
        return Some(([feet.x, feet.y, feet.z + 84.0], 0.0));
    }
    script.places.find(tag).map(|p| (p.location, p.yaw))
}

/// Carry out the queued calls whose characters can be found.
#[allow(clippy::too_many_arguments)]
pub(crate) fn cast(
    mut commands: Commands,
    script: Option<ResMut<LevelScript>>,
    chars: Query<(Entity, &ScriptTags, &Transform)>,
    player: Query<(Entity, &Transform), With<Player>>,
    mut orders: MessageWriter<CharacterOrder>,
    mut damage: MessageWriter<Damage>,
    mut squad: MessageWriter<SquadStanceOrder>,
    log: Res<GameLog>,
) {
    let Some(mut script) = script else { return };
    let script = &mut *script;
    if script.calls.is_empty() {
        return;
    }
    let now = script.director.time;
    let mut blocked: HashSet<String> = HashSet::new();
    let mut keep = Vec::new();
    for call in std::mem::take(&mut script.calls) {
        if blocked.contains(&call.script) {
            keep.push(call);
            continue;
        }
        let at = format!(
            "{PART}: t={now:.2} {} #{} {}",
            call.script, call.index, call.kind
        );
        let a = &call.action;
        // Squad orders need no character.
        if call.kind == "changesquadstance" {
            let stance = a
                .text("StanceType")
                .unwrap_or("StanceFormUp")
                .to_ascii_lowercase();
            let order = match stance.as_str() {
                "stanceformup" | "stanceformuptight" | "stancedefault" => {
                    Some(SquadStanceOrder::FormUp)
                }
                "stancesearchanddestroy" => Some(SquadStanceOrder::SearchAndDestroy),
                "stancesecureposition" => named(a, "ObjectiveTag")
                    .and_then(|t| script.places.find(&t))
                    .map(|p| SquadStanceOrder::SecurePosition(p.location)),
                _ => None,
            };
            match order {
                Some(o) => {
                    log.line(format!("{at}: squad order {o:?}"));
                    script.stats.squad_orders.push(format!("{o:?}"));
                    squad.write(o);
                }
                None => {
                    script.stats.not_done += 1;
                    log.line(format!("{at}: squad stance {stance} (not done yet)"));
                }
            }
            continue;
        }
        // Who it means.
        let tag = tag_field(&call.kind).and_then(|k| named(a, k));
        let mut targets: Vec<(Entity, bool, Vec3)> = Vec::new();
        let mut is_player = false;
        match (&tag, &call.owner) {
            (Some(t), _) => {
                if script.director.settings.is_player(t) {
                    is_player = true;
                    if let Ok((e, tr)) = player.single() {
                        targets.push((e, false, tr.translation));
                    }
                }
                for (e, st, tr) in &chars {
                    if st.tag.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(t)) {
                        targets.push((e, st.follows_orders, tr.translation));
                    }
                }
            }
            (None, Some(o)) => {
                for (e, st, tr) in &chars {
                    if st
                        .script
                        .as_deref()
                        .is_some_and(|x| x.eq_ignore_ascii_case(o))
                    {
                        targets.push((e, st.follows_orders, tr.translation));
                    }
                }
            }
            (None, None) => {}
        }
        let who = tag
            .clone()
            .or_else(|| {
                call.owner
                    .as_ref()
                    .map(|o| format!("the characters running {o}"))
            })
            .unwrap_or_else(|| "nobody".into());
        if targets.is_empty() {
            if now < call.deadline && (tag.is_some() || call.owner.is_some()) {
                blocked.insert(call.script.clone());
                keep.push(call);
                continue;
            }
            script.stats.not_done += 1;
            script.stats.no_character.push(who.clone());
            log.line(format!(
                "{at}: no character {who} in the game (not spawned, or a kind the game doesn't have yet); skipped"
            ));
            if call.waits {
                script.director.action_done(&call.script, call.index);
            }
            continue;
        }
        let n = targets.len();
        // Done here, for any character.
        match call.kind.as_str() {
            "makeinvulnerable" => {
                let on = a.flag("bMakeGod", true);
                for (e, _, _) in &targets {
                    if on {
                        commands.entity(*e).insert(Invulnerable);
                    } else {
                        commands.entity(*e).remove::<Invulnerable>();
                    }
                }
                log.line(format!(
                    "{at}: {who} ({n}) {}",
                    if on {
                        "can't be hurt"
                    } else {
                        "can be hurt again"
                    }
                ));
                script.stats.invulnerable.push(format!("{who} {on}"));
                continue;
            }
            "setcannotdie" => {
                let on = a.flag("bCannotDie", true);
                for (e, _, _) in &targets {
                    if on {
                        commands.entity(*e).insert(CannotDie);
                    } else {
                        commands.entity(*e).remove::<CannotDie>();
                    }
                }
                log.line(format!(
                    "{at}: {who} ({n}) {}",
                    if on { "can't die" } else { "can die again" }
                ));
                continue;
            }
            "hurtactor" => {
                let amount = a.float("amount").unwrap_or(0.0);
                let kind = match a.text("Type") {
                    Some(t) if t.contains("Explosion") => DamageKind::Explosion,
                    _ => DamageKind::Other,
                };
                for (e, _, at_point) in &targets {
                    damage.write(Damage {
                        target: *e,
                        amount,
                        kind,
                        instigator: None,
                        point: *at_point,
                        direction: Vec3::NEG_Y,
                    });
                }
                script.stats.characters_hurt += n;
                log.line(format!("{at}: {who} ({n}) take {amount} damage"));
                continue;
            }
            _ => {}
        }
        // For the part that owns the character.
        let act = match call.kind.as_str() {
            "movetopoint" => {
                let dest = named(a, "DestinationTag");
                match dest.as_deref().and_then(|d| find_place(script, d, &player)) {
                    Some((to, yaw)) => Some(CharacterAct::MoveTo {
                        to,
                        walk: a.flag("WalkToPoint", false),
                        face: a.flag("OrientToPoint", false).then_some(yaw),
                    }),
                    None => {
                        log.line(format!(
                            "{at}: no place {} in the map; skipped",
                            dest.unwrap_or_default()
                        ));
                        None
                    }
                }
            }
            "teleporttopoint" => {
                let dest = named(a, "DestinationTag");
                match dest.as_deref().and_then(|d| find_place(script, d, &player)) {
                    Some((to, yaw)) => Some(CharacterAct::Teleport { to, yaw }),
                    None => {
                        log.line(format!(
                            "{at}: no place {} in the map; skipped",
                            dest.unwrap_or_default()
                        ));
                        None
                    }
                }
            }
            "movetoplayer" => Some(CharacterAct::MoveToPlayer {
                tolerance: a.float("Tolerance").unwrap_or(128.0),
            }),
            "turntowardplayer" => Some(CharacterAct::TurnToPlayer),
            "playanim" => named(a, "BaseAnim").map(|name| CharacterAct::PlayAnim {
                name,
                looping: a.flag("bLoopAnim", false),
            }),
            "waitforanimend" => Some(CharacterAct::WaitForAnimEnd),
            "waitforplayer" => Some(CharacterAct::WaitForPlayer {
                distance: a.float("Distance").unwrap_or(256.0),
            }),
            "destroypawn" | "destroyactor" => Some(CharacterAct::Remove),
            "sethidden" => Some(CharacterAct::Hidden(a.flag("bHidden", false))),
            _ => None,
        };
        let Some(act) = act else {
            script.stats.not_done += 1;
            if call.waits {
                script.director.action_done(&call.script, call.index);
            }
            continue;
        };
        let mut sent = 0;
        for (e, follows, _) in &targets {
            if *follows {
                orders.write(CharacterOrder {
                    target: *e,
                    act: act.clone(),
                    script: call.script.clone(),
                    index: call.index,
                    waits: call.waits,
                });
                sent += 1;
            }
        }
        if sent == 0 {
            script.stats.not_done += 1;
            log.line(format!(
                "{at}: {who} {}: {act:?} (not done yet for this character)",
                if is_player {
                    "is the player"
                } else {
                    "doesn't follow scripts yet"
                }
            ));
            if call.waits {
                script.director.action_done(&call.script, call.index);
            }
        } else {
            script
                .stats
                .character_orders
                .push(format!("{} {}", call.kind, who));
            log.line(format!("{at}: {who} ({sent}): {act:?}"));
        }
    }
    script.calls = keep;
}
