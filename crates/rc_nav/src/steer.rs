//! Turning a target point or a route into move input for `rc_movement`.
//!
//! Computer characters move with the same physics as the player: each
//! frame they produce a `MoveInput` (forward, strafe, facing) and
//! `Pawn::step` does the rest. That keeps walking speed, steps, slopes and
//! collision identical for everyone, and the speeds come from each
//! character's own data file.

use rc_movement::{MoveInput, Pawn, Vec3};

/// How close (sideways) counts as "at" an in-between point, uu. Generous,
/// so characters cut corners a little instead of stopping at every node.
pub const PASS_RADIUS: f32 = 64.0;
/// How close counts as arrived at the last point, uu.
pub const ARRIVE_RADIUS: f32 = 24.0;
/// How far below or above a point the character's centre may be and still
/// count as there, uu (path nodes sit at about standing height, and floors
/// slope).
pub const HEIGHT_TOLERANCE: f32 = 120.0;

/// Facing (radians, Unreal convention: 0 is +X, a quarter turn is +Y) from
/// `from` towards `to`, ignoring height.
pub fn yaw_towards(from: Vec3, to: Vec3) -> f32 {
    let d = to - from;
    d.y.atan2(d.x)
}

fn at(pawn: &Pawn, target: Vec3, radius: f32) -> bool {
    let d = target - pawn.location;
    d.length_2d() <= radius && d.z.abs() <= HEIGHT_TOLERANCE
}

/// Move input that walks `pawn` straight towards `target`, facing where it
/// goes. Returns the input and whether the pawn has arrived (within
/// `arrive_radius` sideways). On arrival the input is empty (stand still).
pub fn steer(pawn: &Pawn, target: Vec3, arrive_radius: f32, walk: bool) -> (MoveInput, bool) {
    if at(pawn, target, arrive_radius) {
        return (MoveInput { yaw: current_yaw(pawn), walk, ..Default::default() }, true);
    }
    let yaw = yaw_towards(pawn.location, target);
    (MoveInput { forward: 1.0, yaw, walk, ..Default::default() }, false)
}

/// Move input that walks towards `target` while facing `face_yaw` (for
/// characters that can strafe: shooting while moving sideways). The
/// direction of travel is split into forward and sideways parts.
pub fn steer_facing(pawn: &Pawn, target: Vec3, face_yaw: f32, arrive_radius: f32, walk: bool) -> (MoveInput, bool) {
    if at(pawn, target, arrive_radius) {
        return (MoveInput { yaw: face_yaw, walk, ..Default::default() }, true);
    }
    let dir = (target - pawn.location).flat().normalize_or_zero();
    let (fwd, right) = rc_movement::axes(face_yaw);
    (
        MoveInput {
            forward: dir.dot(fwd),
            strafe: dir.dot(right),
            yaw: face_yaw,
            walk,
            ..Default::default()
        },
        false,
    )
}

/// The direction the pawn is moving, or 0 if it's standing still.
fn current_yaw(pawn: &Pawn) -> f32 {
    let v = pawn.velocity;
    if v.length_2d() > 1.0 {
        v.y.atan2(v.x)
    } else {
        0.0
    }
}

/// Walks a pawn along a list of points, one frame at a time.
#[derive(Debug, Clone)]
pub struct PathFollower {
    pub points: Vec<Vec3>,
    next: usize,
    pub walk: bool,
    /// Seconds without getting closer to the current point.
    pub stuck_time: f32,
    best_distance: f32,
    /// Total distance moved while following, uu (for the log).
    pub travelled: f32,
    last: Option<Vec3>,
}

impl PathFollower {
    pub fn new(points: Vec<Vec3>) -> Self {
        PathFollower {
            points,
            next: 0,
            walk: false,
            stuck_time: 0.0,
            best_distance: f32::INFINITY,
            travelled: 0.0,
            last: None,
        }
    }

    /// Index of the point being walked to.
    pub fn next_index(&self) -> usize {
        self.next
    }

    /// The point being walked to, if any are left.
    pub fn target(&self) -> Option<Vec3> {
        self.points.get(self.next).copied()
    }

    pub fn finished(&self) -> bool {
        self.next >= self.points.len()
    }

    /// Straight-line distance still to walk, from `from` through the
    /// remaining points.
    pub fn remaining(&self, from: Vec3) -> f32 {
        let mut total = 0.0;
        let mut prev = from;
        for &p in &self.points[self.next.min(self.points.len())..] {
            total += (p - prev).length();
            prev = p;
        }
        total
    }

    /// Input for this frame, or `None` once the last point is reached.
    /// `dt` is the frame time, used to notice being stuck.
    pub fn update(&mut self, pawn: &Pawn, dt: f32) -> Option<MoveInput> {
        if let Some(l) = self.last {
            self.travelled += (pawn.location - l).length_2d();
        }
        self.last = Some(pawn.location);
        loop {
            let target = self.target()?;
            let last = self.next + 1 == self.points.len();
            let radius = if last { ARRIVE_RADIUS } else { PASS_RADIUS };
            let (input, arrived) = steer(pawn, target, radius, self.walk);
            if arrived {
                self.next += 1;
                self.best_distance = f32::INFINITY;
                self.stuck_time = 0.0;
                if self.finished() {
                    return None;
                }
                continue;
            }
            let d = (target - pawn.location).length_2d();
            if d < self.best_distance - 1.0 {
                self.best_distance = d;
                self.stuck_time = 0.0;
            } else {
                self.stuck_time += dt;
            }
            return Some(input);
        }
    }
}
