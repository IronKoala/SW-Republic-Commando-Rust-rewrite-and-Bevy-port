//! Ammo and grenade boxes placed in a map (`DC17mBlasterBox`,
//! `ThermalDetonatorBox`, ...): where each one is, what it refills and by
//! how much. Also the weapons lying in the maps (`PlacedWeapon`).
//!
//! Every such class comes from the engine's `Ammo` pickup class. The map
//! stores only where a box is; what it gives is the class's defaults
//! (`AmmoAmount`, `InventoryType`, `CollisionRadius`, `CollisionHeight`,
//! `StaticMesh`), with the matching "...Defaults" class in
//! `Properties/properties.u` on top, the same way the weapon numbers work
//! (see docs/RESEARCH.md "Weapons and damage"). For example the DC-17m box
//! gives 120 shots in `CTInventory.u`, and `DC17mBlasterBoxDefaults` makes
//! it 300.
//!
//! Like the rest of this crate it opens no files: the caller hands in a
//! function that opens a package by name.

use crate::actors::{self, Placement};
use crate::class_look::{object_path, ClassLook, ClassLooks, Loaded};
use rc_package::properties::Value;
use rc_package::Package;

/// The package holding the "...Defaults" classes.
pub const DEFAULTS_PACKAGE: &str = "Properties";

/// One box in the map.
#[derive(Debug, Clone, PartialEq)]
pub struct AmmoBox {
    /// The box's export index in the map.
    pub export: usize,
    /// Its name in the map (`DC17mBlasterBox14`), which save files use.
    pub name: String,
    /// Its class (`DC17mBlasterBox`).
    pub class: String,
    /// Centre of its collision cylinder, Unreal units, Z up.
    pub location: [f32; 3],
    /// What it refills: the original's ammo class (`DC17mBlasterAmmo`,
    /// `ThermalDetonatorAmmo`, ...).
    pub ammo: String,
    /// How much it gives (`AmmoAmount`).
    pub amount: u32,
    /// Seconds before it comes back after being taken, for the few boxes a
    /// map marks `bForceRespawn` (geo_01a's endless grenade crate). `None`:
    /// once taken it's gone, as for every pickup in single player.
    pub respawn: Option<f32>,
    /// Collision cylinder: radius and half-height, Unreal units.
    pub radius: f32,
    pub height: f32,
    /// How far it was moved down onto the ground (`rest_on_ground`).
    pub dropped: f32,
    /// What the game says when it is taken (`PickupMessage`, for example
    /// "DC-17m Blaster Clips"). Empty if the class has none.
    pub message: String,
    /// The sound it makes when taken (`PickupSound`): its sound package
    /// and its name in that package (`GEN_Sound`, `GenericWeaponPickup`).
    pub sound: Option<(String, String)>,
}

/// The class's look and values with its "...Defaults" class on top, if
/// there is one; else the class's own.
pub fn box_look(
    looks: &mut ClassLooks,
    map: &Package,
    actor: usize,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> Option<ClassLook> {
    let class = map.export_class_name(map.exports.get(actor)?).to_string();
    looks
        .for_class(DEFAULTS_PACKAGE, &format!("{class}Defaults"), open)
        .filter(|l| l.chain.iter().any(|c| c.eq_ignore_ascii_case(&class)))
        .or_else(|| looks.for_actor(map, actor, open))
}

/// Whether the actor is an ammo or grenade box (its class comes from the
/// engine's `Ammo`).
pub fn is_ammo_box(
    looks: &mut ClassLooks,
    map: &Package,
    actor: usize,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> bool {
    looks
        .for_actor(map, actor, open)
        .is_some_and(|l| l.is_a("Ammo"))
}

/// Whether the actor is any pickup (ammo boxes, weapons on the floor,
/// health). Pickups don't stop the player in the game: walking into one
/// is how you take it.
pub fn is_pickup(
    looks: &mut ClassLooks,
    map: &Package,
    actor: usize,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> bool {
    looks
        .for_actor(map, actor, open)
        .is_some_and(|l| l.is_a("Pickup"))
}

fn float(look: &ClassLook, name: &str) -> Option<f32> {
    match look.find(name, 0).map(|(_, p)| &p.value) {
        Some(Value::Float(f)) => Some(*f),
        _ => None,
    }
}

/// A pickup's `PickupSound` as (sound package, sound name): the map's own
/// setting first, then its classes'.
fn pickup_sound(
    map: &Package,
    props: &[rc_package::properties::Property],
    look: &ClassLook,
) -> Option<(String, String)> {
    match rc_package::properties::find(map, props, "PickupSound").map(|p| &p.value) {
        Some(Value::Object(o)) if *o != 0 => Some(object_path(map, *o)),
        _ => look
            .find("PickupSound", 0)
            .and_then(|(l, p)| match p.value {
                Value::Object(o) if o != 0 => Some(object_path(&l.0, o)),
                _ => None,
            }),
    }
    .and_then(|path| Some((path.first()?.clone(), path.last()?.clone())))
    .filter(|(pkg, name)| pkg != name)
}

/// Read one box. `None` if the actor isn't an ammo box or can't be read.
pub fn read_box(
    looks: &mut ClassLooks,
    map: &Package,
    map_data: &[u8],
    actor: usize,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> Option<AmmoBox> {
    if !is_ammo_box(looks, map, actor, open) {
        return None;
    }
    let look = box_look(looks, map, actor, open)?;
    let place = actors::read_placement(map, map_data, actor).ok()?;
    let props = actors::read_actor_properties(map, map_data, actor).ok()?;
    let in_map = |n: &str| rc_package::properties::find(map, &props, n).map(|p| &p.value);
    let amount =
        match in_map("AmmoAmount").or_else(|| look.find("AmmoAmount", 0).map(|(_, p)| &p.value)) {
            Some(Value::Int(n)) => (*n).max(0) as u32,
            _ => 0,
        };
    let ammo = look
        .find("InventoryType", 0)
        .and_then(|(l, p)| match p.value {
            Value::Object(o) if o != 0 => object_path(&l.0, o).last().cloned(),
            _ => None,
        })
        .unwrap_or_default();
    let force = matches!(in_map("bForceRespawn"), Some(Value::Bool(true)));
    let respawn_time = match in_map("RespawnTime") {
        Some(Value::Float(f)) => *f,
        _ => float(&look, "RespawnTime").unwrap_or(30.0),
    };
    let radius = match in_map("CollisionRadius") {
        Some(Value::Float(f)) => *f,
        _ => float(&look, "CollisionRadius").unwrap_or(22.0),
    };
    let height = match in_map("CollisionHeight") {
        Some(Value::Float(f)) => *f,
        _ => float(&look, "CollisionHeight").unwrap_or(22.0),
    };
    // Map first (a level designer may change it), then the classes.
    let message = rc_package::properties::find(map, &props, "PickupMessage")
        .and_then(rc_package::class::string_value)
        .or_else(|| {
            look.find("PickupMessage", 0)
                .and_then(|(_, p)| rc_package::class::string_value(p))
        })
        .unwrap_or_default();
    let sound = pickup_sound(map, &props, &look);
    Some(AmmoBox {
        export: actor,
        name: place.name.clone(),
        class: place.class.clone(),
        location: place.location,
        ammo,
        amount,
        respawn: force.then_some(respawn_time),
        radius,
        height,
        dropped: 0.0,
        message,
        sound,
    })
}

/// Every ammo and grenade box in the map's actor list.
pub fn read_boxes(
    looks: &mut ClassLooks,
    map: &Package,
    map_data: &[u8],
    actors: impl IntoIterator<Item = usize>,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> Vec<AmmoBox> {
    actors
        .into_iter()
        .filter_map(|a| read_box(looks, map, map_data, a, open))
        .collect()
}

/// Give a box with no model of its own in the map its class's model
/// (with the "...Defaults" class on top, which for example swaps the sonic
/// grenade box's model). Returns the package the mesh reference belongs to.
pub fn apply_box_look(
    looks: &mut ClassLooks,
    map: &Package,
    place: &mut Placement,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> Option<Loaded> {
    if place.static_mesh != 0 {
        return None;
    }
    let look = box_look(looks, map, place.export, open)?;
    let (owner, mesh) = look.static_mesh.clone()?;
    place.static_mesh = mesh;
    if let Some(s) = look.draw_scale {
        place.draw_scale = s;
    }
    if let Some(s) = look.draw_scale_3d {
        place.draw_scale_3d = s;
    }
    Some(owner)
}

/// A weapon lying in the map for the player to pick up
/// (`BowcasterPickup`, `DC17mSniperWeaponPickup`, ...).
///
/// These come from the game's `CTPickup` (a `Pickup`), and their
/// `InventoryType` names a weapon class. The map stores only where one is:
/// the weapon, its look and its size come from the pickup class, and how
/// many shots it holds is the weapon's `PickupAmmoCount` (with the
/// weapon's "...Defaults" class in `Properties/properties.u` on top, as for
/// the weapon numbers).
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedWeapon {
    /// The pickup's export index in the map.
    pub export: usize,
    /// Its name in the map (`BowcasterPickup0`), which save files use.
    pub name: String,
    /// Its class (`BowcasterPickup`).
    pub class: String,
    /// Centre of its collision cylinder, Unreal units, Z up.
    pub location: [f32; 3],
    /// The original's weapon class it gives (`Bowcaster`).
    pub weapon: String,
    /// Shots it holds (the weapon's `PickupAmmoCount`).
    pub ammo: u32,
    /// The text the game shows when it is taken (`PickupMessage`,
    /// "Wookiee Bowcaster").
    pub message: String,
    /// The sound it makes when taken (`PickupSound`): sound package and
    /// name.
    pub sound: Option<(String, String)>,
    /// Collision cylinder: radius and half-height, Unreal units.
    pub radius: f32,
    pub height: f32,
    /// How far it was moved down onto the ground (`rest_weapon_on_ground`).
    pub dropped: f32,
    /// The event it sends when the player picks it up (the map's `Event`
    /// on it, like geo_03a's `GotSniper`), if any.
    pub event: Option<String>,
}

/// The weapon class (package, class) a pickup class gives, if its
/// `InventoryType` names a class that comes from the engine's `Weapon`.
/// `home` is the package of the pickup's class, where a weapon class named
/// without a package is.
fn weapon_given(
    looks: &mut ClassLooks,
    look: &ClassLook,
    home: &str,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> Option<(String, String, ClassLook)> {
    let (owner, p) = look.find("InventoryType", 0)?;
    let path = match p.value {
        Value::Object(o) if o != 0 => object_path(&owner.0, o),
        _ => return None,
    };
    let (package, class) = match path.len() {
        0 => return None,
        // A class in the same package as the pickup class.
        1 => (home.to_string(), path[0].clone()),
        n => (path[0].clone(), path[n - 1].clone()),
    };
    let weapon = looks.for_class(&package, &class, open)?;
    weapon.is_a("Weapon").then_some((package, class, weapon))
}

/// The package an actor's class is in (`CTInventory`).
fn class_package(map: &Package, actor: usize) -> String {
    map.exports
        .get(actor)
        .map(|e| object_path(map, e.class_index))
        .and_then(|p| p.first().cloned())
        .unwrap_or_default()
}

/// Whether the actor is a weapon lying in the map: a pickup (not an ammo
/// box, not health) whose `InventoryType` is a weapon.
pub fn is_weapon_pickup(
    looks: &mut ClassLooks,
    map: &Package,
    actor: usize,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> bool {
    let Some(look) = looks.for_actor(map, actor, open) else {
        return false;
    };
    if !look.is_a("Pickup") || look.is_a("Ammo") || look.is_a("PickupHealth") {
        return false;
    }
    let home = class_package(map, actor);
    weapon_given(looks, &look, &home, open).is_some()
}

/// The shots a weapon class's pickup holds: `PickupAmmoCount`, from its
/// "...Defaults" class if that sets it, else the class's own.
fn pickup_ammo(
    looks: &mut ClassLooks,
    class: &str,
    weapon: &ClassLook,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> u32 {
    let defaults = looks
        .for_class(DEFAULTS_PACKAGE, &format!("{class}Defaults"), open)
        .filter(|l| l.chain.iter().any(|c| c.eq_ignore_ascii_case(class)));
    let look = defaults.as_ref().unwrap_or(weapon);
    match look.find("PickupAmmoCount", 0).map(|(_, p)| &p.value) {
        Some(Value::Int(n)) => (*n).max(0) as u32,
        _ => 0,
    }
}

/// Read one weapon pickup. `None` if the actor isn't one or can't be read.
pub fn read_weapon(
    looks: &mut ClassLooks,
    map: &Package,
    map_data: &[u8],
    actor: usize,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> Option<PlacedWeapon> {
    if !is_weapon_pickup(looks, map, actor, open) {
        return None;
    }
    let look = box_look(looks, map, actor, open)?;
    let home = class_package(map, actor);
    let (_, weapon, weapon_look) = weapon_given(looks, &look, &home, open)?;
    let ammo = pickup_ammo(looks, &weapon, &weapon_look, open);
    let place = actors::read_placement(map, map_data, actor).ok()?;
    let props = actors::read_actor_properties(map, map_data, actor).ok()?;
    let in_map = |n: &str| rc_package::properties::find(map, &props, n).map(|p| &p.value);
    let radius = match in_map("CollisionRadius") {
        Some(Value::Float(f)) => *f,
        _ => float(&look, "CollisionRadius").unwrap_or(84.0),
    };
    let height = match in_map("CollisionHeight") {
        Some(Value::Float(f)) => *f,
        _ => float(&look, "CollisionHeight").unwrap_or(8.0),
    };
    let message = look
        .find("PickupMessage", 0)
        .and_then(|(_, p)| rc_package::class::string_value(p))
        .unwrap_or_default();
    let sound = pickup_sound(map, &props, &look);
    let event = match in_map("Event") {
        Some(Value::Name(n)) => Some(map.name(*n).to_string())
            .filter(|e| !e.is_empty() && !e.eq_ignore_ascii_case("None")),
        _ => None,
    };
    Some(PlacedWeapon {
        export: actor,
        name: place.name.clone(),
        class: place.class.clone(),
        location: place.location,
        weapon,
        ammo,
        message,
        sound,
        radius,
        height,
        dropped: 0.0,
        event,
    })
}

/// Every weapon pickup in the map's actor list.
pub fn read_weapons(
    looks: &mut ClassLooks,
    map: &Package,
    map_data: &[u8],
    actors: impl IntoIterator<Item = usize>,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> Vec<PlacedWeapon> {
    actors
        .into_iter()
        .filter_map(|a| read_weapon(looks, map, map_data, a, open))
        .collect()
}

/// Whether the actor is a pickup drawn on its own so it can vanish when
/// taken: an ammo or grenade box, or a weapon.
pub fn is_takeable(
    looks: &mut ClassLooks,
    map: &Package,
    actor: usize,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> bool {
    is_ammo_box(looks, map, actor, open) || is_weapon_pickup(looks, map, actor, open)
}

/// Where a weapon comes to rest, by the same rule as the boxes
/// (`rest_on_ground`). Returns how far it moved.
pub fn rest_weapon_on_ground(w: &mut PlacedWeapon, below: impl Fn([f32; 3]) -> Option<f32>) -> f32 {
    let d = drop_onto_ground(w.location, w.height, below);
    w.location[2] -= d;
    w.dropped += d;
    d
}

/// How far a pickup centred at `at` with half-height `height` falls onto
/// the ground below it: 0 if it rests already, or the ground is more than
/// `MAX_DROP` below.
fn drop_onto_ground(at: [f32; 3], height: f32, below: impl Fn([f32; 3]) -> Option<f32>) -> f32 {
    let Some(ground) = below(at) else {
        return 0.0;
    };
    let drop = at[2] - height - ground;
    if drop > 0.0 && drop <= MAX_DROP {
        drop
    } else {
        0.0
    }
}

/// The furthest a box is moved down onto the ground, Unreal units.
///
/// **Our choice.** Across the 477 campaign boxes, almost all sit 0 to 37
/// units above the floor under them. Ten sit 50 to 745 units above it
/// (geo_05c, ras_03a, yyy_04a, yyy_04e, yyy_05d, yyy_35b): those rest on
/// something the port doesn't count as ground (a shelf or crate the player
/// can't stand on, so it isn't in the collision), so they stay where the
/// map put them.
pub const MAX_DROP: f32 = 40.0;

/// Where a box comes to rest: the game drops pickups when a single-player
/// level starts, until the bottom of the collision cylinder touches what is
/// below. `below` gives the height of the ground under a point (Unreal
/// units), as `crate::settle::Ground::below` does. Boxes already resting
/// (or sunk a little into the floor), and boxes more than `MAX_DROP` above
/// the ground found, stay put. Returns how far it moved.
pub fn rest_on_ground(b: &mut AmmoBox, below: impl Fn([f32; 3]) -> Option<f32>) -> f32 {
    let d = drop_onto_ground(b.location, b.height, below);
    b.location[2] -= d;
    b.dropped += d;
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_box(z: f32) -> AmmoBox {
        AmmoBox {
            export: 0,
            name: "DC17mBlasterBox0".into(),
            class: "DC17mBlasterBox".into(),
            location: [0.0, 0.0, z],
            ammo: "DC17mBlasterAmmo".into(),
            amount: 300,
            respawn: None,
            radius: 40.0,
            height: 15.0,
            dropped: 0.0,
            message: String::new(),
            sound: None,
        }
    }

    #[test]
    fn a_floating_box_drops_onto_the_floor() {
        let mut b = a_box(45.0);
        let d = rest_on_ground(&mut b, |_| Some(0.0));
        assert_eq!(d, 30.0);
        assert_eq!(b.location[2], 15.0);
        assert_eq!(b.dropped, 30.0);
    }

    #[test]
    fn a_resting_box_stays() {
        let mut b = a_box(10.0);
        assert_eq!(rest_on_ground(&mut b, |_| Some(0.0)), 0.0);
        assert_eq!(b.location[2], 10.0);
        let mut b = a_box(10.0);
        assert_eq!(rest_on_ground(&mut b, |_| None), 0.0);
    }

    #[test]
    fn a_box_far_above_the_ground_found_stays() {
        let mut b = a_box(760.0);
        assert_eq!(rest_on_ground(&mut b, |_| Some(0.0)), 0.0);
        assert_eq!(b.location[2], 760.0);
    }
}
