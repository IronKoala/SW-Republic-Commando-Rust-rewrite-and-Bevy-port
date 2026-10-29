//! Where enemies come from: the map's spawners ("pawn factories"), the
//! spawn points they use, and the walk-in triggers that switch them on.
//!
//! How the original works, from the property names and the script source in
//! CTGame.u (read only; described in our own words):
//! - A `PawnFactory` makes characters of one class (`Prototype`, for
//!   example `Properties.BattleDroidTM`). It starts switched off and is
//!   switched on by an event whose name is the factory's `Tag`.
//! - It places each new character at one of the `SpawnPoint`s that share
//!   its `Tag` (in turn, or at random when `SpawnMode` is 1), or at the
//!   factory itself when `SpawnMode` is 2 or there are no spawn points.
//! - `Capacity` is how many it makes in all (UE default 1 when not stored
//!   is our assumption), `MaxAlive` how many may be alive at once,
//!   `bGenerateForever` keeps replacing them.
//! - `StartStateFromFactory` is the character's first behaviour; 6 is
//!   "stationary attack" (hold a position and shoot), used by every
//!   battle droid spawner in geo_01a.
//! - A `Trigger` actor fires its `Event` when the player walks within its
//!   `CollisionRadius`. Many spawners are switched on that way; the rest by
//!   level scripts (`ScriptedTrigger`), which we don't run yet.

use rc_level::actors;
use rc_movement::Vec3;
use rc_package::properties::{self, Property, Value};
use rc_package::Package;

#[derive(Debug, Clone, PartialEq)]
pub enum StartState {
    Idle,
    Wander,
    Perched,
    PerchedCeiling,
    Shutdown,
    Asleep,
    StationaryAttack,
    Other(u8),
}

impl StartState {
    fn from_byte(b: u8) -> Self {
        // Order of Pawn.EStartState in Engine.u.
        match b {
            0 => StartState::Idle,
            1 => StartState::Wander,
            2 => StartState::Perched,
            3 => StartState::PerchedCeiling,
            4 => StartState::Shutdown,
            5 => StartState::Asleep,
            6 => StartState::StationaryAttack,
            o => StartState::Other(o),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SpawnSpot {
    pub location: Vec3,
    /// Facing, radians (Unreal convention).
    pub yaw: f32,
}

#[derive(Debug, Clone)]
pub struct Factory {
    pub name: String,
    pub tag: String,
    /// Class name of what it makes, e.g. `BattleDroidTM`.
    pub prototype: String,
    pub location: Vec3,
    pub yaw: f32,
    pub spots: Vec<SpawnSpot>,
    pub capacity: u32,
    pub max_alive: u32,
    pub generate_forever: bool,
    pub spawn_mode: u8,
    pub spawn_interval: f32,
    pub start_state: StartState,
    pub team: Option<i32>,
    /// Walk-in triggers that switch it on: (centre, radius).
    pub triggers: Vec<(Vec3, f32)>,
}

impl Factory {
    /// Where the n-th character goes.
    pub fn spot(&self, n: usize) -> SpawnSpot {
        if self.spots.is_empty() || self.spawn_mode == 2 {
            SpawnSpot { location: self.location, yaw: self.yaw }
        } else {
            self.spots[n % self.spots.len()].clone()
        }
    }
}

fn vec3(v: &Value) -> Option<Vec3> {
    match v {
        Value::Raw(b) if b.len() == 12 => {
            let f = |i: usize| f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
            Some(Vec3::new(f(0), f(1), f(2)))
        }
        _ => None,
    }
}

fn yaw(v: Option<&Value>) -> f32 {
    match v {
        Some(Value::Raw(b)) if b.len() == 12 => {
            i32::from_le_bytes(b[4..8].try_into().unwrap()) as f32 / 65536.0 * std::f32::consts::TAU
        }
        _ => 0.0,
    }
}

struct Actor {
    class: String,
    name: String,
    props: Vec<Property>,
}

/// Read every pawn factory in a map, with its spawn points and triggers.
pub fn read_factories(pkg: &Package, data: &[u8]) -> Result<Vec<Factory>, String> {
    let level = actors::read_level(pkg, data).map_err(|e| format!("level: {e}"))?;
    let mut list = Vec::new();
    for &a in level.actors.iter().flatten() {
        let e = &pkg.exports[a];
        let class = pkg.export_class_name(e).to_string();
        if !matches!(class.as_str(), "PawnFactory" | "SpawnPoint" | "Trigger") {
            continue;
        }
        if let Ok(props) = actors::read_actor_properties(pkg, data, a) {
            list.push(Actor { class, name: pkg.name(e.object_name).to_string(), props });
        }
    }
    let get = |a: &Actor, n: &str| properties::find(pkg, &a.props, n).map(|p| p.value.clone());
    let name_of = |a: &Actor, n: &str| match get(a, n) {
        Some(Value::Name(i)) => Some(pkg.name(i).to_string()),
        _ => None,
    };
    let int = |a: &Actor, n: &str| match get(a, n) {
        Some(Value::Int(i)) => Some(i),
        _ => None,
    };
    let loc = |a: &Actor| get(a, "Location").and_then(|v| vec3(&v)).unwrap_or(Vec3::ZERO);

    let mut out = Vec::new();
    for f in list.iter().filter(|a| a.class == "PawnFactory") {
        let tag = name_of(f, "Tag").unwrap_or_else(|| "PawnFactory".into());
        let prototype = match get(f, "Prototype") {
            Some(Value::Object(o)) => pkg.object_name(o).to_string(),
            _ => String::new(),
        };
        let spots = list
            .iter()
            .filter(|a| a.class == "SpawnPoint" && name_of(a, "Tag").as_deref() == Some(tag.as_str()))
            .map(|a| SpawnSpot { location: loc(a), yaw: yaw(get(a, "Rotation").as_ref()) })
            .collect();
        let triggers = list
            .iter()
            .filter(|a| a.class == "Trigger" && name_of(a, "Event").as_deref() == Some(tag.as_str()))
            .map(|a| {
                let r = match get(a, "CollisionRadius") {
                    Some(Value::Float(r)) => r,
                    _ => 40.0, // Trigger's UE2 default radius
                };
                (loc(a), r)
            })
            .collect();
        out.push(Factory {
            name: f.name.clone(),
            tag,
            prototype,
            location: loc(f),
            yaw: yaw(get(f, "Rotation").as_ref()),
            spots,
            capacity: int(f, "Capacity").unwrap_or(1).max(0) as u32,
            max_alive: int(f, "MaxAlive").unwrap_or(1).max(1) as u32,
            generate_forever: matches!(get(f, "bGenerateForever"), Some(Value::Bool(true))),
            spawn_mode: match get(f, "SpawnMode") {
                Some(Value::Byte(b)) => b,
                _ => 0,
            },
            spawn_interval: match get(f, "SpawnInterval") {
                Some(Value::Float(x)) => x,
                _ => 1.0,
            },
            start_state: match get(f, "StartStateFromFactory") {
                Some(Value::Byte(b)) => StartState::from_byte(b),
                _ => StartState::Idle,
            },
            team: int(f, "TeamIndexFromFactory"),
            triggers,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}
