//! Screen shake: the original game's view shake, step for step.
//!
//! In Republic Commando every shake (a script's `ACTION_ShakeView`, a
//! `ViewShaker` actor, explosions, weapons) ends in one call on the
//! player's controller, `PlayerController.ShakeView(InTime, SustainTime,
//! OutTime, XMag, YMag, ZMag, YawMag, PitchMag, Frequency)`, and every frame
//! the controller's `ViewShake(DeltaTime)` moves the view. Both are
//! UnrealScript in the game's own `Engine.u`; the engine's native
//! `APlayerController::UpdateRotation` (Engine.dll) calls `ViewShake` once
//! a frame, right after the player's own turning and its limits. What this
//! file does, and where each rule comes from:
//!
//! - **A new shake** ([`ShakeState::request`], `ShakeView`): ignored while
//!   a shake is running if its movement or its turn is smaller than the
//!   running one's at this moment (magnitude times the fade); otherwise it
//!   replaces it, starting from the beginning with the noise at zero.
//! - **The fade** (`LerpFactor`): rises from 0 to 1 over `InTime`, holds 1
//!   for `SustainTime`, falls over `OutTime` (as `1 - elapsed / end`, the
//!   original's own formula, which starts the fall below 1 when the shake
//!   had a rise or a hold), then 0.
//! - **The noise**: for each axis a value that walks toward a new random
//!   target in -1..1 every frame, by at most `Frequency` per frame.
//! - **Turning** ([`ShakeStep::pitch`], [`ShakeStep::yaw`]): every frame the
//!   view's pitch and yaw get `magnitude x noise x fade` added, in Unreal
//!   rotation units (65536 a full turn), cut to a whole number toward zero
//!   (the rotation is whole numbers; the engine's float-to-int conversion,
//!   Core.dll `execFloatToInt`, truncates). These turns stay: the shake
//!   moves where the player aims, as in the original. Note the original's
//!   swap, kept here: the `Yaw` value of a shake's rotation turns the view
//!   up and down, and the `Pitch` value turns it left and right.
//! - **Moving** ([`ShakeStep::offset`]): the camera moves by
//!   `magnitude x noise x fade` along the world's X, Y and Z (Unreal units),
//!   on top of the eye position (`Pawn.EyePosition()`).
//! - **Stopping** ([`ShakeState::stop`], `StopShake`): magnitudes go to
//!   zero and the shake ends at once. The original leaves the last camera
//!   movement in place until the next shake; so does this.
//!
//! The original adds the turn every frame, so how far the view wanders
//! depends on the frame rate; this runs once a frame the same way.
//!
//! No game code is copied: this is our own Rust following those rules. The
//! numbers of each shake come from the maps and the game's class defaults
//! at run time.

use bevy::prelude::*;

/// Shake the player's view: the original's `ShakeView` call. Times are
/// seconds; `offset` is how far the camera may move along the world's X, Y
/// and Z (Unreal units); `yaw_mag` and `pitch_mag` are a shake rotation's
/// `Yaw` and `Pitch` values (Unreal rotation units, 65536 a full turn).
/// Sent by the scripts (`ACTION_ShakeView`, `ViewShaker`); weapons and
/// explosions may send it too.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct ShakeView {
    pub in_time: f32,
    pub sustain_time: f32,
    pub out_time: f32,
    pub offset: [f32; 3],
    pub yaw_mag: f32,
    pub pitch_mag: f32,
    pub frequency: f32,
    /// What asked for it, for the log.
    pub by: String,
}

/// Stop any shake at once: the original's `StopShake`.
#[derive(Message, Debug, Clone, Default)]
pub struct StopShake;

/// The player controller's shake values (`ShakeOffsetMag`, `ShakeRotMag`,
/// `ShakeFrequency`, `ShakeTimeElapsed`, `ShakeTimeIn`, `ShakeTimeSustain`,
/// `ShakeTimeOut`, `ShakeRotLastNoise`, `ShakeOffsetLastNoise`).
#[derive(Debug, Clone, PartialEq)]
pub struct ShakeState {
    pub offset_mag: [f32; 3],
    /// `[YawMag, PitchMag, 0]`, as the original stores them.
    pub rot_mag: [f32; 3],
    pub frequency: f32,
    /// Seconds since the shake began; -1 for no shake (the original's
    /// starting value, from `PlayerController`'s defaults).
    pub elapsed: f32,
    /// End of the rise, of the hold and of the fall, seconds from the start.
    pub time_in: f32,
    pub time_sustain: f32,
    pub time_out: f32,
    pub rot_noise: [f32; 3],
    pub offset_noise: [f32; 3],
}

impl Default for ShakeState {
    fn default() -> Self {
        ShakeState {
            offset_mag: [0.0; 3],
            rot_mag: [0.0; 3],
            frequency: 0.0,
            elapsed: -1.0,
            time_in: 0.0,
            time_sustain: 0.0,
            time_out: 0.0,
            rot_noise: [0.0; 3],
            offset_noise: [0.0; 3],
        }
    }
}

/// What one frame of shake does.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ShakeStep {
    /// Added to the view's pitch (up), Unreal rotation units.
    pub pitch: i32,
    /// Added to the view's yaw (right), Unreal rotation units.
    pub yaw: i32,
    /// The camera's movement from the eyes, Unreal units, world X, Y, Z.
    pub offset: [f32; 3],
}

/// UnrealScript's `FClamp(V, A, B)`: `A` if below it, else `V` if below
/// `B`, else `B` (no panic when `A > B`, unlike Rust's `clamp`).
fn fclamp(v: f32, a: f32, b: f32) -> f32 {
    if v < a {
        a
    } else if v < b {
        v
    } else {
        b
    }
}

fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

impl ShakeState {
    /// Running: `elapsed` isn't the "no shake" value.
    pub fn running(&self) -> bool {
        self.elapsed != -1.0
    }

    /// The fade now (`LerpFactor`): 0 before the first shake and after the
    /// fall.
    pub fn fade(&self) -> f32 {
        let t = self.elapsed;
        if t < self.time_in {
            t / self.time_in
        } else if t < self.time_sustain {
            1.0
        } else if t < self.time_out {
            1.0 - t / self.time_out
        } else {
            0.0
        }
    }

    /// A new shake (`ShakeView`). Returns `false` when the running shake is
    /// stronger right now and the new one is ignored.
    pub fn request(&mut self, s: &ShakeView) -> bool {
        if self.running() {
            let fade = self.fade();
            let new_offset = length(s.offset);
            let new_rot = length([s.yaw_mag, s.pitch_mag, 0.0]);
            let now_offset = length(self.offset_mag.map(|m| m * fade));
            let now_rot = length(self.rot_mag.map(|m| m * fade));
            if new_offset < now_offset || new_rot < now_rot {
                return false;
            }
        }
        // `ClientShake`.
        self.offset_mag = s.offset;
        self.rot_mag = [s.yaw_mag, s.pitch_mag, 0.0];
        self.frequency = s.frequency;
        self.elapsed = 0.0;
        self.time_in = s.in_time;
        self.time_sustain = self.time_in + s.sustain_time;
        self.time_out = self.time_sustain + s.out_time;
        self.rot_noise = [0.0; 3];
        self.offset_noise = [0.0; 3];
        true
    }

    /// Stop at once (`StopShake`).
    pub fn stop(&mut self) {
        self.offset_mag = [0.0; 3];
        self.rot_mag = [0.0; 3];
        self.frequency = 1.0;
        self.time_in = 0.0;
        self.time_sustain = 0.0;
        self.time_out = 0.0;
        self.elapsed = -1.0;
    }

    /// One frame (`ViewShake`). `frand` gives a random number in 0..=1, as
    /// the engine's `FRand` does. `None` when no shake is running (the
    /// camera keeps its last movement then).
    pub fn step(&mut self, dt: f32, mut frand: impl FnMut() -> f32) -> Option<ShakeStep> {
        if !self.running() {
            return None;
        }
        let fade = self.fade();
        let f = self.frequency;
        let mut walk = |last: f32| fclamp((frand() * 2.0 - 1.0) - last, -f, f);
        // The original draws all three turning numbers (the third is
        // unused), then the three moving ones.
        let rot_new = [
            walk(self.rot_noise[0]),
            walk(self.rot_noise[1]),
            walk(self.rot_noise[2]),
        ];
        let pitch = (self.rot_mag[0] * (self.rot_noise[0] + rot_new[0]) * fade) as i32;
        let yaw = (self.rot_mag[1] * (self.rot_noise[1] + rot_new[1]) * fade) as i32;
        let off_new = [
            walk(self.offset_noise[0]),
            walk(self.offset_noise[1]),
            walk(self.offset_noise[2]),
        ];
        let offset =
            [0, 1, 2].map(|i| self.offset_mag[i] * (self.offset_noise[i] + off_new[i]) * fade);
        for i in 0..3 {
            self.rot_noise[i] += rot_new[i];
            self.offset_noise[i] += off_new[i];
        }
        self.elapsed += dt;
        Some(ShakeStep { pitch, yaw, offset })
    }
}

/// The player's view shake: the state, this frame's camera movement, and
/// switches for what it may touch. The player's part steps it once a frame
/// after turning the view; the camera adds `offset`.
///
/// A VR mod should switch `turn_view` off (turning someone's head for them
/// makes people sick) and may switch `move_camera` off too; the rest of the
/// game doesn't care.
#[derive(Resource, Debug, Clone)]
pub struct ViewShake {
    pub state: ShakeState,
    /// The camera's movement from the eyes this frame, Unreal units, world
    /// X, Y, Z.
    pub offset: [f32; 3],
    pub turn_view: bool,
    pub move_camera: bool,
    rng: u64,
}

impl Default for ViewShake {
    fn default() -> Self {
        ViewShake {
            state: ShakeState::default(),
            offset: [0.0; 3],
            turn_view: true,
            move_camera: true,
            rng: 0x2545_F491_4F6C_DD1D,
        }
    }
}

impl ViewShake {
    /// A fresh controller (a new level): no shake, camera in place. The
    /// switches stay.
    pub fn reset(&mut self) {
        self.state = ShakeState::default();
        self.offset = [0.0; 3];
    }

    /// The engine's `FRand`: a whole number 0..=32767 over 32767 (MSVC's
    /// `rand()` range; Core.dll `execFRand` multiplies by 1/32767). Our
    /// numbers come from a small xorshift, not the C library's.
    pub fn frand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        let r = (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 49) as u32 & 0x7fff;
        r as f32 / 32767.0
    }

    /// One frame of shake; keeps the camera movement. `None` when nothing
    /// is shaking.
    pub fn step(&mut self, dt: f32) -> Option<ShakeStep> {
        let mut state = std::mem::take(&mut self.state);
        let out = state.step(dt, || self.frand());
        self.state = state;
        if let Some(s) = out {
            self.offset = s.offset;
        }
        out
    }

    /// The camera movement to use now (zero if `move_camera` is off).
    pub fn camera_offset(&self) -> [f32; 3] {
        if self.move_camera {
            self.offset
        } else {
            [0.0; 3]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shake(
        in_t: f32,
        sus: f32,
        out: f32,
        off: [f32; 3],
        yaw: f32,
        pitch: f32,
        f: f32,
    ) -> ShakeView {
        ShakeView {
            in_time: in_t,
            sustain_time: sus,
            out_time: out,
            offset: off,
            yaw_mag: yaw,
            pitch_mag: pitch,
            frequency: f,
            by: "test".into(),
        }
    }

    #[test]
    fn nothing_shakes_before_a_shake() {
        let mut s = ShakeState::default();
        assert!(!s.running());
        assert_eq!(s.step(0.1, || 1.0), None);
    }

    #[test]
    fn fade_rises_holds_and_falls_by_the_original_formula() {
        let mut s = ShakeState::default();
        // ACTION_ShakeView's defaults: in 0.3, hold 1.0, out 0.7.
        s.request(&shake(0.3, 1.0, 0.7, [3.0; 3], 200.0, 200.0, 1.0));
        assert_eq!((s.time_in, s.time_sustain, s.time_out), (0.3, 1.3, 2.0));
        s.elapsed = 0.15;
        assert!((s.fade() - 0.5).abs() < 1e-6);
        s.elapsed = 1.0;
        assert_eq!(s.fade(), 1.0);
        // The fall is 1 - t / end: at 1.3 s it is already 0.35, not 1.
        s.elapsed = 1.3;
        assert!((s.fade() - 0.35).abs() < 1e-6);
        s.elapsed = 1.99;
        assert!(s.fade() > 0.0 && s.fade() < 0.01);
        s.elapsed = 2.0;
        assert_eq!(s.fade(), 0.0);
    }

    #[test]
    fn weaker_shake_is_ignored_while_a_strong_one_runs() {
        let mut s = ShakeState::default();
        assert!(s.request(&shake(0.0, 1.0, 0.0, [5.0; 3], 500.0, 500.0, 1.0)));
        s.elapsed = 0.5;
        assert!(!s.request(&shake(0.0, 1.0, 0.0, [1.0; 3], 100.0, 100.0, 1.0)));
        // Stronger in both: replaces it and starts over.
        assert!(s.request(&shake(0.0, 1.0, 0.0, [6.0; 3], 600.0, 600.0, 1.0)));
        assert_eq!(s.elapsed, 0.0);
        // Once the old one has faded out, anything is taken.
        s.elapsed = 5.0;
        assert!(s.request(&shake(0.0, 1.0, 0.0, [0.0; 3], 0.0, 0.0, 1.0)));
    }

    #[test]
    fn rotation_values_are_swapped_and_cut_to_whole_numbers() {
        let mut s = ShakeState::default();
        // Rotation Yaw 300, Pitch 0: only the pitch (up and down) moves.
        s.request(&shake(0.0, 10.0, 0.0, [0.0; 3], 300.0, 0.0, 1.0));
        // frand 1.0 -> target +1, walked by at most the frequency 1.
        let st = s.step(0.016, || 1.0).unwrap();
        assert_eq!((st.pitch, st.yaw), (300, 0));
        // A product of 0.9 units turns nothing (cut toward zero).
        let mut s = ShakeState::default();
        s.request(&shake(0.0, 10.0, 0.0, [0.0; 3], 0.9, 0.0, 1.0));
        assert_eq!(s.step(0.016, || 1.0).unwrap().pitch, 0);
    }

    #[test]
    fn noise_walks_at_most_frequency_per_frame() {
        let mut s = ShakeState::default();
        s.request(&shake(0.0, 10.0, 0.0, [10.0, 0.0, 0.0], 0.0, 0.0, 0.25));
        let mut xs = Vec::new();
        for _ in 0..8 {
            xs.push(s.step(0.016, || 1.0).unwrap().offset[0]);
        }
        // 0.25 a frame toward +1, then stays at 1.
        assert_eq!(xs, vec![2.5, 5.0, 7.5, 10.0, 10.0, 10.0, 10.0, 10.0]);
    }

    #[test]
    fn stop_ends_it_and_keeps_the_last_camera_move() {
        let mut v = ViewShake::default();
        v.state
            .request(&shake(0.0, 10.0, 0.0, [10.0; 3], 0.0, 0.0, 1.0));
        v.step(0.016).unwrap();
        let last = v.offset;
        assert!(last.iter().any(|x| *x != 0.0));
        v.state.stop();
        assert_eq!(v.step(0.016), None);
        assert_eq!(v.offset, last);
        assert_eq!(v.state.elapsed, -1.0);
    }

    #[test]
    fn frand_covers_zero_to_one() {
        let mut v = ViewShake::default();
        let xs: Vec<f32> = (0..10000).map(|_| v.frand()).collect();
        assert!(xs.iter().all(|x| (0.0..=1.0).contains(x)));
        let mean = xs.iter().sum::<f32>() / xs.len() as f32;
        assert!((mean - 0.5).abs() < 0.02, "{mean}");
    }
}
