//! Things the player uses by holding the Use button (bacta dispensers),
//! and what that does to the player and the HUD.
//!
//! The level scripts part knows the dispensers and fills in [`UseItem`]
//! every frame; the player part reads [`UseItem::lock`] to pull the body
//! into place and hold it there, and the HUD reads [`UseItem::panel`] and
//! [`UseItem::prompt`] to draw the use panel round the crosshair and the
//! "PRESS AND HOLD" line. Nothing here assumes a flat
//! screen or a mouse: a VR mod can show the same lock and progress its
//! own way.

use bevy::prelude::*;

/// What using something asks of the player and the HUD this frame.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct UseItem {
    /// While set, the player is pulled to a spot and held there (the
    /// original's `bLockPlayerLocation`): no walking, jumping, crouching,
    /// shooting or squad orders, and the Use button keeps working.
    pub lock: Option<UseLock>,
    /// The use panel round the crosshair.
    pub panel: UsePanel,
    /// The line the HUD shows while the crosshair is on something usable
    /// (the original's `ActivatePromptText`, "PRESS AND HOLD F TO HEAL"),
    /// with the Use button's name already filled in.
    pub prompt: Option<String>,
    /// While set, the first-person arms put the weapon away and show the
    /// hands using the thing instead (the original's `HandSignal`
    /// "weapon"). The name picks which moves the hands play, from
    /// `data/combat/hand_signal.toml` (`"bacta"`: plugging into a bacta
    /// dispenser). A VR mod can read the same flag to pose its own hands.
    pub hands: Option<String>,
}

/// Where a used thing holds the player.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UseLock {
    /// Where the player's feet go, across the floor (x, y, Unreal units);
    /// the height stays the body's own.
    pub stand: [f32; 2],
    /// The way the player is turned to face, radians (Unreal yaw).
    pub face_yaw: f32,
    /// How fast the player is pulled there, units a second, and turned,
    /// radians a second.
    pub speed: f32,
    pub turn: f32,
}

/// The use panel at the crosshair (the original's, for a bacta
/// dispenser: the thing's icon, and round it a ring that fills).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum UsePanel {
    /// Nothing to use here.
    #[default]
    Hidden,
    /// The crosshair is on something usable that can't be used now (a
    /// dispenser at full health or recharging): its icon only, coloured
    /// by the `0` to `1` value (a bacta dispenser: the player's health).
    Icon(f32),
    /// It can be used: the icon, and the ring `0` to `1` full (a bacta
    /// dispenser: the player's health). Hold Use to start.
    Ready(f32),
    /// Being used, `0` to `1` full.
    Working(f32),
}

impl UseLock {
    /// Where to put the feet (x, y) and the facing this step, moving from
    /// `from` and `yaw` at most `speed` units and `turn` radians. Returns
    /// whether both have arrived.
    pub fn pull(&self, from: [f32; 2], yaw: f32, speed: f32, turn: f32) -> ([f32; 2], f32, bool) {
        use std::f32::consts::{PI, TAU};
        let dx = self.stand[0] - from[0];
        let dy = self.stand[1] - from[1];
        let d = (dx * dx + dy * dy).sqrt();
        let (pos, there) = if d <= speed || d < 1e-3 {
            (self.stand, true)
        } else {
            ([from[0] + dx / d * speed, from[1] + dy / d * speed], false)
        };
        let off = (self.face_yaw - yaw + PI).rem_euclid(TAU) - PI;
        let (yaw, facing) = if off.abs() <= turn {
            (self.face_yaw.rem_euclid(TAU), true)
        } else {
            ((yaw + turn * off.signum()).rem_euclid(TAU), false)
        };
        (pos, yaw, there && facing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_moves_and_turns_a_step_at_a_time() {
        let lock = UseLock {
            stand: [100.0, 0.0],
            face_yaw: 0.0,
            speed: 0.0,
            turn: 0.0,
        };
        let (pos, yaw, done) = lock.pull([0.0, 0.0], 1.0, 30.0, 0.25);
        assert_eq!(pos, [30.0, 0.0]);
        assert!((yaw - 0.75).abs() < 1e-5);
        assert!(!done);
        let (pos, yaw, done) = lock.pull([95.0, 0.0], 0.1, 30.0, 0.25);
        assert_eq!(pos, [100.0, 0.0]);
        assert_eq!(yaw, 0.0);
        assert!(done);
    }

    #[test]
    fn pull_turns_the_short_way_round() {
        let lock = UseLock {
            stand: [0.0, 0.0],
            face_yaw: 0.1,
            speed: 0.0,
            turn: 0.0,
        };
        // From just below a full turn, the short way is up through zero.
        let (_, yaw, _) = lock.pull([0.0, 0.0], std::f32::consts::TAU - 0.1, 0.0, 0.05);
        assert!(yaw > std::f32::consts::TAU - 0.1 || yaw < 0.1, "{yaw}");
    }
}
