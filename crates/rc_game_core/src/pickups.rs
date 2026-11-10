//! The map's ammo and grenade boxes, handed from the level part (which
//! reads the map) to the combat part (which lets the player take them).

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
