//! What a map says about the squad: the marked spots where a squad mate
//! can do a job when ordered to (the "context orders": use a bacta
//! dispenser, set a demolition charge, breach a door, snipe from here), and
//! where the map's own scripts would bring Delta Squad in.
//!
//! In the original a marked job is two actors: the thing itself (the
//! "anchor", like a `BactaDispenserGEO`) and a `SquadMarkerSpot` next to it,
//! where the squad mate stands to do it. The spot names its anchor in its
//! `MyAnchor` setting. Squad mates arrive through `PawnFactory` actors whose
//! `Prototype` is one of the `Commando..Delta` classes.

use rc_level::actors::{self, Placement};
use rc_movement::Vec3;
use rc_package::properties::{self, Value};
use rc_package::{ObjectRef, Package};

/// A marked job a squad mate can be sent to.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkerSpot {
    /// The spot's actor name, like `SquadMarkerSpot3`.
    pub name: String,
    /// What the job is, in a word or two ("Bacta", "Demolition").
    pub job: String,
    /// The anchor's actor and class, like `DemolitionCharge2` /
    /// `DemolitionCharge`.
    pub anchor: String,
    pub anchor_class: String,
    /// Where the squad mate stands (the spot actor's location).
    pub stand: Vec3,
    /// Facing at the spot, radians.
    pub yaw: f32,
    /// What the player aims at to order it: the anchor's location, or the
    /// spot's if the anchor can't be found.
    pub aim: Vec3,
}

/// Where a map's own scripts bring in a squad mate.
#[derive(Debug, Clone, PartialEq)]
pub struct SquadFactory {
    pub actor: String,
    /// The character class, like `Commando62Delta`.
    pub class: String,
    pub location: Vec3,
    /// The event that triggers it (`levelstart` means at the start).
    pub tag: String,
}

#[derive(Debug, Clone, Default)]
pub struct MapSquadInfo {
    pub markers: Vec<MarkerSpot>,
    pub factories: Vec<SquadFactory>,
}

fn v(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], p[1], p[2])
}

/// The job's short name, from the anchor's class.
pub fn job_name(anchor_class: &str) -> String {
    let c = anchor_class.to_ascii_lowercase();
    let known = [
        ("bacta", "Bacta"),
        ("demolition", "Demolition"),
        ("breach", "Breach door"),
        ("hack", "Hack"),
        ("disarm", "Disarm trap"),
        ("turret", "Man turret"),
        ("snipe", "Snipe"),
        ("grenade", "Grenade"),
        ("antiarmor", "Anti-armour"),
        ("mine", "Set mine"),
    ];
    known
        .iter()
        .find(|(k, _)| c.contains(k))
        .map(|(_, n)| n.to_string())
        .unwrap_or_else(|| anchor_class.to_string())
}

/// Read the squad's spots and factories from a map package.
pub fn read(pkg: &Package, data: &[u8]) -> Result<MapSquadInfo, String> {
    let level = actors::read_level(pkg, data).map_err(|e| format!("level: {e}"))?;
    let mut info = MapSquadInfo::default();
    let placed: Vec<(usize, Placement)> = level
        .actors
        .iter()
        .flatten()
        .filter_map(|&a| actors::read_placement(pkg, data, a).ok().map(|p| (a, p)))
        .collect();
    let location_of = |export: usize| {
        placed
            .iter()
            .find(|(a, _)| *a == export)
            .map(|(_, p)| v(p.location))
    };
    for (a, p) in &placed {
        let class = p.class.as_str();
        let Ok(props) = actors::read_actor_properties(pkg, data, *a) else {
            continue;
        };
        let object = |name: &str| match properties::find(pkg, &props, name).map(|q| &q.value) {
            Some(Value::Object(o)) => *o,
            _ => 0,
        };
        if class == "SquadMarkerSpot" {
            let anchor = object("MyAnchor");
            let (anchor_name, anchor_class, aim) = match ObjectRef::from_index(anchor) {
                ObjectRef::Export(k) => (
                    pkg.object_name(anchor).to_string(),
                    pkg.export_class_name(&pkg.exports[k]).to_string(),
                    location_of(k).unwrap_or(v(p.location)),
                ),
                _ => (String::new(), String::new(), v(p.location)),
            };
            info.markers.push(MarkerSpot {
                name: p.name.clone(),
                job: job_name(&anchor_class),
                anchor: anchor_name,
                anchor_class,
                stand: v(p.location),
                yaw: p.rotation[1] as f32 / 65536.0 * std::f32::consts::TAU,
                aim,
            });
        } else if class == "PawnFactory" {
            let proto = object("Prototype");
            let proto_name = pkg.object_name(proto).to_string();
            let lower = proto_name.to_ascii_lowercase();
            if lower.starts_with("commando") && lower.ends_with("delta") {
                let tag = match properties::find(pkg, &props, "Tag").map(|q| &q.value) {
                    Some(Value::Name(n)) => pkg.name(*n).to_string(),
                    _ => String::new(),
                };
                info.factories.push(SquadFactory {
                    actor: p.name.clone(),
                    class: proto_name,
                    location: v(p.location),
                    tag,
                });
            }
        }
    }
    Ok(info)
}
