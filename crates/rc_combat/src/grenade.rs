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

/// A blast's lasting effect on the characters it reaches (the original's
/// `ExplosionStatusEffect`): the flash grenade stuns, the EMP grenade
/// electrocutes, the thermal detonator sets characters near it alight.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusParams {
    /// Seconds for a character at the centre (`StatusEffectMaxDuration`).
    /// It falls off in a straight line to nothing at the edge.
    pub max_time: f32,
    /// Damage per second while it lasts (`StatusEffectDamagePerSec`).
    pub damage_per_sec: f32,
    /// The status effect's damage type. A character whose damage table
    /// gives it x0 is immune (droids to stuns, for example).
    pub damage_type: String,
    /// Clone commandos are affected half as long (the electrocution).
    pub half_on_commandos: bool,
    /// Characters the player controls are only affected when looking
    /// towards the blast (the stun: you can look away from a flash).
    pub player_must_face: bool,
    /// Added to the blast radius for the effect's reach
    /// (`StatusEffectRadiusModifier`): the thermal's burn reaches 400 less.
    pub radius_change: f32,
    /// Computer-controlled characters can't act while it lasts (stun and
    /// electrocution). **Our reading**: burning doesn't stop them.
    pub stops: bool,
    /// The kind of the damage it does each second.
    pub damage_kind: DamageKind,
}

/// Everything about one kind of grenade.
#[derive(Debug, Clone, PartialEq)]
pub struct GrenadeParams {
    /// File name without `.toml`, like `thermal_detonator`.
    pub id: String,
    pub name: String,
    /// Most carried (`MaxAmmo`), and how many at the start
    /// (`InitAmmoAmount`).
    pub max_count: u32,
    pub start_count: u32,
    /// Throw speed, uu/s, along the view (`Speed`).
    pub speed: f32,
    /// Extra upward speed added to the throw, uu/s (`TossZ`).
    pub toss_up: f32,
    /// Where it leaves the hand, from the eye: forward, right, up
    /// (`FPGrenadeOffset`).
    pub throw_offset: [f32; 3],
    /// Seconds between throws.
    pub throw_delay: f32,
    /// Seconds from the throw to the blast (`FuseTime`); 0 for none.
    pub fuse_time: f32,
    /// After touching anything, it goes off within this many seconds
    /// (`PostImpactFuseTime`); 0 for no change.
    pub after_impact: f32,
    /// Goes off on touching a character (`bActorDetonate`) or the level
    /// (`bWorldDetonate`).
    pub explode_on_actor: bool,
    pub explode_on_world: bool,
    /// Sticks where it lands on the level (`bStickToWorld`), or to the
    /// character it touches (`bStickToActors`).
    pub stick_to_world: bool,
    pub stick_to_actors: bool,
    /// Once stuck, goes off when a character it can see comes this close,
    /// uu (`ExplosionProximity` with `bProximityDetonate`); 0 for never.
    pub proximity: f32,
    /// Seconds between those checks (`ElapseTime`).
    pub proximity_interval: f32,
    /// Speed kept after bouncing off the level (`BounceDampening`) and off
    /// a character (`ActorBounceDampening`).
    pub bounce: f32,
    pub actor_bounce: f32,
    /// Below this speed after a bounce it stops bouncing (`RestSpeed`).
    pub rest_speed: f32,
    /// Downward pull, uu/s² (the level's gravity, as a positive number).
    pub gravity: f32,
    /// Damage at the centre (`Damage`), and the blast radius
    /// (`DamageRadius`).
    pub damage: f32,
    pub radius: f32,
    /// The damage type class (`MyDamageType`), looked up in the target's
    /// damage table.
    pub damage_type: String,
    pub damage_kind: DamageKind,
    pub status: Option<StatusParams>,
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
        let flag = |t: &mut Table, n: &str| t.boolean(n).map(|v| v.unwrap_or(false));
        let damage_kind = match t.text("blast.damage_kind")? {
            None => DamageKind::Explosion,
            Some(k) => DamageKind::from_name(&k)
                .ok_or_else(|| format!("blast.damage_kind `{k}` is not a known kind"))?,
        };
        let status = match t.number("status.max_time")? {
            None => None,
            Some(max_time) => Some(StatusParams {
                max_time,
                damage_per_sec: t.number("status.damage_per_sec")?.unwrap_or(0.0),
                damage_type: t
                    .text("status.damage_type")?
                    .ok_or("`status.damage_type` is missing")?,
                half_on_commandos: flag(&mut t, "status.half_on_commandos")?,
                player_must_face: flag(&mut t, "status.player_must_face")?,
                radius_change: t.number("status.radius_change")?.unwrap_or(0.0),
                stops: t.boolean("status.stops")?.unwrap_or(true),
                damage_kind: match t.text("status.damage_kind")? {
                    None => DamageKind::Electrical,
                    Some(k) => DamageKind::from_name(&k)
                        .ok_or_else(|| format!("status.damage_kind `{k}` is not a known kind"))?,
                },
            }),
        };
        let p = GrenadeParams {
            id: id.to_string(),
            name: t.text("name")?.unwrap_or_else(|| id.to_string()),
            max_count: need(t.number("ammo.max_count")?, "ammo.max_count")? as u32,
            start_count: t.number("ammo.start_count")?.unwrap_or(0.0) as u32,
            speed: need(t.number("throw.speed")?, "throw.speed")?,
            toss_up: t.number("throw.toss_up")?.unwrap_or(0.0),
            throw_offset: t.vec3("throw.offset")?.unwrap_or([0.0; 3]),
            throw_delay: t.number("throw.delay")?.unwrap_or(0.0),
            fuse_time: t.number("fuse.time")?.unwrap_or(0.0).max(0.0),
            after_impact: t.number("fuse.after_impact")?.unwrap_or(0.0).max(0.0),
            explode_on_actor: flag(&mut t, "fuse.on_actor")?,
            explode_on_world: flag(&mut t, "fuse.on_world")?,
            stick_to_world: flag(&mut t, "flight.stick_to_world")?,
            stick_to_actors: flag(&mut t, "flight.stick_to_actors")?,
            proximity: t.number("fuse.proximity")?.unwrap_or(0.0),
            proximity_interval: t.number("fuse.proximity_interval")?.unwrap_or(0.5),
            bounce: t.number("flight.bounce")?.unwrap_or(0.0).clamp(0.0, 1.0),
            actor_bounce: t
                .number("flight.actor_bounce")?
                .unwrap_or(0.0)
                .clamp(0.0, 1.0),
            rest_speed: t.number("flight.rest_speed")?.unwrap_or(20.0),
            gravity: t.number("flight.gravity")?.unwrap_or(1100.0),
            damage: t.number("blast.damage")?.unwrap_or(0.0),
            radius: need(t.number("blast.radius")?, "blast.radius")?,
            damage_type: t
                .text("blast.damage_type")?
                .unwrap_or_else(|| "CTDamageExplosion".to_string()),
            damage_kind,
            status,
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
        if p.fuse_time == 0.0
            && p.after_impact == 0.0
            && !p.explode_on_actor
            && !p.explode_on_world
            && p.proximity == 0.0
        {
            return Err("the grenade never goes off: set fuse.time, after_impact, on_actor, on_world or proximity".into());
        }
        Ok(GrenadeParams {
            start_count: p.start_count.min(p.max_count),
            ..p
        })
    }

    /// The starting speed of a throw along `view` (length 1): the view
    /// times `speed`, plus `toss_up` upwards (`TossZ`).
    pub fn throw_velocity(&self, view: Vec3) -> Vec3 {
        view * self.speed + Vec3::Z * self.toss_up
    }
}

/// What a grenade touched this step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Touch {
    /// 0 to 1 along the step's line.
    pub t: f32,
    /// The surface's facing, length 1.
    pub normal: Vec3,
    /// A character (not the level).
    pub actor: bool,
}

/// A grenade in the air, lying on the ground or stuck to something.
#[derive(Debug, Clone, PartialEq)]
pub struct Flight {
    pub position: Vec3,
    pub velocity: Vec3,
    /// Seconds since the throw.
    pub age: f32,
    /// Seconds left on the fuse, if it has one running.
    pub fuse_left: Option<f32>,
    /// Lying still on the ground.
    pub resting: bool,
    /// Stuck to the level or (`stuck_to_actor`) a character.
    pub stuck: bool,
    pub stuck_to_actor: bool,
    pub bounces: u32,
    /// Seconds until the next proximity check.
    pub next_check: f32,
}

/// What happened during one step of flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StepResult {
    pub bounced: bool,
    /// The fuse ran out, or it touched what sets it off.
    pub explode: bool,
    /// It went off on touching a character (that character takes the full
    /// damage directly).
    pub hit_actor: bool,
    /// It stuck this step (to a character if `hit_actor`).
    pub stuck: bool,
}

impl Flight {
    pub fn new(p: &GrenadeParams, position: Vec3, velocity: Vec3) -> Self {
        Flight {
            position,
            velocity,
            age: 0.0,
            fuse_left: (p.fuse_time > 0.0).then_some(p.fuse_time),
            resting: false,
            stuck: false,
            stuck_to_actor: false,
            bounces: 0,
            next_check: p.proximity_interval,
        }
    }

    /// Fly `dt` seconds. `sweep(from, to)` returns the first thing in the
    /// way. The rules are the original's grenade script, in our words:
    /// gravity pulls it down; on touching something its fuse may shorten
    /// (`after_impact`), it may go off or stick, or else it bounces off the
    /// surface keeping `bounce` (level) or `actor_bounce` (character) of
    /// its speed. Slower than `rest_speed` on a floor, it lies still.
    pub fn step(
        &mut self,
        p: &GrenadeParams,
        dt: f32,
        mut sweep: impl FnMut(Vec3, Vec3) -> Option<Touch>,
    ) -> StepResult {
        let mut out = StepResult::default();
        self.age += dt;
        if let Some(left) = self.fuse_left.as_mut() {
            *left -= dt;
            if *left <= 0.0 {
                out.explode = true;
                return out;
            }
        }
        if self.resting || self.stuck {
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
            let Some(hit) = sweep(self.position, to) else {
                self.position = to;
                self.velocity = v1;
                break;
            };
            // Stop just short of the surface.
            let t = hit.t.clamp(0.0, 1.0);
            let back = (to - self.position).normalize_or_zero() * 0.5;
            self.position = self.position + (to - self.position) * t - back;
            let v = v0 - Vec3::Z * p.gravity * left * t;
            // Touching anything may shorten the fuse.
            if p.after_impact > 0.0 && self.fuse_left.is_none_or(|f| p.after_impact < f) {
                self.fuse_left = Some(p.after_impact);
            }
            if (hit.actor && p.explode_on_actor) || (!hit.actor && p.explode_on_world) {
                out.explode = true;
                out.hit_actor = hit.actor;
                return out;
            }
            if (hit.actor && p.stick_to_actors) || (!hit.actor && p.stick_to_world) {
                self.stuck = true;
                self.stuck_to_actor = hit.actor;
                self.velocity = Vec3::ZERO;
                out.stuck = true;
                out.hit_actor = hit.actor;
                return out;
            }
            let n = hit.normal.normalize_or_zero();
            let keep = if hit.actor { p.actor_bounce } else { p.bounce };
            self.velocity = (v - 2.0 * v.dot(n) * n) * keep;
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
        out
    }

    /// Whether to check for characters nearby this frame (stuck proximity
    /// grenades). Counts down by `dt`.
    pub fn proximity_due(&mut self, p: &GrenadeParams, dt: f32) -> bool {
        if p.proximity <= 0.0 || !self.stuck {
            return false;
        }
        self.next_check -= dt;
        if self.next_check <= 0.0 {
            self.next_check += p.proximity_interval.max(0.05);
            return true;
        }
        false
    }
}

/// How long a status effect lasts on a character `dist` uu from the blast:
/// the original's rule, full time at the centre falling in a straight line
/// to nothing at `radius` less the character's own radius.
pub fn status_time(max_time: f32, dist: f32, character_radius: f32, radius: f32) -> f32 {
    let reach = radius - character_radius;
    if reach <= 0.0 {
        return 0.0;
    }
    max_time * (1.0 - dist / reach).clamp(0.0, 1.0)
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
        [throw]\nspeed = 1000\ntoss_up = 200\n[fuse]\ntime = 12\nafter_impact = 1\n\
        [flight]\nbounce = 0.5\nactor_bounce = 0.1\ngravity = 1000\n[blast]\ndamage = 100\nradius = 500\n";

    fn floor(a: Vec3, b: Vec3) -> Option<Touch> {
        (b.z < 0.0 && a.z >= 0.0).then(|| Touch {
            t: a.z / (a.z - b.z),
            normal: Vec3::Z,
            actor: false,
        })
    }

    #[test]
    fn reads_a_grenade_file() {
        let p = GrenadeParams::from_text("test", TEST).unwrap();
        assert_eq!((p.fuse_time, p.after_impact), (12.0, 1.0));
        assert_eq!((p.max_count, p.start_count), (4, 2));
        let e = GrenadeParams::from_text("t", &TEST.replace("toss_up", "tos_up")).unwrap_err();
        assert!(e.contains("unknown setting `throw.tos_up`"), "{e}");
    }

    #[test]
    fn flies_in_an_arc_bounces_and_goes_off_a_second_after_landing() {
        let p = GrenadeParams::from_text("test", TEST).unwrap();
        let mut f = Flight::new(
            &p,
            Vec3::new(0.0, 0.0, 100.0),
            Vec3::new(1000.0, 0.0, 200.0),
        );
        let mut highest: f32 = 0.0;
        let mut first_bounce = None;
        let mut went_off = None;
        for i in 0..300 {
            let r = f.step(&p, 1.0 / 60.0, floor);
            highest = highest.max(f.position.z);
            if r.bounced && first_bounce.is_none() {
                first_bounce = Some((i, f.position.x, f.velocity));
            }
            if r.explode {
                went_off = Some(f.age);
                break;
            }
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
        // The 12 s fuse drops to 1 s at the first touch.
        let t = went_off.expect("went off");
        assert!((t - 1.69).abs() < 0.05, "went off at {t} s");
    }

    #[test]
    fn impact_grenades_go_off_on_touch_and_sticky_ones_stick() {
        let wall = |a: Vec3, b: Vec3| {
            (b.x > 300.0).then(|| Touch {
                t: (300.0 - a.x) / (b.x - a.x),
                normal: -Vec3::X,
                actor: false,
            })
        };
        let text = TEST.replace("after_impact = 1", "on_world = true");
        let p = GrenadeParams::from_text("test", &text).unwrap();
        let mut f = Flight::new(&p, Vec3::ZERO, Vec3::new(1000.0, 0.0, 0.0));
        let mut steps = 0;
        while !f.step(&p, 1.0 / 60.0, wall).explode {
            steps += 1;
            assert!(steps < 60);
        }
        assert!((f.position.x - 300.0).abs() < 1.0, "{:?}", f.position);

        let text = TEST
            .replace("[flight]", "[flight]\nstick_to_world = true")
            .replace("after_impact = 1", "proximity = 400");
        let p = GrenadeParams::from_text("test", &text).unwrap();
        let mut f = Flight::new(&p, Vec3::new(0.0, 0.0, 1000.0), Vec3::new(1000.0, 0.0, 0.0));
        let mut stuck = false;
        for _ in 0..60 {
            stuck |= f.step(&p, 1.0 / 60.0, wall).stuck;
        }
        assert!(stuck && f.stuck);
        let x = f.position.x;
        f.step(&p, 1.0, wall);
        assert_eq!(f.position.x, x, "doesn't move once stuck");
        assert!(f.proximity_due(&p, 0.6));
    }

    #[test]
    fn blast_falls_off_with_distance() {
        assert_eq!(blast_scale(0.0, 40.0, 500.0), 1.0);
        assert_eq!(blast_scale(40.0, 40.0, 500.0), 1.0);
        assert!((blast_scale(290.0, 40.0, 500.0) - 0.5).abs() < 1e-6);
        assert_eq!(blast_scale(540.0, 40.0, 500.0), 0.0);
        assert_eq!(blast_scale(600.0, 40.0, 500.0), 0.0);
        assert_eq!(status_time(18.0, 0.0, 40.0, 4096.0), 18.0);
        assert!((status_time(18.0, 2028.0, 40.0, 4096.0) - 9.0).abs() < 1e-3);
        assert_eq!(status_time(18.0, 5000.0, 40.0, 4096.0), 0.0);
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
