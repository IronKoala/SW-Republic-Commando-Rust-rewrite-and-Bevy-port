//! The little bit of 3D maths skinning needs: vectors, quaternions (a
//! compact way to store a rotation) and rigid transforms (a rotation plus a
//! move). Plain textbook formulas.

pub type Vec3 = [f32; 3];

pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn scale(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub fn length(a: Vec3) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

pub fn lerp(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    add(a, scale(sub(b, a), t))
}

/// Rotation as x, y, z, w with w the "real" part.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quat(pub [f32; 4]);

impl Quat {
    pub const IDENTITY: Quat = Quat([0.0, 0.0, 0.0, 1.0]);

    /// The opposite rotation (for a unit quaternion).
    pub fn conjugate(self) -> Quat {
        let [x, y, z, w] = self.0;
        Quat([-x, -y, -z, w])
    }

    pub fn normalize(self) -> Quat {
        let l = self.0.iter().map(|v| v * v).sum::<f32>().sqrt();
        if l > 0.0 {
            Quat(self.0.map(|v| v / l))
        } else {
            Quat::IDENTITY
        }
    }

    /// `self * b`: first rotate by `b`, then by `self`.
    pub fn compose(self, b: Quat) -> Quat {
        let [ax, ay, az, aw] = self.0;
        let [bx, by, bz, bw] = b.0;
        Quat([
            aw * bx + ax * bw + ay * bz - az * by,
            aw * by - ax * bz + ay * bw + az * bx,
            aw * bz + ax * by - ay * bx + az * bw,
            aw * bw - ax * bx - ay * by - az * bz,
        ])
    }

    pub fn rotate(self, v: Vec3) -> Vec3 {
        // v' = v + 2w(q x v) + 2 q x (q x v)
        let [x, y, z, w] = self.0;
        let q = [x, y, z];
        let c1 = cross(q, v);
        let c2 = cross(q, c1);
        add(v, add(scale(c1, 2.0 * w), scale(c2, 2.0)))
    }

    /// Smooth blend between two rotations, taking the shorter way round.
    pub fn slerp(self, b: Quat, t: f32) -> Quat {
        let mut b = b.0;
        let mut d: f32 = self.0.iter().zip(&b).map(|(x, y)| x * y).sum();
        if d < 0.0 {
            b = b.map(|v| -v);
            d = -d;
        }
        if d > 0.9995 {
            // Nearly the same: a straight blend is accurate enough.
            let mut out = [0f32; 4];
            for i in 0..4 {
                out[i] = self.0[i] + (b[i] - self.0[i]) * t;
            }
            return Quat(out).normalize();
        }
        let theta = d.acos();
        let s = theta.sin();
        let wa = ((1.0 - t) * theta).sin() / s;
        let wb = (t * theta).sin() / s;
        let mut out = [0f32; 4];
        for i in 0..4 {
            out[i] = self.0[i] * wa + b[i] * wb;
        }
        Quat(out)
    }
}

pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// A rotation followed by a move.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub rot: Quat,
    pub pos: Vec3,
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        rot: Quat::IDENTITY,
        pos: [0.0; 3],
    };

    pub fn apply(&self, p: Vec3) -> Vec3 {
        add(self.rot.rotate(p), self.pos)
    }

    /// `self` after `child`: places something given relative to `self`.
    pub fn then(&self, child: &Transform) -> Transform {
        Transform {
            rot: self.rot.compose(child.rot).normalize(),
            pos: self.apply(child.pos),
        }
    }

    pub fn inverse(&self) -> Transform {
        let r = self.rot.conjugate();
        Transform {
            rot: r,
            pos: scale(r.rotate(self.pos), -1.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        length(sub(a, b)) < 1e-4
    }

    #[test]
    fn quarter_turn_about_z() {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let q = Quat([0.0, 0.0, h, h]);
        assert!(close(q.rotate([1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]));
        assert!(close(q.compose(q).rotate([1.0, 0.0, 0.0]), [-1.0, 0.0, 0.0]));
        assert!(close(
            q.slerp(Quat::IDENTITY, 1.0).rotate([1.0, 0.0, 0.0]),
            [1.0, 0.0, 0.0]
        ));
    }

    #[test]
    fn inverse_undoes_transform() {
        let t = Transform {
            rot: Quat([0.1, 0.2, 0.3, 0.9]).normalize(),
            pos: [5.0, -2.0, 7.0],
        };
        let p = [1.0, 2.0, 3.0];
        assert!(close(t.inverse().apply(t.apply(p)), p));
        assert!(close(t.then(&t.inverse()).apply(p), p));
    }
}
