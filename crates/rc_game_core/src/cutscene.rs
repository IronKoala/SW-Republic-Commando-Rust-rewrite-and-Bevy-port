//! What a level's cutscene asks of the player and the screen.
//!
//! Republic Commando's cutscenes (the prologue, the briefings, the
//! epilogue, moments inside missions) play in the game's own world, seen
//! through the player's eyes. The level scripts hold the player still and
//! limit where they can look (`ACTION_Briefing`), move them between rooms
//! (`ACTION_TeleportToPoint`), and fade or flash the screen
//! (`ACTION_ScreenFade`, `ACTION_AnimateVisorFX`). The scripts part fills
//! in [`Cutscene`] and sends [`TeleportPlayer`]; the player, the HUD and
//! the screen read them. Nothing here knows about a flat screen: a VR
//! mod can show the same tint and limits its own way.

use bevy::prelude::*;

/// The cutscene's hold on the player and the screen. All off outside
/// cutscenes.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct Cutscene {
    /// The player is held for a briefing or cutscene: no walking,
    /// jumping, crouching, shooting, grenades or orders. Looking around
    /// stays, within the limits.
    pub held: Option<HeldView>,
    /// The body doesn't move at all, not even falling (the script set
    /// the player's physics to none).
    pub frozen: bool,
    /// A colour over the whole screen: red, green, blue, opacity, each 0
    /// to 1 (sRGB). Fades to black and white flashes. Opacity 0 is off.
    pub tint: [f32; 4],
    /// The HUD is hidden (a briefing with `bDrawHUD` off).
    pub hide_hud: bool,
    /// The helmet frame is hidden (`ACTION_ShowVisor` off: the prologue's
    /// clone has no helmet yet).
    pub hide_visor: bool,
    /// What the player holds instead of their own arms and gun
    /// (`BriefingWeapon`, like the prologue's `CTInventory.Baby` and
    /// `CTInventory.Boy`). Those hands aren't drawn yet (placeholder: no
    /// arms or gun at all), so the grown commando's arms stay away.
    pub briefing_weapon: Option<String>,
}

/// Where a held player may look. Angles are radians, Unreal's way round
/// (yaw turns right, pitch up), measured in the world, not from where the
/// player faced. `None`: no limit that way.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HeldView {
    pub yaw: Option<(f32, f32)>,
    pub pitch: Option<(f32, f32)>,
    /// Weapon changes are allowed (`bOkToChangeWeapon`).
    pub weapon_change: bool,
}

impl HeldView {
    /// Keep a look direction inside the limits. Yaw wraps around: it is
    /// measured from the middle of its range.
    pub fn limit(&self, yaw: f32, pitch: f32) -> (f32, f32) {
        use std::f32::consts::{PI, TAU};
        let yaw = match self.yaw {
            Some((lo, hi)) if hi - lo < TAU => {
                let mid = (lo + hi) * 0.5;
                let half = (hi - lo) * 0.5;
                let off = (yaw - mid + PI).rem_euclid(TAU) - PI;
                (mid + off.clamp(-half, half)).rem_euclid(TAU)
            }
            _ => yaw,
        };
        let pitch = match self.pitch {
            Some((lo, hi)) => pitch.clamp(lo, hi),
            None => pitch,
        };
        (yaw, pitch)
    }
}

/// Move the player somewhere at once (`ACTION_TeleportToPoint`).
#[derive(Message, Debug, Clone, PartialEq)]
pub struct TeleportPlayer {
    /// Where the middle of the player's body goes, Unreal units.
    pub location: [f32; 3],
    /// Facing, radians (Unreal yaw).
    pub yaw: f32,
    /// Who asked, for the log.
    pub by: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn look_limits_wrap_around() {
        // A range across the back (170 to 190 degrees).
        let v = HeldView {
            yaw: Some((170f32.to_radians(), 190f32.to_radians())),
            pitch: Some((-0.2, 0.3)),
            weapon_change: false,
        };
        let (y, p) = v.limit(-175f32.to_radians(), 1.0);
        assert!((y - 185f32.to_radians()).abs() < 1e-4, "{y}");
        assert_eq!(p, 0.3);
        let (y, _) = v.limit(0.0, 0.0);
        // Straight ahead is 180 degrees from the middle: clamped to an edge.
        assert!((y - 170f32.to_radians()).abs() < 1e-4 || (y - 190f32.to_radians()).abs() < 1e-4);
        // Negative limits (the map's -35000 to -24000) work too.
        let v = HeldView {
            yaw: Some((-3.36, -2.30)),
            pitch: None,
            weapon_change: false,
        };
        let (y, _) = v.limit(-2.8f32.rem_euclid(2.0 * PI), 0.0);
        assert!((y - (-2.8f32).rem_euclid(2.0 * PI)).abs() < 1e-4);
    }
}
