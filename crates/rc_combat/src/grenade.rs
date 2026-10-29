//! Grenades: their numbers (from `data/grenades/*.toml`), how a thrown one
//! flies, bounces and goes off, and how a blast hurts what is near it.
//! Plain Rust, no Bevy systems.
//!
//! The rules follow the game's script (`CTInventory`'s grenade classes and
//! the engine's `Actor.HurtRadius`), described in our own words in
//! docs/DESIGN.md, "Grenades". Everything is in Unreal space (uu, Z up).

use crate::data::Table;
use crate::health::DamageKind;
use bevy::math::Vec3;

/// When a grenade goes off.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fuse {
    /// This many seconds after the throw, wherever it is.
    Timed(f32),
    /// The first time it touches anything.
    Impact,
}

/// Everything about one kind of grenade.
#[derive(Debug, Clone, PartialEq)]
pub struct GrenadeParams {
    /// File name without `.toml`, like `thermal_detonator`.
    pub id: String,
    pub name: String,
    /// Most carried (`MaxAmmo`), and how many at the start
    /// (`PickupAmmoCount`).
    pub max_count: u32,
    pub start_count: u32,
    /// Throw speed, uu/s, along the view (`Speed` / `MaxSpeed`).
    pub speed: f32,
    /// Extra upward speed added to the throw, uu/s (`TossZ`).
    pub toss_up: f32,
    /// Where it leaves the hand, from the eye: forward, right, up.
    pub throw_offset: [f32; 3],
    /// Seconds between throws.
    pub throw_delay: f32,
    pub fuse: Fuse,
    /// Speed kept after a bounce, 0 to 1 (`Bounciness` / velocity
    /// dampening).
    pub bounce: f32,
    /// Below this speed after a bounce, it stops and lies still.
    pub rest_speed: f32,
    /// Downward pull, uu/s² (the zone's gravity, positive number).
    pub gravity: f32,
    pub damage: f32,
    /// Blast radius, uu (`DamageRadius`).
    pub radius: f32,
    pub damage_kind: DamageKind,
    /// Push given to characters, uu/s at the centre (`MomentumTransfer`).
    pub momentum: f32,
    /// Seconds a character in the blast can't act (flash and EMP).
    pub stun_time: f32,
    /// Which kinds of character are stunned (`"droid"`, `"organic"`);
    /// empty for every kind.
    pub stun_only: Vec<String>,
    /// Damage multipliers by kind of character, like `droid = 3`.
    pub damage_vs: Vec<(String, f32)>,
    pub sound_package: String,
    pub throw_sound: Option<String>,
    pub bounce_sound: Option<String>,
    pub blast_sound: Option<String>,
    /// The thrown model: a static mesh in `model_package`.
    pub model_package: String,
    pub model: Option<String>,
    /// Colour of the blast flash we draw, red, green, blue from 0 to 1.
    pub blast_colour: [f32; 3],
}

impl GrenadeParams {
    pub fn from_text(id: &str, text: &str) -> Result<GrenadeParams, String> {
        let mut t = Table::parse(text)?;
        let need = |v: Option<f32>, name: &str| v.ok_or_else(|| format!("`{name}` is missing"));
        let fuse = match t.text("fuse.kind")?.as_deref() {
            Some("timed") => Fuse::Timed(need(t.number("fuse.time")?, "fuse.time")?),
            Some("impact") => Fuse::Impact,
            Some(o) => return Err(format!("fuse.kind `{o}`: use \"timed\" or \"impact\"")),
            None => return Err("`fuse.kind` is missing".to_string()),
        };
        let damage_kind = match t.text("blast.damage_kind")? {
            None => DamageKind::Explosion,
            Some(k) => DamageKind::from_name(&k)
                .ok_or_else(|| format!("blast.damage_kind `{k}` is not a known kind"))?,
        };
        let mut damage_vs = Vec::new();
        for kind in CHARACTER_KINDS {
            if let Some(m) = t.number(&format!("damage_vs.{kind}"))? {
                damage_vs.push((kind.to_string(), m));
            }
        }
        let p = GrenadeParams {
            id: id.to_string(),
            name: t.text("name")?.unwrap_or_else(|| id.to_string()),
            max_count: need(t.number("ammo.max_count")?, "ammo.max_count")? as u32,
            start_count: t.number("ammo.start_count")?.unwrap_or(0.0) as u32,
            speed: need(t.number("throw.speed")?, "throw.speed")?,
            toss_up: t.number("throw.toss_up")?.unwrap_or(0.0),
            throw_offset: t.vec3("throw.offset")?.unwrap_or([0.0; 3]),
            throw_delay: t.number("throw.delay")?.unwrap_or(0.0),
            fuse,
            bounce: t.number("flight.bounce")?.unwrap_or(0.0).clamp(0.0, 1.0),
            rest_speed: t.number("flight.rest_speed")?.unwrap_or(20.0),
            gravity: t.number("flight.gravity")?.unwrap_or(950.0),
            damage: need(t.number("blast.damage")?, "blast.damage")?,
            radius: need(t.number("blast.radius")?, "blast.radius")?,
            damage_kind,
            momentum: t.number("blast.momentum")?.unwrap_or(0.0),
            stun_time: t.number("blast.stun_time")?.unwrap_or(0.0),
            stun_only: t.texts("blast.stun_only")?.unwrap_or_default(),
            damage_vs,
            sound_package: t
                .text("sound.package")?
                .unwrap_or_else(|| "gen_sound".to_string()),
            throw_sound: t.text("sound.throw")?,
            bounce_sound: t.text("sound.bounce")?,
            blast_sound: t.text("sound.blast")?,
            model_package: t
                .text("model.package")?
                .unwrap_or_else(|| "weapons".to_string()),
            model: t.text("model.mesh")?,
            blast_colour: t.vec3("model.blast_colour")?.unwrap_or([1.0, 0.7, 0.3]),
        };
        t.finish()?;
        if p.max_count == 0 || p.radius < 0.0 || p.speed < 0.0 {
            return Err(
                "ammo.max_count must be more than 0, throw.speed and blast.radius 0 or more".into(),
            );
        }
        Ok(GrenadeParams {
            start_count: p.start_count.min(p.max_count),
            ..p
        })
    }

    /// The damage multiplier against a kind of character (1 if not listed).
    pub fn multiplier_vs(&self, kind: &str) -> f32 {
        self.damage_vs
            .iter()
            .find(|(k, _)| k == kind)
            .map_or(1.0, |(_, m)| *m)
    }

    /// Whether a character of this kind is stunned by the blast.
    pub fn stuns(&self, kind: &str) -> bool {
        self.stun_time > 0.0
            && (self.stun_only.is_empty() || self.stun_only.iter().any(|k| k == kind))
    }

    /// The starting speed of a throw along `view` (length 1).
    pub fn throw_velocity(&self, view: Vec3) -> Vec3 {
        view * self.speed + Vec3::Z * self.toss_up
    }
}

/// The kinds of character a damage table can name. Each character's kind
/// comes from its own settings (droids are `droid`, commandos, Geonosians
/// and Trandoshans `organic`).
pub const CHARACTER_KINDS: [&str; 2] = ["droid", "organic"];

/// A grenade in the air or lying on the ground.
#[derive(Debug, Clone, PartialEq)]
pub struct Flight {
    pub position: Vec3,
    pub velocity: Vec3,
    /// Seconds since the throw.
    pub age: f32,
    /// Lying still on the ground.
    pub resting: bool,
    pub bounces: u32,
}

/// What happened during one step of flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StepResult {
    pub bounced: bool,
    /// The fuse ran out or an impact grenade touched something.
    pub explode: bool,
}

impl Flight {
    pub fn new(position: Vec3, velocity: Vec3) -> Self {
        Flight {
            position,
            velocity,
            age: 0.0,
            resting: false,
            bounces: 0,
        }
    }

    /// Fly `dt` seconds. `sweep(from, to)` returns where along the line
    /// (0 to 1) something is in the way, and the surface's facing.
    /// Gravity pulls down, and on touching something it bounces off with
    /// `bounce` of its speed (the engine's `PHYS_Falling` with
    /// `bBounce`, as the grenade's `HitWall` does it).
    pub fn step(
        &mut self,
        p: &GrenadeParams,
        dt: f32,
        mut sweep: impl FnMut(Vec3, Vec3) -> Option<(f32, Vec3)>,
    ) -> StepResult {
        let mut out = StepResult::default();
        self.age += dt;
        if let Fuse::Timed(t) = p.fuse {
            if self.age >= t {
                out.explode = true;
                return out;
            }
        }
        if self.resting {
            return out;
        }
        // Up to a few touches in one step (a corner), then give up for
        // this frame.
        let mut left = dt;
        for _ in 0..4 {
            if left <= 0.0 {
                break;
            }
            let v0 = self.velocity;
            let v1 = v0 - Vec3::Z * p.gravity * left;
            let to = self.position + (v0 + v1) * 0.5 * left;
            match sweep(self.position, to) {
                None => {
                    self.position = to;
                    self.velocity = v1;
                    break;
                }
                Some((t, normal)) => {
                    // Stop just short of the surface.
                    let t = t.clamp(0.0, 1.0);
                    let back = (to - self.position).normalize_or_zero() * 0.5;
                    self.position = self.position + (to - self.position) * t - back;
                    let v = v0 - Vec3::Z * p.gravity * left * t;
                    if p.fuse == Fuse::Impact {
                        out.explode = true;
                        return out;
                    }
                    let n = normal.normalize_or_zero();
                    self.velocity = (v - 2.0 * v.dot(n) * n) * p.bounce;
                    self.bounces += 1;
                    out.bounced = true;
                    left *= 1.0 - t;
                    // On a floor, slow enough: it lies still.
                    if n.z > 0.7 && self.velocity.length() < p.rest_speed {
                        self.velocity = Vec3::ZERO;
                        self.resting = true;
                        break;
                    }
                }
            }
        }
        out
    }
}

/// How much of a blast reaches a character: the engine's `HurtRadius`
/// rule. Full at the centre, falling in a straight line to nothing at the
/// edge, measured from the character's centre less its own radius (so a
/// big character is hurt a little further out). `dist` is from the blast
/// to the character's centre.
pub fn blast_scale(dist: f32, character_radius: f32, blast_radius: f32) -> f32 {
    if blast_radius <= 0.0 {
        return 0.0;
    }
    if dist > blast_radius + character_radius {
        return 0.0;
    }
    (1.0 - ((dist - character_radius).max(0.0) / blast_radius)).clamp(0.0, 1.0)
}

/// A character's grenades: kinds carried, how many of each, and which one
/// is selected. The player gets one from `data/combat/player.toml`.
#[derive(bevy::prelude::Component, Debug, Clone, Default, PartialEq)]
pub struct Grenades {
    /// Grenade ids and counts, in the order the "next grenade" button
    /// cycles through.
    pub carried: Vec<(String, u32)>,
    /// Index into `carried`.
    pub selected: usize,
    /// Seconds until the next throw is allowed.
    pub cooldown: f32,
}

impl Grenades {
    pub fn selected(&self) -> Option<&(String, u32)> {
        self.carried.get(self.selected)
    }

    /// Select the next kind that has any left (or stay put if none do).
    pub fn select_next(&mut self) -> bool {
        let n = self.carried.len();
        for i in 1..=n {
            let j = (self.selected + i) % n;
            if self.carried[j].1 > 0 && j != self.selected {
                self.selected = j;
                return true;
            }
        }
        false
    }

    /// Take one of the selected kind. Returns its id.
    pub fn take(&mut self) -> Option<String> {
        if self.cooldown > 0.0 {
            return None;
        }
        let (id, count) = self.carried.get_mut(self.selected)?;
        if *count == 0 {
            return None;
        }
        *count -= 1;
        Some(id.clone())
    }

    /// Add `n` (a pickup), up to the most carried.
    pub fn add(&mut self, id: &str, n: u32, max: u32) -> u32 {
        match self.carried.iter_mut().find(|(i, _)| i == id) {
            Some((_, c)) => {
                let before = *c;
                *c = (*c + n).min(max);
                *c - before
            }
            None => {
                self.carried.push((id.to_string(), n.min(max)));
                n.min(max)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST: &str = "name = \"Test\"\n[ammo]\nmax_count = 4\nstart_count = 2\n\
        [throw]\nspeed = 1000\ntoss_up = 200\n[fuse]\nkind = \"timed\"\ntime = 2\n\
        [flight]\nbounce = 0.5\ngravity = 1000\n[blast]\ndamage = 100\nradius = 500\n\
        [damage_vs]\ndroid = 2\n";

    #[test]
    fn reads_a_grenade_file() {
        let p = GrenadeParams::from_text("test", TEST).unwrap();
        assert_eq!(p.fuse, Fuse::Timed(2.0));
        assert_eq!((p.max_count, p.start_count), (4, 2));
        assert_eq!(p.multiplier_vs("droid"), 2.0);
        assert_eq!(p.multiplier_vs("organic"), 1.0);
        let e = GrenadeParams::from_text("t", &TEST.replace("toss_up", "tos_up")).unwrap_err();
        assert!(e.contains("unknown setting `throw.tos_up`"), "{e}");
    }

    #[test]
    fn flies_in_an_arc_and_bounces_off_the_floor() {
        let p = GrenadeParams::from_text("test", TEST).unwrap();
        let floor =
            |a: Vec3, b: Vec3| (b.z < 0.0 && a.z >= 0.0).then(|| (a.z / (a.z - b.z), Vec3::Z));
        let mut f = Flight::new(Vec3::new(0.0, 0.0, 100.0), Vec3::new(1000.0, 0.0, 200.0));
        let mut highest: f32 = 0.0;
        let mut first_bounce = None;
        for i in 0..120 {
            let r = f.step(&p, 1.0 / 60.0, floor);
            highest = highest.max(f.position.z);
            if r.bounced && first_bounce.is_none() {
                first_bounce = Some((i, f.position.x, f.velocity));
            }
            assert!(!r.explode, "fuse is 2 s");
            assert!(f.position.z > -1.0, "never through the floor");
        }
        // Peak: 100 + 200²/(2·1000) = 120.
        assert!((highest - 120.0).abs() < 1.0, "peak {highest}");
        let (_, x, v) = first_bounce.expect("bounced");
        // Falls 120 uu from the peak at 0.2 s: lands at about t = 0.69 s.
        assert!((x - 690.0).abs() < 20.0, "first bounce at x {x}");
        assert!(
            v.z > 0.0 && (v.x - 500.0).abs() < 1.0,
            "half speed kept: {v:?}"
        );
        let r = f.step(&p, 0.01, floor);
        assert!(r.explode, "goes off at 2 s");
    }

    #[test]
    fn impact_grenades_go_off_on_touch() {
        let text = TEST.replace("kind = \"timed\"\ntime = 2", "kind = \"impact\"");
        let p = GrenadeParams::from_text("test", &text).unwrap();
        let wall =
            |a: Vec3, b: Vec3| (b.x > 300.0).then(|| ((300.0 - a.x) / (b.x - a.x), -Vec3::X));
        let mut f = Flight::new(Vec3::ZERO, Vec3::new(1000.0, 0.0, 0.0));
        let mut steps = 0;
        while !f.step(&p, 1.0 / 60.0, wall).explode {
            steps += 1;
            assert!(steps < 60);
        }
        assert!((f.position.x - 300.0).abs() < 1.0, "{:?}", f.position);
    }

    #[test]
    fn blast_falls_off_with_distance() {
        assert_eq!(blast_scale(0.0, 40.0, 500.0), 1.0);
        assert_eq!(blast_scale(40.0, 40.0, 500.0), 1.0);
        assert!((blast_scale(290.0, 40.0, 500.0) - 0.5).abs() < 1e-6);
        assert_eq!(blast_scale(540.0, 40.0, 500.0), 0.0);
        assert_eq!(blast_scale(600.0, 40.0, 500.0), 0.0);
    }

    #[test]
    fn carrying_and_cycling() {
        let mut g = Grenades {
            carried: vec![("a".into(), 1), ("b".into(), 0), ("c".into(), 2)],
            ..Default::default()
        };
        assert_eq!(g.take().as_deref(), Some("a"));
        assert_eq!(g.take(), None, "none left");
        assert!(g.select_next());
        assert_eq!(g.selected().unwrap().0, "c", "skips empty kinds");
        assert_eq!(g.add("b", 9, 4), 4);
        assert_eq!(g.add("c", 9, 4), 2);
    }
}
