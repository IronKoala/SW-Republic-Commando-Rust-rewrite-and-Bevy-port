//! A free-flying camera ("noclip"), kept apart from the level code so it can
//! later be swapped for a first-person player or a VR headset.
//!
//! Controls: W A S D move, E or Space up, Q or Ctrl down, hold Shift for
//! faster, hold the right mouse button and move the mouse to look around,
//! mouse wheel changes the speed.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;

#[derive(Component)]
pub struct FlyCamera {
    /// Turn left/right and up/down, in radians.
    pub yaw: f32,
    pub pitch: f32,
    /// Bevy units per second.
    pub speed: f32,
}

impl FlyCamera {
    pub fn rotation(&self) -> Quat {
        Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0)
    }
}

/// Unreal yaw (65536 per turn, 0 = looking along +X, increasing towards +Y)
/// to the fly camera's yaw. Bevy cameras look along -Z; Unreal +X is Bevy +X
/// and Unreal +Y is Bevy +Z after the axis swap.
pub fn yaw_from_unreal(yaw: i32) -> f32 {
    let a = yaw as f32 / 65536.0 * std::f32::consts::TAU;
    // Facing direction in Bevy: (cos a, 0, sin a). A camera with yaw y faces
    // (-sin y, 0, -cos y). Solve for y.
    let (dx, dz) = (a.cos(), a.sin());
    (-dx).atan2(-dz)
}

pub fn fly(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut cams: Query<(&mut FlyCamera, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (mut cam, mut t) in &mut cams {
        if buttons.pressed(MouseButton::Right) {
            cam.yaw -= motion.delta.x * 0.003;
            cam.pitch = (cam.pitch - motion.delta.y * 0.003).clamp(-1.54, 1.54);
        }
        if scroll.delta.y != 0.0 {
            cam.speed = (cam.speed * 1.2f32.powf(scroll.delta.y)).clamp(0.5, 500.0);
        }
        t.rotation = cam.rotation();
        let mut dir = Vec3::ZERO;
        let fwd = t.forward().as_vec3();
        let right = t.right().as_vec3();
        if keys.pressed(KeyCode::KeyW) {
            dir += fwd;
        }
        if keys.pressed(KeyCode::KeyS) {
            dir -= fwd;
        }
        if keys.pressed(KeyCode::KeyD) {
            dir += right;
        }
        if keys.pressed(KeyCode::KeyA) {
            dir -= right;
        }
        if keys.any_pressed([KeyCode::KeyE, KeyCode::Space]) {
            dir += Vec3::Y;
        }
        if keys.any_pressed([KeyCode::KeyQ, KeyCode::ControlLeft]) {
            dir -= Vec3::Y;
        }
        let boost = if keys.pressed(KeyCode::ShiftLeft) {
            4.0
        } else {
            1.0
        };
        t.translation += dir.normalize_or_zero() * cam.speed * boost * dt;
    }
}
