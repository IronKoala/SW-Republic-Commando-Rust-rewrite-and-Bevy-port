//! Emitters the map places (an `Emitter` actor, such as geo_01a's
//! `REP_Gunship_GroundDust0`, the dust blown up under the landing gunship,
//! or its `Fire_Medium` and `SpireBigSmoke`): their effect runs at the
//! place and turn the map gives, from the level's start, for as long as
//! the level lasts. Drawn by the effects part ([`SpawnEffect`], carried by
//! a small entity of our own so it never stops on its own).
//!
//! From the game's files: which actors these are (their class is an effect
//! class whose parents lead to engine.u's `Emitter`), where they are and
//! how they're turned (the map), what each sends (its class in
//! `Properties/cteffects.u`), and that none of geo_01a's starts switched
//! off (the map's copy of the gunship dust's `SpriteEmitter123` sets
//! neither `Disabled` nor anything else that would hold it back).
//!
//! What a level event naming the emitter's tag does, from engine.dll
//! (decompiled for reference only, nothing copied): engine.u's
//! `Emitter.Trigger` calls each emitter's native `Trigger()`;
//! `UParticleEmitter::execTrigger` flips the emitter's `Disabled` when its
//! `TriggerDisabled` is set (engine.u's default) and starts it over when
//! `ResetOnTrigger` is set; `AEmitter::Tick` and `AEmitter::Render` skip a
//! disabled emitter, so its particles stop and vanish at once, and come
//! back where they were if a second event switches it on. The effects
//! part does this ([`TriggerEffect`]). geo_01a's script sends
//! `REP_Gunship_GroundDust` as the gunship leaves for the spider droid.
//!
//! Not done: the map's own copies of an emitter's settings (we draw the
//! class's), the actor's `DrawScale` (logged), and keeping which are off in
//! save files. Logged with the `scripts:` prefix.

use crate::PART;
use bevy::prelude::*;
use rc_effects_plugin::{SpawnEffect, TriggerEffect};
use rc_game_core::units::to_bevy;
use rc_game_core::{GameLog, ScriptEvent};
use rc_level::scripts::{MapScripts, ScriptObject, Val};
use rc_movement::Vec3 as UVec3;

/// One emitter placed in the map.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedEmitter {
    pub name: String,
    /// Its class, which is the effect it sends.
    pub class: String,
    pub tag: String,
    pub location: [f32; 3],
    /// Unreal pitch, yaw, roll.
    pub rotation: [i32; 3],
    /// The actor's `DrawScale` when the map sets one (logged, not used).
    pub draw_scale: Option<f32>,
}

/// The level's placed emitters while it is played.
#[derive(Resource, Default)]
pub struct LevelEmitters {
    pub list: Vec<PlacedEmitter>,
    /// The entity carrying each one's effect.
    carriers: Vec<Option<Entity>>,
    started: bool,
    /// For the self-tests: the emitters a level event triggered, by name,
    /// in order.
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
}

/// Carries a placed emitter's effect; the effect fades when it's gone.
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

/// Every actor in the map whose class `is_emitter` says is an effect.
pub fn read(map: &MapScripts, is_emitter: &dyn Fn(&str) -> bool) -> Vec<PlacedEmitter> {
    map.actors
        .iter()
        .filter(|a| is_emitter(&a.obj.class))
        .map(|a| PlacedEmitter {
            name: a.obj.name.clone(),
            class: a.obj.class.clone(),
            tag: a.obj.tag().to_string(),
            location: a.location,
            rotation: rotation(&a.obj),
            draw_scale: a.obj.float("DrawScale"),
        })
        .collect()
}

pub fn log_list(list: &[PlacedEmitter], log: &GameLog) {
    log.line(format!("{PART}: {} emitters placed in the map", list.len()));
    for e in list {
        log.line(format!(
            "{PART}:   emitter {} ({}, tag {}) at ({:.0}, {:.0}, {:.0}) turned {:?}{}",
            e.name,
            e.class,
            e.tag,
            e.location[0],
            e.location[1],
            e.location[2],
            e.rotation,
            e.draw_scale
                .map(|s| format!(", map draw scale {s} (not used)"))
                .unwrap_or_default()
        ));
    }
}

fn switch_on(
    i: usize,
    em: &mut LevelEmitters,
    commands: &mut Commands,
    spawn: &mut MessageWriter<SpawnEffect>,
) {
    let e = &em.list[i];
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
        effect: e.class.clone(),
        at,
        by: e.name.clone(),
        rotation: e.rotation,
        follow: Some(carrier),
        endless: true,
        ..default()
    });
    em.carriers[i] = Some(carrier);
}

/// Start every placed emitter once the level plays, and trigger the ones a
/// level event names.
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
        for i in 0..em.list.len() {
            switch_on(i, em, &mut commands, &mut spawn);
        }
        if !em.list.is_empty() {
            log.line(format!("{PART}: {} placed emitters started", em.list.len()));
        }
    }
    for ev in events.read() {
        for i in 0..em.list.len() {
            if !em.list[i].tag.eq_ignore_ascii_case(&ev.name) {
                continue;
            }
            let name = em.list[i].name.clone();
            if let Some(c) = em.carriers[i] {
                trigger.write(TriggerEffect { carrier: c });
            }
            log.line(format!(
                "{PART}: emitter {name} triggered by {} (each of its emitters with TriggerDisabled switches on or off, engine.dll's rule)",
                ev.name
            ));
            em.triggered.push(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_level::scripts::{ScriptActor, Vals};

    #[test]
    fn finds_the_gunship_dust_as_the_map_places_it() {
        // geo_01a's REP_Gunship_GroundDust0, values as the map stores them.
        let mut v = Vals::new();
        v.insert(
            "Rotation".into(),
            Val::Raw(vec![209, 254, 255, 255, 220, 17, 0, 0, 229, 254, 255, 255]),
        );
        let map = MapScripts {
            actors: vec![
                ScriptActor {
                    obj: ScriptObject {
                        export: 0,
                        class: "REP_Gunship_GroundDust".into(),
                        name: "REP_Gunship_GroundDust0".into(),
                        values: v,
                    },
                    location: [2309.69, 1708.559, -2569.9277],
                    actions: vec![],
                },
                ScriptActor {
                    obj: ScriptObject {
                        export: 1,
                        class: "Trigger".into(),
                        name: "Trigger26".into(),
                        values: Vals::new(),
                    },
                    location: [0.0; 3],
                    actions: vec![],
                },
            ],
            ..MapScripts::default()
        };
        let list = read(&map, &|c: &str| {
            c.eq_ignore_ascii_case("REP_Gunship_GroundDust")
        });
        assert_eq!(list.len(), 1);
        let e = &list[0];
        // No tag of its own: Unreal's default, the class name, which is
        // the event geo_01a's gunship script sends.
        assert_eq!(e.tag, "REP_Gunship_GroundDust");
        assert_eq!(e.rotation, [-303, 4572, -283]);
        assert_eq!(e.draw_scale, None);
    }
}
