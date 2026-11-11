//! Projectiles made by projectile spawners (`ProjectileFactory`): what a
//! projectile class is (read from the game's script packages) and how it
//! flies and blows up. Plain Rust, Unreal units; `rc_scripts_plugin` draws
//! them and does the damage.
//!
//! How the original works, from the script source in `Engine.u`
//! (`Projectile`) and `CTInventory.u` (`CTProjExplosive`; read only,
//! described in our own words):
//!
//! - It flies by its physics: `PHYS_Projectile` straight on at its
//!   speed, `PHYS_Falling` pulled down by the zone's gravity. A spawner
//!   sets both (see `factory`).
//! - On the way it carries its `ProjectileEffect` (an effect fixed to it);
//!   when the projectile is gone that effect fades out (sends no more and
//!   ends when its last particle dies).
//! - An explosive one (`CTProjExplosive`) blows up when its `FuseTime` runs
//!   out, when it hits the level and `bWorldDetonate` is set, or when it
//!   touches a character and `bActorDetonate` is set. Blowing up shows its
//!   hit effect: the `HitEffectsByMaterial` class's effect for the surface
//!   (its `DefaultMaterialEffect` when the surface has none of its own),
//!   else `ExplosionEffect`; and hurts everything within `DamageRadius`
//!   (`HurtRadius`, `Damage` falling off with distance, `MyDamageType`).
//!   Hitting the level shows the hit effect twice: once from `HitWall`
//!   (`SpawnEffect`) and once from `Explode`.
//! - Its `LifeSpan` ends it without a blast.
//!
//! Not done: bouncing (explosive projectiles without `bWorldDetonate`,
//! thermal detonators) and sticking; touching characters (`bActorDetonate`);
//! laser bolts (`bLaserBolt`) and projectiles that aren't explosive; a
//! zone's own gravity (the `PhysicsVolume` default, -1100, is used); hit
//! decals (`HitDecal`) and the surface's own material.

use crate::read::EffectPackage;
use crate::tagged::{Tagged, Values, boolean, float, get};

/// What a projectile class is, read from its class and its parents.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectileDef {
    pub class: String,
    /// Its class and parents, class first (`GEO_01AProj`,
    /// `CTProjExplosive`, `Projectile`, ...).
    pub chain: Vec<String>,
    /// Model drawn (`StaticMesh`, path package first) when `DrawType` is a
    /// static mesh.
    pub mesh: Option<Vec<String>>,
    pub draw_scale: f32,
    /// The effect it carries (`ProjectileEffect`), class name.
    pub trail: Option<String>,
    /// The effect shown when it blows up, class name.
    pub hit_effect: Option<String>,
    pub explosive: bool,
    pub laser_bolt: bool,
    pub damage: f32,
    pub damage_radius: f32,
    pub damage_type: String,
    pub fuse_time: f32,
    pub life_span: f32,
    pub world_detonate: bool,
    pub actor_detonate: bool,
}

/// `PhysicsVolume`'s default gravity (Unreal Z, uu/s²), `Engine.u`.
pub const GRAVITY_Z: f32 = -1100.0;
/// `PhysicsVolume`'s `TerminalVelocity`.
pub const TERMINAL_VELOCITY: f32 = 12000.0;

/// `DrawType` 8, a static mesh.
const DT_STATIC_MESH: u8 = 8;

/// The packages projectile classes and their parents live in, read once.
pub struct ClassPackages {
    pub packages: Vec<EffectPackage>,
}

impl ClassPackages {
    /// `GameData/Properties/properties.u` and `physicalmaterials.u`,
    /// `GameData/System/ctinventory.u` and `engine.u`.
    pub fn open(game_data: &std::path::Path) -> Result<ClassPackages, String> {
        let mut files = Vec::new();
        for (dir, name) in [
            ("Properties", "properties"),
            ("Properties", "physicalmaterials"),
            ("System", "ctinventory"),
            ("System", "engine"),
        ] {
            let path = game_data.join(dir).join(format!("{name}.u"));
            let data =
                std::fs::read(&path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
            files.push((name, data));
        }
        // A class's values are named by properties its parents declare,
        // which may be in another of these packages.
        let mut names = Vec::new();
        for (_, data) in &files {
            if let Ok(p) = rc_package::Package::parse(data) {
                names.extend(rc_package::class_defaults::property_names(&p));
            }
        }
        let packages = files
            .into_iter()
            .map(|(name, data)| EffectPackage::new(name, data, &names))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ClassPackages { packages })
    }

    /// A class's values, its own on top of its parents', and the chain.
    pub fn values(&self, class: &str) -> Option<(Values, Vec<String>)> {
        let mut chain = Vec::new();
        let mut levels = Vec::new();
        let mut name = class.to_string();
        for _ in 0..16 {
            let Some(p) = self.packages.iter().find(|p| p.has(&name)) else {
                break;
            };
            levels.push(p.class_values(&name).unwrap_or_default());
            chain.push(name.clone());
            match p.parent(&name) {
                Some(parent) => name = parent,
                None => break,
            }
        }
        if chain.is_empty() {
            return None;
        }
        let mut all = Values::new();
        for v in levels.into_iter().rev() {
            all.extend(v);
        }
        Some((all, chain))
    }

    /// Read a projectile class.
    pub fn projectile(&self, class: &str) -> Result<ProjectileDef, String> {
        let (v, chain) = self
            .values(class)
            .ok_or_else(|| format!("no class {class}"))?;
        let object = |k: &str| match get(&v, k) {
            Some(Tagged::Object(p)) if !p.is_empty() => Some(p.clone()),
            _ => None,
        };
        let class_name = |k: &str| object(k).and_then(|p| p.last().cloned());
        let draw_type = match get(&v, "DrawType") {
            Some(Tagged::Byte(b)) => *b,
            _ => 0,
        };
        let explosion = class_name("ExplosionEffect");
        let hit_effect = class_name("HitEffectsByMaterial")
            .and_then(|m| self.material_effect(&m))
            .or(explosion);
        let has = |c: &str| chain.iter().any(|n| n.eq_ignore_ascii_case(c));
        Ok(ProjectileDef {
            class: chain[0].clone(),
            mesh: (draw_type == DT_STATIC_MESH)
                .then(|| object("StaticMesh"))
                .flatten(),
            draw_scale: float(&v, "DrawScale").unwrap_or(1.0),
            trail: class_name("ProjectileEffect"),
            hit_effect,
            explosive: has("CTProjExplosive"),
            laser_bolt: boolean(&v, "bLaserBolt").unwrap_or(false),
            damage: float(&v, "Damage").unwrap_or(0.0),
            damage_radius: float(&v, "DamageRadius").unwrap_or(0.0),
            damage_type: class_name("MyDamageType").unwrap_or_default(),
            fuse_time: float(&v, "FuseTime").unwrap_or(0.0),
            life_span: float(&v, "LifeSpan").unwrap_or(0.0),
            world_detonate: boolean(&v, "bWorldDetonate").unwrap_or(false),
            actor_detonate: boolean(&v, "bActorDetonate").unwrap_or(false),
            chain,
        })
    }

    /// The effect a `MaterialTypeInfo` class shows on a surface with no
    /// entry of its own: its `DefaultMaterialEffect`'s `Emitter`.
    pub fn material_effect(&self, class: &str) -> Option<String> {
        let (v, _) = self.values(class)?;
        match get(&v, "DefaultMaterialEffect") {
            Some(Tagged::Struct(s)) => match get(s, "Emitter") {
                Some(Tagged::Object(p)) => p.last().cloned(),
                _ => None,
            },
            _ => None,
        }
    }
}

/// How a flight ended, if it did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum End {
    /// Blew up at this place; `normal` faces away from what it hit
    /// (straight up for a fuse running out in the air).
    Blast {
        at: [f32; 3],
        normal: [f32; 3],
        hit_world: bool,
    },
    /// Its life span ran out, or it hit the level and doesn't blow up on
    /// that (not done: it should bounce); removed without a blast.
    Gone,
}

/// One projectile in flight.
#[derive(Debug, Clone, PartialEq)]
pub struct Flight {
    pub location: [f32; 3],
    pub velocity: [f32; 3],
    pub physics: u8,
    pub age: f32,
}

impl Flight {
    /// Made at `location`, turned by `rotation`, given `physics` and
    /// `speed` by its spawner.
    pub fn new(location: [f32; 3], rotation: [i32; 3], physics: u8, speed: f32) -> Self {
        let forward = crate::sim::rotation_axes(rotation)[0];
        Flight {
            location,
            velocity: forward.map(|f| f * speed),
            physics,
            age: 0.0,
        }
    }

    /// Where it would be after `dt` seconds (doesn't move it).
    pub fn next(&self, dt: f32) -> ([f32; 3], [f32; 3]) {
        let mut v = self.velocity;
        if self.physics == crate::factory::PHYS_FALLING {
            v[2] = (v[2] + GRAVITY_Z * dt).max(-TERMINAL_VELOCITY);
        }
        let to = [0, 1, 2].map(|k| self.location[k] + (self.velocity[k] + v[k]) * 0.5 * dt);
        (to, v)
    }

    /// Move on by `dt`. `hit` tells where the move from `a` to `b` first
    /// hits the level: the fraction of the way, and the surface's normal.
    pub fn step(
        &mut self,
        def: &ProjectileDef,
        dt: f32,
        hit: &mut dyn FnMut([f32; 3], [f32; 3]) -> Option<(f32, [f32; 3])>,
    ) -> Option<End> {
        self.age += dt;
        let moves = matches!(
            self.physics,
            crate::factory::PHYS_FALLING | crate::factory::PHYS_PROJECTILE
        );
        if moves {
            let (to, v) = self.next(dt);
            if let Some((t, normal)) = hit(self.location, to) {
                let at = [0, 1, 2].map(|k| self.location[k] + (to[k] - self.location[k]) * t);
                self.location = at;
                return Some(if def.explosive && def.world_detonate {
                    End::Blast {
                        at,
                        normal,
                        hit_world: true,
                    }
                } else {
                    End::Gone
                });
            }
            self.location = to;
            self.velocity = v;
        }
        if def.explosive && def.fuse_time > 0.0 && self.age >= def.fuse_time {
            return Some(End::Blast {
                at: self.location,
                normal: [0.0, 0.0, 1.0],
                hit_world: false,
            });
        }
        if def.life_span > 0.0 && self.age >= def.life_span {
            return Some(End::Gone);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::{PHYS_FALLING, PHYS_PROJECTILE};

    fn bombardment() -> ProjectileDef {
        // GEO_01AProj as read from the game (properties.u, on top of
        // CTProjExplosive and Projectile).
        ProjectileDef {
            class: "GEO_01AProj".into(),
            chain: vec!["GEO_01AProj".into(), "CTProjExplosive".into()],
            mesh: None,
            draw_scale: 1.0,
            trail: Some("GEO_01AProjectile".into()),
            hit_effect: Some("Explosion_Barrel_AlongX".into()),
            explosive: true,
            laser_bolt: false,
            damage: 400.0,
            damage_radius: 512.0,
            damage_type: "CTDamageExplosion".into(),
            fuse_time: 15.0,
            life_span: 15.0,
            world_detonate: true,
            actor_detonate: true,
        }
    }

    /// A flat floor at Z = 0.
    fn floor(a: [f32; 3], b: [f32; 3]) -> Option<(f32, [f32; 3])> {
        (a[2] > 0.0 && b[2] <= 0.0).then(|| (a[2] / (a[2] - b[2]), [0.0, 0.0, 1.0]))
    }

    #[test]
    fn flies_straight_and_blows_up_on_the_ground() {
        let def = bombardment();
        // Pointing straight down (pitch -16384) at 10000 uu/s from 21000 up.
        let mut f = Flight::new(
            [0.0, 0.0, 21000.0],
            [-16384, 0, 0],
            PHYS_PROJECTILE,
            10000.0,
        );
        let mut end = None;
        for _ in 0..300 {
            end = f.step(&def, 1.0 / 60.0, &mut floor);
            if end.is_some() {
                break;
            }
        }
        match end {
            Some(End::Blast { at, hit_world, .. }) => {
                assert!(hit_world);
                assert!(at[2].abs() < 1.0 && at[0].abs() < 1.0, "{at:?}");
            }
            e => panic!("{e:?}"),
        }
        // 21000 uu at 10000 uu/s: about 2.1 s.
        assert!((f.age - 2.1).abs() < 0.05, "{}", f.age);
    }

    #[test]
    fn falling_speeds_up_and_the_fuse_blows_it_in_the_air() {
        let mut def = bombardment();
        def.fuse_time = 1.0;
        let mut f = Flight::new([0.0, 0.0, 50000.0], [0; 3], PHYS_FALLING, 0.0);
        let mut end = None;
        for _ in 0..61 {
            end = f.step(&def, 1.0 / 60.0, &mut floor).or(end);
        }
        assert!(matches!(
            end,
            Some(End::Blast {
                hit_world: false,
                ..
            })
        ));
        // About 1100 uu/s down after a second, 550 uu fallen.
        assert!((f.velocity[2] + 1100.0).abs() < 25.0, "{:?}", f.velocity);
        assert!(
            (50000.0 - f.location[2] - 550.0).abs() < 20.0,
            "{:?}",
            f.location
        );
    }

    #[test]
    fn not_blowing_up_on_the_world_means_gone() {
        let mut def = bombardment();
        def.world_detonate = false;
        let mut f = Flight::new([0.0, 0.0, 100.0], [-16384, 0, 0], PHYS_PROJECTILE, 5000.0);
        let mut end = None;
        for _ in 0..10 {
            end = f.step(&def, 1.0 / 60.0, &mut floor).or(end);
        }
        assert_eq!(end, Some(End::Gone));
    }
}

#[cfg(test)]
mod game_files {
    use super::*;

    /// With the game's files (`RC_GAME_DIR`): geo_01a's two projectile
    /// classes read as the game has them.
    #[test]
    fn reads_geo_01a_projectiles() {
        let Some(dir) = std::env::var_os("RC_GAME_DIR") else {
            println!("skipped: no game folder");
            return;
        };
        let lib = ClassPackages::open(&std::path::Path::new(&dir).join("GameData")).unwrap();
        let p = lib.projectile("GEO_01AProj").unwrap();
        println!("{p:?}");
        assert_eq!(p.hit_effect.as_deref(), Some("Explosion_Barrel_AlongX"));
        assert_eq!(p.trail.as_deref(), Some("GEO_01AProjectile"));
        assert_eq!(
            (p.damage, p.damage_radius, p.fuse_time),
            (400.0, 512.0, 15.0)
        );
        assert!(p.explosive && p.world_detonate && p.actor_detonate);
        assert!(
            p.mesh
                .as_ref()
                .is_some_and(|m| m.last().unwrap() == "TranMissile")
        );
        let b = lib.projectile("GEO_01A_BunkerMissile").unwrap();
        println!("{b:?}");
        assert!(b.world_detonate && !b.actor_detonate);
    }
}
