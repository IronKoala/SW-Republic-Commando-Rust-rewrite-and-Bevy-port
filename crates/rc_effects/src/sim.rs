//! Running an effect: sending out particles, moving them and working out
//! each one's size, colour, spin and picture frame. Plain Rust, Unreal
//! units, so it can be tested without a renderer.
//!
//! These are our reading of how the game's emitters behave (from the
//! property names and the effects' values), not the engine's own code:
//!
//! - an emitter starts after its `initial_delay`;
//! - with an `initial_rate`, its first `max_particles` come out at that
//!   rate (an explosion's rate is huge, so they come out at once);
//!   afterwards, if it `respawn`s, a new particle replaces each dead one at
//!   `rate` a second (or `max_particles / lifetime.max` with `auto_rate`);
//! - an emitter that doesn't respawn sends `max_particles` in all, then is
//!   done once they have died;
//! - a particle's size and colour follow its `size_scale` and
//!   `colour_scale` over its life, linearly between the points.

use crate::def::{EffectDef, EmitterDef, Range, RangeVector, StartShape, VelocityFrom};

/// A small random number maker (xorshift), so effects look the same in
/// every test run.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }
    /// 0 to 1.
    pub fn unit(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn range(&mut self, r: Range) -> f32 {
        r.at(self.unit())
    }
    pub fn range3(&mut self, r: &RangeVector) -> [f32; 3] {
        [self.range(r.x), self.range(r.y), self.range(r.z)]
    }
}

/// One live particle.
#[derive(Debug, Clone, PartialEq)]
pub struct Particle {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub age: f32,
    pub lifetime: f32,
    /// Its start size (x, y), before the size scale.
    pub size: [f32; 2],
    pub colour_multiplier: [f32; 3],
    /// Turns.
    pub spin: f32,
    pub spin_rate: f32,
    pub velocity_loss: [f32; 3],
    /// Its fixed frame, for random-frame emitters.
    pub frame: u32,
    /// How much speed it keeps on a bounce.
    pub damping: f32,
}

/// What to draw for one particle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sprite {
    pub position: [f32; 3],
    /// Half width and half height, uu.
    pub half_size: [f32; 2],
    /// RGBA 0-1 from the colour scale and multiplier (no fades).
    pub colour: [f32; 4],
    /// The fades and opacity, 0-1: multiplies alpha when blending, and
    /// brightness when adding light.
    pub fade: f32,
    /// Turns.
    pub spin: f32,
    pub frame: u32,
    /// Its flight, for sparks.
    pub velocity: [f32; 3],
}

/// One emitter running.
#[derive(Debug, Clone)]
pub struct EmitterState {
    pub particles: Vec<Particle>,
    /// Particles sent so far.
    pub sent: u32,
    /// Seconds until it starts.
    pub delay: f32,
    /// Part of a particle owed from earlier steps.
    pub owed: f32,
}

/// An effect running at `origin` (uu).
#[derive(Debug, Clone)]
pub struct EffectState {
    pub origin: [f32; 3],
    /// The height of the ground under it, for particles that bounce.
    pub floor: Option<f32>,
    pub age: f32,
    pub emitters: Vec<EmitterState>,
    rng: Rng,
}

fn lerp_table<T: Copy>(table: &[(f32, T)], t: f32, mix: impl Fn(T, T, f32) -> T) -> Option<T> {
    let first = table.first()?;
    if t <= first.0 {
        return Some(first.1);
    }
    for w in table.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b.0 {
            let span = (b.0 - a.0).max(1e-6);
            return Some(mix(a.1, b.1, (t - a.0) / span));
        }
    }
    table.last().map(|l| l.1)
}

/// The size scale at relative time `t`.
pub fn size_at(def: &EmitterDef, t: f32) -> f32 {
    lerp_table(&def.size_scale, t, |a, b, k| a + (b - a) * k).unwrap_or(1.0)
}

/// The colour at relative time `t`, before the multiplier and fades.
pub fn colour_at(def: &EmitterDef, t: f32) -> [f32; 4] {
    lerp_table(&def.colour_scale, t, |a, b, k| {
        [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * k)
    })
    .unwrap_or([1.0; 4])
}

impl EffectState {
    pub fn new(def: &EffectDef, origin: [f32; 3], seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let emitters = def
            .emitters
            .iter()
            .map(|e| EmitterState {
                particles: Vec::new(),
                sent: 0,
                delay: rng.range(e.initial_delay),
                owed: 0.0,
            })
            .collect();
        EffectState {
            origin,
            floor: None,
            age: 0.0,
            emitters,
            rng,
        }
    }

    /// Particles alive now, in all emitters.
    pub fn live(&self) -> usize {
        self.emitters.iter().map(|e| e.particles.len()).sum()
    }

    /// Whether it has nothing more to show: every emitter that ends has
    /// ended, and none loops; or its life span is over.
    pub fn done(&self, def: &EffectDef) -> bool {
        if def.life_span > 0.0 && self.age >= def.life_span {
            return true;
        }
        def.emitters.iter().zip(&self.emitters).all(|(d, s)| {
            d.disabled || (!d.respawn && s.sent >= d.max_particles && s.particles.is_empty())
        })
    }

    fn spawn(&mut self, def: &EmitterDef, i: usize) {
        let rng = &mut self.rng;
        let mut offset = match def.start_shape {
            StartShape::Box => rng.range3(&def.start_range),
            StartShape::Sphere | StartShape::Polar => {
                // A random direction, at a random distance.
                let z = rng.unit() * 2.0 - 1.0;
                let a = rng.unit() * std::f32::consts::TAU;
                let r = (1.0 - z * z).max(0.0).sqrt();
                let d = rng.range(def.sphere_radius);
                [r * a.cos() * d, r * a.sin() * d, z * d]
            }
        };
        for (o, s) in offset.iter_mut().zip(def.start_offset) {
            *o += s;
        }
        let mut velocity = rng.range3(&def.start_velocity);
        let len = (offset[0].powi(2) + offset[1].powi(2) + offset[2].powi(2)).sqrt();
        let away = if len > 1e-4 {
            offset.map(|o| o / len)
        } else {
            [0.0, 0.0, 1.0]
        };
        match def.velocity_from {
            VelocityFrom::None => {}
            VelocityFrom::StartPositionAndOwner => {
                velocity = [0, 1, 2].map(|i| away[i] * velocity[i].abs());
            }
            VelocityFrom::OwnerAndStartPosition => {
                velocity = [0, 1, 2].map(|i| -away[i] * velocity[i].abs());
            }
            VelocityFrom::AddRadial => {
                let r = rng.range(def.radial_velocity);
                for k in 0..3 {
                    velocity[k] += away[k] * r;
                }
            }
        }
        let sx = rng.unit();
        let size = if def.uniform_size {
            let s = def.start_size.x.at(sx);
            [s, s]
        } else {
            [def.start_size.x.at(sx), def.start_size.y.at(rng.unit())]
        };
        let frames = def.frames();
        let frame = if def.random_frame && frames > 1 {
            let lo = def.frame_start.min(frames - 1);
            let hi = if def.frame_end == 0 {
                frames
            } else {
                def.frame_end.min(frames)
            };
            lo + ((rng.unit() * (hi.saturating_sub(lo)).max(1) as f32) as u32)
                .min(hi.saturating_sub(lo + 1))
        } else {
            0
        };
        let p = Particle {
            position: [0, 1, 2].map(|k| self.origin[k] + offset[k]),
            velocity,
            age: 0.0,
            lifetime: rng.range(def.lifetime).max(0.01),
            size,
            colour_multiplier: rng.range3(&def.colour_multiplier),
            spin: if def.spin {
                rng.range(def.start_spin)
            } else {
                0.0
            },
            spin_rate: if def.spin {
                rng.range(def.spins_per_second)
            } else {
                0.0
            },
            velocity_loss: rng.range3(&def.velocity_loss),
            frame,
            damping: rng.range(def.damping),
        };
        self.emitters[i].particles.push(p);
        self.emitters[i].sent += 1;
    }

    /// Move everything on by `dt` seconds.
    pub fn step(&mut self, def: &EffectDef, dt: f32) {
        self.age += dt;
        for (i, d) in def.emitters.iter().enumerate() {
            if d.disabled
                || !matches!(
                    d.kind,
                    crate::def::EmitterKind::Sprite
                        | crate::def::EmitterKind::Spark
                        | crate::def::EmitterKind::Mesh(_)
                )
            {
                continue;
            }
            // Age and move the live ones.
            let s = &mut self.emitters[i];
            for p in &mut s.particles {
                p.age += dt;
                for k in 0..3 {
                    p.velocity[k] += d.acceleration[k] * dt;
                    p.velocity[k] *= (1.0 - p.velocity_loss[k] * dt).max(0.0);
                    p.position[k] += p.velocity[k] * dt;
                }
                p.spin += p.spin_rate * dt;
                if let Some(f) = self.floor.filter(|_| d.collides) {
                    if p.position[2] < f && p.velocity[2] < 0.0 {
                        p.position[2] = f;
                        p.velocity = p.velocity.map(|v| v * p.damping);
                        p.velocity[2] = -p.velocity[2];
                        // Slow ones settle and stop spinning.
                        if p.velocity[2] < 20.0 {
                            p.velocity = [0.0; 3];
                            p.spin_rate = 0.0;
                        }
                    }
                }
            }
            s.particles.retain(|p| p.age < p.lifetime);
            // Send new ones.
            let mut dt_left = dt;
            if s.delay > 0.0 {
                s.delay -= dt;
                if s.delay > 0.0 {
                    continue;
                }
                dt_left = -s.delay;
            }
            let max = d.max_particles;
            let rate = if d.initial_rate > 0.0 && s.sent < max {
                d.initial_rate
            } else if d.rate > 0.0 {
                d.rate
            } else if d.auto_rate {
                max as f32 / d.lifetime.max.max(0.01)
            } else {
                0.0
            };
            // The very first step of a burst sends at least one.
            s.owed += rate * dt_left.max(if s.sent == 0 { 1e-3 } else { 0.0 });
            if s.sent == 0 && rate > 0.0 {
                s.owed = s.owed.max(1.0);
            }
            let mut n = s.owed.floor() as u32;
            s.owed -= n as f32;
            // Room left: no more than max alive, and without respawning no
            // more than max in all.
            let alive = s.particles.len() as u32;
            let room = if d.respawn {
                max.saturating_sub(alive)
            } else {
                max.saturating_sub(s.sent)
            };
            if n > room {
                n = room;
                s.owed = 0.0;
            }
            for _ in 0..n {
                self.spawn(d, i);
            }
        }
    }

    /// What to draw for emitter `i` now.
    pub fn sprites(&self, def: &EffectDef, i: usize) -> Vec<Sprite> {
        let (Some(d), Some(s)) = (def.emitters.get(i), self.emitters.get(i)) else {
            return Vec::new();
        };
        let frames = d.frames();
        s.particles
            .iter()
            .map(|p| {
                let t = (p.age / p.lifetime).clamp(0.0, 1.0);
                let scale = size_at(d, t);
                let mut c = colour_at(d, t);
                for (ck, m) in c.iter_mut().zip(p.colour_multiplier) {
                    *ck *= m;
                }
                let mut fade = d.opacity;
                if let Some(start) = d.fade_out {
                    if p.age > start && p.lifetime > start {
                        fade *= 1.0 - (p.age - start) / (p.lifetime - start);
                    }
                }
                if let Some(end) = d.fade_in {
                    if end > 0.0 && p.age < end {
                        fade *= p.age / end;
                    }
                }
                let frame = if frames <= 1 {
                    0
                } else if d.random_frame {
                    p.frame
                } else {
                    // Runs through its frames over its life.
                    let lo = d.frame_start.min(frames - 1);
                    let hi = if d.frame_end == 0 {
                        frames
                    } else {
                        d.frame_end.min(frames)
                    };
                    let span = hi.saturating_sub(lo).max(1);
                    lo + ((t * span as f32) as u32).min(span - 1)
                };
                Sprite {
                    position: p.position,
                    half_size: [p.size[0] * scale, p.size[1] * scale],
                    colour: c,
                    fade: fade.clamp(0.0, 1.0),
                    spin: p.spin,
                    frame,
                    velocity: p.velocity,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::def::{EffectDef, EmitterDef};

    fn burst() -> EffectDef {
        EffectDef {
            name: "Test".into(),
            emitters: vec![EmitterDef {
                max_particles: 8,
                respawn: false,
                auto_rate: false,
                initial_rate: 5000.0,
                lifetime: Range::new(1.0, 1.5),
                start_velocity: RangeVector {
                    x: Range::new(-100.0, 100.0),
                    y: Range::new(-100.0, 100.0),
                    z: Range::new(0.0, 300.0),
                },
                size_scale: vec![(0.0, 0.5), (1.0, 2.0)],
                colour_scale: vec![(0.0, [1.0, 0.8, 0.4, 1.0]), (1.0, [0.2, 0.2, 0.2, 1.0])],
                fade_out: Some(0.5),
                ..EmitterDef::default()
            }],
            sound: None,
            life_span: 0.0,
        }
    }

    #[test]
    fn a_burst_comes_out_at_once_and_ends() {
        let def = burst();
        let mut s = EffectState::new(&def, [0.0, 0.0, 100.0], 7);
        s.step(&def, 1.0 / 60.0);
        assert_eq!(s.live(), 8, "an explosion's particles all come out at once");
        s.step(&def, 0.5);
        assert_eq!(s.emitters[0].sent, 8, "no more than max without respawn");
        for _ in 0..200 {
            s.step(&def, 1.0 / 60.0);
        }
        assert_eq!(s.live(), 0);
        assert!(s.done(&def));
    }

    #[test]
    fn size_and_colour_follow_their_scales() {
        let def = burst();
        let d = &def.emitters[0];
        assert_eq!(size_at(d, 0.0), 0.5);
        assert!((size_at(d, 0.5) - 1.25).abs() < 1e-5);
        assert_eq!(size_at(d, 1.0), 2.0);
        let c = colour_at(d, 0.5);
        assert!((c[0] - 0.6).abs() < 1e-5 && (c[2] - 0.3).abs() < 1e-5);
        assert_eq!(colour_at(&EmitterDef::default(), 0.3), [1.0; 4]);
    }

    #[test]
    fn particles_fly_grow_and_fade() {
        let def = burst();
        let mut s = EffectState::new(&def, [0.0; 3], 3);
        s.step(&def, 1.0 / 60.0);
        let before = s.sprites(&def, 0);
        for _ in 0..30 {
            s.step(&def, 1.0 / 60.0);
        }
        let after = s.sprites(&def, 0);
        assert_eq!(before.len(), after.len());
        assert!(after[0].half_size[0] > before[0].half_size[0]);
        assert!(after.iter().any(|p| p.position[2] > 1.0), "they fly");
        // Past the fade start (0.5 s), alpha drops.
        for _ in 0..20 {
            s.step(&def, 1.0 / 60.0);
        }
        let late = s.sprites(&def, 0);
        assert!(late.iter().all(|p| p.fade < 1.0));
    }

    #[test]
    fn a_looping_emitter_keeps_its_count_and_never_ends() {
        let def = EffectDef {
            emitters: vec![EmitterDef {
                max_particles: 5,
                lifetime: Range::fixed(1.0),
                ..EmitterDef::default()
            }],
            ..EffectDef::default()
        };
        let mut s = EffectState::new(&def, [0.0; 3], 1);
        for _ in 0..600 {
            s.step(&def, 1.0 / 60.0);
            assert!(s.live() <= 5);
        }
        assert!(s.live() >= 4);
        assert!(!s.done(&def));
        let timed = EffectDef {
            life_span: 2.0,
            ..def.clone()
        };
        assert!(s.done(&timed), "a life span ends it");
    }

    #[test]
    fn frames_run_through_the_picture() {
        let def = EffectDef {
            emitters: vec![EmitterDef {
                max_particles: 1,
                respawn: false,
                initial_rate: 100.0,
                lifetime: Range::fixed(1.0),
                frames_u: 4,
                frames_v: 4,
                ..EmitterDef::default()
            }],
            ..EffectDef::default()
        };
        let mut s = EffectState::new(&def, [0.0; 3], 1);
        s.step(&def, 0.001);
        assert_eq!(s.sprites(&def, 0)[0].frame, 0);
        for _ in 0..59 {
            s.step(&def, 1.0 / 60.0);
        }
        assert!(s.sprites(&def, 0)[0].frame >= 14);
    }

    #[test]
    fn debris_bounces_off_the_floor() {
        let def = EffectDef {
            emitters: vec![EmitterDef {
                max_particles: 3,
                respawn: false,
                initial_rate: 100.0,
                lifetime: Range::fixed(10.0),
                acceleration: [0.0, 0.0, -500.0],
                collides: true,
                damping: Range::fixed(0.4),
                ..EmitterDef::default()
            }],
            ..EffectDef::default()
        };
        let mut s = EffectState::new(&def, [0.0, 0.0, 100.0], 1);
        s.floor = Some(0.0);
        for _ in 0..600 {
            s.step(&def, 1.0 / 60.0);
        }
        for p in &s.emitters[0].particles {
            assert!(p.position[2] >= 0.0 && p.position[2] < 1.0, "{p:?}");
        }
    }

    #[test]
    fn a_delay_holds_an_emitter_back() {
        let mut def = burst();
        def.emitters[0].initial_delay = Range::fixed(0.25);
        let mut s = EffectState::new(&def, [0.0; 3], 1);
        s.step(&def, 0.1);
        assert_eq!(s.live(), 0);
        s.step(&def, 0.2);
        assert_eq!(s.live(), 8);
    }
}
