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
//! - **Automatic turrets** ([`Automatic`]: the original's `AutoTurret`,
//!   such as the core ship's laser turrets). Nobody gets on these. One
//!   that isn't deployed (`bDeployed`) waits, folded away, until a level
//!   event with its tag switches it on; the same event again switches it
//!   off. While on it turns toward the nearest enemy it can see and fires
//!   every `ShotDelay` seconds; after firing for `TimeUntilCooldown`
//!   seconds it stops for `CooldownDuration` seconds to cool down. Its
//!   firing clock is only reset by cooling down, as in the original, so
//!   coming back to a target after a break can start with a cool-down. It
//!   sends `FirstDamagedEvent` the first time it is hurt and its `Event`
//!   when destroyed, after which it does nothing more.
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
    /// An automatic turret's state; `None` for one a gunner works.
    pub automatic: Option<Automatic>,
}

/// An automatic turret's state (`AutoTurret`).
#[derive(Debug, Clone, PartialEq)]
pub struct Automatic {
    /// Switched on (`bDeployed`).
    pub on: bool,
    /// How much damage it can still take; `max_health` 0: it can't be hurt.
    pub health: f32,
    pub max_health: f32,
    /// Hurt at least once (its `FirstDamagedEvent` has gone).
    pub hurt: bool,
    /// Destroyed.
    pub dead: bool,
    /// When it began firing since it last cooled down (`BeginFireTime`).
    pub firing_since: Option<f32>,
    /// When it began cooling down (`BeginCooldownTime`).
    pub cooling_since: Option<f32>,
    /// When it last fired (`LastFireTime`).
    pub last_shot: f32,
    /// How many shots it has fired (picks the next barrel).
    pub shots: usize,
    /// When it has finished unfolding after being switched on and starts
    /// to aim and fire (the original plays `Deploy` to its end first).
    pub ready_at: f32,
}

/// What switching an automatic turret did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switched {
    On,
    Off,
    /// Destroyed, or not an automatic turret: nothing.
    Nothing,
}

/// What a hit on an automatic turret did: the events to send.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hurt {
    pub events: Vec<String>,
    pub destroyed: bool,
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
        let automatic = p.automatic.as_ref().map(|a| {
            let health = place.health.unwrap_or(a.health).max(0.0);
            Automatic {
                on: place.deployed.unwrap_or(a.deployed),
                health,
                max_health: health,
                hurt: false,
                dead: false,
                firing_since: None,
                cooling_since: None,
                last_shot: f32::NEG_INFINITY,
                shots: 0,
                ready_at: f32::NEG_INFINITY,
            }
        });
        Turret {
            automatic,
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

    /// The turret's own axes in the level, from its placed rotation (pitch,
    /// yaw and roll; the core ship's wall turrets are on their sides):
    /// forward, right and up, each a unit vector in Unreal axes. The
    /// original's rotation matrix (`FRotationMatrix`).
    pub fn axes(&self) -> [[f32; 3]; 3] {
        let [p, y, r] = self.place.rotation.map(|u| to_radians(u as f32));
        let (sp, cp) = p.sin_cos();
        let (sy, cy) = y.sin_cos();
        let (sr, cr) = r.sin_cos();
        [
            [cp * cy, cp * sy, sp],
            [sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, -sr * cp],
            [-(cr * sp * cy + sr * sy), cy * sr - cr * sp * sy, cr * cp],
        ]
    }

    /// A point given in the turret's own axes (forward, right, up, uu from
    /// its centre) in the level, uu.
    pub fn to_level(&self, local: [f32; 3]) -> [f32; 3] {
        let [f, r, u] = self.axes();
        let [x, y, z] = self.place.location;
        [
            x + local[0] * f[0] + local[1] * r[0] + local[2] * u[0],
            y + local[0] * f[1] + local[1] * r[1] + local[2] * u[1],
            z + local[0] * f[2] + local[1] * r[2] + local[2] * u[2],
        ]
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

    /// Can the player get on now? Never an automatic turret (the
    /// original's `IsAvailableFor`).
    pub fn usable_by_player(&self) -> bool {
        self.player_usable && self.gunner == Gunner::Nobody && self.automatic.is_none()
    }

    /// An automatic turret that is on and not destroyed.
    pub fn is_working(&self) -> bool {
        self.automatic.as_ref().is_some_and(|a| a.on && !a.dead)
    }

    /// An automatic turret that is on, unfolded and not destroyed at time
    /// `now`: it aims and fires.
    pub fn is_ready(&self, now: f32) -> bool {
        self.automatic
            .as_ref()
            .is_some_and(|a| a.on && !a.dead && now >= a.ready_at)
    }

    /// A level event with the turret's tag (`Trigger`) at time `now`: an
    /// automatic turret that is off switches on, unfolding for
    /// `deploy_secs` before it aims; one that is on switches off (it stops
    /// firing and swings back to rest).
    pub fn trigger(&mut self, now: f32, deploy_secs: f32) -> Switched {
        let Some(a) = self.automatic.as_mut() else {
            return Switched::Nothing;
        };
        if a.dead {
            return Switched::Nothing;
        }
        a.on = !a.on;
        if a.on {
            a.ready_at = now + deploy_secs.max(0.0);
            Switched::On
        } else {
            Switched::Off
        }
    }

    /// An automatic turret, at time `now` (seconds), with or without a
    /// target in its sights: should it fire now? `shot_delay` is seconds
    /// between shots, `fire_for` how long it fires before cooling down
    /// (0: never) and `cool_down` how long that takes. The original's
    /// `AutoTurret` rules (`Turret.Fire`, `DetectTarget`).
    pub fn auto_fire(
        &mut self,
        now: f32,
        has_target: bool,
        shot_delay: f32,
        fire_for: f32,
        cool_down: f32,
    ) -> bool {
        let Some(a) = self.automatic.as_mut() else {
            return false;
        };
        if !has_target || !a.on || a.dead || now < a.ready_at {
            return false;
        }
        // Firing for the first time since it last cooled down.
        if a.firing_since.is_none() && a.cooling_since.is_none() {
            a.firing_since = Some(now);
        }
        if now - a.last_shot <= shot_delay {
            return false;
        }
        if let Some(c) = a.cooling_since {
            if now >= c + cool_down {
                a.cooling_since = None;
            }
            return false;
        }
        if fire_for > 0.0 && a.firing_since.is_some_and(|f| now >= f + fire_for) {
            a.firing_since = None;
            a.cooling_since = Some(now);
            return false;
        }
        a.last_shot = now;
        a.shots += 1;
        true
    }

    /// Whether an automatic turret is cooling down now.
    pub fn cooling(&self) -> bool {
        self.automatic
            .as_ref()
            .is_some_and(|a| a.cooling_since.is_some())
    }

    /// An automatic turret takes `amount` damage (after its damage table).
    /// Returns the events to send: `FirstDamagedEvent` the first time,
    /// its `Event` when destroyed.
    pub fn hurt(&mut self, amount: f32) -> Hurt {
        let mut out = Hurt::default();
        let first = self.place.first_damaged_event.clone();
        let event = self.place.event.clone();
        let Some(a) = self.automatic.as_mut() else {
            return out;
        };
        if a.dead || a.max_health <= 0.0 || amount <= 0.0 {
            return out;
        }
        if !a.hurt {
            a.hurt = true;
            out.events.extend(first);
        }
        a.health -= amount;
        if a.health <= 0.0 {
            a.health = 0.0;
            a.dead = true;
            a.on = false;
            a.firing_since = None;
            a.cooling_since = None;
            out.destroyed = true;
            out.events.extend(event);
        }
        out
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
        // Nobody gets on an automatic turret (`IsAvailableFor`).
        if self.automatic.is_some() {
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
        self.turn_toward_by(from, target, dt, (0.0, 0.0));
    }

    /// [`Turret::turn_toward`] for a gun whose barrels don't point straight
    /// along its resting direction: `rest` is where they point at rest
    /// (yaw, pitch in rotation units, in the turret's own axes), so the
    /// barrels end up on the target.
    pub fn turn_toward_by(&mut self, from: [f32; 3], target: [f32; 3], dt: f32, rest: (f32, f32)) {
        let d = [
            target[0] - from[0],
            target[1] - from[1],
            target[2] - from[2],
        ];
        // In the turret's own axes, so a turret on its side aims the same
        // way (the original works in its yaw bone's space).
        let [f, r, u] = self.axes();
        let dot = |a: [f32; 3]| a[0] * d[0] + a[1] * d[1] + a[2] * d[2];
        let local = [dot(f), dot(r), dot(u)];
        let flat = (local[0] * local[0] + local[1] * local[1]).sqrt();
        let want_yaw = to_units(local[1].atan2(local[0])) - rest.0;
        let want_pitch = to_units(local[2].atan2(flat)) - rest.1;
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
