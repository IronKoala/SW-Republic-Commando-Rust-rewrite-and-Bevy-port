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
//!   `colour_scale` over its life, linearly between the points;
//! - an effect can be turned and sized where it is placed
//!   ([`EffectState::placed`], used by the level's effect spawners): an
//!   emitter whose `CoordinateSystem` is `PTCS_Relative` has its start
//!   places, flight and acceleration turned with the effect; every
//!   particle's size is multiplied by the effect's draw scale. **Our
//!   reading**: the engine's code for both is compiled, not in the game's
//!   files; the values (the rotation, `EmitterDrawScale`,
//!   `CoordinateSystem`) are the game's.

use crate::def::{
    CoordinateSystem, EffectDef, EmitterDef, Facing, Range, RangeVector, StartShape, VelocityFrom,
};

/// The three axes (forward, right, up; Unreal X, Y, Z) of an Unreal
/// rotation (pitch, yaw, roll; 65536 a full turn), the engine's own order.
pub fn rotation_axes(rot: [i32; 3]) -> [[f32; 3]; 3] {
    let to_rad = std::f32::consts::TAU / 65536.0;
    let (sp, cp) = (rot[0] as f32 * to_rad).sin_cos();
    let (sy, cy) = (rot[1] as f32 * to_rad).sin_cos();
    let (sr, cr) = (rot[2] as f32 * to_rad).sin_cos();
    [
        [cp * cy, cp * sy, sp],
        [sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, -sr * cp],
        [-(cr * sp * cy + sr * sy), cy * sr - cr * sp * sy, cr * cp],
    ]
}

/// No turn.
pub const IDENTITY_AXES: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// A vector in the effect's own axes, in the level's.
fn turn(axes: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|c| v[0] * axes[0][c] + v[1] * axes[1][c] + v[2] * axes[2][c])
}

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
    /// Where it was before its last move (`Particle.OldLocation`): the
    /// engine turns `Up` and `Forward` sprites by `position - old_position`.
    pub old_position: [f32; 3],
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
    /// For emitters that mix frames (`BlendBetweenSubdivisions`): the
    /// frame being mixed in, and how much of it shows (0-1, in 255 steps
    /// as the engine stores it). Otherwise `frame` and 0.
    pub next_frame: u32,
    pub frame_mix: f32,
    /// Its flight, for sparks.
    pub velocity: [f32; 3],
    /// How far it moved in the last step (`Location - OldLocation`), for
    /// sprites that face along their movement ([`sprite_axes`]).
    pub movement: [f32; 3],
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
    /// The effect's own axes (forward, right, up) in the level's; for
    /// emitters with `PTCS_Relative`. No turn unless [`Self::placed`].
    pub axes: [[f32; 3]; 3],
    /// Multiplies every particle's size (the effect's draw scale).
    pub scale: f32,
    /// The height of the ground under it, for particles that bounce.
    pub floor: Option<f32>,
    pub age: f32,
    pub emitters: Vec<EmitterState>,
    /// Faded out (`Emitter.FadeOut`): no new particles; done when the last
    /// one dies.
    pub fading: bool,
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

/// The frame a particle shows at relative time `t` (0-1) of its life, for
/// an emitter whose picture is split into frames and not picked at random.
/// From Engine.dll (`USpriteEmitter::FillVertexBuffer`): with
/// `SubdivisionEnd` set, `SubdivisionStart + floor(max(End - Start, 1) x t)`;
/// otherwise `floor(frames x t)` (`SubdivisionStart` is then not used);
/// either way kept within the picture's frames. (`UseSubdivisionScale`, a
/// table of times, isn't read yet; no bacta effect uses it.)
pub fn frame_at(def: &EmitterDef, t: f32) -> u32 {
    let frames = def.frames() as i64;
    let t = t.clamp(0.0, 1.0);
    let f = if def.frame_end != 0 {
        let n = (def.frame_end as i64 - def.frame_start as i64).max(1);
        def.frame_start as i64 + (n as f32 * t).floor() as i64
    } else {
        (frames as f32 * t).floor() as i64
    };
    f.clamp(0, frames - 1) as u32
}

/// For an emitter that mixes frames (`BlendBetweenSubdivisions`, without
/// `UseRandomSubdivision`): the frame mixed in at relative time `t` and
/// how much of it shows. From Engine.dll
/// (`USpriteEmitter::FillVertexBuffer` and `RenderParticles`), with `n`
/// the frame count used by [`frame_at`] (`max(End - Start, 1)` with
/// `SubdivisionEnd` set, otherwise all frames):
/// - the next frame is `frame + 1`, or `frame` itself when `frame + 1`
///   equals `n`;
/// - the share is the part of `n x t` after the point, stored as a byte
///   (`trunc(share x 255)`) in the vertex colour's alpha.
///
/// The graphics driver (D3DDrv.dll, `FD3DRenderInterface::SetParticleMaterial`)
/// then draws the frame with the next one mixed in by that alpha
/// (`D3DTOP_BLENDDIFFUSEALPHA`), times the vertex colour, and takes the
/// opacity from the mixed picture alone: the vertex alpha is used up as
/// the share, so the particle's own alpha does not show.
pub fn frame_blend(def: &EmitterDef, t: f32) -> Option<(u32, f32)> {
    if !def.blend_frames || def.random_frame || def.frames() <= 1 {
        return None;
    }
    let t = t.clamp(0.0, 1.0);
    let n = if def.frame_end != 0 {
        (def.frame_end as i64 - def.frame_start as i64).max(1)
    } else {
        def.frames() as i64
    };
    let frame = frame_at(def, t);
    let next = if frame as i64 + 1 == n {
        frame
    } else {
        frame + 1
    };
    let nt = n as f32 * t;
    let share = ((nt - nt.trunc()) * 255.0).trunc() / 255.0;
    Some((next, share))
}

/// Where frame `frame` of a picture split into `frames_u` x `frames_v` sits,
/// as (column, row), from the top left. The engine counts frames down
/// each column first (Engine.dll, `USpriteEmitter::FillVertexBuffer`: the
/// column is `frame / TextureVSubdivisions`, the row
/// `frame % TextureVSubdivisions`).
pub fn frame_cell(frame: u32, frames_u: u32, frames_v: u32) -> (u32, u32) {
    let (u, v) = (frames_u.max(1), frames_v.max(1));
    ((frame / v) % u, frame % v)
}

/// `FVector::GetNonParallel` (Core.dll): an axis that isn't close to
/// lying along `v`.
fn non_parallel(v: [f32; 3]) -> [f32; 3] {
    if v[0].abs() > 0.57 {
        [0.0, 1.0, 0.0]
    } else if v[1].abs() > 0.57 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `FVector::SafeNormal`: the same way, length 1; nothing for nothing.
fn safe_normal(v: [f32; 3]) -> [f32; 3] {
    let l2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if l2 == 0.0 {
        return v;
    }
    let k = 1.0 / l2.sqrt();
    v.map(|c| c * k)
}

/// A sprite's two half-axes in Unreal units, `(right, up)`: its corners are
/// `position -/+ right -/+ up`, the picture's top along `+up`. `None` for a
/// sprite flat to the camera, which the renderer turns to its camera
/// itself (using `half_size[0]` for both: the engine draws those square,
/// from `Size.X` alone).
///
/// From Engine.dll (`USpriteEmitter::FillVertexBuffer`), with `d` the
/// particle's last move made length 1:
/// - `Up`: `up = d x Size.Y`, `right = (d cross (position - camera))`
///   made length 1, `x Size.X`;
/// - `Forward`: `n` = `GetNonParallel(d)`, `up = (d cross n) x Size.Y`
///   (not made length 1, as in the engine), `right = (d cross up)` made
///   length 1, `x Size.X`.
///
/// A particle that hasn't moved has no `d`, so it is drawn with no size,
/// as in the engine.
pub fn sprite_axes(
    facing: Facing,
    sprite: &Sprite,
    camera: [f32; 3],
) -> Option<([f32; 3], [f32; 3])> {
    let d = safe_normal(sprite.movement);
    let [sx, sy] = sprite.half_size;
    match facing {
        Facing::Up => {
            let to_cam = [0, 1, 2].map(|k| sprite.position[k] - camera[k]);
            let right = safe_normal(cross(d, to_cam)).map(|c| c * sx);
            Some((right, d.map(|c| c * sy)))
        }
        Facing::Forward => {
            let up = cross(d, non_parallel(d)).map(|c| c * sy);
            let right = safe_normal(cross(d, up)).map(|c| c * sx);
            Some((right, up))
        }
        Facing::Camera | Facing::Normal | Facing::UpAndNormal => None,
    }
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
            axes: IDENTITY_AXES,
            scale: 1.0,
            floor: None,
            age: 0.0,
            emitters,
            fading: false,
            rng,
        }
    }

    /// `Emitter.FadeOut` (what a projectile's trail gets when the
    /// projectile is gone): every emitter stops sending particles and the
    /// effect ends when its last particle dies.
    pub fn fade_out(&mut self) {
        self.fading = true;
    }

    /// Move the effect to `origin` (an effect carried by something that
    /// moves, as a projectile's trail is). Particles already sent stay
    /// where they are, except those of `PTCS_Relative` emitters, which
    /// move with it.
    pub fn move_to(&mut self, def: &EffectDef, origin: [f32; 3]) {
        let delta = [0, 1, 2].map(|k| origin[k] - self.origin[k]);
        self.origin = origin;
        for (d, s) in def.emitters.iter().zip(&mut self.emitters) {
            if d.coordinate_system == CoordinateSystem::Relative {
                for p in &mut s.particles {
                    for k in 0..3 {
                        p.position[k] += delta[k];
                    }
                }
            }
        }
    }

    /// Turned by `rotation` (Unreal pitch, yaw, roll) and sized by
    /// `scale`, as an effect spawner places its effects. Call before the
    /// first step.
    pub fn placed(mut self, rotation: [i32; 3], scale: f32) -> Self {
        self.axes = rotation_axes(rotation);
        self.scale = scale;
        self
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
        if self.fading {
            return self.live() == 0;
        }
        def.emitters.iter().zip(&self.emitters).all(|(d, s)| {
            d.disabled || (!d.respawn && s.sent >= d.max_particles && s.particles.is_empty())
        })
    }

    fn spawn(&mut self, def: &EmitterDef, i: usize) {
        let rng = &mut self.rng;
        // Engine.dll (UParticleEmitter::SpawnParticle): a box start is
        // `StartLocationOffset` plus a random point of
        // `StartLocationRange`; for an emitter in the level's axes
        // (PTCS_Independent) that random point and the start velocity are
        // turned by the effect's axes, the offset is not. A PTCS_Relative
        // emitter keeps both in its own axes and is turned as it is drawn,
        // which comes to turning everything here.
        let independent_box = def.coordinate_system == CoordinateSystem::Independent
            && def.start_shape == StartShape::Box;
        let mut offset = match def.start_shape {
            StartShape::Box if independent_box => turn(&self.axes, rng.range3(&def.start_range)),
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
        match def.coordinate_system {
            CoordinateSystem::Relative => {
                offset = turn(&self.axes, offset);
                velocity = turn(&self.axes, velocity);
            }
            CoordinateSystem::Independent => velocity = turn(&self.axes, velocity),
            CoordinateSystem::Absolute => {}
        }
        let sx = rng.unit();
        let size = if def.uniform_size {
            let s = def.start_size.x.at(sx);
            [s, s]
        } else {
            [def.start_size.x.at(sx), def.start_size.y.at(rng.unit())]
        }
        .map(|s| s * self.scale);
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
        let at = [0, 1, 2].map(|k| self.origin[k] + offset[k]);
        let p = Particle {
            position: at,
            old_position: at,
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
            let acceleration = if d.coordinate_system == CoordinateSystem::Relative {
                turn(&self.axes, d.acceleration)
            } else {
                d.acceleration
            };
            let s = &mut self.emitters[i];
            for p in &mut s.particles {
                p.age += dt;
                p.old_position = p.position;
                for k in 0..3 {
                    p.velocity[k] += acceleration[k] * dt;
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
            if self.fading {
                continue;
            }
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
                    frame_at(d, t)
                };
                let (next_frame, frame_mix) = if frames <= 1 {
                    (frame, 0.0)
                } else {
                    frame_blend(d, t).unwrap_or((frame, 0.0))
                };
                Sprite {
                    position: p.position,
                    half_size: [p.size[0] * scale, p.size[1] * scale],
                    colour: c,
                    fade: fade.clamp(0.0, 1.0),
                    spin: p.spin,
                    frame,
                    next_frame,
                    frame_mix,
                    velocity: p.velocity,
                    movement: [0, 1, 2].map(|k| p.position[k] - p.old_position[k]),
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
            ambient_sound: None,
            life_span: 0.0,
            lit: false,
        }
    }

    #[test]
    fn a_carried_trail_moves_and_fades_out() {
        // A trail that keeps sending (respawns), as a missile's does.
        let mut def = burst();
        def.emitters[0].respawn = true;
        def.emitters[0].start_velocity = RangeVector::fixed(0.0);
        let mut e = EffectState::new(&def, [0.0; 3], 3);
        e.step(&def, 0.1);
        let first = e.emitters[0].particles[0].position;
        // Carried 1000 uu: old particles stay (independent emitter), new
        // ones start at the new place.
        e.move_to(&def, [1000.0, 0.0, 0.0]);
        assert_eq!(e.emitters[0].particles[0].position, first);
        // Faded out: nothing new, done once the last dies.
        e.fade_out();
        let sent = e.emitters[0].sent;
        assert!(!e.done(&def));
        for _ in 0..40 {
            e.step(&def, 0.05);
        }
        assert_eq!(e.emitters[0].sent, sent);
        assert!(e.done(&def));
        // A relative one's particles move with it.
        def.emitters[0].coordinate_system = crate::def::CoordinateSystem::Relative;
        let mut e = EffectState::new(&def, [0.0; 3], 3);
        e.step(&def, 0.1);
        let x = e.emitters[0].particles[0].position[0];
        e.move_to(&def, [1000.0, 0.0, 0.0]);
        assert_eq!(e.emitters[0].particles[0].position[0], x + 1000.0);
    }

    #[test]
    fn a_relative_emitter_turns_with_the_effect_and_scales() {
        let mut def = burst();
        def.emitters[0].coordinate_system = crate::def::CoordinateSystem::Relative;
        def.emitters[0].start_velocity = RangeVector::fixed(0.0);
        def.emitters[0].start_velocity.x = Range::fixed(8000.0);
        // Yaw a quarter turn: the effect's forward is the level's +Y.
        let mut s = EffectState::new(&def, [0.0; 3], 5).placed([0, 16384, 0], 2.0);
        s.step(&def, 0.01);
        let p = &s.emitters[0].particles[0];
        assert!(
            p.velocity[0].abs() < 1.0 && (p.velocity[1] - 8000.0).abs() < 1.0,
            "{:?}",
            p.velocity
        );
        let plain = {
            let mut s = EffectState::new(&def, [0.0; 3], 5);
            s.step(&def, 0.01);
            s.emitters[0].particles[0].size
        };
        assert!((p.size[0] - plain[0] * 2.0).abs() < 1e-3);
        // Independent emitters' start flight turns too (Engine.dll,
        // SpawnParticle); Absolute ones' doesn't.
        def.emitters[0].coordinate_system = crate::def::CoordinateSystem::Independent;
        let mut s = EffectState::new(&def, [0.0; 3], 5).placed([0, 16384, 0], 1.0);
        s.step(&def, 0.01);
        assert!((s.emitters[0].particles[0].velocity[1] - 8000.0).abs() < 1.0);
        def.emitters[0].coordinate_system = crate::def::CoordinateSystem::Absolute;
        let mut s = EffectState::new(&def, [0.0; 3], 5).placed([0, 16384, 0], 1.0);
        s.step(&def, 0.01);
        assert!((s.emitters[0].particles[0].velocity[0] - 8000.0).abs() < 1.0);
    }

    #[test]
    fn an_independent_start_box_turns_but_its_offset_does_not() {
        // The bacta hologram's clone: 50 forward and 100 up of the
        // dispenser's Eye bone, in the bone's axes.
        let mut def = burst();
        let e = &mut def.emitters[0];
        e.coordinate_system = crate::def::CoordinateSystem::Independent;
        e.start_velocity = RangeVector::fixed(0.0);
        e.start_range = RangeVector::fixed(0.0);
        e.start_range.x = Range::fixed(50.0);
        e.start_range.z = Range::fixed(100.0);
        e.start_offset = [7.0, 0.0, 0.0];
        let mut s = EffectState::new(&def, [0.0; 3], 5).placed([0, 16384, 0], 1.0);
        s.step(&def, 0.01);
        let p = s.emitters[0].particles[0].position;
        // Forward is +Y now; the offset stays on +X.
        assert!(
            (p[0] - 7.0).abs() < 1e-3 && (p[1] - 50.0).abs() < 1e-3,
            "{p:?}"
        );
        assert!((p[2] - 100.0).abs() < 1e-3, "{p:?}");
    }

    #[test]
    fn frames_follow_the_engine() {
        let mut d = EmitterDef {
            frames_u: 1,
            frames_v: 8,
            ..Default::default()
        };
        // The bacta power bar: 8 frames over its life.
        assert_eq!(frame_at(&d, 0.0), 0);
        assert_eq!(frame_at(&d, 0.5), 4);
        assert_eq!(frame_at(&d, 1.0), 7);
        // SubdivisionStart is only used with an end.
        d.frame_start = 2;
        assert_eq!(frame_at(&d, 0.0), 0);
        d.frame_end = 6;
        assert_eq!(frame_at(&d, 0.0), 2);
        assert_eq!(frame_at(&d, 0.99), 5);
        assert_eq!(frame_at(&d, 1.0), 6);
        // Down each column first: 2 x 4 (the bacta text).
        assert_eq!(frame_cell(0, 2, 4), (0, 0));
        assert_eq!(frame_cell(3, 2, 4), (0, 3));
        assert_eq!(frame_cell(4, 2, 4), (1, 0));
        assert_eq!(frame_cell(7, 2, 4), (1, 3));
    }

    #[test]
    fn frames_mix_as_the_engine_mixes_them() {
        // 1 x 8, like the bacta power bar.
        let mut d = EmitterDef {
            frames_u: 1,
            frames_v: 8,
            ..Default::default()
        };
        assert_eq!(
            frame_blend(&d, 0.3),
            None,
            "only with BlendBetweenSubdivisions"
        );
        d.blend_frames = true;
        // 8 x 0.3 = 2.4: frame 2 with 0.4 of frame 3 (as a byte: 102/255).
        let (next, share) = frame_blend(&d, 0.3).unwrap();
        assert_eq!((frame_at(&d, 0.3), next), (2, 3));
        assert!((share - 101.0 / 255.0).abs() < 1.5 / 255.0, "{share}");
        // The last frame mixes with itself.
        assert_eq!(frame_blend(&d, 0.95).unwrap().0, 7);
        // Random frames don't mix.
        d.random_frame = true;
        assert_eq!(frame_blend(&d, 0.3), None);
        d.random_frame = false;
        // With an end, "last" is frame + 1 == End - Start, as the engine
        // compares it.
        d.frame_start = 2;
        d.frame_end = 6;
        assert_eq!(frame_at(&d, 0.5), 4);
        assert_eq!(frame_blend(&d, 0.5).unwrap().0, 5);
        assert_eq!(frame_at(&d, 0.25), 3);
        assert_eq!(frame_blend(&d, 0.25).unwrap().0, 3);
    }

    #[test]
    fn sprites_face_the_way_the_engine_turns_them() {
        let s = Sprite {
            position: [100.0, 0.0, 0.0],
            half_size: [10.0, 40.0],
            colour: [1.0; 4],
            fade: 1.0,
            spin: 0.0,
            frame: 0,
            next_frame: 0,
            frame_mix: 0.0,
            velocity: [0.0; 3],
            movement: [0.0, 0.0, 2.0],
        };
        // Up: long side along the move (up), short side across it and the
        // line to the camera.
        let (r, u) = sprite_axes(Facing::Up, &s, [0.0; 3]).unwrap();
        assert_eq!(u, [0.0, 0.0, 40.0]);
        assert!(
            (r[0]).abs() < 1e-4 && (r[1].abs() - 10.0).abs() < 1e-4,
            "{r:?}"
        );
        // Forward, moving up: GetNonParallel gives X, so up is along Y
        // and right along X (made length 1, times Size.X).
        let (r, u) = sprite_axes(Facing::Forward, &s, [0.0; 3]).unwrap();
        assert!((u[1].abs() - 40.0).abs() < 1e-4, "{u:?}");
        assert!((r[0].abs() - 10.0).abs() < 1e-4, "{r:?}");
        // Not moved: no size.
        let still = Sprite {
            movement: [0.0; 3],
            ..s
        };
        let (r, u) = sprite_axes(Facing::Up, &still, [0.0; 3]).unwrap();
        assert_eq!((r, u), ([0.0; 3], [0.0; 3]));
        assert!(sprite_axes(Facing::Camera, &s, [0.0; 3]).is_none());
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
