//! rc_turrets: mounted turrets, with no Bevy in them.
//!
//! A turret (the original's `Turret` pawn: the Wookiee heavy turret on
//! Kashyyyk, the Trandoshan missile turrets, the E-Web) is something one
//! gunner at a time gets on. This crate holds its rules, read from the
//! original's script defaults and written down in our own words:
//!
//! - **Who is on it** ([`Gunner`]). Nobody, the player, or a character a
//!   level script put there (`ACTION_AttachToTurret`). Only one at a time.
//!   The player may get on only while the turret is player-usable
//!   (`bPlayerUsable`, which maps and `ACTION_SetTurretUsable` change).
//! - **The events it sends.** Getting on sends `PlayerControlEvent` (the
//!   player only) and then `ControlEvent` (anyone); getting off sends
//!   `ReleaseEvent`. The last level's ending waits for the player's turret
//!   to send `MountedTurret4`.
//! - **Aiming.** The gun turns from its resting direction by `yaw` and
//!   `pitch` (rotation units, 65536 a full turn), at most `speed` units a
//!   second each way, and stops at its limits (`MinYaw`..`MaxYaw`,
//!   `MinPitch`..`MaxPitch`, or all the way round with `bUnlimitedYaw`).
//!   As in the original, the yaw wraps to -32768..32768 after the limits,
//!   so limits of a whole turn or more mean no limit.
//! - **Where the gunner is.** Held `height` above the turret's centre while
//!   on it, and put at the map's exit spot (`NavPts`) when getting off.
//!
//! The Bevy side (`rc_turrets_plugin`) draws the turrets, reads the
//! player's Use button and look, and fires the turret's weapon.

pub mod map;
pub mod params;

pub use map::PlacedTurret;
pub use params::TurretParams;

/// Rotation units in a whole turn.
pub const TURN: f32 = 65536.0;

/// Rotation units to radians.
pub fn to_radians(units: f32) -> f32 {
    units / TURN * std::f32::consts::TAU
}

/// Radians to rotation units.
pub fn to_units(radians: f32) -> f32 {
    radians / std::f32::consts::TAU * TURN
}

/// Who is on a turret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gunner {
    Nobody,
    Player,
    /// A character a script sent there: the script's tag (the
    /// `ScriptedSequence` it runs).
    Character(String),
}

/// What happened when someone tried to get on or off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mounted {
    /// On. The events to send, in order.
    On(Vec<String>),
    /// Off. The events to send.
    Off(Vec<String>),
    /// Someone else is on it.
    Taken,
    /// The player may not use it now.
    NotUsable,
    /// Nobody was on it.
    NotOn,
}

/// The limits and speed one turret aims with (its class's, with the map's
/// own on top).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub speed: f32,
    pub min_yaw: i32,
    pub max_yaw: i32,
    pub min_pitch: i32,
    pub max_pitch: i32,
    pub unlimited_yaw: bool,
}

/// One turret in the level.
#[derive(Debug, Clone, PartialEq)]
pub struct Turret {
    pub place: PlacedTurret,
    /// Its data file (`data/turrets/<id>.toml`).
    pub kind: String,
    pub limits: Limits,
    /// Where the gun points from its resting direction, rotation units.
    pub yaw: f32,
    pub pitch: f32,
    pub gunner: Gunner,
    pub player_usable: bool,
    /// The gunner's body is held this far above the centre, uu.
    pub height: f32,
    /// Where the gunner gets off when the map has no spot: forward, left,
    /// up from the centre, uu.
    pub exit: [f32; 3],
}

impl Turret {
    pub fn new(place: PlacedTurret, kind: &str, p: &TurretParams) -> Turret {
        let limits = Limits {
            speed: place.speed.unwrap_or(p.aim.speed),
            min_yaw: place.min_yaw.unwrap_or(p.aim.min_yaw),
            max_yaw: place.max_yaw.unwrap_or(p.aim.max_yaw),
            min_pitch: place.min_pitch.unwrap_or(p.aim.min_pitch),
            max_pitch: place.max_pitch.unwrap_or(p.aim.max_pitch),
            unlimited_yaw: place.unlimited_yaw.unwrap_or(p.aim.unlimited_yaw),
        };
        Turret {
            player_usable: place.player_usable.unwrap_or(p.gunner.player_usable),
            exit: place.exit_offset.unwrap_or(p.gunner.exit),
            place,
            kind: kind.to_string(),
            limits,
            yaw: 0.0,
            pitch: 0.0,
            gunner: Gunner::Nobody,
            height: p.gunner.height,
        }
    }

    /// The turret's own facing (its placed yaw), radians.
    pub fn base_yaw(&self) -> f32 {
        to_radians(self.place.rotation[1] as f32)
    }

    /// Where the gun points, radians in the level (Unreal convention: yaw
    /// from +X towards +Y, pitch up positive).
    pub fn view(&self) -> (f32, f32) {
        (
            self.base_yaw() + to_radians(self.yaw),
            to_radians(self.pitch),
        )
    }

    /// Where the gunner's body is held: the centre of its collision, uu.
    pub fn gunner_body(&self) -> [f32; 3] {
        let [x, y, z] = self.place.location;
        [x, y, z + self.height]
    }

    /// Where the gunner gets off, uu: the map's spot, or the offset
    /// turned with the turret.
    pub fn exit_point(&self) -> [f32; 3] {
        if let Some(s) = self.place.exit_spot {
            return s;
        }
        let (s, c) = self.base_yaw().sin_cos();
        let [f, r, u] = self.exit;
        let [x, y, z] = self.place.location;
        [x + f * c - r * s, y + f * s + r * c, z + u]
    }

    /// Can the player get on now?
    pub fn usable_by_player(&self) -> bool {
        self.player_usable && self.gunner == Gunner::Nobody
    }

    /// `who` tries to get on.
    pub fn mount(&mut self, who: Gunner) -> Mounted {
        if who == Gunner::Nobody {
            return Mounted::NotOn;
        }
        if self.gunner != Gunner::Nobody {
            return if self.gunner == who {
                Mounted::On(Vec::new())
            } else {
                Mounted::Taken
            };
        }
        if who == Gunner::Player && !self.player_usable {
            return Mounted::NotUsable;
        }
        let mut events = Vec::new();
        if who == Gunner::Player {
            events.extend(self.place.player_control_event.clone());
        }
        events.extend(self.place.control_event.clone());
        self.gunner = who;
        Mounted::On(events)
    }

    /// The gunner gets off.
    pub fn leave(&mut self) -> Mounted {
        if self.gunner == Gunner::Nobody {
            return Mounted::NotOn;
        }
        self.gunner = Gunner::Nobody;
        Mounted::Off(self.place.release_event.iter().cloned().collect())
    }

    /// Turn the gun by up to `d_yaw` and `d_pitch` rotation units (the
    /// gunner's look this frame), at most `speed * dt` each way. Returns
    /// whether it stopped at a limit.
    pub fn turn(&mut self, d_yaw: f32, d_pitch: f32, dt: f32) -> bool {
        let most = self.limits.speed * dt;
        self.yaw += d_yaw.clamp(-most, most);
        self.pitch += d_pitch.clamp(-most, most);
        let mut stopped = false;
        let l = self.limits;
        if !l.unlimited_yaw {
            if self.yaw > l.max_yaw as f32 {
                self.yaw = l.max_yaw as f32;
                stopped = true;
            } else if self.yaw < l.min_yaw as f32 {
                self.yaw = l.min_yaw as f32;
                stopped = true;
            }
        }
        if self.pitch > l.max_pitch as f32 {
            self.pitch = l.max_pitch as f32;
            stopped = true;
        } else if self.pitch < l.min_pitch as f32 {
            self.pitch = l.min_pitch as f32;
            stopped = true;
        }
        self.yaw = wrap(self.yaw);
        self.pitch = wrap(self.pitch);
        stopped
    }

    /// Turn the gun toward pointing at `target` (uu) from `from` (uu, where
    /// it turns: its yaw bone), at its speed. For a character gunner.
    pub fn turn_toward(&mut self, from: [f32; 3], target: [f32; 3], dt: f32) {
        let d = [
            target[0] - from[0],
            target[1] - from[1],
            target[2] - from[2],
        ];
        let flat = (d[0] * d[0] + d[1] * d[1]).sqrt();
        let want_yaw = to_units(d[1].atan2(d[0]) - self.base_yaw());
        let want_pitch = to_units(d[2].atan2(flat));
        let dy = wrap(want_yaw - self.yaw);
        let dp = want_pitch - self.pitch;
        self.turn(dy, dp, dt);
    }

    /// With nobody on it, the gun swings back to rest at its speed
    /// (`ResetTurret`). Returns whether it is at rest.
    pub fn settle(&mut self, dt: f32) -> bool {
        if self.yaw.abs() < 10.0 && self.pitch.abs() < 10.0 {
            return true;
        }
        let (y, p) = (-self.yaw, -self.pitch);
        self.turn(y, p, dt);
        false
    }
}

/// Into -32768..32768 rotation units.
pub fn wrap(units: f32) -> f32 {
    let w = units.rem_euclid(TURN);
    if w > TURN / 2.0 {
        w - TURN
    } else {
        w
    }
}

/// The first turret tagged `tag`, in the map's order, as the original's
/// scripts find it.
pub fn first_tagged(list: &[Turret], tag: &str) -> Option<usize> {
    list.iter()
        .position(|t| t.place.tag.eq_ignore_ascii_case(tag))
}

#[cfg(test)]
mod tests;
