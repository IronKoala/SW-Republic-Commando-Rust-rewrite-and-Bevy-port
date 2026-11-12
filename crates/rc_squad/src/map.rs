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
//! Squad mates are placed in most mission maps as `Commando07Delta`,
//! `Commando40Delta` and `Commando62Delta` actors, and join the player's
//! squad when the level starts. A few arrive later through `PawnFactory`
//! actors whose `Prototype` is one of those classes (Scorch on geo_01a,
//! Fixer on ras_02b); those start outside the squad until a script's
//! `ACTION_ChangeSquadMember` adds them. Squad mates are not carried from
//! one map to the next: each map has its own (the original only carries
//! their health over).

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
    /// The order marker's actor name (`MarkerSnipe1`), which level scripts
    /// switch on and off; empty when the job has none (bacta).
    pub order: String,
}

/// Where a map's own scripts bring in a squad mate.
#[derive(Debug, Clone, PartialEq)]
pub struct SquadFactory {
    pub actor: String,
    /// The character class, like `Commando62Delta`.
    pub class: String,
    pub location: Vec3,
    /// Facing, radians.
    pub yaw: f32,
    /// The event that triggers it (`levelstart` means at the start).
    pub tag: String,
    /// The name scripts call what it makes (`PawnTagFromFactory`, like
    /// `Delta40`); the class name when it gives none.
    pub pawn_tag: String,
    /// The `ScriptedSequence` what it makes follows
    /// (`AIScriptFromFactory`), if any.
    pub script: Option<String>,
}

/// A squad mate placed in the map.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedMate {
    pub actor: String,
    /// The character class, like `Commando62Delta`.
    pub class: String,
    /// Feet are `location` less the collision height; this is the actor's
    /// own location (its middle), as the map gives it.
    pub location: Vec3,
    /// Facing, radians.
    pub yaw: f32,
    /// The name scripts call them (`Tag`): the class name unless the map
    /// gave another (`Seven`, `Delta07`).
    pub tag: String,
    /// The `ScriptedSequence` they follow (`AIScriptTag`), if any.
    pub script: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct MapSquadInfo {
    pub markers: Vec<MarkerSpot>,
    pub factories: Vec<SquadFactory>,
    /// Squad mates placed in the map (in the squad from the start).
    pub placed: Vec<PlacedMate>,
    /// The map has a `CTSquad` actor: the original sets the squad up only
    /// in maps that have one (the playable missions, not the prologue,
    /// briefings or epilogue).
    pub has_squad_actor: bool,
}

impl MapSquadInfo {
    /// Whether this map has a squad at all: a `CTSquad` actor, or any
    /// squad marker or squad-mate spawner.
    pub fn wants_squad(&self) -> bool {
        self.has_squad_actor || !self.markers.is_empty() || !self.factories.is_empty()
    }

    /// Where this map has the squad mate of `class` (like
    /// `Commando62Delta`): placed in the map, else made by a spawner.
    pub fn mate(&self, class: &str) -> Option<MateSource<'_>> {
        self.placed
            .iter()
            .filter(|p| p.class.eq_ignore_ascii_case(class))
            // A map with two (the prologue) has the one scripts call by
            // the class name in the squad.
            .min_by_key(|p| !p.tag.eq_ignore_ascii_case(class))
            .map(MateSource::Placed)
            .or_else(|| {
                self.factories
                    .iter()
                    .find(|f| f.class.eq_ignore_ascii_case(class))
                    .map(MateSource::Factory)
            })
    }
}

/// Where a map has a squad mate: see [`MapSquadInfo::mate`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MateSource<'a> {
    Placed(&'a PlacedMate),
    Factory(&'a SquadFactory),
}

impl MateSource<'_> {
    /// The name scripts call the mate.
    pub fn tag(&self) -> &str {
        match self {
            MateSource::Placed(p) => &p.tag,
            MateSource::Factory(f) => &f.pawn_tag,
        }
    }

    /// The `ScriptedSequence` the mate follows, if any.
    pub fn script(&self) -> Option<&str> {
        match self {
            MateSource::Placed(p) => p.script.as_deref(),
            MateSource::Factory(f) => f.script.as_deref(),
        }
    }
}

fn is_mate_class(class: &str) -> bool {
    let lower = class.to_ascii_lowercase();
    lower.starts_with("commando") && lower.ends_with("delta")
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
    let mut info = MapSquadInfo {
        has_squad_actor: pkg
            .exports
            .iter()
            .any(|e| pkg.export_class_name(e).eq_ignore_ascii_case("CTSquad")),
        ..Default::default()
    };
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
        let is_mate = is_mate_class(class);
        if class != "SquadMarkerSpot" && class != "PawnFactory" && !is_order && !is_mate {
            continue;
        }
        let Ok(props) = actors::read_actor_properties(pkg, data, *a) else {
            continue;
        };
        let object = |name: &str| match properties::find(pkg, &props, name).map(|q| &q.value) {
            Some(Value::Object(o)) => *o,
            _ => 0,
        };
        let name = |key: &str| match properties::find(pkg, &props, key).map(|q| &q.value) {
            Some(Value::Name(n)) => Some(pkg.name(*n).to_string())
                .filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("None")),
            _ => None,
        };
        if is_mate {
            info.placed.push(PlacedMate {
                actor: p.name.clone(),
                class: p.class.clone(),
                location: v(p.location),
                yaw: yaw_of(p),
                tag: name("Tag").unwrap_or_else(|| p.class.clone()),
                script: name("AIScriptTag"),
            });
        } else if class == "SquadMarkerSpot" {
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
            if is_mate_class(&proto_name) {
                info.factories.push(SquadFactory {
                    actor: p.name.clone(),
                    location: v(p.location),
                    yaw: yaw_of(p),
                    tag: name("Tag").unwrap_or_default(),
                    pawn_tag: name("PawnTagFromFactory").unwrap_or_else(|| proto_name.clone()),
                    script: name("AIScriptFromFactory"),
                    class: proto_name,
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
                order: o.name.clone(),
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
                order: o.name.clone(),
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
            order: String::new(),
        });
    }
    Ok(info)
}
