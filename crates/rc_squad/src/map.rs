//! What a map says about the squad: the marked spots where a squad mate
//! can do a job when ordered to (the "context orders": use a bacta
//! dispenser, set a demolition charge, breach a door, snipe from here), and
//! where the map's own scripts would bring Delta Squad in.
//!
//! In the original a marked job is up to three actors (read from the maps,
//! see docs/RESEARCH.md, "Squad markers"):
//! - the thing itself, the "anchor": a `BactaDispenserGEO`, a
//!   `BreachDoorRas`, a `DemolitionCharge`, or a `MarkerSnipeAnchorRAS` (a
//!   firing position);
//! - the order marker, a `CTMarker` class from CTMarkers.u
//!   (`MarkerDoorBreach`, `MarkerHackDoor`, `MarkerHackTerminal`,
//!   `MarkerDemolition`, `MarkerSnipe`, `MarkerGrenade`, `MarkerAntiArmor`,
//!   ...), whose `Anchor` names the anchor. A grenade marker also names a
//!   `WeaponTarget`, the area to throw at. Bacta dispensers have none;
//! - `SquadMarkerSpot`s next to the anchor, where squad mates stand (one
//!   per member a job needs: three at a door breach), each naming the anchor
//!   in `MyAnchor`. A firing position often has no spot: the squad mate
//!   stands at the anchor.
//!
//! Squad mates arrive through `PawnFactory` actors whose `Prototype` is one
//! of the `Commando..Delta` classes.

use rc_level::actors::{self, Placement};
use rc_movement::Vec3;
use rc_package::properties::{self, Value};
use rc_package::{ObjectRef, Package};

/// A marked job a squad mate can be sent to.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MarkerSpot {
    /// The spot's actor name, like `SquadMarkerSpot3` (or the order
    /// marker's, like `MarkerSnipe1`, for a position with no spot).
    pub name: String,
    /// The order marker's class, like `MarkerDoorBreach`; empty when the
    /// job has none (bacta).
    pub marker_class: String,
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
    /// What the player aims at to order it: the order marker's location,
    /// else the anchor's, else the spot's.
    pub aim: Vec3,
    /// A grenade marker's target area (its `WeaponTarget`).
    pub target: Option<Vec3>,
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
    let export_of = |o: i32| match ObjectRef::from_index(o) {
        ObjectRef::Export(k) => Some(k),
        _ => None,
    };
    let yaw_of = |p: &Placement| p.rotation[1] as f32 / 65536.0 * std::f32::consts::TAU;
    // Spots, by the anchor they name.
    struct Spot {
        name: String,
        stand: Vec3,
        yaw: f32,
        anchor: Option<usize>,
    }
    struct Order {
        name: String,
        class: String,
        at: Vec3,
        yaw: f32,
        anchor: Option<usize>,
        target: Option<Vec3>,
    }
    let mut spots: Vec<Spot> = Vec::new();
    let mut orders: Vec<Order> = Vec::new();
    for (a, p) in &placed {
        let class = p.class.as_str();
        let is_order = class.starts_with("Marker") && !class.starts_with("MarkerSnipeAnchor");
        if class != "SquadMarkerSpot" && class != "PawnFactory" && !is_order {
            continue;
        }
        let Ok(props) = actors::read_actor_properties(pkg, data, *a) else {
            continue;
        };
        let object = |name: &str| match properties::find(pkg, &props, name).map(|q| &q.value) {
            Some(Value::Object(o)) => *o,
            _ => 0,
        };
        if class == "SquadMarkerSpot" {
            spots.push(Spot {
                name: p.name.clone(),
                stand: v(p.location),
                yaw: yaw_of(p),
                anchor: export_of(object("MyAnchor")),
            });
        } else if is_order {
            orders.push(Order {
                name: p.name.clone(),
                class: p.class.clone(),
                at: v(p.location),
                yaw: yaw_of(p),
                anchor: export_of(object("Anchor")),
                target: export_of(object("WeaponTarget")).and_then(location_of),
            });
        } else {
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
    let anchor_names = |k: Option<usize>| match k {
        Some(k) => (
            pkg.object_name((k + 1) as i32).to_string(),
            pkg.export_class_name(&pkg.exports[k]).to_string(),
        ),
        None => (String::new(), String::new()),
    };
    // Each order marker, at each spot of its anchor (or at the anchor
    // itself when it has no spot).
    let mut used = vec![false; spots.len()];
    for o in &orders {
        let (anchor, anchor_class) = anchor_names(o.anchor);
        let anchor_at = o.anchor.and_then(location_of);
        let mine: Vec<usize> = (0..spots.len())
            .filter(|&i| o.anchor.is_some() && spots[i].anchor == o.anchor)
            .collect();
        let job = job_name(&o.class);
        if mine.is_empty() {
            info.markers.push(MarkerSpot {
                name: o.name.clone(),
                marker_class: o.class.clone(),
                job,
                anchor,
                anchor_class,
                stand: anchor_at.unwrap_or(o.at),
                yaw: o.yaw,
                aim: o.at,
                target: o.target,
            });
            continue;
        }
        for i in mine {
            used[i] = true;
            info.markers.push(MarkerSpot {
                name: spots[i].name.clone(),
                marker_class: o.class.clone(),
                job: job.clone(),
                anchor: anchor.clone(),
                anchor_class: anchor_class.clone(),
                stand: spots[i].stand,
                yaw: spots[i].yaw,
                aim: o.at,
                target: o.target,
            });
        }
    }
    // Spots with no order marker (bacta dispensers, and the rest).
    for (i, sp) in spots.iter().enumerate() {
        if used[i] {
            continue;
        }
        let (anchor, anchor_class) = anchor_names(sp.anchor);
        info.markers.push(MarkerSpot {
            name: sp.name.clone(),
            marker_class: String::new(),
            job: job_name(&anchor_class),
            anchor,
            anchor_class,
            stand: sp.stand,
            yaw: sp.yaw,
            aim: sp.anchor.and_then(location_of).unwrap_or(sp.stand),
            target: None,
        });
    }
    Ok(info)
}
