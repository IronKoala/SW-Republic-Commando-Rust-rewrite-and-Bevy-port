//! Effects the map places (an `Emitter` actor, such as geo_01a's
//! `REP_Gunship_GroundDust0`, the dust blown up under the landing gunship,
//! its `Fire_Medium`s and `SpireBigSmoke`s, or ras_02a's steam vents):
//! each runs at the place and turn the map gives, from the level's start,
//! for as long as the level lasts. Drawn by the effects part
//! ([`SpawnEffect`], carried by a small entity of our own so it never stops
//! on its own).
//!
//! From the game's files (read by `rc_effects::read::Library::placed_effects`):
//! - which actors these are (their class is an effect class whose parents
//!   lead to engine.u's `Emitter`, or `Emitter` itself, built in the map);
//! - what each sends: the map's own copies of its emitters. When a level
//!   designer places an effect, the editor copies the class's emitters into
//!   the map, and they are often edited there (695 of the campaign's 830
//!   placed effects differ from their class); the game runs the copies;
//! - the actor's own `SpawnSound`, `AmbientSound`, `LifeSpan`, `bUnlit`,
//!   `bHidden` and `AutoDestroy` where the map sets them;
//! - which parts start switched off (`Disabled`).
//!
//! From engine.dll, read for reference only (nothing copied here):
//! - a level event naming the effect's `Tag` runs engine.u's
//!   `Emitter.Trigger`, which calls each part's native `Trigger()`
//!   (`UParticleEmitter::execTrigger`): a part with `TriggerDisabled` (on by
//!   default) is switched off if on, on if off, and a part with
//!   `ResetOnTrigger` starts over. A part switched off is neither moved nor
//!   drawn (`AEmitter::Tick`, `AEmitter::Render`), so it vanishes at once
//!   and comes back where it was when switched on again. See
//!   `rc_effects::EffectState::trigger`;
//! - the actor's `DrawScale` isn't used while playing (no particle code
//!   reads it; the editor's `UParticleEmitter::Scale` already wrote any
//!   scaling into the map's copies), so it is only logged.
//!
//! Not done: `AttachTag` (98 placed effects ride on something that moves,
//! such as a droid dispenser's thrusters; they stay at their map place),
//! the Bloom and camera-shake values of an `Emitter`, and keeping which
//! parts are off in save files. Logged with the `scripts:` prefix.

use crate::PART;
use bevy::prelude::*;
use rc_effects::def::EffectDef;
use rc_effects::read::PlacedEffect;
use rc_effects_plugin::{SpawnEffect, TriggerEffect};
use rc_game_core::units::to_bevy;
use rc_game_core::{GameLog, ScriptEvent};
use rc_level::scripts::{MapScripts, ScriptObject, Val};
use rc_movement::Vec3 as UVec3;
use std::sync::Arc;

/// One effect placed in the map.
#[derive(Debug, Clone)]
pub struct PlacedEmitter {
    pub name: String,
    /// Its class (`Emitter` for one built in the map alone).
    pub class: String,
    pub tag: String,
    pub location: [f32; 3],
    /// Unreal pitch, yaw, roll.
    pub rotation: [i32; 3],
    /// The actor's `DrawScale` when the map sets one (logged; the engine
    /// doesn't use it while playing).
    pub draw_scale: Option<f32>,
    /// `AttachTag`: what it rides on in the original (not done; logged).
    pub attach_tag: Option<String>,
    /// What it sends (the map's copies), `None` if they couldn't be read.
    pub def: Option<Arc<EffectDef>>,
    /// Why `def` is missing.
    pub problem: Option<String>,
    /// `bHidden`: the engine draws nothing for it.
    pub hidden: bool,
    /// `AutoDestroy`: gone once it has nothing more to show.
    pub auto_destroy: bool,
    /// How many of its parts differ from its class's (for the log).
    pub changed_from_class: Option<usize>,
    /// The name the effects part keeps its definition under: the map's
    /// and the actor's (`geo_01a.Fire_Medium3`).
    pub key: String,
}

/// The level's placed effects while it is played.
#[derive(Resource, Default)]
pub struct LevelEmitters {
    pub list: Vec<PlacedEmitter>,
    /// The entity carrying each one's effect, once started.
    carriers: Vec<Option<Entity>>,
    started: bool,
    /// For the self-tests: names of effects a level event reached, in order.
    pub triggered: Vec<String>,
}

impl LevelEmitters {
    pub fn new(list: Vec<PlacedEmitter>) -> Self {
        LevelEmitters {
            carriers: vec![None; list.len()],
            list,
            started: false,
            triggered: Vec::new(),
        }
    }

    /// How many are running.
    pub fn running(&self) -> usize {
        self.carriers.iter().flatten().count()
    }
}

/// Carries a placed emitter's effect.
#[derive(Component)]
pub struct EmitterCarrier;

fn rotation(o: &ScriptObject) -> [i32; 3] {
    match o.get("Rotation") {
        Some(Val::Raw(b)) if b.len() >= 12 => {
            let i = |k: usize| i32::from_le_bytes([b[k], b[k + 1], b[k + 2], b[k + 3]]);
            [i(0), i(4), i(8)]
        }
        _ => [0; 3],
    }
}

/// Every effect the map places: `placed` as the effects library read it
/// from the map, with each actor's place, turn and tag from `map`.
pub fn read(map: &MapScripts, map_name: &str, placed: &[PlacedEffect]) -> Vec<PlacedEmitter> {
    placed
        .iter()
        .map(|p| {
            let actor = map.actors.iter().find(|a| a.obj.export == p.export);
            let (def, problem) = match &p.def {
                Ok(d) => (Some(Arc::new(d.clone())), None),
                Err(e) => (None, Some(e.clone())),
            };
            PlacedEmitter {
                name: p.name.clone(),
                class: p.class.clone(),
                tag: actor.map_or(p.class.clone(), |a| a.obj.tag().to_string()),
                location: actor.map_or([0.0; 3], |a| a.location),
                rotation: actor.map_or([0; 3], |a| rotation(&a.obj)),
                draw_scale: actor.and_then(|a| a.obj.float("DrawScale")),
                attach_tag: actor.and_then(|a| a.obj.text("AttachTag").map(str::to_string)),
                def,
                problem: problem.or_else(|| {
                    actor
                        .is_none()
                        .then(|| "the actor's place couldn't be read".to_string())
                }),
                hidden: p.hidden,
                auto_destroy: p.auto_destroy,
                changed_from_class: p.changed_from_class,
                key: format!("{map_name}.{}", p.name),
            }
        })
        .collect()
}

pub fn log_list(list: &[PlacedEmitter], log: &GameLog) {
    let changed = list
        .iter()
        .filter(|e| e.changed_from_class.is_some_and(|n| n > 0))
        .count();
    let own = list
        .iter()
        .filter(|e| e.class.eq_ignore_ascii_case("Emitter"))
        .count();
    log.line(format!(
        "{PART}: {} effects placed in the map ({changed} with parts edited in the map, {own} built in the map alone); each runs the map's own copy",
        list.len()
    ));
    for e in list {
        let parts = e.def.as_ref().map_or(0, |d| d.emitters.len());
        let off = e
            .def
            .as_ref()
            .map_or(0, |d| d.emitters.iter().filter(|m| m.disabled).count());
        log.line(format!(
            "{PART}:   effect {} ({}, tag {}) at ({:.0}, {:.0}, {:.0}) turned {:?}: {parts} parts, {off} start off{}{}{}{}{}{}",
            e.name,
            e.class,
            e.tag,
            e.location[0],
            e.location[1],
            e.location[2],
            e.rotation,
            e.changed_from_class
                .map(|n| format!(", {n} edited in the map"))
                .unwrap_or_default(),
            if e.hidden { ", hidden (not shown)" } else { "" },
            if e.auto_destroy { ", goes when done" } else { "" },
            e.draw_scale
                .map(|s| format!(", map draw scale {s} (not used while playing, as in the engine)"))
                .unwrap_or_default(),
            e.attach_tag
                .as_ref()
                .map(|t| format!(", rides on {t} (not done: stays at its place)"))
                .unwrap_or_default(),
            e.problem
                .as_ref()
                .map(|p| format!(", NOT SHOWN: {p}"))
                .unwrap_or_default(),
        ));
    }
}

/// Start every placed effect once the level plays, and pass the level
/// events naming one on to it.
pub fn run(
    mut commands: Commands,
    em: Option<ResMut<LevelEmitters>>,
    mut events: MessageReader<ScriptEvent>,
    mut spawn: MessageWriter<SpawnEffect>,
    mut trigger: MessageWriter<TriggerEffect>,
    log: Res<GameLog>,
) {
    let Some(mut em) = em else {
        events.clear();
        return;
    };
    let em = &mut *em;
    if !em.started {
        em.started = true;
        let mut started = 0;
        for i in 0..em.list.len() {
            let e = &em.list[i];
            let Some(def) = e.def.clone() else { continue };
            if e.hidden {
                continue;
            }
            let at = to_bevy(UVec3::new(e.location[0], e.location[1], e.location[2]));
            let carrier = commands
                .spawn((
                    EmitterCarrier,
                    Transform::from_translation(at),
                    GlobalTransform::from_translation(at),
                    DespawnOnExit(rc_game_core::GameState::InGame),
                ))
                .id();
            spawn.write(SpawnEffect {
                effect: e.key.clone(),
                def: Some(def),
                at,
                by: e.name.clone(),
                rotation: e.rotation,
                follow: Some(carrier),
                endless: !e.auto_destroy,
                ..default()
            });
            em.carriers[i] = Some(carrier);
            started += 1;
        }
        if !em.list.is_empty() {
            log.line(format!(
                "{PART}: {started} of {} placed effects started",
                em.list.len()
            ));
        }
    }
    for ev in events.read() {
        for i in 0..em.list.len() {
            if !em.list[i].tag.eq_ignore_ascii_case(&ev.name) {
                continue;
            }
            let name = em.list[i].name.clone();
            match em.carriers[i] {
                Some(c) => {
                    trigger.write(TriggerEffect {
                        carrier: c,
                        by: ev.name.clone(),
                    });
                    log.line(format!("{PART}: event {} reaches effect {name}", ev.name));
                }
                None => log.line(format!(
                    "{PART}: event {} names effect {name}, which isn't shown",
                    ev.name
                )),
            }
            em.triggered.push(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_level::scripts::{ScriptActor, Vals};

    #[test]
    fn places_the_gunship_dust_as_the_map_does() {
        // geo_01a's REP_Gunship_GroundDust0, values as the map stores them.
        let mut v = Vals::new();
        v.insert(
            "Rotation".into(),
            Val::Raw(vec![209, 254, 255, 255, 220, 17, 0, 0, 229, 254, 255, 255]),
        );
        let map = MapScripts {
            actors: vec![ScriptActor {
                obj: ScriptObject {
                    export: 7,
                    class: "REP_Gunship_GroundDust".into(),
                    name: "REP_Gunship_GroundDust0".into(),
                    values: v,
                },
                location: [2309.69, 1708.559, -2569.9277],
                actions: vec![],
            }],
            ..MapScripts::default()
        };
        let placed = [PlacedEffect {
            export: 7,
            name: "REP_Gunship_GroundDust0".into(),
            class: "REP_Gunship_GroundDust".into(),
            def: Ok(EffectDef::default()),
            hidden: false,
            auto_destroy: false,
            changed_from_class: Some(0),
        }];
        let list = read(&map, "geo_01a", &placed);
        assert_eq!(list.len(), 1);
        let e = &list[0];
        // No tag of its own: Unreal's default, the class name, which is
        // the event geo_01a's gunship script sends.
        assert_eq!(e.tag, "REP_Gunship_GroundDust");
        assert_eq!(e.rotation, [-303, 4572, -283]);
        assert_eq!(e.location, [2309.69, 1708.559, -2569.9277]);
        assert_eq!(e.key, "geo_01a.REP_Gunship_GroundDust0");
        assert!(e.def.is_some() && e.problem.is_none());
    }
}
