//! How each room of a map sounds: its zone's `ZoneAudioClass`.
//!
//! Every Unreal map is split into zones, each with its own `ZoneInfo`
//! actor (zones without one use the `LevelInfo`, which is a `ZoneInfo`
//! too). Republic Commando's `ZoneInfo` names an audio class
//! (engine.u: `var(ZoneSound) class<AudioEnvironment> ZoneAudioClass;`),
//! such as `AudioEnv.GEO_03A_eggRoom_sfx01` in `Properties/AudioEnv.u`.
//! That class's defaults are what the room sounds like:
//!
//! - `Effect`: the room's echo, an `I3DL2Listener` object (in AudioEnv.u,
//!   of a class from `Properties/Reverb.u` such as `EFFECT_CITY_MUSEUM`,
//!   over the `I3DL2Listener` defaults in engine.u). Its values are the
//!   EAX 3.0 listener settings, one for one (`Reverb`).
//! - `PreAmbient`: a sound that plays for as long as the player is in the
//!   room (`amb_GEO_eggRoom_01`).
//! - `RandomOneShot`, `ROSintervalMin`/`Max`, `ROSradiusMin`/`Max`: a sound
//!   played again and again at random times, around the player.
//! - `iMuseState` and the overlays: music. Not read here: the game plays
//!   music from a `PhysicsVolume`'s `AudioClass`, not from the zone (see
//!   docs/DESIGN.md).
//!
//! How the game uses these values was read from its `Engine.dll` and
//! `ALAudio.dll` by disassembly (kept off the repository); see
//! docs/DESIGN.md, "Room sound". Plain Rust, no Bevy, opens no files: the
//! caller hands in a function that opens a package by name.

use crate::actors::{self, LevelInfo};
use crate::bsp::Bsp;
use crate::class_look::{object_path, ClassLooks, Loaded};
use rc_package::properties::{self, Property, Value};
use rc_package::{ObjectRef, Package};
use std::collections::HashMap;

/// A room's echo: the EAX 3.0 listener settings (`EAXLISTENERPROPERTIES`),
/// copied from the `I3DL2Listener` object field by field, as the game's
/// `UALAudioSubsystem::SetAudioEnvReverb` does. Levels in millibels
/// (mB, hundredths of a decibel; -10000 is silent), times in seconds,
/// frequencies in hertz.
#[derive(Debug, Clone, PartialEq)]
pub struct Reverb {
    /// The `I3DL2Listener` object (`EFFECT_CITY_MUSEUM1`) and its class
    /// (`EFFECT_CITY_MUSEUM`).
    pub name: String,
    pub class: String,
    pub environment_size: f32,
    pub diffusion: f32,
    pub room: i32,
    pub room_hf: i32,
    pub room_lf: i32,
    pub decay_time: f32,
    pub decay_hf_ratio: f32,
    pub decay_lf_ratio: f32,
    pub reflections: i32,
    pub reflections_delay: f32,
    pub reflections_pan: [f32; 3],
    pub reverb: i32,
    pub reverb_delay: f32,
    pub reverb_pan: [f32; 3],
    pub echo_time: f32,
    pub echo_depth: f32,
    pub modulation_time: f32,
    pub modulation_depth: f32,
    pub air_absorption_hf: f32,
    pub hf_reference: f32,
    pub lf_reference: f32,
    pub room_rolloff_factor: f32,
    /// EAX 3.0 listener flags, built from the object's bools as the game
    /// does: `bDecayTimeScale` 0x1, `bReflectionsScale` 0x2,
    /// `bReflectionsDelayScale` 0x4, `bReverbScale` 0x8, `bReverbDelayScale`
    /// 0x10, `bDecayHFLimit` 0x20, `bEchoTimeScale` 0x40,
    /// `bModulationTimeScale` 0x80.
    pub flags: u32,
}

impl Reverb {
    /// What the game sends when the room has no echo (no audio class, or a
    /// class with no `Effect`): EAX's "generic" values but with every
    /// level at -10000 mB, so nothing is heard. From `SetAudioEnvReverb`.
    pub fn off() -> Self {
        Reverb {
            name: "none".into(),
            class: "none".into(),
            environment_size: 7.5,
            diffusion: 1.0,
            room: -10000,
            room_hf: -10000,
            room_lf: -10000,
            decay_time: 1.49,
            decay_hf_ratio: 0.83,
            decay_lf_ratio: 1.0,
            reflections: -2602,
            reflections_delay: 0.007,
            reflections_pan: [0.0; 3],
            reverb: 200,
            reverb_delay: 0.011,
            reverb_pan: [0.0; 3],
            echo_time: 0.25,
            echo_depth: 0.0,
            modulation_time: 0.25,
            modulation_depth: 0.0,
            air_absorption_hf: 0.0,
            hf_reference: 5000.0,
            lf_reference: 250.0,
            room_rolloff_factor: 0.0,
            flags: 0x3f,
        }
    }

    /// Whether anything of the echo is heard.
    pub fn is_heard(&self) -> bool {
        self.room > -10000
    }
}

/// One audio class: what a room sounds like.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioEnv {
    /// The class (`GEO_03A_eggRoom_sfx01`) and its package (`AudioEnv`).
    pub class: String,
    pub package: String,
    /// The echo, `None` if the class has no `Effect` (the echo is then
    /// off: `Reverb::off`).
    pub reverb: Option<Reverb>,
    /// Sound paths, package first:
    /// `["GEO_Sound", "GEO_03", "amb_GEO_eggRoom_01"]`.
    pub pre_ambient: Option<Vec<String>>,
    pub random_one_shot: Option<Vec<String>>,
    /// Unreal units; `ROSradiusMax` of 0 or less plays the one-shot
    /// "everywhere" (not placed).
    pub ros_radius_min: f32,
    pub ros_radius_max: f32,
    /// Seconds.
    pub ros_interval_min: f32,
    pub ros_interval_max: f32,
}

/// One zone of the map.
#[derive(Debug, Clone, PartialEq)]
pub struct Zone {
    /// The zone's actor (`ZoneInfo12`), or the map's `LevelInfo` for a
    /// zone with none of its own.
    pub actor: String,
    /// Its `Tag` (scripts change a zone's audio class by tag).
    pub tag: String,
    /// Index into `ZoneAudio::envs`, `None` for no audio class.
    pub env: Option<usize>,
}

/// One node of the map's zone tree (the BSP), enough to find the zone a
/// point is in.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Node {
    plane: [f32; 4],
    front: i32,
    back: i32,
    zone: [u8; 2],
}

/// Every zone's audio, the audio classes the scripts switch to, and the
/// zone tree.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ZoneAudio {
    /// By zone number.
    pub zones: Vec<Zone>,
    /// Every audio class read: the zones' and the ones the map's
    /// `ACTION_ChangeZoneAudio` actions name.
    pub envs: Vec<AudioEnv>,
    /// Audio classes named but not read, with why.
    pub problems: Vec<String>,
    nodes: Vec<Node>,
}

impl ZoneAudio {
    /// The zone a point (Unreal units) is in: the walk of UE2's
    /// `UModel::PointRegion`, as `Bsp::point_zone`.
    pub fn zone_at(&self, p: [f32; 3]) -> Option<usize> {
        let mut i = 0usize;
        for _ in 0..self.nodes.len() {
            let n = self.nodes.get(i)?;
            let d = n.plane[0] * p[0] + n.plane[1] * p[1] + n.plane[2] * p[2] - n.plane[3];
            let front = d >= 0.0;
            let child = if front { n.front } else { n.back };
            if child < 0 || child as usize >= self.nodes.len() {
                return Some(n.zone[front as usize] as usize);
            }
            i = child as usize;
        }
        None
    }

    /// An audio class by name (any case).
    pub fn env_named(&self, class: &str) -> Option<usize> {
        self.envs
            .iter()
            .position(|e| e.class.eq_ignore_ascii_case(class))
    }
}

/// Read every zone's audio class in `map`.
pub fn read(
    looks: &mut ClassLooks,
    map: &Package,
    data: &[u8],
    level: &LevelInfo,
    bsp: &Bsp,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> ZoneAudio {
    let mut out = ZoneAudio {
        nodes: bsp
            .nodes
            .iter()
            .map(|n| Node {
                plane: n.plane,
                front: n.front,
                back: n.back,
                zone: n.zone,
            })
            .collect(),
        ..Default::default()
    };
    let level_info = level.actors.iter().flatten().copied().find(|&a| {
        map.export_class_name(&map.exports[a])
            .eq_ignore_ascii_case("LevelInfo")
    });
    let mut readers = EnvReader::default();
    for actor in bsp.zone_actors.iter() {
        let Some(a) = actor.or(level_info) else {
            out.zones.push(Zone {
                actor: "none".into(),
                tag: String::new(),
                env: None,
            });
            continue;
        };
        let props = actors::read_actor_properties(map, data, a).unwrap_or_default();
        let name = map.name(map.exports[a].object_name).to_string();
        let class = map.export_class_name(&map.exports[a]).to_string();
        let tag = match properties::find(map, &props, "Tag").map(|p| &p.value) {
            Some(Value::Name(n)) => map.name(*n).to_string(),
            // UE2: an actor's Tag starts as its class's name.
            _ => class.clone(),
        };
        let class_ref = match properties::find(map, &props, "ZoneAudioClass").map(|p| &p.value) {
            Some(Value::Object(o)) => Some(object_path(map, *o)),
            _ => looks.for_actor(map, a, open).and_then(|l| {
                l.find("ZoneAudioClass", 0).and_then(|(pkg, p)| match p.value {
                    Value::Object(o) => Some(object_path(&pkg.0, o)),
                    _ => None,
                })
            }),
        };
        let env = class_ref
            .filter(|p| p.len() >= 2)
            .and_then(|p| readers.env(&mut out, looks, &p[0], &p[p.len() - 1], open));
        out.zones.push(Zone {
            actor: name,
            tag,
            env,
        });
    }
    // The classes the scripts switch zones to.
    for (i, e) in map.exports.iter().enumerate() {
        if !map.export_class_name(e).eq_ignore_ascii_case("ACTION_ChangeZoneAudio") {
            continue;
        }
        let Ok(props) = actors::read_actor_properties(map, data, i) else {
            continue;
        };
        if let Some(Value::Object(o)) =
            properties::find(map, &props, "ZoneAudioClass").map(|p| &p.value)
        {
            let p = object_path(map, *o);
            if p.len() >= 2 {
                readers.env(&mut out, looks, &p[0], &p[p.len() - 1], open);
            }
        }
    }
    out
}

/// Reads audio classes once each.
#[derive(Default)]
struct EnvReader {
    seen: HashMap<String, Option<usize>>,
}

impl EnvReader {
    fn env(
        &mut self,
        out: &mut ZoneAudio,
        looks: &mut ClassLooks,
        package: &str,
        class: &str,
        open: &mut dyn FnMut(&str) -> Option<Loaded>,
    ) -> Option<usize> {
        let key = format!("{package}.{class}").to_ascii_lowercase();
        if let Some(e) = self.seen.get(&key) {
            return *e;
        }
        let got = match read_env(looks, package, class, open) {
            Ok(e) => {
                out.envs.push(e);
                Some(out.envs.len() - 1)
            }
            Err(why) => {
                out.problems.push(format!("{package}.{class}: {why}"));
                None
            }
        };
        self.seen.insert(key, got);
        got
    }
}

/// Read one audio class's defaults.
pub fn read_env(
    looks: &mut ClassLooks,
    package: &str,
    class: &str,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> Result<AudioEnv, String> {
    let look = looks
        .for_class(package, class, open)
        .ok_or("class not found")?;
    if !look.is_a("AudioEnvironment") {
        return Err(format!("not an AudioEnvironment ({})", look.chain.join(" < ")));
    }
    let sound = |n: &str| match look.find(n, 0) {
        Some((pkg, p)) => match p.value {
            Value::Object(o) if o != 0 => Some(object_path(&pkg.0, o)).filter(|p| p.len() >= 2),
            _ => None,
        },
        None => None,
    };
    let float = |n: &str| match look.find(n, 0).map(|(_, p)| &p.value) {
        Some(Value::Float(f)) => *f,
        // An unset float is 0 in Unreal.
        _ => 0.0,
    };
    let reverb = match look.find("Effect", 0) {
        Some((pkg, p)) => match p.value {
            Value::Object(o) if o != 0 => Some(read_reverb(looks, &pkg.clone(), o, open)?),
            _ => None,
        },
        None => None,
    };
    Ok(AudioEnv {
        class: class.to_string(),
        package: package.to_string(),
        reverb,
        pre_ambient: sound("PreAmbient"),
        random_one_shot: sound("RandomOneShot"),
        ros_radius_min: float("ROSradiusMin"),
        ros_radius_max: float("ROSradiusMax"),
        ros_interval_min: float("ROSintervalMin"),
        ros_interval_max: float("ROSintervalMax"),
    })
}

/// The `I3DL2Listener` object `reference` (in `pkg`): its own values over
/// its class's defaults, over `I3DL2Listener`'s. A value set nowhere is 0,
/// as in Unreal.
fn read_reverb(
    looks: &mut ClassLooks,
    pkg: &Loaded,
    reference: i32,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> Result<Reverb, String> {
    // The object itself: an export here, or in the package an import
    // points to.
    let (owner, export) = match ObjectRef::from_index(reference) {
        ObjectRef::Export(k) => (pkg.clone(), k),
        ObjectRef::Import(_) => {
            let path = object_path(&pkg.0, reference);
            let other = open(&path[0]).ok_or_else(|| format!("package {} not found", path[0]))?;
            let name = path.last().unwrap();
            let k = other
                .0
                .exports
                .iter()
                .position(|e| other.0.name(e.object_name).eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("{} not found", path.join(".")))?;
            (other, k)
        }
        ObjectRef::None => return Err("no effect".into()),
    };
    let (p, d) = (&owner.0, &owner.1);
    let e = &p.exports[export];
    let name = p.name(e.object_name).to_string();
    let class_path = object_path(p, e.class_index);
    if class_path.len() < 2 {
        return Err(format!("{name}: class not found"));
    }
    let class = class_path[class_path.len() - 1].clone();
    let own = actors::read_actor_properties(p, d, export).map_err(|e| format!("{name}: {e}"))?;
    let look = looks
        .for_class(&class_path[0], &class, open)
        .ok_or_else(|| format!("{name}: class {} not found", class_path.join(".")))?;
    if !look.is_a("I3DL2Listener") {
        return Err(format!("{name}: not an I3DL2Listener"));
    }
    let value = |n: &str| -> Option<Value> {
        properties::find(p, &own, n)
            .map(|x: &Property| x.value.clone())
            .or_else(|| look.find(n, 0).map(|(_, x)| x.value.clone()))
    };
    let float = |n: &str| match value(n) {
        Some(Value::Float(f)) => f,
        _ => 0.0,
    };
    let int = |n: &str| match value(n) {
        Some(Value::Int(i)) => i,
        _ => 0,
    };
    let flag = |n: &str| matches!(value(n), Some(Value::Bool(true)));
    let vector = |n: &str| match value(n) {
        Some(Value::Raw(b)) if b.len() == 12 => {
            let f = |i: usize| f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
            [f(0), f(1), f(2)]
        }
        _ => [0.0; 3],
    };
    let flags = [
        ("bDecayTimeScale", 0x1),
        ("bReflectionsScale", 0x2),
        ("bReflectionsDelayScale", 0x4),
        ("bReverbScale", 0x8),
        ("bReverbDelayScale", 0x10),
        ("bDecayHFLimit", 0x20),
        ("bEchoTimeScale", 0x40),
        ("bModulationTimeScale", 0x80),
    ]
    .iter()
    .filter(|(n, _)| flag(n))
    .fold(0, |a, (_, b)| a | b);
    Ok(Reverb {
        name,
        class,
        environment_size: float("EnvironmentSize"),
        diffusion: float("EnvironmentDiffusion"),
        room: int("Room"),
        room_hf: int("RoomHF"),
        room_lf: int("RoomLF"),
        decay_time: float("DecayTime"),
        decay_hf_ratio: float("DecayHFRatio"),
        decay_lf_ratio: float("DecayLFRatio"),
        reflections: int("Reflections"),
        reflections_delay: float("ReflectionsDelay"),
        reflections_pan: vector("ReflectionsPan"),
        reverb: int("Reverb"),
        reverb_delay: float("ReverbDelay"),
        reverb_pan: vector("ReverbPan"),
        echo_time: float("EchoTime"),
        echo_depth: float("EchoDepth"),
        modulation_time: float("ModulationTime"),
        modulation_depth: float("ModulationDepth"),
        air_absorption_hf: float("AirAbsorptionHF"),
        hf_reference: float("HFReference"),
        lf_reference: float("LFReference"),
        room_rolloff_factor: float("RoomRolloffFactor"),
        flags,
    })
}
