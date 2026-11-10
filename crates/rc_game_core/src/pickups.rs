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
}

/// Every box in the level just loaded. Inserted while the level loads.
#[derive(Resource, Debug, Clone, Default)]
pub struct LevelAmmoBoxes(pub Vec<PlacedAmmoBox>);
