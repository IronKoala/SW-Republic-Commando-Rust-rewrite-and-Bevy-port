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
//! - an effect with an `AttachTag` rides on the actor it names
//!   (`UGameEngine::LoadMap`: after `PostBeginPlay`, each actor whose
//!   `AttachTag` is set is based, `SetBase`, on the first actor in the
//!   level's list, itself left out, whose `Tag` or object name is that
//!   tag). When the base moves (`ULevel::MoveActor`), the rider moves by
//!   the base's move and is swung round the base's origin by its turn (its
//!   place relative to the base, `RelativeLocation`, kept), and its own
//!   turn gets the base's change of turn added (`Rotation` plus the
//!   rotator difference). 98 of the campaign's placed effects ride: 92 on
//!   movers (droid dispensers' thrusters, ships' engine glows), 5 on
//!   `KarmaProp`s and 1 on geo_05c's `GunshipHigh`. See [`ride`].
//!
//! Not done: riding on a `KarmaProp` (the port has no Karma physics, so
//! they never move and the effect stays put, as it would on a prop at
//! rest), the Bloom and camera-shake values of an `Emitter`, and keeping
//! which parts are off in save files. Logged with the `scripts:` prefix.

use crate::PART;
use bevy::prelude::*;
use rc_effects::def::EffectDef;
use rc_effects::read::PlacedEffect;
use rc_effects_plugin::{CarrierHidden, CarrierTurn, SpawnEffect, TriggerEffect};
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
    /// `AttachTag`: what it rides on.
    pub attach_tag: Option<String>,
    /// The actor `attach_tag` names, found as the engine does.
    pub base: Option<Base>,
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

/// The actor a placed effect rides on.
#[derive(Debug, Clone, PartialEq)]
pub struct Base {
    pub export: usize,
    pub name: String,
    pub class: String,
}

/// Where a base is and how it is turned (Unreal units and angle units).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasePose {
    pub pos: [f32; 3],
    pub rot: [f32; 3],
}

/// Where a rider is now, from where it and its base were when it was
/// based (`start`, `base_start`) and where the base is now. As
/// engine.dll's `ULevel::MoveActor` moves an attached actor: its place
/// relative to the base turns with the base, and its own turn gets the
/// base's change of turn added. Unreal units and angle units.
pub fn rider_now(
    start: [f32; 3],
    start_rot: [i32; 3],
    base_start: BasePose,
    base_now: BasePose,
) -> ([f32; 3], [i32; 3]) {
    let motion = rc_movers::Motion::new(
        rc_movers::Pose {
            pos: UVec3::new(base_start.pos[0], base_start.pos[1], base_start.pos[2]),
            rot: base_start.rot,
        },
        rc_movers::Pose {
            pos: UVec3::new(base_now.pos[0], base_now.pos[1], base_now.pos[2]),
            rot: base_now.rot,
        },
    );
    let p = motion.apply(UVec3::new(start[0], start[1], start[2]));
    let rot = [0, 1, 2]
        .map(|k| start_rot[k].wrapping_add((base_now.rot[k] - base_start.rot[k]).round() as i32));
    ([p.x, p.y, p.z], rot)
}

/// The level's placed effects while it is played.
#[derive(Resource, Default)]
pub struct LevelEmitters {
    pub list: Vec<PlacedEmitter>,
    /// The entity carrying each one's effect, once started.
    carriers: Vec<Option<Entity>>,
    /// Where each one's base was when the effect started.
    base_start: Vec<Option<BasePose>>,
    /// How far each rider has been carried from its map place (uu), for
    /// the log and the self-tests.
    pub carried: Vec<f32>,
    started: bool,
    /// For the self-tests: names of effects a level event reached, in order.
    pub triggered: Vec<String>,
}

impl LevelEmitters {
    pub fn new(list: Vec<PlacedEmitter>) -> Self {
        LevelEmitters {
            carriers: vec![None; list.len()],
            base_start: vec![None; list.len()],
            carried: vec![0.0; list.len()],
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
            let attach_tag = actor
                .and_then(|a| a.obj.text("AttachTag").map(str::to_string))
                .filter(|t| !t.eq_ignore_ascii_case("None"));
            // As `UGameEngine::LoadMap`: the first actor in the level's
            // list, the rider left out, whose tag or object name is it.
            let base = attach_tag.as_ref().and_then(|t| {
                map.actors
                    .iter()
                    .find(|a| {
                        a.obj.export != p.export
                            && (a.obj.tag().eq_ignore_ascii_case(t)
                                || a.obj.name.eq_ignore_ascii_case(t))
                    })
                    .map(|a| Base {
                        export: a.obj.export,
                        name: a.obj.name.clone(),
                        class: a.obj.class.clone(),
                    })
            });
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
                attach_tag,
                base,
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
            match (&e.attach_tag, &e.base) {
                (Some(t), Some(b)) => format!(", rides on {t} ({} {})", b.class, b.name),
                (Some(t), None) => format!(", rides on {t}, which the map doesn't have (stays at its place)"),
                _ => String::new(),
            },
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
            let at = to_bevy(UVec3::new(e.location[0], e.location[1], e.location[2]));
            let mut c = commands.spawn((
                EmitterCarrier,
                Transform::from_translation(at),
                GlobalTransform::from_translation(at),
                DespawnOnExit(rc_game_core::GameState::InGame),
            ));
            // A hidden one runs but isn't drawn (`CarrierHidden`) until a
            // script shows it.
            if e.hidden {
                c.insert(CarrierHidden);
            }
            let carrier = c.id();
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

/// Where a base is now: a mover (`rc_movers_plugin`), or a character or
/// vehicle the cast part moves (`rc_cast::Cast`, which only turns about
/// the vertical). `None` for a base that never moves in the port.
fn base_pose(
    b: &Base,
    movers: Option<&rc_movers_plugin::LevelMovers>,
    cast: &Query<&rc_cast::Cast>,
) -> Option<BasePose> {
    if let Some(p) = movers.and_then(|m| m.pose_of_export(b.export)) {
        return Some(BasePose {
            pos: [p.pos.x, p.pos.y, p.pos.z],
            rot: p.rot,
        });
    }
    cast.iter()
        .find(|c| c.name.eq_ignore_ascii_case(&b.name))
        .map(|c| BasePose {
            pos: c.location,
            rot: [0.0, c.yaw * 65536.0 / std::f32::consts::TAU, 0.0],
        })
}

/// Placed effects ride on their bases (`AttachTag`), as engine.dll's
/// `ULevel::MoveActor` moves attached actors with their base.
pub fn ride(
    mut commands: Commands,
    em: Option<ResMut<LevelEmitters>>,
    movers: Option<Res<rc_movers_plugin::LevelMovers>>,
    cast: Query<&rc_cast::Cast>,
    log: Res<GameLog>,
) {
    let Some(mut em) = em else { return };
    let em = &mut *em;
    for i in 0..em.list.len() {
        let (Some(base), Some(carrier)) = (em.list[i].base.clone(), em.carriers[i]) else {
            continue;
        };
        let Some(now) = base_pose(&base, movers.as_deref(), &cast) else {
            continue;
        };
        let e = &em.list[i];
        let start = match em.base_start[i] {
            Some(s) => s,
            None => {
                log.line(format!(
                    "{PART}: effect {} rides on {} {} at ({:.0}, {:.0}, {:.0})",
                    e.name, base.class, base.name, now.pos[0], now.pos[1], now.pos[2]
                ));
                em.base_start[i] = Some(now);
                now
            }
        };
        let (at, rot) = rider_now(e.location, e.rotation, start, now);
        let far = (0..3)
            .map(|k| (at[k] - e.location[k]).powi(2))
            .sum::<f32>()
            .sqrt();
        if far >= 16.0 && em.carried[i] < 16.0 {
            log.line(format!(
                "{PART}: effect {} carried by {} {} ({far:.0} uu from its map place so far, turned {rot:?})",
                e.name, base.class, base.name
            ));
        }
        em.carried[i] = em.carried[i].max(far);
        let at = to_bevy(UVec3::new(at[0], at[1], at[2]));
        if let Ok(mut c) = commands.get_entity(carrier) {
            c.insert((
                Transform::from_translation(at),
                GlobalTransform::from_translation(at),
                CarrierTurn(rot),
            ));
        }
    }
}

/// `ACTION_SetHidden` naming a placed effect's tag shows or hides it
/// (sets its `bHidden`), as for any actor with that tag. geo_01briefing
/// shows the advisor's hologram glow (`Holo_Glow_V1`) when he appears and
/// hides it when he goes. Taken off the list the characters part works
/// through, as no character answers to an effect's tag.
pub fn hide(
    mut commands: Commands,
    script: Option<ResMut<crate::LevelScript>>,
    em: Option<ResMut<LevelEmitters>>,
    log: Res<GameLog>,
) {
    let (Some(mut script), Some(mut em)) = (script, em) else {
        return;
    };
    if !script.calls.iter().any(|c| c.kind == "sethidden") {
        return;
    }
    let calls = std::mem::take(&mut script.calls);
    let mut keep = Vec::with_capacity(calls.len());
    for call in calls {
        let tag = call
            .action
            .text("HideActorTag")
            .filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("None"));
        let matches: Vec<usize> = match (call.kind.as_str(), tag) {
            ("sethidden", Some(t)) => (0..em.list.len())
                .filter(|&i| em.list[i].tag.eq_ignore_ascii_case(t))
                .collect(),
            _ => Vec::new(),
        };
        if matches.is_empty() {
            keep.push(call);
            continue;
        }
        let hide = call.action.flag("bHidden", false);
        for i in matches {
            em.list[i].hidden = hide;
            if let Some(c) = em.carriers[i] {
                if hide {
                    commands.entity(c).insert(CarrierHidden);
                } else {
                    commands.entity(c).remove::<CarrierHidden>();
                }
            }
            log.line(format!(
                "{PART}: t={:.2} {} #{}: effect {} {}",
                script.director.time,
                call.script,
                call.index,
                em.list[i].name,
                if hide { "hidden" } else { "shown" }
            ));
        }
        if call.waits {
            script.director.action_done(&call.script, call.index);
        }
    }
    script.calls = keep;
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

    #[test]
    fn a_rider_moves_and_turns_with_its_base() {
        // A base 100 uu east of the rider turns a quarter (yaw 16384) and
        // rises 50: the rider swings round the base's origin and its own
        // turn gets the quarter added, as ULevel::MoveActor does.
        let start = BasePose {
            pos: [100.0, 0.0, 0.0],
            rot: [0.0; 3],
        };
        let now = BasePose {
            pos: [100.0, 0.0, 50.0],
            rot: [0.0, 16384.0, 0.0],
        };
        let (at, rot) = rider_now([0.0, 0.0, 0.0], [0, 1000, 0], start, now);
        // Was 100 uu behind the base (-X); a quarter turn puts it at -Y.
        assert!((at[0] - 100.0).abs() < 0.01, "{at:?}");
        assert!((at[1] + 100.0).abs() < 0.01, "{at:?}");
        assert!((at[2] - 50.0).abs() < 0.01, "{at:?}");
        assert_eq!(rot, [0, 17384, 0]);
        // Not moved: stays where it was.
        let (at, rot) = rider_now([1.0, 2.0, 3.0], [5, 6, 7], start, start);
        assert_eq!((at, rot), ([1.0, 2.0, 3.0], [5, 6, 7]));
    }
}
