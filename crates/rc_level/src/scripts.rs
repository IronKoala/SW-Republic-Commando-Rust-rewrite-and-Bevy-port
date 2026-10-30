//! Reads what a map's level scripting is made of: every actor's own values
//! (its `Tag`, its `Event`, and whatever its class needs), and the list of
//! actions each `ScriptedTrigger` and `ScriptedSequence` runs.
//!
//! Republic Commando keeps a level's script in the map itself, as data:
//! a `ScriptedTrigger` actor holds an `Actions` list of small objects
//! (`ACTION_WaitForEvent`, `ACTION_TriggerEvent`, `ACTION_PlaySound`, ...),
//! each with its own values (`ExternalEvent=levelstart`,
//! `Event=SpawnLandingZone`, ...). This module only reads them; running
//! them is `rc_scripts`'s job.
//!
//! Values are kept by name, as text, numbers and lists, so new action
//! classes need no code here. Values a map leaves out keep their class's
//! default, which the runner knows (see `rc_scripts`).

use crate::actors::{self, LevelInfo};
use rc_package::properties::{read_properties, Property, PropertyType, Value};
use rc_package::{Package, Reader};
use std::collections::{BTreeMap, HashMap};

/// One value, simplified.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Bool(bool),
    Int(i32),
    Float(f32),
    /// Byte values (enums and small numbers).
    Byte(u8),
    /// Names, strings and object references (the object's name).
    Text(String),
    /// A list of object references (by name).
    List(Vec<String>),
    /// A struct written as named values (most of Republic Commando's).
    Struct(Vals),
    /// A struct written as three floats (`Vector`).
    Vector([f32; 3]),
    /// Anything else, as stored.
    Raw(Vec<u8>),
}

/// Values by name. Array elements after the first are `Name[1]`, `Name[2]`.
pub type Vals = BTreeMap<String, Val>;

/// A map object with its values: an actor, or one of a script's actions.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptObject {
    /// Export index (0-based) in the map.
    pub export: usize,
    pub class: String,
    pub name: String,
    pub values: Vals,
}

impl ScriptObject {
    pub fn get(&self, key: &str) -> Option<&Val> {
        self.values
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }
    /// A name, string or object value; `None` if missing or `None`.
    pub fn text(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Val::Text(t)) if !t.eq_ignore_ascii_case("None") && !t.is_empty() => Some(t),
            _ => None,
        }
    }
    pub fn float(&self, key: &str) -> Option<f32> {
        match self.get(key) {
            Some(Val::Float(f)) => Some(*f),
            Some(Val::Int(i)) => Some(*i as f32),
            Some(Val::Byte(b)) => Some(*b as f32),
            _ => None,
        }
    }
    pub fn int(&self, key: &str) -> Option<i32> {
        match self.get(key) {
            Some(Val::Int(i)) => Some(*i),
            Some(Val::Byte(b)) => Some(*b as i32),
            _ => None,
        }
    }
    /// A true/false value, or `default` when the map leaves it out.
    pub fn flag(&self, key: &str, default: bool) -> bool {
        match self.get(key) {
            Some(Val::Bool(b)) => *b,
            _ => default,
        }
    }
    /// The actor's `Tag`. Unreal sets it to the class name when the map
    /// doesn't give one.
    pub fn tag(&self) -> &str {
        self.text("Tag").unwrap_or(&self.class)
    }
    /// Whether the map gave this actor a `Tag` of its own.
    pub fn has_own_tag(&self) -> bool {
        self.text("Tag")
            .is_some_and(|t| !t.eq_ignore_ascii_case(&self.class))
    }
    /// The event this actor sends when it goes off (`Event`), if any.
    pub fn event(&self) -> Option<&str> {
        self.text("Event")
    }
    /// A struct value (named values), e.g. a prop's `DestroyedState`.
    pub fn structure(&self, key: &str) -> Option<&Vals> {
        match self.get(key) {
            Some(Val::Struct(s)) => Some(s),
            _ => None,
        }
    }
}

/// An actor in the level, with its place and (for scripts) its actions.
#[derive(Debug, Clone)]
pub struct ScriptActor {
    pub obj: ScriptObject,
    /// Unreal units.
    pub location: [f32; 3],
    /// The `Actions` list, in order (empty for actors that aren't scripts).
    pub actions: Vec<ScriptObject>,
}

/// Everything a map's scripting needs.
#[derive(Debug, Clone, Default)]
pub struct MapScripts {
    pub actors: Vec<ScriptActor>,
    /// Actors that couldn't be read, and why.
    pub failures: Vec<String>,
}

impl MapScripts {
    /// Actors of a class (case doesn't matter).
    pub fn of_class<'a>(&'a self, class: &'a str) -> impl Iterator<Item = &'a ScriptActor> + 'a {
        self.actors
            .iter()
            .filter(move |a| a.obj.class.eq_ignore_ascii_case(class))
    }
    /// The scripts: actors with an `Actions` list.
    pub fn scripts(&self) -> impl Iterator<Item = &ScriptActor> {
        self.actors.iter().filter(|a| !a.actions.is_empty())
    }
}

fn to_val(pkg: &Package, p: &Property) -> Val {
    match &p.value {
        Value::Bool(b) => Val::Bool(*b),
        Value::Int(i) => Val::Int(*i),
        Value::Float(f) => Val::Float(*f),
        Value::Byte(b) => Val::Byte(*b),
        Value::Name(n) => Val::Text(pkg.name(*n).to_string()),
        Value::Object(o) => Val::Text(pkg.object_name(*o).to_string()),
        Value::Raw(b) => match p.kind {
            // A string: length (with the closing zero), then the letters.
            PropertyType::Str => {
                let mut r = Reader::new(b);
                let len = r.compact_index().unwrap_or(0).max(0) as usize;
                let start = b.len().saturating_sub(len);
                let text = &b[start..b.len().saturating_sub(1).max(start)];
                Val::Text(String::from_utf8_lossy(text).trim().to_string())
            }
            // A list: count, then that many object references. Lists of
            // other things are kept raw.
            PropertyType::Array => {
                let mut r = Reader::new(b);
                let Ok(n) = r.compact_index() else {
                    return Val::Raw(b.clone());
                };
                let mut out = Vec::new();
                for _ in 0..n.max(0) {
                    match r.compact_index() {
                        Ok(o) => out.push(pkg.object_name(o).to_string()),
                        Err(_) => return Val::Raw(b.clone()),
                    }
                }
                if r.pos() == b.len() {
                    Val::List(out)
                } else {
                    Val::Raw(b.clone())
                }
            }
            PropertyType::Struct => {
                let sname = p.struct_name.map(|s| pkg.name(s)).unwrap_or("");
                if sname.eq_ignore_ascii_case("Vector") && b.len() == 12 {
                    let f = |i: usize| f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
                    return Val::Vector([f(0), f(1), f(2)]);
                }
                // Republic Commando writes most structs as named values.
                let mut r = Reader::new(b);
                match read_properties(pkg, &mut r) {
                    Ok(ps) if r.pos() == b.len() => Val::Struct(to_vals(pkg, &ps)),
                    _ => Val::Raw(b.clone()),
                }
            }
            _ => Val::Raw(b.clone()),
        },
    }
}

fn to_vals(pkg: &Package, props: &[Property]) -> Vals {
    let mut out = Vals::new();
    for p in props {
        let name = pkg.name(p.name);
        let key = if p.array_index > 0 {
            format!("{name}[{}]", p.array_index)
        } else {
            name.to_string()
        };
        out.insert(key, to_val(pkg, p));
    }
    out
}

/// Read one object's values (actor or action).
pub fn read_object(pkg: &Package, data: &[u8], export: usize) -> Result<ScriptObject, String> {
    let e = &pkg.exports[export];
    let props = actors::read_actor_properties(pkg, data, export).map_err(|e| e.to_string())?;
    Ok(ScriptObject {
        export,
        class: pkg.export_class_name(e).to_string(),
        name: pkg.name(e.object_name).to_string(),
        values: to_vals(pkg, &props),
    })
}

/// Classes with nothing for scripts in them, skipped to save time.
const SKIP: &[&str] = &[
    "StaticMeshActor",
    "PathNode",
    "CoverPoint",
    "FlyPoint",
    "Light",
];

/// Read every actor in the level and every script's actions.
pub fn read(pkg: &Package, data: &[u8], level: &LevelInfo) -> MapScripts {
    let mut out = MapScripts::default();
    // Actions are found by name in the `Actions` list; map names to exports.
    let mut by_name: HashMap<String, usize> = HashMap::new();
    for (i, e) in pkg.exports.iter().enumerate() {
        by_name.insert(pkg.name(e.object_name).to_ascii_lowercase(), i);
    }
    for &a in level.actors.iter().flatten() {
        let class = pkg.export_class_name(&pkg.exports[a]);
        if SKIP.iter().any(|s| s.eq_ignore_ascii_case(class)) {
            continue;
        }
        let obj = match read_object(pkg, data, a) {
            Ok(o) => o,
            Err(e) => {
                out.failures
                    .push(format!("{}: {e}", pkg.name(pkg.exports[a].object_name)));
                continue;
            }
        };
        let location = match obj.get("Location") {
            Some(Val::Vector(v)) => *v,
            _ => [0.0; 3],
        };
        let mut actions = Vec::new();
        if let Some(Val::List(names)) = obj.get("Actions") {
            for n in names {
                // An empty slot in the list: the original skips it.
                if n.eq_ignore_ascii_case("None") {
                    continue;
                }
                match by_name.get(&n.to_ascii_lowercase()) {
                    Some(&i) => match read_object(pkg, data, i) {
                        Ok(o) => actions.push(o),
                        Err(e) => out.failures.push(format!("{} action {n}: {e}", obj.name)),
                    },
                    None => out
                        .failures
                        .push(format!("{} action {n}: not in the map", obj.name)),
                }
            }
        }
        out.actors.push(ScriptActor {
            obj,
            location,
            actions,
        });
    }
    out
}

/// Actors a script can switch off or remove: anything but a mover with a
/// `Tag` of its own (props like barricades, blocking volumes, bunkers).
/// Export index to (class, tag). Used to give their triangles and drawing
/// a separate place, as movers have.
pub fn tagged_actors(
    pkg: &Package,
    data: &[u8],
    level: &LevelInfo,
) -> HashMap<usize, (String, String)> {
    let mut out = HashMap::new();
    for &a in level.actors.iter().flatten() {
        let class = pkg.export_class_name(&pkg.exports[a]);
        if SKIP.iter().any(|s| s.eq_ignore_ascii_case(class)) && class != "StaticMeshActor" {
            continue;
        }
        let Ok(props) = actors::read_actor_properties(pkg, data, a) else {
            continue;
        };
        let Some(p) = rc_package::properties::find(pkg, &props, "Tag") else {
            continue;
        };
        if let Value::Name(n) = p.value {
            let tag = pkg.name(n);
            if !tag.eq_ignore_ascii_case(class) && !tag.eq_ignore_ascii_case("None") {
                out.insert(a, (class.to_string(), tag.to_string()));
            }
        }
    }
    out
}

/// Somewhere a script can send a character (`DestinationTag`): any actor,
/// path nodes included.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub class: String,
    /// Its `Tag` (the class name when the map gives none, as in UE2).
    pub tag: String,
    /// Unreal units.
    pub location: [f32; 3],
    /// Facing, radians (Unreal's convention).
    pub yaw: f32,
}

/// Every actor's name, tag, location and facing, for scripts that send
/// characters places (`ACTION_MoveToPoint`, `ACTION_TeleportToPoint`).
/// Level meshes and lights are left out: scripts don't send characters to
/// those.
pub fn places(pkg: &Package, data: &[u8], level: &LevelInfo) -> Vec<Place> {
    let mut out = Vec::new();
    for &a in level.actors.iter().flatten() {
        let class = pkg.export_class_name(&pkg.exports[a]);
        if class.eq_ignore_ascii_case("StaticMeshActor") || class.eq_ignore_ascii_case("Light") {
            continue;
        }
        let Ok(props) = actors::read_actor_properties(pkg, data, a) else {
            continue;
        };
        let find = |n: &str| rc_package::properties::find(pkg, &props, n).map(|p| &p.value);
        let raw3 = |n: &str| match find(n) {
            Some(Value::Raw(b)) if b.len() == 12 => {
                Some([0, 4, 8].map(|i| <[u8; 4]>::try_from(&b[i..i + 4]).unwrap()))
            }
            _ => None,
        };
        let location = raw3("Location").map_or([0.0; 3], |v| v.map(f32::from_le_bytes));
        let yaw = raw3("Rotation").map_or(0.0, |v| {
            i32::from_le_bytes(v[1]) as f32 / 65536.0 * std::f32::consts::TAU
        });
        let tag = match find("Tag") {
            Some(Value::Name(n)) => pkg.name(*n).to_string(),
            _ => class.to_string(),
        };
        out.push(Place {
            name: pkg.name(pkg.exports[a].object_name).to_string(),
            class: class.to_string(),
            tag,
            location,
            yaw,
        });
    }
    out
}
