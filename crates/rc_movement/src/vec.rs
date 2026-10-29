//! A tiny 3D vector, so this crate needs no maths library.

use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);
    pub const UP: Vec3 = Vec3::new(0.0, 0.0, 1.0);

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Vec3 { x, y, z }
    }

    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// Length ignoring height.
    pub fn length_2d(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    pub fn flat(self) -> Vec3 {
        Vec3::new(self.x, self.y, 0.0)
    }

    /// Same direction, length 1. Zero stays zero.
    pub fn normalize_or_zero(self) -> Vec3 {
        let l = self.length();
        if l > 1e-6 {
            self * (1.0 / l)
        } else {
            Vec3::ZERO
        }
    }

    /// Shorten to at most `max` long.
    pub fn clamp_length(self, max: f32) -> Vec3 {
        let l = self.length();
        if l > max && l > 0.0 {
            self * (max / l)
        } else {
            self
        }
    }

    /// Remove the part of the vector that points along `normal` (length 1).
    pub fn slide_along(self, normal: Vec3) -> Vec3 {
        self - normal * self.dot(normal)
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl AddAssign for Vec3 {
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl SubAssign for Vec3 {
    fn sub_assign(&mut self, o: Vec3) {
        *self = *self - o;
    }
}

impl Mul<f32> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f32) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}
