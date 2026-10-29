//! Shot maths: where a shot starts, how far off it goes, how a bolt flies,
//! and whether a line hits a character. Plain maths, no Bevy systems.
//!
//! Everything here is in Unreal space: Unreal units, Z up, X forward, Y
//! right (the game's own axes). Bevy's `Vec3` is only used as a maths type.

use bevy::math::Vec3;

/// A small random number maker (xorshift64*), so shots can be repeated
/// exactly in tests and logs. Not for anything secret.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }

    /// A number from 0 up to (not including) 1, like the script's `FRand`.
    pub fn next_f32(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        let r = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (r >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// The view's forward, right and up directions from yaw and pitch
/// (radians; yaw 0 faces +X, positive yaw turns right toward +Y, positive
/// pitch looks up). Like the script's `GetAxes`.
pub fn view_axes(yaw: f32, pitch: f32) -> (Vec3, Vec3, Vec3) {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let forward = Vec3::new(cp * cy, cp * sy, sp);
    let right = Vec3::new(-sy, cy, 0.0);
    let up = Vec3::new(-sp * cy, -sp * sy, cp);
    (forward, right, up)
}

/// Where a shot leaves the weapon: the eye moved by the weapon's fire
/// offset along the view's forward, right and up (`GetFireStart`).
pub fn fire_start(eye: Vec3, axes: (Vec3, Vec3, Vec3), offset: [f32; 3]) -> Vec3 {
    eye + axes.0 * offset[0] + axes.1 * offset[1] + axes.2 * offset[2]
}

/// Aim a shot from `start` at the point under the crosshair, the way the
/// player's controller does (`PlayerController.AdjustAim`): the bolt leaves
/// the muzzle, which is off to the side, and flies to where the crosshair
/// points. If that point is behind or far to the side of the muzzle
/// (`dot < 0.4`), it just fires straight ahead, as the game does.
pub fn aim_at_crosshair(start: Vec3, view: Vec3, crosshair: Vec3) -> Vec3 {
    if (crosshair - start).normalize_or_zero().dot(view) < 0.4 {
        start + view * 100.0
    } else {
        crosshair
    }
}

/// The game's aim error (`AdjustAimError`): move the target point by a
/// random amount, up to `aim_error` uu for every 1000 uu of distance, along
/// the world's Y and Z axes (not the view's; that is how the game does it).
pub fn add_aim_error(start: Vec3, target: Vec3, aim_error: f32, rng: &mut Rng) -> Vec3 {
    let spread = aim_error * (target - start).length() * 0.001;
    let mut t = target;
    t.z += 2.0 * (rng.next_f32() - 0.5) * spread;
    t.y += 2.0 * (rng.next_f32() - 0.5) * spread;
    t
}

/// A laser bolt in flight. The game starts laser bolts at rest and speeds
/// them up along their direction (`Projectile.PostBeginPlay` with
/// `bLaserBolt`: speed 0, acceleration `AccelFactor`), up to `MaxSpeed`.
#[derive(Debug, Clone, PartialEq)]
pub struct Bolt {
    pub position: Vec3,
    /// Length 1.
    pub direction: Vec3,
    pub speed: f32,
    pub accel: f32,
    pub max_speed: f32,
    /// Seconds flown, and seconds before it disappears.
    pub age: f32,
    pub life_span: f32,
}

impl Bolt {
    /// Fly `dt` seconds. Returns the line flown, start to end.
    pub fn advance(&mut self, dt: f32) -> (Vec3, Vec3) {
        let from = self.position;
        let v0 = self.speed;
        let v1 = (v0 + self.accel * dt).min(self.max_speed);
        // Distance with constant acceleration up to the speed limit, then
        // constant speed: exact for any frame time.
        let dist = if self.accel > 0.0 && v1 > v0 {
            let t_acc = (v1 - v0) / self.accel;
            (v0 + v1) * 0.5 * t_acc + v1 * (dt - t_acc).max(0.0)
        } else {
            v0 * dt
        };
        self.speed = v1;
        self.age += dt;
        self.position = from + self.direction * dist;
        (from, self.position)
    }

    pub fn expired(&self) -> bool {
        self.age >= self.life_span
    }
}

/// Where along the line `a`..`b` (0 to 1) it first enters an upright
/// cylinder: `center` is the cylinder's middle, `radius` and `half_height`
/// its size. Unreal Engine 2 gives characters this cylinder shape for
/// collision. `None` if the line misses. A line starting inside counts as
/// hitting at 0.
pub fn line_hits_cylinder(
    a: Vec3,
    b: Vec3,
    center: Vec3,
    radius: f32,
    half_height: f32,
) -> Option<f32> {
    let d = b - a;
    let rel = a - center;
    let inside = |p: Vec3| p.x * p.x + p.y * p.y <= radius * radius && p.z.abs() <= half_height;
    if inside(rel) {
        return Some(0.0);
    }
    let mut best: Option<f32> = None;
    let mut consider = |t: f32| {
        if (0.0..=1.0).contains(&t) && best.is_none_or(|b| t < b) {
            best = Some(t);
        }
    };
    // The round side: solve |rel.xy + t d.xy| = radius.
    let qa = d.x * d.x + d.y * d.y;
    if qa > 1e-12 {
        let qb = 2.0 * (rel.x * d.x + rel.y * d.y);
        let qc = rel.x * rel.x + rel.y * rel.y - radius * radius;
        let disc = qb * qb - 4.0 * qa * qc;
        if disc >= 0.0 {
            let t = (-qb - disc.sqrt()) / (2.0 * qa);
            if (rel.z + t * d.z).abs() <= half_height {
                consider(t);
            }
        }
    }
    // The flat top and bottom.
    if d.z.abs() > 1e-12 {
        for cap in [half_height, -half_height] {
            let t = (cap - rel.z) / d.z;
            let p = rel + d * t;
            if p.x * p.x + p.y * p.y <= radius * radius {
                consider(t);
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_numbers_are_even_and_repeatable() {
        let mut r = Rng::new(7);
        let v: Vec<f32> = (0..10000).map(|_| r.next_f32()).collect();
        assert!(v.iter().all(|x| (0.0..1.0).contains(x)));
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        assert!((mean - 0.5).abs() < 0.01, "{mean}");
        assert_eq!(Rng::new(7).next_f32(), v[0]);
    }

    #[test]
    fn axes_match_the_game() {
        let (f, r, u) = view_axes(0.0, 0.0);
        assert_eq!((f, r, u), (Vec3::X, Vec3::Y, Vec3::Z));
        let (f, r, _) = view_axes(std::f32::consts::FRAC_PI_2, 0.0);
        assert!((f - Vec3::Y).length() < 1e-6 && (r + Vec3::X).length() < 1e-6);
        let (f, _, u) = view_axes(0.0, 0.5);
        assert!(f.dot(u).abs() < 1e-6 && f.z > 0.0);
    }

    #[test]
    fn aim_error_stays_in_its_cone() {
        let mut rng = Rng::new(1);
        let start = Vec3::ZERO;
        let target = Vec3::new(2000.0, 0.0, 0.0);
        let mut most = 0.0f32;
        for _ in 0..2000 {
            let t = add_aim_error(start, target, 15.0, &mut rng);
            assert_eq!(t.x, 2000.0);
            most = most.max(t.y.abs()).max(t.z.abs());
        }
        // 15 uu per 1000 uu at 2000 uu is up to 30 uu each way.
        assert!(most <= 30.0 && most > 29.0, "{most}");
    }

    #[test]
    fn bolt_speeds_up_then_holds_speed() {
        let mut b = Bolt {
            position: Vec3::ZERO,
            direction: Vec3::X,
            speed: 0.0,
            accel: 60000.0,
            max_speed: 10000.0,
            age: 0.0,
            life_span: 10.0,
        };
        // 1/6 s to reach 10000: 833.3 uu. Then 10000 uu/s.
        let mut small = b.clone();
        b.advance(0.5);
        for _ in 0..50 {
            small.advance(0.01);
        }
        let want = 10000.0 / 6.0 * 0.5 + 10000.0 * (0.5 - 1.0 / 6.0);
        assert!((b.position.x - want).abs() < 0.5, "{}", b.position.x);
        assert!(
            (small.position.x - want).abs() < 0.5,
            "step size doesn't matter"
        );
    }

    #[test]
    fn cylinder_hits() {
        let c = Vec3::new(1000.0, 0.0, 0.0);
        let t = line_hits_cylinder(Vec3::ZERO, Vec3::new(2000.0, 0.0, 0.0), c, 40.0, 84.0).unwrap();
        assert!((t * 2000.0 - 960.0).abs() < 1e-2);
        assert!(
            line_hits_cylinder(Vec3::ZERO, Vec3::new(2000.0, 50.0, 0.0), c, 40.0, 84.0).is_some()
        );
        assert!(
            line_hits_cylinder(Vec3::ZERO, Vec3::new(2000.0, 100.0, 0.0), c, 40.0, 84.0).is_none()
        );
        assert!(
            line_hits_cylinder(Vec3::ZERO, Vec3::new(500.0, 0.0, 0.0), c, 40.0, 84.0).is_none()
        );
        // Straight down onto the top.
        let t =
            line_hits_cylinder(c + Vec3::Z * 200.0, c - Vec3::Z * 200.0, c, 40.0, 84.0).unwrap();
        assert!((t * 400.0 - 116.0).abs() < 1e-2);
        // Over the top.
        assert!(line_hits_cylinder(
            Vec3::new(0.0, 0.0, 100.0),
            Vec3::new(2000.0, 0.0, 100.0),
            c,
            40.0,
            84.0
        )
        .is_none());
    }
}
