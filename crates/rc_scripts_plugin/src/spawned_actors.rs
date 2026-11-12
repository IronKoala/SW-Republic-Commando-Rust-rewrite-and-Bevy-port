//! Effects a level script spawns (`ACTION_SpawnActor`): the explosions
//! in geo_05a-c (`Explosion_Med`, `Explosion_Barrel_Metal`,
//! `Explosion_Sparks`, `Explosion_Fiery`), the grenade-target holograms
//! (`Holo_EngageEnemy` in geo_01a-c), the glow under a thrown hologram
//! projector (`Holo_Glow_V1` in geo_01briefing, geo_03a, geo_04c) and the
//! gunship's ground dust in geo_01briefing (`REP_Gunship_GroundDust`).
//!
//! From the game's files, nothing guessed:
//!
//! - gameplay.u's `ACTION_SpawnActor` (its script text ships in the
//!   package): `InitActionFor` spawns `ActorClass` at the script's own
//!   place plus `LocationOffset`, turned as the script plus
//!   `RotationOffset` (with `bOffsetFromScriptedPawn`, the scripted
//!   character's place and turn instead), and gives it `ActorTag` as its
//!   tag when one is set. It doesn't wait. With no class, nothing is
//!   spawned.
//! - Every class those maps spawn this way is an `Emitter` (its parents in
//!   `Properties/cteffects.u` lead to engine.u's `Emitter`), with
//!   `AutoDestroy` on and its own `SpawnSound`. The effects part plays it
//!   and ends it as the engine does: a burst goes with its last particle,
//!   a loop (`Holo_Glow_V1`, the gunship dust) stays.
//! - gameplay.u's `ACTION_DestroyActor` destroys every actor with
//!   `DestroyTag`: the spawned glows go at once (geo_01briefing, geo_03a,
//!   geo_04c).
//! - gameplay.u's `ACTION_TeleportToPoint` moves the first actor with
//!   `ActorToTeleportTag` to the first actor with `DestinationTag` (its
//!   place and turn): geo_01c moves `HoloTarget2` onto `Sunfac`.
//!
//! Classes that aren't effects (geo_04's falling rocks) are left to their
//! own part and logged. Logged with the `scripts:` prefix.

use crate::{LevelScript, PART};
use bevy::prelude::*;
use rc_combat::Hittable;
use rc_effects_plugin::{EffectEnded, SpawnEffect};
use rc_game_core::units::{to_bevy, to_unreal};
use rc_game_core::{GameLog, ScriptTags};
use rc_level::scripts::{MapScripts, ScriptObject, Val};
use rc_movement::Vec3 as UVec3;
use std::collections::HashMap;

/// What a script asked of the spawned effects.
#[derive(Debug, Clone, PartialEq)]
pub enum Order {
    /// `ACTION_SpawnActor`.
    Spawn {
        /// Who asked (`ScriptedTrigger39 #2`), for the log.
        by: String,
        script: String,
        class: String,
        tag: Option<String>,
        location_offset: [f32; 3],
        rotation_offset: [i32; 3],
        from_pawn: bool,
    },
    /// `ACTION_DestroyActor`.
    Destroy { tag: String },
    /// `ACTION_TeleportToPoint`.
    Teleport { tag: String, to: String },
}

/// Read an `ACTION_SpawnActor` (or `None` for any other action).
pub fn spawn_order(a: &ScriptObject, script: &str, by: String) -> Option<Order> {
    if !a.class.eq_ignore_ascii_case("ACTION_SpawnActor") {
        return None;
    }
    let tag = a
        .text("ActorTag")
        .filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("None"))
        .map(str::to_string);
    Some(Order::Spawn {
        by,
        script: script.to_string(),
        class: a.text("ActorClass").unwrap_or("").to_string(),
        tag,
        location_offset: vector(a.get("LocationOffset")),
        rotation_offset: rotator(a.get("RotationOffset")),
        from_pawn: a.flag("bOffsetFromScriptedPawn", false),
    })
}

fn vector(v: Option<&Val>) -> [f32; 3] {
    match v {
        Some(Val::Vector(x)) => *x,
        Some(Val::Raw(b)) if b.len() >= 12 => {
            let f = |k: usize| f32::from_le_bytes([b[k], b[k + 1], b[k + 2], b[k + 3]]);
            [f(0), f(4), f(8)]
        }
        Some(Val::Struct(s)) => {
            let g = |k: &str| match s.get(k) {
                Some(Val::Float(f)) => *f,
                _ => 0.0,
            };
            [g("X"), g("Y"), g("Z")]
        }
        _ => [0.0; 3],
    }
}

fn rotator(v: Option<&Val>) -> [i32; 3] {
    match v {
        Some(Val::Raw(b)) if b.len() >= 12 => {
            let i = |k: usize| i32::from_le_bytes([b[k], b[k + 1], b[k + 2], b[k + 3]]);
            [i(0), i(4), i(8)]
        }
        Some(Val::Struct(s)) => {
            let g = |k: &str| match s.get(k) {
                Some(Val::Int(i)) => *i,
                _ => 0,
            };
            [g("Pitch"), g("Yaw"), g("Roll")]
        }
        _ => [0; 3],
    }
}

/// One effect a script spawned, while it's there.
#[derive(Debug, Clone)]
pub struct Live {
    pub class: String,
    pub tag: Option<String>,
    carrier: Entity,
    id: u64,
}

/// The level's script-spawned effects.
#[derive(Resource, Default)]
pub struct SpawnedActors {
    /// Each script actor's place (Unreal units) and turn, by name (lower
    /// case): where `ACTION_SpawnActor` puts what it spawns.
    sources: HashMap<String, ([f32; 3], [i32; 3])>,
    /// Which classes are effects, and the tag each class gives itself
    /// (lower case class name -> its `Tag`, if the class sets one).
    effects: HashMap<String, Option<String>>,
    pub live: Vec<Live>,
    next_id: u64,
    /// For the self-tests and the log: what was spawned (class, tag,
    /// Unreal place), destroyed (tag) and moved (tag), in order.
    pub spawned: Vec<(String, Option<String>, [f32; 3])>,
    pub destroyed: Vec<String>,
    pub moved: Vec<String>,
    /// Effects that ended on their own (class).
    pub ended: Vec<String>,
}

/// The class of every `ACTION_SpawnActor` in the map that is switched on.
pub fn classes(map: &MapScripts) -> Vec<String> {
    let mut out: Vec<String> = map
        .actors
        .iter()
        .flat_map(|a| a.actions.iter())
        .filter(|a| a.class.eq_ignore_ascii_case("ACTION_SpawnActor"))
        .filter(|a| !a.flag("bDisabled", false))
        .filter_map(|a| a.text("ActorClass"))
        .filter(|c| !c.is_empty() && !c.eq_ignore_ascii_case("None"))
        .map(str::to_string)
        .collect();
    out.sort_unstable_by_key(|c| c.to_ascii_lowercase());
    out.dedup_by_key(|c| c.to_ascii_lowercase());
    out
}

impl SpawnedActors {
    /// `effects`: the spawned classes that are effects, with the tag each
    /// class sets for itself.
    pub fn new(map: &MapScripts, effects: Vec<(String, Option<String>)>) -> Self {
        let sources = map
            .actors
            .iter()
            .filter(|a| !a.actions.is_empty())
            .map(|a| {
                (
                    a.obj.name.to_ascii_lowercase(),
                    (a.location, crate::placed_emitters::rotation(&a.obj)),
                )
            })
            .collect();
        SpawnedActors {
            sources,
            effects: effects
                .into_iter()
                .map(|(c, t)| (c.to_ascii_lowercase(), t))
                .collect(),
            ..default()
        }
    }

    /// Where a script's spawns go (Unreal place and turn).
    pub fn source(&self, script: &str) -> Option<([f32; 3], [i32; 3])> {
        self.sources.get(&script.to_ascii_lowercase()).copied()
    }

    pub fn is_effect(&self, class: &str) -> bool {
        self.effects.contains_key(&class.to_ascii_lowercase())
    }
}

fn add_rot(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [
        a[0].wrapping_add(b[0]),
        a[1].wrapping_add(b[1]),
        a[2].wrapping_add(b[2]),
    ]
}

/// Marks the entity carrying a spawned effect.
#[derive(Component)]
pub struct SpawnedCarrier;

/// Carry out the scripts' orders, and forget effects that ended.
#[allow(clippy::too_many_arguments)]
pub fn run(
    mut commands: Commands,
    script: Option<ResMut<LevelScript>>,
    spawned: Option<ResMut<SpawnedActors>>,
    chars: Query<(&ScriptTags, &Transform, Option<&Hittable>), Without<SpawnedCarrier>>,
    carriers: Query<&Transform, With<SpawnedCarrier>>,
    mut spawn: MessageWriter<SpawnEffect>,
    mut ended: MessageReader<EffectEnded>,
    log: Res<GameLog>,
) {
    let (Some(mut script), Some(mut sp)) = (script, spawned) else {
        ended.clear();
        return;
    };
    let sp = &mut *sp;
    let now = script.director.time;
    // What run() needs to know before this runs again: the tags here.
    script.spawned_tags = sp
        .live
        .iter()
        .filter_map(|l| l.tag.as_ref().map(|t| t.to_ascii_lowercase()))
        .collect();
    for e in ended.read() {
        if let Some(i) = sp.live.iter().position(|l| l.id == e.id) {
            let l = sp.live.remove(i);
            commands.entity(l.carrier).try_despawn();
            log.line(format!(
                "{PART}: t={now:.2} spawned {} is over (AutoDestroy: nothing left to show)",
                l.class
            ));
            sp.ended.push(l.class);
        }
    }
    for order in std::mem::take(&mut script.spawns) {
        match order {
            Order::Spawn {
                by,
                script: who,
                class,
                tag,
                location_offset,
                rotation_offset,
                from_pawn,
            } => {
                if class.is_empty() || class.eq_ignore_ascii_case("None") {
                    log.line(format!(
                        "{PART}: t={now:.2} {by} SpawnActor names no class: nothing spawned (as in the game)"
                    ));
                    continue;
                }
                if !sp.is_effect(&class) {
                    log.line(format!(
                        "{PART}: t={now:.2} {by} SpawnActor {class}: not an effect (not done yet)"
                    ));
                    script.stats.not_done += 1;
                    continue;
                }
                if from_pawn {
                    // No map uses it switched on (geo_04c's one is off).
                    log.line(format!(
                        "{PART}: t={now:.2} {by} SpawnActor {class} from the scripted character (not done yet)"
                    ));
                    script.stats.not_done += 1;
                    continue;
                }
                let Some((place, turn)) = sp.source(&who) else {
                    log.line(format!(
                        "{PART}: t={now:.2} {by} SpawnActor {class}: no place for script {who}; nothing spawned"
                    ));
                    continue;
                };
                let at_u = [
                    place[0] + location_offset[0],
                    place[1] + location_offset[1],
                    place[2] + location_offset[2],
                ];
                let rotation = add_rot(turn, rotation_offset);
                let tag = tag.or_else(|| {
                    sp.effects
                        .get(&class.to_ascii_lowercase())
                        .cloned()
                        .flatten()
                });
                let at = to_bevy(UVec3::new(at_u[0], at_u[1], at_u[2]));
                let carrier = commands
                    .spawn((
                        SpawnedCarrier,
                        Transform::from_translation(at),
                        GlobalTransform::from_translation(at),
                        DespawnOnExit(rc_game_core::GameState::InGame),
                        Name::new(format!("spawned {class}")),
                    ))
                    .id();
                sp.next_id += 1;
                // Ids from the top of the range, apart from other senders'.
                let id = u64::MAX / 2 + sp.next_id;
                spawn.write(SpawnEffect {
                    effect: class.clone(),
                    at,
                    by: by.clone(),
                    rotation,
                    id: Some(id),
                    follow: Some(carrier),
                    vanish_with_carrier: true,
                    endless: true,
                    auto_destroy: true,
                    ..default()
                });
                log.line(format!(
                    "{PART}: t={now:.2} {by} spawned {class} (tag {}) at ({:.0}, {:.0}, {:.0}) turned {rotation:?}",
                    tag.as_deref().unwrap_or("none"),
                    at_u[0],
                    at_u[1],
                    at_u[2]
                ));
                sp.spawned.push((class.clone(), tag.clone(), at_u));
                sp.live.push(Live {
                    class,
                    tag,
                    carrier,
                    id,
                });
            }
            Order::Destroy { tag } => {
                let mut n = 0;
                sp.live.retain(|l| {
                    if l.tag
                        .as_deref()
                        .is_some_and(|t| t.eq_ignore_ascii_case(&tag))
                    {
                        commands.entity(l.carrier).try_despawn();
                        n += 1;
                        false
                    } else {
                        true
                    }
                });
                if n > 0 {
                    log.line(format!(
                        "{PART}: t={now:.2} destroyed {n} spawned effect(s) tagged {tag}"
                    ));
                    sp.destroyed.push(tag);
                }
            }
            Order::Teleport { tag, to } => {
                let Some(l) = sp.live.iter().find(|l| {
                    l.tag
                        .as_deref()
                        .is_some_and(|t| t.eq_ignore_ascii_case(&tag))
                }) else {
                    continue;
                };
                // The destination's place: a character's middle (Unreal
                // places a character by its middle), else a map place.
                let dest = chars
                    .iter()
                    .find(|(t, _, _)| {
                        t.tag
                            .as_deref()
                            .is_some_and(|x| x.eq_ignore_ascii_case(&to))
                    })
                    .map(|(_, tr, h)| {
                        let mut u = to_unreal(tr.translation);
                        u.z += h.map_or(0.0, |h| h.center_up);
                        [u.x, u.y, u.z]
                    })
                    .or_else(|| script.places.find(&to).map(|p| p.location));
                match dest {
                    Some(d) => {
                        // Through commands: a carrier spawned this same
                        // step (geo_01c spawns, then moves at once) isn't
                        // in the world yet.
                        let to_at = to_bevy(UVec3::new(d[0], d[1], d[2]));
                        let turn = carriers
                            .get(l.carrier)
                            .map(|t| t.rotation)
                            .unwrap_or_default();
                        commands.entity(l.carrier).insert((
                            Transform::from_translation(to_at).with_rotation(turn),
                            GlobalTransform::from_translation(to_at),
                        ));
                        log.line(format!(
                            "{PART}: t={now:.2} spawned {} (tag {tag}) moved to {to} at ({:.0}, {:.0}, {:.0})",
                            l.class, d[0], d[1], d[2]
                        ));
                        sp.moved.push(tag);
                    }
                    None => log.line(format!(
                        "{PART}: t={now:.2} spawned {} (tag {tag}): no {to} to move it to; stays",
                        l.class
                    )),
                }
            }
        }
    }
}

/// The level is over: forget its spawned effects.
pub fn forget(mut commands: Commands) {
    commands.remove_resource::<SpawnedActors>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(values: &[(&str, Val)]) -> ScriptObject {
        ScriptObject {
            export: 0,
            class: "ACTION_SpawnActor".into(),
            name: "ACTION_SpawnActor3".into(),
            values: values
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        }
    }

    #[test]
    fn reads_a_spawn_as_gameplay_u_defines_it() {
        let a = action(&[
            ("ActorClass", Val::Text("Holo_EngageEnemy".into())),
            ("ActorTag", Val::Text("GunnerTarget".into())),
            ("LocationOffset", Val::Vector([1.0, 2.0, 3.0])),
        ]);
        let o = spawn_order(&a, "ScriptedTrigger39", "ScriptedTrigger39 #2".into()).unwrap();
        assert_eq!(
            o,
            Order::Spawn {
                by: "ScriptedTrigger39 #2".into(),
                script: "ScriptedTrigger39".into(),
                class: "Holo_EngageEnemy".into(),
                tag: Some("GunnerTarget".into()),
                location_offset: [1.0, 2.0, 3.0],
                rotation_offset: [0; 3],
                from_pawn: false,
            }
        );
        // No tag: the class's own.
        let b = action(&[("ActorClass", Val::Text("Explosion_Med".into()))]);
        match spawn_order(&b, "S", "S #0".into()).unwrap() {
            Order::Spawn { tag, .. } => assert_eq!(tag, None),
            _ => unreachable!(),
        }
    }

    #[test]
    fn rotation_offsets_add_to_the_script_turn() {
        assert_eq!(add_rot([0, 16384, 0], [100, -16384, 5]), [100, 0, 5]);
        let raw: Vec<u8> = [10i32, 20, 30]
            .iter()
            .flat_map(|i| i.to_le_bytes())
            .collect();
        assert_eq!(rotator(Some(&Val::Raw(raw))), [10, 20, 30]);
    }

    fn app() -> App {
        use rc_scripts::{Director, Settings};
        let mut app = App::new();
        app.add_message::<SpawnEffect>()
            .add_message::<EffectEnded>()
            .insert_resource(GameLog::terminal())
            .insert_resource(LevelScript::bare(Director::new(
                &MapScripts::default(),
                &[],
                Settings::default(),
            )))
            .add_systems(Update, run);
        let mut sp = SpawnedActors::default();
        sp.sources.insert(
            "scriptedtrigger9".into(),
            ([100.0, 200.0, 300.0], [0, 16384, 0]),
        );
        sp.effects
            .insert("holo_engageenemy".into(), Some("Holo_EngageEnemy".into()));
        sp.effects
            .insert("explosion_med".into(), Some("Explosion_Lrg".into()));
        app.insert_resource(sp);
        app
    }

    fn order(app: &mut App, o: Order) {
        app.world_mut().resource_mut::<LevelScript>().spawns.push(o);
    }

    fn spawn(class: &str, tag: Option<&str>) -> Order {
        Order::Spawn {
            by: "ScriptedTrigger9 #5".into(),
            script: "ScriptedTrigger9".into(),
            class: class.into(),
            tag: tag.map(str::to_string),
            location_offset: [0.0, 0.0, 10.0],
            rotation_offset: [0, 0, 0],
            from_pawn: false,
        }
    }

    fn sent(app: &mut App) -> Vec<SpawnEffect> {
        let mut m = app.world_mut().resource_mut::<Messages<SpawnEffect>>();
        m.drain().collect()
    }

    #[test]
    fn spawns_at_the_script_moves_and_destroys_as_gameplay_u_does() {
        let mut a = app();
        // geo_01c: the hologram is spawned at the script, then moved onto
        // Sun Fac (a character: its middle, Unreal's place for a pawn).
        let feet = to_bevy(UVec3::new(5000.0, 0.0, 0.0));
        a.world_mut().spawn((
            ScriptTags {
                tag: Some("Sunfac".into()),
                script: None,
                follows_orders: true,
            },
            Transform::from_translation(feet),
            Hittable::COMMANDO,
        ));
        order(&mut a, spawn("Holo_EngageEnemy", Some("HoloTarget2")));
        order(
            &mut a,
            Order::Teleport {
                tag: "HoloTarget2".into(),
                to: "Sunfac".into(),
            },
        );
        a.update();
        let s = sent(&mut a);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].effect, "Holo_EngageEnemy");
        let u = to_unreal(s[0].at);
        assert!(
            (u.x - 100.0).abs() < 0.01 && (u.y - 200.0).abs() < 0.01 && (u.z - 310.0).abs() < 0.01
        );
        assert_eq!(s[0].rotation, [0, 16384, 0]);
        assert!(s[0].endless && s[0].auto_destroy && s[0].vanish_with_carrier);
        let carrier = s[0].follow.unwrap();
        let t = a.world().get::<Transform>(carrier).unwrap().translation;
        let u = to_unreal(t);
        assert!(
            (u.x - 5000.0).abs() < 0.01 && (u.z - 84.0).abs() < 0.01,
            "on Sun Fac's middle: {u:?}"
        );
        assert_eq!(
            a.world().resource::<SpawnedActors>().moved,
            vec!["HoloTarget2".to_string()]
        );
        // Its tag is known to the scripts.
        a.update();
        assert!(a
            .world()
            .resource::<LevelScript>()
            .is_spawned_tag("holotarget2"));
        // ACTION_DestroyActor: gone at once.
        order(
            &mut a,
            Order::Destroy {
                tag: "HoloTarget2".into(),
            },
        );
        a.update();
        assert!(a.world().get_entity(carrier).is_err());
        assert!(a.world().resource::<SpawnedActors>().live.is_empty());
    }

    #[test]
    fn class_tag_without_actor_tag_and_ends_on_its_own() {
        let mut a = app();
        order(&mut a, spawn("Explosion_Med", None));
        // Not an effect: left for its own part.
        order(&mut a, spawn("K_GEO4_FallingRock1", None));
        // No class (geo_03d, geo_05c): nothing, as in the game.
        order(&mut a, spawn("", None));
        a.update();
        let s = sent(&mut a);
        assert_eq!(s.len(), 1);
        let sp = a.world().resource::<SpawnedActors>();
        assert_eq!(sp.live[0].tag.as_deref(), Some("Explosion_Lrg"));
        let carrier = sp.live[0].carrier;
        let id = s[0].id.unwrap();
        a.world_mut()
            .write_message(EffectEnded { id, by: "x".into() });
        a.update();
        assert!(a.world().get_entity(carrier).is_err());
        assert_eq!(
            a.world().resource::<SpawnedActors>().ended,
            vec!["Explosion_Med".to_string()]
        );
    }
}
