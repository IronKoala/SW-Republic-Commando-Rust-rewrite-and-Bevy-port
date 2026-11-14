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
    /// While set, the player is held as the original's `CTPlayer` state
    /// `UsingItem` holds them at a console, door lock or charge: see
    /// [`UseHold`]. (A bacta dispenser uses [`UseItem::lock`] instead.)
    pub hold: Option<UseHold>,
    /// Which picture the use panel shows in its middle.
    pub icon: UseIcon,
    /// Seconds left on the job (the item's `DurationRemaining`), shown by
    /// the HUD as a "00:10" clock in the middle of the screen while above
    /// zero, as the original's helmet does for a panel target.
    pub timer: Option<f32>,
}

/// How the player is held while using a console, door lock or charge:
/// the original's `CTPlayer.UsingItem.PlayerMove` (ctgame.u). Each frame:
///
/// - Where they should stand (`stand`): `Target.Location + PlayerOffset *
///   Vector(Target.Rotation)` across the floor when the item has
///   `bLockPlayerLocation`; otherwise where they are (`None`), so they
///   stay put.
/// - Further away than `450 * dt`: the body is pushed with acceleration
///   `15 * (stand - feet) - velocity` (no height), and walks there under
///   its own movement. Closer: put on the spot (if more than 1 unit off)
///   and stopped.
/// - The view turns towards `look_at` (the item's place, raised by its
///   `HUDMarkerIconZOffset`) from the eye, each of yaw and pitch at most
///   45000 rotation units a second. The player's own looking does nothing.
///
/// Nothing here assumes a flat screen: a VR mod can move the body the
/// same way and point the head itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UseHold {
    /// Unreal units (x, y).
    pub stand: Option<[f32; 2]>,
    /// Unreal units.
    pub look_at: [f32; 3],
}

impl UseHold {
    /// The original's numbers (`UsingItem.PlayerMove`).
    pub const SPEED: f32 = 450.0;
    pub const PULL: f32 = 15.0;
    /// Rotation units (65536 a turn) a second, for yaw and for pitch.
    pub const TURN: f32 = 45000.0;

    /// One frame: from feet `at` (x, y) moving at `velocity` (x, y), what
    /// to do. `dt` seconds.
    pub fn step(&self, at: [f32; 2], velocity: [f32; 2], dt: f32) -> HoldStep {
        let Some(stand) = self.stand else {
            return HoldStep::Stop(None);
        };
        let to = [stand[0] - at[0], stand[1] - at[1]];
        let dist = (to[0] * to[0] + to[1] * to[1]).sqrt();
        if dist > Self::SPEED * dt {
            HoldStep::Push([
                Self::PULL * to[0] - velocity[0],
                Self::PULL * to[1] - velocity[1],
            ])
        } else if dist > 1.0 {
            HoldStep::Stop(Some(stand))
        } else {
            HoldStep::Stop(None)
        }
    }

    /// Turn `from` (radians) towards `want` (radians) by at most the
    /// original's 45000 units a second, the short way round, on the
    /// original's rotation units (whole numbers, 65536 a turn).
    pub fn turn(from: f32, want: f32, dt: f32) -> f32 {
        use std::f32::consts::TAU;
        let unit = |r: f32| (r / TAU * 65536.0).round() as i32;
        let mut d = (unit(want) - unit(from)) & 65535;
        if d > 32768 {
            d -= 65536;
        }
        let most = Self::TURN * dt;
        let d = (d as f32).clamp(-most, most);
        from + d / 65536.0 * TAU
    }
}

/// The clock the original's helmet shows for a console, door lock or
/// charge (`UHelmet::DrawHelmet`, Engine.dll 0x10438c13): only while
/// `remaining` is above 0; minutes are the whole seconds over 60, seconds
/// are `remaining + 0.5` rounded down, modulo 60, and a 0 there shows as
/// 1; printed `"%02i:%02i"`.
pub fn timer_text(remaining: f32) -> Option<String> {
    if remaining <= 0.0 {
        return None;
    }
    let minutes = remaining as i32 / 60;
    let mut seconds = (remaining + 0.5) as i32 % 60;
    if seconds == 0 {
        seconds = 1;
    }
    Some(format!("{minutes:02}:{seconds:02}"))
}

/// What [`UseHold::step`] says to do with the body this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HoldStep {
    /// Walk with this acceleration (x, y), Unreal units a second squared.
    Push([f32; 2]),
    /// Stand still: velocity and acceleration nothing; put the feet here
    /// first when given (x, y).
    Stop(Option<[f32; 2]>),
}

/// The picture in the middle of the use panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UseIcon {
    /// `data/ui/hud.toml`'s `[use_panel] icon` (a bacta dispenser's).
    #[default]
    Default,
    /// This square of the icon sheet (the squad marker's `HUDIconX`,
    /// `HUDIconY`).
    Square([u32; 2]),
    /// None: the item has no squad marker (the original draws no icon).
    Nothing,
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
    fn the_clock_reads_as_the_original_prints_it() {
        assert_eq!(timer_text(0.0), None);
        assert_eq!(timer_text(10.0).as_deref(), Some("00:10"));
        assert_eq!(timer_text(9.4).as_deref(), Some("00:09"));
        assert_eq!(timer_text(9.6).as_deref(), Some("00:10"));
        // A whole minute: the seconds' 0 shows as 1.
        assert_eq!(timer_text(60.0).as_deref(), Some("01:01"));
        assert_eq!(timer_text(0.2).as_deref(), Some("00:01"));
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
