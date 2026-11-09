//! The player's movement: walking, running, strafing, jumping, falling,
//! crouching and climbing steps, with Republic Commando's own numbers.
//!
//! Plain Rust with no Bevy, so it can be tested on its own. Everything is in
//! Unreal units with Z up, like the game's data. The Bevy side converts.
//!
//! What comes from the game and what is our own guess is written down in
//! docs/DESIGN.md ("Player movement") and docs/RESEARCH.md. In short: the
//! numbers and the script-side rules (jump, crouch, eye height, walk toggle)
//! were read from the game's script packages. The physics step itself lives
//! in the game's compiled engine, which we don't read, so it follows how
//! Unreal Engine 2 is publicly documented to work. No game code is copied.

pub mod params;
pub mod room;
pub mod triangles;
pub mod vec;
pub mod world;

pub use params::MoveParams;
pub use vec::Vec3;
pub use triangles::TriangleWorld;
pub use world::{Brush, BrushWorld, Hit, Plane, World};

/// Gap kept between the player and any surface, uu. Stops rounding errors
/// from leaving the box stuck a hair inside a wall.
const SKIN: f32 = 0.1;
/// Longest physics step, s. Longer frames are cut into steps this long.
const MAX_STEP: f32 = 0.05;
/// Shortest forward move when stepping up onto something, uu (about one
/// frame's run at 60 frames a second). See `try_step_up`.
const MIN_STEP_FORWARD: f32 = 8.0;
/// How many times one move may hit something and slide on.
const MAX_SLIDES: usize = 4;

/// What the player is asking for this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MoveInput {
    /// -1 (back) to 1 (forward).
    pub forward: f32,
    /// -1 (left) to 1 (right).
    pub strafe: f32,
    /// Facing, radians. 0 looks along +X, a quarter turn looks along +Y
    /// (Unreal's convention).
    pub yaw: f32,
    /// Jump key held. A jump starts when it goes from up to down.
    pub jump: bool,
    /// Crouch key held (the game's crouch is hold-to-crouch).
    pub crouch: bool,
    /// Walk key held (Shift in the game's default controls).
    pub walk: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Walking,
    Falling,
}

/// Things that happened during a step, for the log.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Jumped { z: f32 },
    LeftGround { z: f32 },
    /// `speed` is the downward speed on landing. `damage` is what the game's
    /// falling-damage rule would take (we don't apply it yet).
    Landed { z: f32, speed: f32, damage: f32 },
    SteppedUp { height: f32 },
    Crouched,
    StoodUp,
    /// Wanted to stand up but the ceiling is too low.
    CantStand,
}

/// The player's body.
#[derive(Debug, Clone)]
pub struct Pawn {
    pub params: MoveParams,
    /// Centre of the collision box.
    pub location: Vec3,
    pub velocity: Vec3,
    pub mode: Mode,
    pub crouched: bool,
    /// Eye (camera) height above `location`, smoothed.
    pub eye_height: f32,
    /// Normal of the floor under the player, while walking.
    pub floor_normal: Vec3,
    jump_was_down: bool,
    wants_crouch: bool,
}

impl Pawn {
    /// A player whose feet are at `feet`.
    pub fn new(params: MoveParams, feet: Vec3) -> Self {
        let location = feet + Vec3::new(0.0, 0.0, params.collision_height + SKIN);
        Pawn {
            eye_height: params.base_eye_height,
            params,
            location,
            velocity: Vec3::ZERO,
            mode: Mode::Falling,
            crouched: false,
            floor_normal: Vec3::UP,
            jump_was_down: false,
            wants_crouch: false,
        }
    }

    /// Half-size of the collision box right now.
    pub fn extent(&self) -> Vec3 {
        let p = &self.params;
        if self.crouched {
            Vec3::new(p.crouch_radius, p.crouch_radius, p.crouch_height)
        } else {
            Vec3::new(p.collision_radius, p.collision_radius, p.collision_height)
        }
    }

    /// Where the camera goes.
    pub fn eye(&self) -> Vec3 {
        self.location + Vec3::new(0.0, 0.0, self.eye_height)
    }

    /// Bottom of the collision box.
    pub fn feet(&self) -> Vec3 {
        self.location - Vec3::new(0.0, 0.0, self.extent().z)
    }

    /// Top speed for the current stance, before direction is counted.
    pub fn stance_speed(&self, walk: bool) -> f32 {
        let p = &self.params;
        let mut s = p.ground_speed;
        if walk {
            s *= p.walk_speed_ratio;
        }
        if self.crouched {
            s *= p.crouch_speed_ratio;
        }
        s
    }

    /// Advance by `dt` seconds.
    pub fn step(&mut self, dt: f32, input: &MoveInput, world: &dyn World) -> Vec<Event> {
        let mut events = Vec::new();
        let mut left = dt.max(0.0);
        while left > 1e-6 {
            let t = left.min(MAX_STEP);
            self.sub_step(t, input, world, &mut events);
            left -= t;
        }
        events
    }

    fn sub_step(&mut self, dt: f32, input: &MoveInput, world: &dyn World, events: &mut Vec<Event>) {
        let old_z = self.location.z;
        let (fwd, right) = axes(input.yaw);
        let wish = (fwd * input.forward + right * input.strafe).clamp_length(1.0);
        let accel = wish * self.params.accel_rate;

        // Jump: only from the ground, not crouched, and once per key press.
        let jump_pressed = input.jump && !self.jump_was_down;
        self.jump_was_down = input.jump;
        if jump_pressed && self.mode == Mode::Walking && !self.crouched && !input.crouch {
            self.velocity.z = self.params.jump_z;
            self.mode = Mode::Falling;
            events.push(Event::Jumped { z: self.feet().z });
        }

        // The game only reads the crouch key while not falling.
        if self.mode != Mode::Falling {
            self.wants_crouch = input.crouch;
        }
        self.update_crouch(world, events);

        match self.mode {
            Mode::Walking => self.walk(dt, accel, input, world, events),
            Mode::Falling => self.fall(dt, accel, world, events),
        }

        self.update_eye_height(dt, old_z);
    }

    fn update_crouch(&mut self, world: &dyn World, events: &mut Vec<Event>) {
        let p = &self.params;
        let adjust = p.collision_height - p.crouch_height;
        if self.wants_crouch && !self.crouched && self.mode == Mode::Walking {
            // Shrink, keeping the feet where they are.
            self.crouched = true;
            self.location.z -= adjust;
            events.push(Event::Crouched);
        } else if !self.wants_crouch && self.crouched {
            // Standing up needs `2 * adjust` of free space above the head.
            let e = self.extent();
            let up = self.location + Vec3::new(0.0, 0.0, 2.0 * adjust);
            if world.sweep(e, self.location, up).is_none() {
                self.crouched = false;
                self.location.z += adjust;
                events.push(Event::StoodUp);
            } else if !matches!(events.last(), Some(Event::CantStand)) {
                events.push(Event::CantStand);
            }
        }
    }

    /// Top speed in the direction the player is pushing. The game stores
    /// separate ratios for backwards and sideways; how the engine blends them
    /// for diagonals isn't known, so this uses an ellipse through them
    /// (our guess).
    fn direction_ratio(&self, accel: Vec3, yaw: f32) -> f32 {
        let dir = accel.normalize_or_zero();
        if dir == Vec3::ZERO {
            return 1.0;
        }
        let (fwd, right) = axes(yaw);
        let f = dir.dot(fwd);
        let s = dir.dot(right);
        let a = if f >= 0.0 { 1.0 } else { self.params.back_speed_ratio };
        let b = self.params.side_speed_ratio;
        1.0 / ((f / a).powi(2) + (s / b).powi(2)).sqrt()
    }

    fn walk(&mut self, dt: f32, accel: Vec3, input: &MoveInput, world: &dyn World, events: &mut Vec<Event>) {
        let max_speed = self.stance_speed(input.walk) * self.direction_ratio(accel, input.yaw);
        let friction = self.params.ground_friction;
        let mut v = self.velocity.flat();
        if accel == Vec3::ZERO {
            // Braking: friction pulls the speed down quickly.
            let old = v;
            v -= v * (2.0 * friction * dt).min(1.0);
            if old.dot(v) <= 0.0 {
                v = Vec3::ZERO;
            }
        } else {
            // Friction turns the current speed toward where the player pushes,
            // then the push adds speed, up to the top speed.
            let speed = v.length();
            let dir = accel.normalize_or_zero();
            v -= (v - dir * speed) * (friction * dt).min(1.0);
            v += accel * dt;
            v = v.clamp_length(max_speed);
        }

        let start = self.location;
        self.move_on_ground(v * dt, world, events);
        if self.mode == Mode::Walking {
            self.find_floor(world, events);
        }
        // Like the engine, the walking speed is what the player really moved,
        // so running into a wall stops the speed too.
        let moved = (self.location - start).flat();
        if self.mode == Mode::Walking {
            self.velocity = moved * (1.0 / dt);
        } else {
            // Walked off an edge: keep the speed we had.
            self.velocity = Vec3::new(v.x, v.y, 0.0);
        }
    }

    /// Move along the ground, sliding along walls and climbing steps.
    fn move_on_ground(&mut self, mut delta: Vec3, world: &dyn World, events: &mut Vec<Event>) {
        for _ in 0..MAX_SLIDES {
            if delta.length() < 1e-4 {
                return;
            }
            let Some((hit, moved)) = self.sweep_and_move(world, delta) else {
                return;
            };
            // What's left of the move, counted from where the box really
            // stopped (a hair short of the hit), so the gap kept from the
            // surface isn't lost from the move. Losing it every frame slowed
            // the player to a stop on ramps at high frame rates, where each
            // frame's move is only a couple of units long.
            let remaining = delta * (1.0 - moved);
            if hit.normal.z >= self.params.min_floor_z {
                // Walking up a slope that counts as floor: follow it, keeping
                // the same direction and speed across the ground and only
                // rising with the slope, so slopes don't slow or turn the
                // player. (Sliding along the slope's surface instead turned
                // every move a little sideways, downhill; that added up to
                // a drift and a crawl at high frame rates.)
                let flat = remaining.flat();
                let rise = -(hit.normal.x * flat.x + hit.normal.y * flat.y) / hit.normal.z;
                delta = Vec3::new(flat.x, flat.y, rise);
                continue;
            }
            if hit.normal.z.abs() < self.params.min_floor_z && self.try_step_up(remaining, world, events) {
                return;
            }
            // A wall: slide along it, staying level so steep slopes can't be
            // walked up.
            let wall = hit.normal.flat().normalize_or_zero();
            let wall = if wall == Vec3::ZERO { hit.normal } else { wall };
            delta = remaining.slide_along(wall).flat();
        }
    }

    /// Step up onto something no taller than `max_step_height`: go up, go
    /// forward, come back down onto floor. Gives up if any part fails.
    fn try_step_up(&mut self, delta: Vec3, world: &dyn World, events: &mut Vec<Event>) -> bool {
        let e = self.extent();
        let start = self.location;
        let step = self.params.max_step_height;
        let up_end = start + Vec3::new(0.0, 0.0, step);
        let up = match world.sweep(e, start, up_end) {
            None => step,
            Some(h) => (step * h.time - SKIN).max(0.0),
        };
        let p1 = start + Vec3::new(0.0, 0.0, up);
        // Go forward at least MIN_STEP_FORWARD, even when this frame's move
        // is shorter. At high frame rates a frame's move is a unit or two,
        // too short to get past the sloped front edge of a low lip: coming
        // back down landed on that edge, which is too steep to stand on, so
        // the step failed every frame and the player stopped at the lip.
        let mut fwd = delta.flat();
        let len = fwd.length();
        if len < 1e-4 {
            return false;
        }
        if len < MIN_STEP_FORWARD {
            fwd = fwd * (MIN_STEP_FORWARD / len);
        }
        let p2 = match world.sweep(e, p1, p1 + fwd) {
            None => p1 + fwd,
            Some(h) => {
                let len = fwd.length();
                if h.start_solid || h.time * len < 1.0 {
                    return false;
                }
                p1 + fwd * (h.time - SKIN / len).max(0.0)
            }
        };
        // Come back down, as far as we went up.
        let down_end = p2 - Vec3::new(0.0, 0.0, up);
        match world.sweep(e, p2, down_end) {
            Some(h) if h.start_solid => false,
            Some(h) if h.normal.z < self.params.min_floor_z => false,
            Some(h) => {
                let drop = (up * h.time - SKIN).max(0.0);
                self.location = p2 - Vec3::new(0.0, 0.0, drop);
                self.floor_normal = h.normal;
                let height = self.location.z - start.z;
                if height > 0.5 {
                    events.push(Event::SteppedUp { height });
                }
                true
            }
            // Nothing under us at the top: stepped onto thin air. Let the
            // floor check below decide whether we fall.
            None => {
                self.location = down_end;
                true
            }
        }
    }

    /// Keep the feet on the floor when walking down slopes and stairs; start
    /// falling if there's no floor within a step's height.
    fn find_floor(&mut self, world: &dyn World, events: &mut Vec<Event>) {
        let e = self.extent();
        let reach = self.params.max_step_height + 2.0 * SKIN;
        let down = self.location - Vec3::new(0.0, 0.0, reach);
        // Look from a little above, so a box that has crept closer than
        // SKIN to the floor (rounding while sliding along a slope adds up)
        // is lifted back to SKIN above it. Left touching, every later move
        // along the floor would "hit" it at once and get nowhere, which
        // stopped the player dead on some slopes at high frame rates.
        let lifted = self.location + Vec3::new(0.0, 0.0, SKIN);
        let (from, hit) = match world.sweep(e, lifted, down) {
            Some(h) if h.start_solid => (self.location, world.sweep(e, self.location, down)),
            h => (lifted, h),
        };
        match hit {
            Some(h) if !h.start_solid && h.normal.z >= self.params.min_floor_z => {
                let floor = from.z - (from.z - down.z) * h.time;
                self.location.z = floor + SKIN;
                self.floor_normal = h.normal;
            }
            _ => {
                self.mode = Mode::Falling;
                events.push(Event::LeftGround { z: self.feet().z });
            }
        }
    }

    fn fall(&mut self, dt: f32, accel: Vec3, world: &dyn World, events: &mut Vec<Event>) {
        let p = self.params.clone();
        // A little steering in the air, but it can't push past running speed.
        let old_h = self.velocity.flat();
        let mut h = old_h + accel * (p.air_control * dt);
        h = h.clamp_length(old_h.length().max(p.ground_speed));
        let old_v = Vec3::new(old_h.x, old_h.y, self.velocity.z);
        let mut v = Vec3::new(h.x, h.y, self.velocity.z + p.gravity_z * dt);
        v.z = v.z.max(-p.terminal_velocity);
        // Average of the old and new speed: exact for steady gravity, so jump
        // heights come out right whatever the frame rate.
        let mut delta = (old_v + v) * (0.5 * dt);
        self.velocity = v;

        for slide in 0..MAX_SLIDES {
            if delta.length() < 1e-4 {
                return;
            }
            let Some((hit, moved)) = self.sweep_and_move(world, delta) else {
                return;
            };
            if hit.normal.z >= p.min_floor_z && self.velocity.z <= 0.0 {
                // Downward speed at the moment of touching, not at the end
                // of the step, so the result doesn't depend on frame rate.
                let speed = if slide == 0 {
                    -(old_v.z + p.gravity_z * dt * hit.time)
                } else {
                    -self.velocity.z
                };
                let damage = if speed > p.max_fall_speed {
                    100.0 * (speed - p.max_fall_speed) / p.max_fall_speed
                } else {
                    0.0
                };
                self.mode = Mode::Walking;
                self.floor_normal = hit.normal;
                self.velocity.z = 0.0;
                events.push(Event::Landed {
                    z: self.feet().z,
                    speed,
                    damage,
                });
                return;
            }
            let remaining = delta * (1.0 - moved);
            delta = remaining.slide_along(hit.normal);
            if self.velocity.dot(hit.normal) < 0.0 {
                self.velocity = self.velocity.slide_along(hit.normal);
            }
        }
    }

    /// Sweep the box by `delta` and move it up to just short of whatever it
    /// hits. Returns the hit, if any, and the fraction of `delta` really
    /// moved (0 to 1).
    fn sweep_and_move(&mut self, world: &dyn World, delta: Vec3) -> Option<(Hit, f32)> {
        let e = self.extent();
        match world.sweep(e, self.location, self.location + delta) {
            None => {
                self.location += delta;
                None
            }
            Some(h) => {
                let mut t = 0.0;
                if !h.start_solid {
                    // Stop SKIN short of the surface, measured straight out
                    // from it (not along the move): a move that only grazes
                    // a surface would otherwise end up touching it, and the
                    // next move along it would hit it again straight away.
                    let approach = -delta.dot(h.normal);
                    if approach > 1e-6 {
                        t = (h.time - SKIN / approach).max(0.0);
                        self.location += delta * t;
                    }
                } else {
                    // Stuck inside something: nudge out along its surface.
                    self.location += h.normal * SKIN;
                }
                Some((h, t))
            }
        }
    }

    /// The game's eye-height smoothing: the eye eases toward its resting
    /// height (10 per second), and while walking it also soaks up sudden
    /// height changes (steps, crouching) so the camera doesn't jump.
    fn update_eye_height(&mut self, dt: f32, old_z: f32) {
        let p = &self.params;
        let base = if self.crouched {
            p.crouched_eye_height()
        } else {
            p.base_eye_height
        };
        let h = self.extent().z;
        let max_eye = h + p.max_step_height;
        let smooth = (10.0 * dt).min(1.0);
        if self.mode == Mode::Walking {
            let lag = self.eye_height - (self.location.z - old_z);
            self.eye_height = (lag * (1.0 - smooth) + base * smooth).clamp(-0.5 * h, max_eye);
        } else {
            self.eye_height = (self.eye_height * (1.0 - smooth) + base * smooth).min(max_eye);
        }
    }
}

/// Forward and right directions for a facing, flat on the ground.
pub fn axes(yaw: f32) -> (Vec3, Vec3) {
    let (s, c) = yaw.sin_cos();
    (Vec3::new(c, s, 0.0), Vec3::new(-s, c, 0.0))
}

#[cfg(test)]
mod tests;
