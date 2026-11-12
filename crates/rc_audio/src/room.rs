//! A room's echo (reverb), driven by the EAX 3.0 listener values the game
//! sends to the sound card for the room the player is in
//! (`rc_level::zone_audio`).
//!
//! **What is the game's and what is ours.** The values (how loud the echo
//! is, how long it lasts, how quickly the high and low notes die away, how
//! late the first reflections and the tail come) are the game's own, read
//! from its files and handed over exactly as its `ALAudio.dll` hands them
//! to EAX. The sound itself was never made by the game: EAX is processed
//! by the sound card's driver (Creative's hardware, or a program like
//! OpenAL Soft standing in for it), and how it turns the values into sound
//! isn't published. This processor is our own: it follows the meaning of
//! each value as the EAX / I3DL2 documents give it (levels in millibels,
//! the decay time is the time to fall by 60 dB, `DecayHFRatio` scales it
//! at `HFReference`, and so on), built from standard parts (delay lines,
//! one-pole filters, a feedback delay network). It will not sound exactly
//! like any one sound card.
//!
//! Not done: the echo effect (`EchoTime`/`EchoDepth`), the modulation
//! (every room has `ModulationDepth` 0), panning of the reflections and
//! tail (every room has 0, 0, 0), and the room's air absorption
//! (`AirAbsorptionHF`, which in EAX dulls far sounds), except as the limit
//! `bDecayHFLimit` puts on the high-note decay.
//!
//! How sounds reach it: every sound playing adds its samples to one shared
//! "send" ([`RoomBus::send`]), and one endless sound takes the send
//! ([`RoomBus::take`]) and plays its echo through [`RoomReverb`] (that
//! sound is in the game program, `rc_game`'s `room_sound.rs`). Plain Rust,
//! no Bevy.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// EAX 3.0 listener values (`EAXLISTENERPROPERTIES`). Levels in millibels
/// (-10000 is silent), times in seconds, frequencies in hertz.
#[derive(Debug, Clone, PartialEq)]
pub struct EaxReverb {
    pub environment_size: f32,
    pub diffusion: f32,
    pub room: i32,
    pub room_hf: i32,
    pub room_lf: i32,
    pub decay_time: f32,
    pub decay_hf_ratio: f32,
    pub decay_lf_ratio: f32,
    pub reflections: i32,
    pub reflections_delay: f32,
    pub reverb: i32,
    pub reverb_delay: f32,
    pub air_absorption_hf: f32,
    pub hf_reference: f32,
    pub lf_reference: f32,
    pub room_rolloff_factor: f32,
    pub flags: u32,
}

/// `bDecayHFLimit` in the flags.
pub const DECAY_HF_LIMIT: u32 = 0x20;

/// Millibels to a gain (amplitude factor).
pub fn mb_to_gain(mb: f32) -> f32 {
    if mb <= -10000.0 {
        0.0
    } else {
        10f32.powf(mb / 2000.0)
    }
}

impl EaxReverb {
    /// Whether any of the echo is heard (EAX's `Room` above -10000 mB).
    pub fn is_heard(&self) -> bool {
        mb_to_gain(self.room as f32) > 0.0
            && (mb_to_gain(self.reflections as f32) > 0.0 || mb_to_gain(self.reverb as f32) > 0.0)
    }
}

/// The speed of sound EAX's air absorption is defined against, metres per
/// second (EFX: `AL_SPEEDOFSOUND`).
const SPEED_OF_SOUND: f32 = 343.3;

/// One-pole low-pass: y += a * (x - y).
#[derive(Debug, Clone, Default)]
struct OnePole {
    a: f32,
    y: f32,
}

impl OnePole {
    fn coefficient(cutoff: f32, rate: f32) -> f32 {
        let c = (cutoff / rate).clamp(1e-5, 0.49);
        1.0 - (-2.0 * std::f32::consts::PI * c).exp()
    }
    fn run(&mut self, x: f32) -> f32 {
        self.y += self.a * (x - self.y);
        self.y
    }
}

/// A gain at low, middle and high frequencies: low part (below
/// `lf_reference`) times `lf`, high part (above `hf_reference`) times
/// `hf`, the rest unchanged.
#[derive(Debug, Clone, Default)]
struct Shelves {
    low: OnePole,
    high: OnePole,
    lf: f32,
    hf: f32,
}

impl Shelves {
    fn set(&mut self, lf_ref: f32, lf: f32, hf_ref: f32, hf: f32, rate: f32) {
        self.low.a = OnePole::coefficient(lf_ref, rate);
        self.high.a = OnePole::coefficient(hf_ref, rate);
        self.lf = lf;
        self.hf = hf;
    }
    fn run(&mut self, x: f32) -> f32 {
        let low = self.low.run(x);
        let below_hf = self.high.run(x);
        let mid = below_hf - low;
        let high = x - below_hf;
        low * self.lf + mid + high * self.hf
    }
}

/// A delay line with a fixed longest length.
#[derive(Debug, Clone)]
struct Delay {
    buf: Vec<f32>,
    pos: usize,
}

impl Delay {
    fn new(len: usize) -> Self {
        Delay {
            buf: vec![0.0; len.max(1)],
            pos: 0,
        }
    }
    fn push(&mut self, x: f32) {
        self.pos = (self.pos + 1) % self.buf.len();
        self.buf[self.pos] = x;
    }
    /// The sample pushed `d` samples ago (`d` < length).
    fn tap(&self, d: usize) -> f32 {
        let n = self.buf.len();
        self.buf[(self.pos + n - d.min(n - 1)) % n]
    }
}

/// Schroeder all-pass, for diffusion.
#[derive(Debug, Clone)]
struct AllPass {
    line: Delay,
    len: usize,
    g: f32,
}

impl AllPass {
    fn new(len: usize) -> Self {
        AllPass {
            line: Delay::new(len + 1),
            len,
            g: 0.0,
        }
    }
    fn run(&mut self, x: f32) -> f32 {
        let delayed = self.line.tap(self.len);
        let v = x + self.g * delayed;
        self.line.push(v);
        delayed - self.g * v
    }
}

/// Lengths of the four tail delay lines at 44100 Hz for EAX's "generic"
/// room size of 7.5 m (mutually prime, our choice), scaled with
/// `EnvironmentSize`.
const LINE_LENGTHS: [f32; 4] = [1031.0, 1327.0, 1523.0, 1871.0];
const GENERIC_SIZE: f32 = 7.5;
/// Longest pre-delay: EAX allows 0.3 s of reflections delay and 0.1 s of
/// reverb delay.
const MAX_PRE_DELAY: f32 = 0.4;

/// The echo processor: one input (the send), two outputs.
#[derive(Debug, Clone)]
pub struct RoomReverb {
    rate: f32,
    params: Option<EaxReverb>,
    input_gain: f32,
    reflections_gain: f32,
    late_gain: f32,
    shelves: Shelves,
    pre: Delay,
    reflections_delay: usize,
    late_delay: usize,
    diffusers: [AllPass; 2],
    lines: [Delay; 4],
    lengths: [usize; 4],
    feedback: [f32; 4],
    damping: [Shelves; 4],
}

impl RoomReverb {
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate.max(8000) as f32;
        let scale = rate / 44100.0;
        let longest_size = 100.0 / GENERIC_SIZE; // EAX's largest room, 100 m
        RoomReverb {
            rate,
            params: None,
            input_gain: 0.0,
            reflections_gain: 0.0,
            late_gain: 0.0,
            shelves: Shelves::default(),
            pre: Delay::new((MAX_PRE_DELAY * rate) as usize + 2),
            reflections_delay: 0,
            late_delay: 0,
            diffusers: [
                AllPass::new((142.0 * scale) as usize),
                AllPass::new((379.0 * scale) as usize),
            ],
            lines: LINE_LENGTHS.map(|l| Delay::new((l * scale * longest_size) as usize + 2)),
            lengths: [1; 4],
            feedback: [0.0; 4],
            damping: Default::default(),
        }
    }

    /// The values now in use, `None` when off.
    pub fn params(&self) -> Option<&EaxReverb> {
        self.params.as_ref()
    }

    /// Switch to new values (`None`: no echo). The tail already ringing
    /// keeps going in the new room's lines.
    pub fn set(&mut self, p: Option<&EaxReverb>) {
        self.params = p.cloned();
        let Some(p) = p else {
            self.input_gain = 0.0;
            return;
        };
        let rate = self.rate;
        self.input_gain = mb_to_gain(p.room as f32);
        self.reflections_gain = mb_to_gain(p.reflections as f32);
        self.late_gain = mb_to_gain(p.reverb as f32);
        self.shelves.set(
            p.lf_reference,
            mb_to_gain(p.room_lf as f32),
            p.hf_reference,
            mb_to_gain(p.room_hf as f32),
            rate,
        );
        let max = (MAX_PRE_DELAY * rate) as usize;
        self.reflections_delay = ((p.reflections_delay.max(0.0) * rate) as usize).min(max);
        self.late_delay =
            (((p.reflections_delay + p.reverb_delay).max(0.0) * rate) as usize).min(max);
        let g = 0.6 * p.diffusion.clamp(0.0, 1.0);
        for d in &mut self.diffusers {
            d.g = g;
        }
        // Tail: each line loses 60 dB over the decay time, the high notes
        // over decay time x DecayHFRatio, the low over x DecayLFRatio.
        let size = (p.environment_size / GENERIC_SIZE).clamp(0.01, 100.0 / GENERIC_SIZE);
        let decay = p.decay_time.max(0.01);
        let mut hf_ratio = p.decay_hf_ratio.max(0.01);
        if p.flags & DECAY_HF_LIMIT != 0 && p.air_absorption_hf < 0.0 {
            // EFX: with the limit on, the high notes can't outlast the
            // time air absorption takes to lose 60 dB.
            let per_metre = p.air_absorption_hf / 2000.0; // decibels/20 per metre
            let limit = -3.0 / (per_metre * SPEED_OF_SOUND) / decay;
            hf_ratio = hf_ratio.min(limit);
        }
        let lf_ratio = p.decay_lf_ratio.max(0.01);
        #[allow(clippy::needless_range_loop)] // four arrays in step
        for i in 0..4 {
            let len = ((LINE_LENGTHS[i] * rate / 44100.0 * size) as usize)
                .clamp(2, self.lines[i].buf.len() - 1);
            self.lengths[i] = len;
            let t = len as f32 / rate;
            let gain_for = |t60: f32| 10f32.powf(-3.0 * t / t60);
            let mid = gain_for(decay);
            self.feedback[i] = mid;
            self.damping[i].set(
                p.lf_reference,
                gain_for(decay * lf_ratio) / mid,
                p.hf_reference,
                gain_for(decay * hf_ratio) / mid,
                rate,
            );
        }
    }

    /// One sample of send in, the echo for left and right out.
    pub fn run(&mut self, x: f32) -> [f32; 2] {
        let input = self.shelves.run(x * self.input_gain);
        self.pre.push(input);
        let early = self.pre.tap(self.reflections_delay);
        let late_in = self.pre.tap(self.late_delay);
        // Early reflections: the delayed send, smeared by the diffusers.
        let mut e = early;
        for d in &mut self.diffusers {
            e = d.run(e);
        }
        let reflections = e * self.reflections_gain;
        // Tail: a four-line feedback network (Householder mixing).
        let outs: [f32; 4] = std::array::from_fn(|i| self.lines[i].tap(self.lengths[i]));
        let sum: f32 = outs.iter().sum::<f32>() * 0.5;
        let late_input = late_in * self.late_gain * 0.5;
        #[allow(clippy::needless_range_loop)] // four arrays in step
        for i in 0..4 {
            let fed = (outs[i] - sum) * self.feedback[i];
            let damped = self.damping[i].run(fed);
            self.lines[i].push(damped + late_input);
        }
        let left = outs[0] - outs[2];
        let right = outs[1] - outs[3];
        [reflections + left * 0.5, reflections + right * 0.5]
    }
}

/// The shared send and the echo's values, between every playing sound
/// (writers) and the one sound that plays the echo (reader).
#[derive(Debug, Default)]
pub struct RoomBus {
    /// The send for the current output sample, an `f32` as bits, summed
    /// by every sound as it plays.
    send: AtomicU32,
    /// New values for the echo, picked up by `render`.
    pending: Mutex<Option<Option<EaxReverb>>>,
    version: AtomicU64,
}

impl RoomBus {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Add one sample to the send.
    pub fn send(&self, x: f32) {
        if x == 0.0 {
            return;
        }
        let mut old = self.send.load(Ordering::Relaxed);
        loop {
            let new = (f32::from_bits(old) + x).to_bits();
            match self
                .send
                .compare_exchange_weak(old, new, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return,
                Err(o) => old = o,
            }
        }
    }

    /// Take the send summed so far, leaving 0.
    pub fn take(&self) -> f32 {
        f32::from_bits(self.send.swap(0, Ordering::Relaxed))
    }

    /// Set the echo's values (`None`: no echo).
    pub fn set(&self, p: Option<EaxReverb>) {
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
        self.version.fetch_add(1, Ordering::Release);
    }

    /// Changes since `seen`, and the new version.
    pub fn changed(&self, seen: u64) -> Option<(u64, Option<EaxReverb>)> {
        let v = self.version.load(Ordering::Acquire);
        if v == seen {
            return None;
        }
        let p = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .flatten();
        Some((v, p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_room() -> EaxReverb {
        // geo_03d's EFFECT_CASTLE_SMALLROOM0, as read from the game's files.
        EaxReverb {
            environment_size: 8.3,
            diffusion: 0.89,
            room: -1100,
            room_hf: -800,
            room_lf: -2000,
            decay_time: 1.22,
            decay_hf_ratio: 0.83,
            decay_lf_ratio: 0.31,
            reflections: -100,
            reflections_delay: 0.022,
            reverb: -750,
            reverb_delay: 0.011,
            air_absorption_hf: -5.0,
            hf_reference: 5168.6,
            lf_reference: 139.5,
            room_rolloff_factor: 0.0,
            flags: 127,
        }
    }

    /// Energy in each 10 ms block of the echo of one click.
    fn blocks(r: &mut RoomReverb, n: usize) -> Vec<f32> {
        let mut out = Vec::new();
        let mut e = 0.0;
        for i in 0..n {
            let [l, rr] = r.run(if i == 0 { 1.0 } else { 0.0 });
            e += l * l + rr * rr;
            if (i + 1) % 441 == 0 {
                out.push(e);
                e = 0.0;
            }
        }
        out
    }

    #[test]
    fn room_at_minus_10000_is_silent() {
        let mut p = small_room();
        p.room = -10000;
        assert!(!p.is_heard());
        let mut r = RoomReverb::new(44100);
        r.set(Some(&p));
        assert!(blocks(&mut r, 44100).iter().all(|&e| e == 0.0));
    }

    #[test]
    fn nothing_before_the_reflections_delay() {
        let mut r = RoomReverb::new(44100);
        r.set(Some(&small_room()));
        let mut first = None;
        for i in 0..4410 {
            let [l, rr] = r.run(if i == 0 { 1.0 } else { 0.0 });
            if first.is_none() && (l != 0.0 || rr != 0.0) {
                first = Some(i);
            }
        }
        // 0.022 s at 44100 Hz = 970 samples.
        assert_eq!(first, Some(970));
    }

    #[test]
    fn tail_falls_about_60_db_over_the_decay_time() {
        let mut r = RoomReverb::new(44100);
        let mut p = small_room();
        // Flat decay at every pitch, to measure the decay time alone.
        p.decay_hf_ratio = 1.0;
        p.decay_lf_ratio = 1.0;
        p.reflections = -10000;
        r.set(Some(&p));
        let b = blocks(&mut r, 44100 * 3);
        // Compare 0.2 s to 0.2 s + decay time: 60 dB of amplitude is 1e-6
        // of energy. Allow a factor of 10 either way (10 dB).
        let at = |t: f32| b[(t * 100.0) as usize];
        let ratio = at(0.2 + p.decay_time) / at(0.2);
        assert!(ratio > 1e-7 && ratio < 1e-5, "ratio {ratio:e}");
    }

    #[test]
    fn bus_sums_and_clears() {
        let bus = RoomBus::new();
        bus.send(0.25);
        bus.send(0.5);
        assert_eq!(bus.take(), 0.75);
        assert_eq!(bus.take(), 0.0);
        assert!(bus.changed(0).is_none());
        bus.set(Some(small_room()));
        let (v, p) = bus.changed(0).unwrap();
        assert_eq!(v, 1);
        assert_eq!(p, Some(small_room()));
        assert!(bus.changed(v).is_none());
    }
}
