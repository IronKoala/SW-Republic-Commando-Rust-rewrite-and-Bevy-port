//! The map's ammo and grenade boxes and the weapons lying in it, handed
//! from the level part (which reads the map) to the combat part (which lets
//! the player take them).

use bevy::prelude::*;

/// One box as the map places it.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedAmmoBox {
    /// Its export index in the map: the drawn model carries the same number
    /// (`rc_level_viewer::draw::MoverPart`), so it can be hidden when taken.
    pub export: usize,
    /// Its name in the map (`DC17mBlasterBox14`), used in save files.
    pub name: String,
    /// Its class (`DC17mBlasterBox`).
    pub class: String,
    /// Centre of its touch cylinder, Bevy units.
    pub at: Vec3,
    /// The original's ammo class it refills (`DC17mBlasterAmmo`).
    pub ammo: String,
    /// How much it gives.
    pub amount: u32,
    /// Seconds before it comes back after being taken; `None`: gone for
    /// good once taken.
    pub respawn: Option<f32>,
    /// Touch cylinder radius and half-height, Unreal units.
    pub radius: f32,
    pub height: f32,
    /// What the game says when it is taken (`PickupMessage`).
    pub message: String,
    /// The sound it makes when taken (`PickupSound`): sound package and
    /// name (`GEN_Sound`, `GenericWeaponPickup`).
    pub sound: Option<(String, String)>,
}

/// Every box in the level just loaded. Inserted while the level loads.
#[derive(Resource, Debug, Clone, Default)]
pub struct LevelAmmoBoxes(pub Vec<PlacedAmmoBox>);

/// One weapon lying in the map, as the level part reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedWeapon {
    /// Its export index in the map: the drawn model carries the same number
    /// (`rc_level_viewer::draw::MoverPart`), so it can be hidden when taken.
    pub export: usize,
    /// Its name in the map (`BowcasterPickup0`), used in save files.
    pub name: String,
    /// Its class (`BowcasterPickup`).
    pub class: String,
    /// Centre of its touch cylinder, Bevy units.
    pub at: Vec3,
    /// The original's weapon class it gives (`Bowcaster`).
    pub weapon: String,
    /// Shots it holds.
    pub ammo: u32,
    /// What the game shows when it's taken ("Wookiee Bowcaster").
    pub message: String,
    /// The sound it makes when taken: sound package and name.
    pub sound: Option<(String, String)>,
    /// Touch cylinder radius and half-height, Unreal units.
    pub radius: f32,
    pub height: f32,
    /// The level event sent when the player picks it up (its `Event` in
    /// the map), if any.
    pub event: Option<String>,
}

/// What the map's player start adds to the player's equipment, as the
/// original's class paths (`CTInventory.DC15s`, `CTInventory.DC17mSniper`):
/// the map's `PlayerStart` `StartInventory`. The original gives the
/// player `PlayerCommando`'s own equipment (the DC-17m blaster and the
/// grenades) and then this list, so the pistol and the DC-17m attachments
/// come from the map (geo_03a's start has only the pistol; its sniper
/// attachment lies in the level). Inserted while the level loads.
#[derive(Resource, Debug, Clone, Default)]
pub struct LevelStartInventory(pub Vec<String>);

/// Every weapon lying in the level just loaded. Inserted while the level
/// loads.
#[derive(Resource, Debug, Clone, Default)]
pub struct LevelWeaponPickups(pub Vec<PlacedWeapon>);

/// The player just picked something up. The HUD shows `text` for a few
/// seconds above the middle of the screen (the original's
/// `PickupMessagePlus`), and `sound` is played (the original plays a
/// pickup's `PickupSound` for the player only, from where they are).
///
/// Anything that can be picked up sends this: ammo boxes do; weapons on
/// the floor and health can do the same.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct PickedUp {
    pub text: String,
    /// Sound package and the sound's name in it.
    pub sound: Option<(String, String)>,
}

/// A level script gives the player ammo, or takes it away (the original's
/// `ACTION_AddAmmo`, gameplay.u). geo_01a starts by taking every grenade
/// away; later levels hand out a full set. The combat part carries it out
/// on the player's weapon or grenade of that class, keeping the count
/// between none and the most carried (`Ammunition.AddAmmo`, engine.u).
#[derive(Message, Debug, Clone, PartialEq)]
pub struct AddAmmo {
    /// The original's weapon class (`WhichWeapon`: `ThermalDetonator`,
    /// `FlashBang`, ...).
    pub weapon_class: String,
    /// How much to add; below 0 takes away (`AmmoAmount`).
    pub amount: i32,
    /// Which script step asked, for the log.
    pub by: String,
}
