//! Mounted turrets: what the turret part (`rc_turrets_plugin`) tells the
//! rest of the game while the player works one, and the level scripts'
//! orders to turrets.
//!
//! The turret part owns the turrets. The player's body, the camera and the
//! player's weapons only read [`Manning`], so a VR mod can still put the
//! view wherever the head is: `Manning` says where the body is held and
//! which way the turret points, not how to draw the view.

use bevy::prelude::*;

/// On the player's entity while they work a mounted turret. Written by the
/// turret part every frame; removed when they get off.
///
/// - The player's body doesn't walk: it is held at `body` (the original
///   puts the gunner `TurretHeight` above the turret's centre).
/// - The view looks where the turret points: `yaw` and `pitch`, radians,
///   Unreal's convention (yaw turns left from +X towards +Y, pitch up is
///   positive). The turret part turns the turret from the look input,
///   within the turret's limits and speed.
/// - The player's own weapons are put away and `weapon` (a file in
///   `data/weapons/`) is the one in hand, like the original's turret
///   first-person weapon (`FPWeaponClass`). Switching weapons does nothing
///   meanwhile.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct Manning {
    /// The turret's entity.
    pub turret: Entity,
    /// The turret's name in the map (`WookieeHeavyTurret0`), for the log.
    pub name: String,
    /// Where the body is held: the centre of its collision, Unreal units.
    pub body: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    /// The weapon in hand while on the turret (`data/weapons/<id>.toml`).
    pub weapon: Option<String>,
}

/// A level script's order to turrets, found by their `Tag` (sent by the
/// level scripts part, carried out by the turret part).
#[derive(Message, Debug, Clone, PartialEq)]
pub enum TurretOrder {
    /// `ACTION_AttachToTurret`: the characters running the script `owner`
    /// (a `ScriptedSequence`'s tag) get on the first turret tagged
    /// `turret` (`attach`), or off it.
    Attach {
        turret: String,
        attach: bool,
        owner: Option<String>,
        /// The script and action, for the log.
        by: String,
    },
    /// `ACTION_SetTurretUsable`: whether the player may get on every turret
    /// tagged `turret`.
    SetUsable {
        turret: String,
        usable: bool,
        by: String,
    },
}
