//! The level's effect spawners (`FXFactory`): read from the map while it
//! loads, switched on and off by the level's events, and run by
//! `rc_effects::factory` (the rules, from the game's own script source,
//! are written there). What they make goes to the effects part as a
//! [`SpawnEffect`], placed and turned as the map says; the effects part
//! reports each one's end ([`EffectEnded`]) so a spawner knows when it may
//! make another.
//!
//! geo_01a's gunships and Geonosian fighters flying overhead
//! (`OverheadVehicles`, switched on by the opening script), its random
//! battle explosions (`BattleExplosions`), and the steam, sparks, fire and
//! explosions of the other levels come from these.
//!
//! Which spawners are on, and how much of their capacity is left, goes
//! into save files (`[effect_spawners]`). Logged with the `scripts:`
//! prefix.

use crate::{LevelScript, PART};
use bevy::prelude::*;
use rc_effects::def::Range;
use rc_effects::factory::{FxFactory, PlayerView, Spawners, Spot};
use rc_effects_plugin::{EffectEnded, SpawnEffect};
use rc_game_core::save::SavePart;
use rc_game_core::units::{to_bevy, to_unreal};
use rc_game_core::{
    FactoryOrder, GameLog, LevelCollision, Player, PlayerCamera, RaiseEvent, ScriptEvent,
};
use rc_level::scripts::{MapScripts, ScriptObject, Val, Vals};
use rc_movement::Vec3 as UVec3;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The level's effect spawners, while a level is played.
#[derive(Resource)]
pub struct LevelFxFactories {
    pub spawners: Spawners,
    /// Effects still there: their number, and which spawner made them.
    live: HashMap<u64, usize>,
    /// For the self-tests: effects started, by spawner name and effect.
    pub made: Vec<(String, String)>,
}

impl LevelFxFactories {
    pub fn new(list: Vec<FxFactory>) -> Self {
        LevelFxFactories {
            spawners: Spawners::new(list, 0x5EED_F00D),
            live: HashMap::new(),
            made: Vec::new(),
        }
    }
}

fn rotation(o: &ScriptObject) -> [i32; 3] {
    match o.get("Rotation") {
        Some(Val::Raw(b)) if b.len() >= 12 => {
            let i = |k: usize| i32::from_le_bytes([b[k], b[k + 1], b[k + 2], b[k + 3]]);
            [i(0), i(4), i(8)]
        }
        _ => [0; 3],
    }
}

fn vals_float(v: &Vals, key: &str) -> Option<f32> {
    match v.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))?.1 {
        Val::Float(f) => Some(*f),
        Val::Int(i) => Some(*i as f32),
        _ => None,
    }
}

/// A `Range` struct value (`Min`, `Max`); a value left out is 0, as stored.
fn range(v: Option<&Vals>) -> Option<Range> {
    let v = v?;
    Some(Range::new(
        vals_float(v, "Min").unwrap_or(0.0),
        vals_float(v, "Max").unwrap_or(0.0),
    ))
}

fn sub<'a>(v: &'a Vals, key: &str) -> Option<&'a Vals> {
    match v.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))?.1 {
        Val::Struct(s) => Some(s),
        _ => None,
    }
}

/// Every `FXFactory` in the map, with its spawn points.
pub fn read(map: &MapScripts) -> Vec<FxFactory> {
    let spots: Vec<(&str, Spot)> = map
        .actors
        .iter()
        .filter(|a| a.obj.class.eq_ignore_ascii_case("SpawnPoint"))
        .map(|a| {
            (
                a.obj.tag(),
                Spot {
                    location: a.location,
                    rotation: rotation(&a.obj),
                },
            )
        })
        .collect();
    let d = FxFactory::default();
    map.actors
        .iter()
        .filter(|a| a.obj.class.eq_ignore_ascii_case("FXFactory"))
        .filter_map(|a| {
            let o = &a.obj;
            let prototype = o.text("Prototype")?.to_string();
            let tag = o.tag().to_string();
            let offset = o.structure("SpawnOffset");
            Some(FxFactory {
                name: o.name.clone(),
                spots: spots
                    .iter()
                    .filter(|(t, _)| t.eq_ignore_ascii_case(&tag))
                    .map(|(_, s)| s.clone())
                    .collect(),
                tag,
                prototype,
                location: a.location,
                rotation: rotation(o),
                capacity: o.int("Capacity").map_or(d.capacity, |c| c.max(0) as u32),
                max_alive: o.int("MaxAlive").map_or(d.max_alive, |c| c.max(0) as u32),
                generate_forever: o.flag("bGenerateForever", d.generate_forever),
                spawn_mode: o.int("SpawnMode").map_or(d.spawn_mode, |m| m as u8),
                spawn_interval: o.float("SpawnInterval").unwrap_or(d.spawn_interval),
                spawn_interval_max: o.float("SpawnIntervalMax").unwrap_or(d.spawn_interval_max),
                min_player_range: o.float("MinPlayerRange").unwrap_or(d.min_player_range),
                max_player_range: o.float("MaxPlayerRange").unwrap_or(d.max_player_range),
                covert: o.flag("bCovert", d.covert),
                spawn_event: o.text("SpawnEvent").map(str::to_string),
                exhausted_event: o.text("ExhaustedEvent").map(str::to_string),
                draw_scale: range(o.structure("EmitterDrawScale")).unwrap_or(d.draw_scale),
                spawn_offset: [
                    range(offset.and_then(|s| sub(s, "X"))).unwrap_or(d.spawn_offset[0]),
                    range(offset.and_then(|s| sub(s, "Y"))).unwrap_or(d.spawn_offset[1]),
                    range(offset.and_then(|s| sub(s, "Z"))).unwrap_or(d.spawn_offset[2]),
                ],
            })
        })
        .collect()
}

/// Log what was read, once per level.
pub fn log_list(list: &[FxFactory], log: &GameLog) {
    log.line(format!("{PART}: {} effect spawners", list.len()));
    for f in list {
        log.line(format!(
            "{PART}:   effect spawner {} (tag {}): {} at ({:.0}, {:.0}, {:.0}) turned {:?}, mode {}, {} spawn points, every {}-{} s, {} alive at most, {}, draw scale {}-{}{}",
            f.name,
            f.tag,
            f.prototype,
            f.location[0],
            f.location[1],
            f.location[2],
            f.rotation,
            f.spawn_mode,
            f.spots.len(),
            f.spawn_interval,
            f.spawn_interval_max.max(f.spawn_interval),
            f.max_alive,
            if f.generate_forever {
                "forever".to_string()
            } else {
                format!("{} in all", f.capacity)
            },
            f.draw_scale.min,
            f.draw_scale.max,
            if f.covert { ", only out of sight" } else { "" }
        ));
    }
}

/// The player, as the spawners see them.
struct View<'a> {
    feet: Option<[f32; 3]>,
    eye: Option<UVec3>,
    forward: UVec3,
    /// Cosine of half the player's field of view across a 4:3 screen (the
    /// original's `FOVAngle`).
    cos_half_fov: f32,
    world: Option<&'a LevelCollision>,
}

impl PlayerView for View<'_> {
    fn location(&self) -> Option<[f32; 3]> {
        self.feet
    }
    fn sees(&self, p: [f32; 3]) -> bool {
        let Some(eye) = self.eye else { return false };
        let at = UVec3::new(p[0], p[1], p[2]);
        let to = (at - eye).normalize_or_zero();
        if self.forward.dot(to) <= self.cos_half_fov {
            return false;
        }
        self.world
            .is_none_or(|w| rc_scripts::sight::clear_line(w.0.as_ref(), eye, at))
    }
}

/// Switch spawners by the level's events, run their timers, and start the
/// effects they make.
#[allow(clippy::too_many_arguments)]
pub fn run(
    time: Res<Time>,
    fx: Option<ResMut<LevelFxFactories>>,
    script: Option<Res<LevelScript>>,
    mut events: MessageReader<ScriptEvent>,
    mut orders: MessageReader<FactoryOrder>,
    mut ended: MessageReader<EffectEnded>,
    player: Query<&Transform, With<Player>>,
    camera: Query<(&GlobalTransform, Option<&Projection>), With<PlayerCamera>>,
    collision: Option<Res<LevelCollision>>,
    mut spawn: MessageWriter<SpawnEffect>,
    mut raise: MessageWriter<RaiseEvent>,
    log: Res<GameLog>,
) {
    let Some(mut fx) = fx else {
        events.clear();
        orders.clear();
        ended.clear();
        return;
    };
    let fx = &mut *fx;
    let now = script.as_ref().map_or(0.0, |s| s.director.time);
    let (eye, forward, cos_half_fov) = match camera.iter().next() {
        Some((g, proj)) => {
            let f = g.forward();
            // The view's height angle, widened to a 4:3 screen's width:
            // the original's FOVAngle (default 85 degrees).
            let vfov = match proj {
                Some(Projection::Perspective(p)) => p.fov,
                _ => 69.0f32.to_radians(),
            };
            let half = ((vfov * 0.5).tan() * 4.0 / 3.0).atan();
            (
                Some(to_unreal(g.translation())),
                UVec3::new(f.x, f.z, f.y).normalize_or_zero(),
                half.cos(),
            )
        }
        None => (None, UVec3::new(1.0, 0.0, 0.0), 0.0),
    };
    let view = View {
        feet: player.iter().next().map(|t| {
            let u = to_unreal(t.translation);
            [u.x, u.y, u.z]
        }),
        eye,
        forward,
        cos_half_fov,
        world: collision.as_deref(),
    };
    for e in ended.read() {
        if fx.live.remove(&e.id).is_some() {
            fx.spawners.ended(e.id);
        }
    }
    let mut steps = Vec::new();
    for o in orders.read() {
        match o {
            FactoryOrder::Reset { tag } => {
                let n = fx.spawners.reset(tag);
                if n > 0 {
                    log.line(format!(
                        "{PART}: t={now:.2} effect spawners {tag}: capacity given back ({n})"
                    ));
                }
            }
            FactoryOrder::Stop { tag, .. } => {
                let n = fx.spawners.stop(tag);
                if n > 0 {
                    log.line(format!(
                        "{PART}: t={now:.2} effect spawners {tag}: switched off ({n})"
                    ));
                }
            }
        }
    }
    for e in events.read() {
        let was: Vec<bool> = fx.spawners.states.iter().map(|s| s.online).collect();
        let st = fx.spawners.trigger(&e.name, &view);
        for (i, s) in fx.spawners.states.iter().enumerate() {
            if s.online != was[i] {
                log.line(format!(
                    "{PART}: t={now:.2} effect spawner {} switched {} by {}",
                    fx.spawners.list[i].name,
                    if s.online { "on" } else { "off" },
                    e.name
                ));
            }
        }
        steps.push(st);
    }
    steps.push(fx.spawners.step(time.delta_secs(), &view));
    for st in steps {
        for s in st.spawns {
            let id = s.id;
            let name = fx.spawners.list[s.factory].name.clone();
            fx.live.insert(id, s.factory);
            log.line(format!(
                "{PART}: t={now:.2} effect spawner {name} makes {} at ({:.0}, {:.0}, {:.0}) turned {:?}, draw scale {}",
                s.effect, s.location[0], s.location[1], s.location[2], s.rotation, s.draw_scale
            ));
            fx.made.push((name.clone(), s.effect.clone()));
            spawn.write(SpawnEffect {
                effect: s.effect,
                at: to_bevy(UVec3::new(s.location[0], s.location[1], s.location[2])),
                floor: None,
                sound: None,
                by: name,
                rotation: s.rotation,
                draw_scale: s.draw_scale,
                id: Some(id),
            });
        }
        for (event, by) in st.events {
            raise.write(RaiseEvent {
                name: event,
                by: format!("{by} (effect spawner)"),
            });
        }
    }
}

/// The `[effect_spawners]` section of a save file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FxFactoriesSave {
    pub spawner: Vec<FxFactorySave>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FxFactorySave {
    pub name: String,
    pub online: bool,
    pub remaining: u32,
}

impl SavePart for FxFactoriesSave {
    const KEY: &'static str = "effect_spawners";

    fn save(world: &mut World) -> Option<Self> {
        let fx = world.get_resource::<LevelFxFactories>()?;
        Some(FxFactoriesSave {
            spawner: fx
                .spawners
                .list
                .iter()
                .zip(&fx.spawners.states)
                .map(|(f, s)| FxFactorySave {
                    name: f.name.clone(),
                    online: s.online,
                    remaining: s.remaining,
                })
                .collect(),
        })
    }

    fn load(world: &mut World, d: Self) -> Result<(), String> {
        let Some(mut fx) = world.get_resource_mut::<LevelFxFactories>() else {
            return Err("no effect spawners in this level".into());
        };
        let fx = &mut *fx;
        // What was showing when the game was saved is gone.
        fx.live.clear();
        let mut unknown = Vec::new();
        for s in d.spawner {
            match fx
                .spawners
                .list
                .iter()
                .position(|f| f.name.eq_ignore_ascii_case(&s.name))
            {
                Some(i) => {
                    let st = &mut fx.spawners.states[i];
                    st.online = s.online;
                    st.remaining = s.remaining;
                    st.alive.clear();
                    // The next tick comes soon after the load.
                    st.timer = 0.0;
                }
                None => unknown.push(s.name),
            }
        }
        if unknown.is_empty() {
            Ok(())
        } else {
            Err(format!("unknown effect spawners {unknown:?}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_level::scripts::{ScriptActor, ScriptObject};

    fn actor(class: &str, name: &str, values: Vals, at: [f32; 3]) -> ScriptActor {
        ScriptActor {
            obj: ScriptObject {
                export: 0,
                class: class.into(),
                name: name.into(),
                values,
            },
            location: at,
            actions: vec![],
        }
    }

    #[test]
    fn reads_an_overhead_spawner_as_the_map_sets_it() {
        // geo_01a's FXFactory19, values as the map stores them.
        let mut v = Vals::new();
        v.insert("Tag".into(), Val::Text("OverheadVehicles".into()));
        v.insert("Prototype".into(), Val::Text("cFX_Gunships".into()));
        v.insert("bGenerateForever".into(), Val::Bool(true));
        v.insert("SpawnInterval".into(), Val::Float(3.0));
        v.insert("SpawnIntervalMax".into(), Val::Float(7.0));
        v.insert("SpawnMode".into(), Val::Byte(2));
        let mut ds = Vals::new();
        ds.insert("Min".into(), Val::Float(2.0));
        ds.insert("Max".into(), Val::Float(2.0));
        v.insert("EmitterDrawScale".into(), Val::Struct(ds));
        let mut y = Vals::new();
        y.insert("Min".into(), Val::Float(-300.0));
        y.insert("Max".into(), Val::Float(300.0));
        let mut off = Vals::new();
        off.insert("Y".into(), Val::Struct(y));
        v.insert("SpawnOffset".into(), Val::Struct(off));
        v.insert(
            "Rotation".into(),
            Val::Raw(vec![48, 13, 0, 0, 209, 205, 255, 255, 221, 255, 255, 255]),
        );
        let map = MapScripts {
            actors: vec![actor(
                "FXFactory",
                "FXFactory19",
                v,
                [-5728.0, 28149.0, 14770.0],
            )],
            ..MapScripts::default()
        };
        let list = read(&map);
        assert_eq!(list.len(), 1);
        let f = &list[0];
        assert_eq!(f.prototype, "cFX_Gunships");
        assert_eq!(f.rotation, [3376, -12847, -35]);
        assert!(f.generate_forever);
        assert_eq!(f.draw_scale, Range::fixed(2.0));
        assert_eq!(f.spawn_offset[1], Range::new(-300.0, 300.0));
        assert_eq!(f.spawn_offset[0], Range::fixed(0.0));
        // Class defaults for what the map leaves out.
        assert_eq!((f.capacity, f.max_alive), (2, 1));
    }
}
