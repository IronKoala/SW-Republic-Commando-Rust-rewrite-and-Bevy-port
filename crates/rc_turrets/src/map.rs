//! The turrets placed in a map, with what the map sets for each: its tag,
//! the events it sends, whether the player may use it, its limits, and
//! the spot its gunner gets off at.

use rc_level::actors::{read_placement, LevelInfo};
use rc_level::scripts::{read_object, ScriptObject, Val};
use rc_package::Package;

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedTurret {
    /// Export number in the map, for saves.
    pub export: usize,
    pub name: String,
    pub class: String,
    /// Its `Tag` (the class name when the map gives none).
    pub tag: String,
    /// Centre of its collision cylinder, uu.
    pub location: [f32; 3],
    /// Pitch, yaw, roll, rotation units.
    pub rotation: [i32; 3],
    /// Sent when anyone gets on (`ControlEvent`).
    pub control_event: Option<String>,
    /// Sent when the player gets on, before `control_event`
    /// (`PlayerControlEvent`).
    pub player_control_event: Option<String>,
    /// Sent when the gunner gets off (`ReleaseEvent`).
    pub release_event: Option<String>,
    /// What the map sets; `None` keeps the class's.
    pub player_usable: Option<bool>,
    pub min_yaw: Option<i32>,
    pub max_yaw: Option<i32>,
    pub min_pitch: Option<i32>,
    pub max_pitch: Option<i32>,
    pub unlimited_yaw: Option<bool>,
    pub speed: Option<f32>,
    /// The spot the gunner gets off at: the map's `NavPts` marker, uu.
    pub exit_spot: Option<[f32; 3]>,
    /// Or its offset from the turret (`NavPtLocs`): forward, right, up.
    pub exit_offset: Option<[f32; 3]>,
}

/// Every actor whose class `claims` says is a turret, in the map's order
/// (scripts find "the first turret with this tag" in that order).
pub fn read(
    pkg: &Package,
    data: &[u8],
    level: &LevelInfo,
    claims: &dyn Fn(&str) -> bool,
) -> (Vec<PlacedTurret>, Vec<String>) {
    let mut out = Vec::new();
    let mut problems = Vec::new();
    for &a in level.actors.iter().flatten() {
        let class = pkg.export_class_name(&pkg.exports[a]);
        if !claims(class) {
            continue;
        }
        let obj = match read_object(pkg, data, a) {
            Ok(o) => o,
            Err(e) => {
                problems.push(format!("{}: {e}", pkg.name(pkg.exports[a].object_name)));
                continue;
            }
        };
        let place = match read_placement(pkg, data, a) {
            Ok(p) => p,
            Err(e) => {
                problems.push(format!("{}: {e}", obj.name));
                continue;
            }
        };
        let exit_spot = match obj.get("NavPts") {
            Some(Val::List(l)) => l.first().and_then(|n| actor_location(pkg, data, n)),
            _ => None,
        };
        let exit_offset = match obj.get("NavPtLocs") {
            Some(Val::Structs(list)) => list.first().and_then(|s| {
                s.iter().find_map(|(k, v)| match v {
                    Val::Vector(v) if k.eq_ignore_ascii_case("Offset") => Some(*v),
                    _ => None,
                })
            }),
            _ => None,
        };
        out.push(PlacedTurret {
            export: a,
            name: obj.name.clone(),
            class: class.to_string(),
            tag: obj.tag().to_string(),
            location: place.location,
            rotation: place.rotation,
            control_event: obj.text("ControlEvent").map(str::to_string),
            player_control_event: obj.text("PlayerControlEvent").map(str::to_string),
            release_event: obj.text("ReleaseEvent").map(str::to_string),
            player_usable: flag(&obj, "bPlayerUsable"),
            min_yaw: obj.int("MinYaw"),
            max_yaw: obj.int("MaxYaw"),
            min_pitch: obj.int("MinPitch"),
            max_pitch: obj.int("MaxPitch"),
            unlimited_yaw: flag(&obj, "bUnlimitedYaw"),
            speed: obj.float("Speed"),
            exit_spot,
            exit_offset,
        });
    }
    (out, problems)
}

fn flag(o: &ScriptObject, key: &str) -> Option<bool> {
    match o.get(key) {
        Some(Val::Bool(b)) => Some(*b),
        _ => None,
    }
}

/// Where the actor called `name` is, uu.
fn actor_location(pkg: &Package, data: &[u8], name: &str) -> Option<[f32; 3]> {
    let i = pkg
        .exports
        .iter()
        .position(|e| pkg.name(e.object_name).eq_ignore_ascii_case(name))?;
    read_placement(pkg, data, i).ok().map(|p| p.location)
}
