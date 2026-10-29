//! Unreal units and Bevy units.
//!
//! The original game measures in Unreal units (uu), with Z up. Bevy uses Y
//! up. Every plugin converts the same way, with these functions.

use bevy::math::Vec3;

/// Unreal units per Bevy unit. The commando is 168 uu (3.36 Bevy units) tall.
pub const UU_PER_UNIT: f32 = 50.0;

/// Unreal position (Z up, left-handed) to Bevy (Y up, right-handed): swap Y
/// and Z, then scale.
pub fn to_bevy(v: rc_movement::Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, v.y) / UU_PER_UNIT
}

/// Bevy position back to Unreal units.
pub fn to_unreal(v: Vec3) -> rc_movement::Vec3 {
    rc_movement::Vec3::new(v.x, v.z, v.y) * UU_PER_UNIT
}
