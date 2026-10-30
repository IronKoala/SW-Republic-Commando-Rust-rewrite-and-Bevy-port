//! Gyro aiming: turning the controller (or a handheld PC) turns the view.
//!
//! A gyro reports how fast it is turning, in radians per second, around
//! three axes. This file turns those readings into view turn, with the usual
//! clean-ups from games that aim well with gyros:
//!
//! - **Calibration.** Every gyro reads a little turning even when it is
//!   still ("drift"). While the device lies still the reading is averaged
//!   and taken off afterwards.
//! - **Player space.** Turning a controller left and right is usually a mix
//!   of its own yaw and roll, depending on how it is held. Using the
//!   accelerometer, which feels gravity, the turn around the real up
//!   direction is used, so any grip turns the view the same way.
//! - **Smoothing** of very small movements only, so hand shake doesn't
//!   wobble the view while real movements keep no delay.
//!
//! The axes are SDL's, for controllers and handheld PCs alike: X to the
//! right, Y up, Z towards the player. A positive reading turns
//! anticlockwise when looking along the axis towards the device.

use bevy::math::{Vec2, Vec3};
use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

/// How the gyro's turn becomes the view's left and right turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GyroSpace {
    /// Only the device's own up axis turns left and right.
    Local,
    /// Turn around the real up direction, found from gravity (the default;
    /// works the same however the device is held).
    #[default]
    Player,
}

/// When the gyro aims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GyroMode {
    Off,
    /// Always, except while the ratchet button is held.
    #[default]
    Always,
    /// Only while zoomed in or aiming.
    Aiming,
}

/// The `[gyro]` part of the settings file.
#[derive(Resource, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GyroSettings {
    pub mode: GyroMode,
    pub space: GyroSpace,
    /// View turn per real turn: 1 is one to one; 2 turns the view twice as
    /// far as the device.
    pub sensitivity: f32,
    /// Up and down speed compared with left and right.
    pub vertical_scale: f32,
    pub invert_x: bool,
    pub invert_y: bool,
    /// Movements slower than this (degrees per second) are smoothed.
    pub smoothing_threshold: f32,
    /// Movements slower than this (degrees per second) are scaled down
    /// towards nothing, which steadies a shaky hand. 0 is off.
    pub tightening: f32,
    /// Learn the gyro's drift while the device lies still.
    pub auto_calibrate: bool,
}

impl Default for GyroSettings {
    fn default() -> Self {
        GyroSettings {
            mode: GyroMode::Always,
            space: GyroSpace::Player,
            sensitivity: 2.0,
            vertical_scale: 1.0,
            invert_x: false,
            invert_y: false,
            smoothing_threshold: 2.0,
            tightening: 0.0,
            auto_calibrate: true,
        }
    }
}

/// How long the device must lie still before its drift is measured.
const STILL_SECONDS: f32 = 1.5;
/// "Still": turning slower than this, radians per second (about 3 degrees).
const STILL_RATE: f32 = 0.05;
/// "Still": the accelerometer changing less than this, m/s².
const STILL_ACCEL: f32 = 0.2;
/// How fast the up direction follows the accelerometer, seconds.
const GRAVITY_SECONDS: f32 = 0.25;
/// How long the small-movement smoothing averages over, seconds.
const SMOOTH_SECONDS: f32 = 0.1;
/// Player space: how much of a mixed yaw and roll still counts as turning.
const PLAYER_SPACE_RELAX: f32 = 1.41;

/// One device's gyro clean-up, carried from sample to sample.
#[derive(Debug, Clone)]
pub struct GyroFilter {
    /// The drift taken off every reading, radians per second.
    pub bias: Vec3,
    /// Up, in the device's axes (from the accelerometer), length 1.
    pub up: Vec3,
    up_known: bool,
    still_time: f32,
    still_sum: Vec3,
    still_count: u32,
    still_accel: Option<Vec3>,
    smoothed: Vec2,
    /// How many times the drift was measured, for the log.
    pub calibrations: u32,
}

impl Default for GyroFilter {
    fn default() -> Self {
        GyroFilter {
            bias: Vec3::ZERO,
            up: Vec3::Y,
            up_known: false,
            still_time: 0.0,
            still_sum: Vec3::ZERO,
            still_count: 0,
            still_accel: None,
            smoothed: Vec2::ZERO,
            calibrations: 0,
        }
    }
}

impl GyroFilter {
    /// Feed one reading: `rate` in radians per second, `accel` in m/s² if
    /// the device has an accelerometer, `dt` the seconds since the last
    /// reading. Returns the view turn for this reading, radians: `x` turns
    /// right, `y` looks up. Settings' mode is not applied here; the caller
    /// decides whether to use the result, so calibration carries on either
    /// way.
    pub fn sample(&mut self, rate: Vec3, accel: Option<Vec3>, dt: f32, s: &GyroSettings) -> Vec2 {
        let dt = dt.clamp(0.0, 0.1);
        if let Some(a) = accel {
            if a.length() > 1.0 {
                // The first reading is taken whole; later ones are eased
                // in, so shaking doesn't swing the up direction about.
                let k = if self.up_known {
                    (dt / GRAVITY_SECONDS).min(1.0)
                } else {
                    1.0
                };
                self.up_known = true;
                self.up = self.up.lerp(a.normalize(), k).normalize_or(Vec3::Y);
            }
        }
        if s.auto_calibrate {
            self.calibrate(rate, accel, dt);
        }
        let r = rate - self.bias;
        // Turn speeds: yaw right and pitch up, radians per second. A
        // positive Y reading turns the device to the left.
        let yaw = match s.space {
            GyroSpace::Local => -r.y,
            GyroSpace::Player => {
                // Turn around the real up direction, plus a little of the
                // rest so a sideways grip doesn't lose turn.
                let world = -(r.y * self.up.y + r.z * self.up.z);
                let local = (r.y * r.y + r.z * r.z).sqrt();
                world.signum() * (world.abs() * PLAYER_SPACE_RELAX).min(local)
            }
        };
        let mut v = Vec2::new(yaw, r.x);
        // Small movements: blend towards a short average.
        let k = (dt / SMOOTH_SECONDS).min(1.0);
        self.smoothed = self.smoothed.lerp(v, k);
        let t = s.smoothing_threshold.to_radians();
        if t > 0.0 {
            let w = ((v.length() - t * 0.5) / (t * 0.5)).clamp(0.0, 1.0);
            v = v * w + self.smoothed * (1.0 - w);
        }
        let tight = s.tightening.to_radians();
        if tight > 0.0 && v.length() < tight {
            v *= v.length() / tight;
        }
        let sx = if s.invert_x { -1.0 } else { 1.0 };
        let sy = if s.invert_y { -1.0 } else { 1.0 };
        Vec2::new(v.x * sx, v.y * sy * s.vertical_scale) * s.sensitivity * dt
    }

    fn calibrate(&mut self, rate: Vec3, accel: Option<Vec3>, dt: f32) {
        let accel_still = match (accel, self.still_accel) {
            (Some(a), Some(b)) => (a - b).length() < STILL_ACCEL,
            (None, _) => true,
            (Some(_), None) => true,
        };
        let mean = if self.still_count > 0 {
            self.still_sum / self.still_count as f32
        } else {
            rate
        };
        if accel_still && (rate - mean).length() < STILL_RATE && (rate - self.bias).length() < 0.5 {
            if self.still_accel.is_none() {
                self.still_accel = accel;
            }
            self.still_time += dt;
            self.still_sum += rate;
            self.still_count += 1;
            if self.still_time >= STILL_SECONDS {
                self.bias = self.still_sum / self.still_count as f32;
                self.calibrations += 1;
                self.reset_still();
            }
        } else {
            self.reset_still();
        }
    }

    fn reset_still(&mut self) {
        self.still_time = 0.0;
        self.still_sum = Vec3::ZERO;
        self.still_count = 0;
        self.still_accel = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    const G: f32 = 9.81;

    fn plain() -> GyroSettings {
        GyroSettings {
            sensitivity: 1.0,
            smoothing_threshold: 0.0,
            auto_calibrate: false,
            ..GyroSettings::default()
        }
    }

    /// Run `seconds` of a steady reading at 250 readings a second.
    fn run(f: &mut GyroFilter, rate: Vec3, accel: Vec3, seconds: f32, s: &GyroSettings) -> Vec2 {
        let dt = 1.0 / 250.0;
        let n = (seconds / dt).round() as usize;
        (0..n).map(|_| f.sample(rate, Some(accel), dt, s)).sum()
    }

    #[test]
    fn a_quarter_turn_left_turns_the_view_a_quarter_left() {
        // Held flat (gravity along +Y), turned anticlockwise seen from
        // above: a quarter turn in one second.
        let mut f = GyroFilter::default();
        let turn = run(
            &mut f,
            Vec3::new(0.0, FRAC_PI_2, 0.0),
            Vec3::Y * G,
            1.0,
            &plain(),
        );
        assert!((turn.x + FRAC_PI_2).abs() < 0.01, "{turn}");
        assert!(turn.y.abs() < 1e-4);
    }

    #[test]
    fn sensitivity_and_pitch() {
        let s = GyroSettings {
            sensitivity: 2.0,
            vertical_scale: 0.5,
            ..plain()
        };
        let mut f = GyroFilter::default();
        // Tilting the top away from the player looks up.
        let turn = run(&mut f, Vec3::new(0.4, 0.0, 0.0), Vec3::Y * G, 1.0, &s);
        assert!((turn.y - 0.4).abs() < 0.01, "{turn}");
        let inv = GyroSettings {
            invert_y: true,
            ..s
        };
        let turn = run(&mut f, Vec3::new(0.4, 0.0, 0.0), Vec3::Y * G, 1.0, &inv);
        assert!((turn.y + 0.4).abs() < 0.01, "{turn}");
    }

    #[test]
    fn player_space_uses_roll_when_lying_back() {
        // A handheld lying back with its screen facing the ceiling: up is
        // the device's +Z, so turning around the real up direction shows
        // on the device's Z axis (its roll).
        let s = plain();
        let mut f = GyroFilter::default();
        let turn = run(&mut f, Vec3::new(0.0, 0.0, 1.0), Vec3::Z * G, 1.0, &s);
        assert!((turn.x + 1.0).abs() < 0.05, "player space {turn}");
        let local = GyroSettings {
            space: GyroSpace::Local,
            ..s
        };
        let mut f = GyroFilter::default();
        let turn = run(&mut f, Vec3::new(0.0, 0.0, 1.0), Vec3::Z * G, 1.0, &local);
        assert!(turn.x.abs() < 1e-4, "local space ignores roll {turn}");
    }

    #[test]
    fn drift_is_learned_while_still() {
        let s = GyroSettings {
            auto_calibrate: true,
            ..plain()
        };
        let drift = Vec3::new(0.01, -0.02, 0.005);
        let mut f = GyroFilter::default();
        let before = run(&mut f, drift, Vec3::Y * G, 0.5, &s);
        assert!(before.length() > 0.004, "drift shows before calibrating");
        run(&mut f, drift, Vec3::Y * G, 2.0, &s);
        assert_eq!(f.calibrations, 1);
        let after = run(&mut f, drift, Vec3::Y * G, 1.0, &s);
        assert!(after.length() < 1e-4, "{after}");
        // A real turn is not taken for drift.
        let mut g = GyroFilter::default();
        run(&mut g, Vec3::new(0.0, 1.0, 0.0), Vec3::Y * G, 3.0, &s);
        assert_eq!(g.calibrations, 0);
    }

    #[test]
    fn tiny_shake_is_steadied_and_real_moves_are_not() {
        let s = GyroSettings {
            smoothing_threshold: 5.0,
            tightening: 2.0,
            ..plain()
        };
        // A shake of 1 degree a second, switching direction every reading.
        let mut f = GyroFilter::default();
        let mut wobble = 0.0f32;
        for i in 0..250 {
            let r = if i % 2 == 0 { 1.0f32 } else { -1.0 }.to_radians();
            let v = f.sample(Vec3::new(0.0, r, 0.0), Some(Vec3::Y * G), 0.004, &s);
            wobble = wobble.max(v.x.abs());
        }
        let raw = 1.0f32.to_radians() * 0.004;
        assert!(wobble < raw * 0.2, "{wobble} vs {raw}");
        // A fast turn comes through whole.
        let mut f = GyroFilter::default();
        let turn = run(&mut f, Vec3::new(0.0, 2.0, 0.0), Vec3::Y * G, 1.0, &s);
        assert!((turn.x + 2.0).abs() < 0.05, "{turn}");
    }
}
