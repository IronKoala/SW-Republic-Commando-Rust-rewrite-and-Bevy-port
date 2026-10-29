//! Numbers the on-screen display shows. Gameplay plugins write these; the
//! display only reads them, so either side can change on its own.

use bevy::prelude::{Message, Resource};

/// The player's health and shield. Written by combat.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct PlayerVitals {
    pub health: f32,
    pub max_health: f32,
    pub shield: f32,
    pub max_shield: f32,
    /// True when the player is down and waiting for a squad mate.
    pub incapacitated: bool,
}

impl Default for PlayerVitals {
    fn default() -> Self {
        // Placeholders until combat sets the game's own numbers.
        PlayerVitals {
            health: 100.0,
            max_health: 100.0,
            shield: 100.0,
            max_shield: 100.0,
            incapacitated: false,
        }
    }
}

/// The weapon in hand. Written by weapons. `name` empty means no weapon,
/// and the display hides the ammo counter.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct WeaponReadout {
    pub name: String,
    /// Shots left in the magazine. `None` for weapons that don't use one
    /// (the pistol recharges).
    pub clip: Option<u32>,
    pub clip_size: Option<u32>,
    /// Spare ammo. `None` for unlimited.
    pub reserve: Option<u32>,
    /// Name and count of the selected grenade, if any.
    pub grenade: Option<(String, u32)>,
    /// How spread out the crosshair is, 0 (tight) to 1 (wide).
    pub spread: f32,
    /// True while the crosshair is over an enemy (the original turns it red).
    pub on_target: bool,
    /// While zoomed in (the sniper attachment): the field of view to use,
    /// degrees across a 4:3 screen like the video setting. `None` when not
    /// zoomed.
    pub zoom_fov: Option<f32>,
}

/// One squad mate on the display.
#[derive(Debug, Clone, PartialEq)]
pub struct SquadMember {
    /// "Fixer", "Sev", "Scorch".
    pub name: String,
    /// The number shown on the display (40, 62, 07).
    pub number: String,
    /// 0 to 1.
    pub health: f32,
    pub shield: f32,
    pub status: SquadStatus,
    /// What they're doing, in a few words ("Form up", "Sniping").
    pub order: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SquadStatus {
    #[default]
    Ok,
    /// Down, waiting to be revived.
    Incapacitated,
}

/// The squad on the display. Written by the squad plugin. Empty means no
/// squad, and the display hides that panel.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct SquadReadout {
    pub members: Vec<SquadMember>,
}

/// A line of text for the display to show for a few seconds ("Grenades
/// picked up", "Objective: ...").
#[derive(Message, Debug, Clone)]
pub struct HudMessage {
    pub text: String,
    pub seconds: f32,
}

/// Kick the view (weapon recoil), radians: `pitch` up, `yaw` right. Sent by
/// weapons; the player's view applies it. A VR mod can ignore it.
#[derive(Message, Debug, Clone, Copy)]
pub struct ViewKick {
    pub pitch: f32,
    pub yaw: f32,
}
