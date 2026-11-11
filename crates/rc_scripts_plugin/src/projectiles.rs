//! Projectiles made by the level's projectile spawners
//! (`ProjectileFactory`, run with the effect spawners in `fx_factories`):
//! geo_01a's bombardment falling on the arena (`GEO_01AProj`) and the
//! bunkers' missiles (`GEO_01A_BunkerMissile`), rockets and grenades in
//! other levels. What a projectile class is and how it flies and blows up
//! is `rc_effects::projectile` (the rules, from the game's own script
//! source, are written there); this draws them, carries their trail
//! effect, checks them against the level's walls and floors, and on a blast
//! shows the hit effect and does the damage (`rc_combat::Explode`).
//!
//! Projectiles in flight are not kept in save files (as effects aren't).
//! Logged with the `scripts:` prefix.

use crate::PART;
use bevy::prelude::*;
use rc_combat::{DamageKind, Explode};
use rc_effects::projectile::{ClassPackages, End, Flight, ProjectileDef};
use rc_effects_plugin::{EffectEnded, SpawnEffect};
use rc_game_core::units::to_bevy;
use rc_game_core::{GameLog, GamePaths, LevelCollision};
use rc_movement::Vec3 as UVec3;
use std::collections::HashMap;
use std::sync::Arc;

/// "Make this projectile here now", from a projectile spawner.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct SpawnProjectile {
    /// The class (`GEO_01AProj`).
    pub class: String,
    /// Unreal units and rotation.
    pub location: [f32; 3],
    pub rotation: [i32; 3],
    /// Unreal `EPhysics` number and speed along `rotation`.
    pub physics: u8,
    pub speed: f32,
    /// Its number; its end is reported as an [`EffectEnded`] with it.
    pub id: u64,
    /// The spawner, for the log.
    pub by: String,
}

/// "Remove these projectiles now" (`DestroyFactoryOffspring`), by number.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct RemoveProjectiles(pub Vec<u64>);

/// One projectile flying.
#[derive(Component)]
pub struct Missile {
    pub def: Arc<ProjectileDef>,
    pub flight: Flight,
    pub id: u64,
    pub by: String,
}

/// Projectile classes read so far, and what happened (for the self-tests
/// and the log).
#[derive(Resource, Default)]
pub struct Projectiles {
    lib: Option<Result<ClassPackages, String>>,
    classes: HashMap<String, Option<Loaded>>,
    /// Made: (spawner, class).
    pub made: Vec<(String, String)>,
    /// Blasts: (class, Unreal place, hit the level).
    pub blasts: Vec<(String, [f32; 3], bool)>,
    /// Highest point any projectile reached, Unreal Z.
    pub highest: Option<f32>,
}

#[derive(Clone)]
struct Loaded {
    def: Arc<ProjectileDef>,
    /// Model sections, in the model's own units turned to Bevy's axes.
    model: Vec<(Handle<Mesh>, Handle<StandardMaterial>)>,
}

impl Projectiles {
    /// Read a class once (and its model). `None` if it can't be read or
    /// isn't one we fly (logged once).
    #[allow(clippy::too_many_arguments)]
    fn load(
        &mut self,
        class: &str,
        paths: &GamePaths,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        images: &mut Assets<Image>,
        log: &GameLog,
    ) -> Option<Loaded> {
        let key = class.to_ascii_lowercase();
        if let Some(l) = self.classes.get(&key) {
            return l.clone();
        }
        let lib = self
            .lib
            .get_or_insert_with(|| ClassPackages::open(&paths.game_data()));
        let loaded = match lib {
            Err(e) => {
                log.line(format!("{PART}: projectiles can't be read: {e}"));
                None
            }
            Ok(lib) => match lib.projectile(class) {
                Err(e) => {
                    log.line(format!("{PART}: projectile {class} can't be read: {e}"));
                    None
                }
                Ok(def) if !def.explosive || def.laser_bolt => {
                    log.line(format!(
                        "{PART}: projectile {class} ({}) isn't flown yet: only explosive ones are",
                        def.chain.join(" < ")
                    ));
                    None
                }
                Ok(def) => {
                    let mut model = Vec::new();
                    if let Some(path) = &def.mesh {
                        let mut mats =
                            rc_level_viewer::materials::Materials::new(&paths.game_data());
                        match rc_effects_plugin::static_mesh(&mut mats, path) {
                            Ok(sections) => {
                                for (mesh, pic) in sections {
                                    // `Projectile` is drawn unlit.
                                    let material = StandardMaterial {
                                        base_color_texture: pic.map(|p| {
                                            images.add(rc_level_viewer::draw::to_image(&p))
                                        }),
                                        unlit: true,
                                        double_sided: true,
                                        cull_mode: None,
                                        ..default()
                                    };
                                    model.push((meshes.add(mesh), materials.add(material)));
                                }
                            }
                            Err(e) => log.line(format!(
                                "{PART}: projectile {class}: model {} can't be read: {e}",
                                path.join(".")
                            )),
                        }
                    }
                    log.line(format!(
                        "{PART}: projectile {class} ({}): model {}, trail {}, blast effect {}, damage {} within {} uu ({}), fuse {} s, life {} s{}{}",
                        def.chain.join(" < "),
                        def.mesh.as_ref().map_or("none".into(), |m| m.join(".")),
                        def.trail.as_deref().unwrap_or("none"),
                        def.hit_effect.as_deref().unwrap_or("none"),
                        def.damage,
                        def.damage_radius,
                        def.damage_type,
                        def.fuse_time,
                        def.life_span,
                        if def.world_detonate { ", blows up on the level" } else { "" },
                        if def.actor_detonate { ", blows up on characters (not done)" } else { "" },
                    ));
                    Some(Loaded {
                        def: Arc::new(def),
                        model,
                    })
                }
            },
        };
        self.classes.insert(key, loaded.clone());
        loaded
    }
}

/// Unreal axes (forward, right, up) to a Bevy turn, for a model whose
/// points are in Bevy axes (x, z, y) of its own units.
fn bevy_turn(rotation: [i32; 3]) -> Quat {
    let ax = rc_effects::sim::rotation_axes(rotation);
    let b = |v: [f32; 3]| Vec3::new(v[0], v[2], v[1]);
    Quat::from_mat3(&Mat3::from_cols(b(ax[0]), b(ax[2]), b(ax[1])))
}

/// Unreal rotation pointing along `dir` (`rotator(dir)`: pitch and yaw).
pub fn rotator(dir: [f32; 3]) -> [i32; 3] {
    let k = 32768.0 / std::f32::consts::PI;
    let yaw = dir[1].atan2(dir[0]) * k;
    let pitch = dir[2].atan2((dir[0] * dir[0] + dir[1] * dir[1]).sqrt()) * k;
    [pitch.round() as i32, yaw.round() as i32, 0]
}

/// Make, fly and blow up projectiles.
#[allow(clippy::too_many_arguments)]
pub fn fly(
    mut commands: Commands,
    time: Res<Time>,
    mut state: ResMut<Projectiles>,
    mut asks: MessageReader<SpawnProjectile>,
    mut removes: MessageReader<RemoveProjectiles>,
    mut missiles: Query<(Entity, &mut Missile, &mut Transform)>,
    collision: Option<Res<LevelCollision>>,
    paths: Option<Res<GamePaths>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    images: Option<ResMut<Assets<Image>>>,
    mut effects: MessageWriter<SpawnEffect>,
    mut ended: MessageWriter<EffectEnded>,
    mut explode: MessageWriter<Explode>,
    log: Res<GameLog>,
) {
    let dt = time.delta_secs().min(0.1);
    let mut gone: Vec<u64> = removes.read().flat_map(|r| r.0.clone()).collect();
    let (Some(paths), Some(mut meshes), Some(mut materials), Some(mut images)) =
        (paths, meshes, materials, images)
    else {
        for a in asks.read() {
            ended.write(EffectEnded {
                id: a.id,
                by: a.by.clone(),
            });
        }
        return;
    };
    for a in asks.read() {
        let Some(l) = state.load(
            &a.class,
            &paths,
            &mut meshes,
            &mut materials,
            &mut images,
            &log,
        ) else {
            ended.write(EffectEnded {
                id: a.id,
                by: a.by.clone(),
            });
            continue;
        };
        let flight = Flight::new(a.location, a.rotation, a.physics, a.speed);
        let at = to_bevy(UVec3::new(a.location[0], a.location[1], a.location[2]));
        // Drawn turned as made (a projectile keeps its turn in flight).
        let turn = bevy_turn(a.rotation);
        let e = commands
            .spawn((
                Missile {
                    def: l.def.clone(),
                    flight,
                    id: a.id,
                    by: a.by.clone(),
                },
                Transform::from_translation(at)
                    .with_rotation(turn)
                    .with_scale(Vec3::splat(l.def.draw_scale)),
                Visibility::default(),
                DespawnOnExit(rc_game_core::GameState::InGame),
            ))
            .with_children(|c| {
                for (m, mat) in &l.model {
                    c.spawn((Mesh3d(m.clone()), MeshMaterial3d(mat.clone())));
                }
            })
            .id();
        if let Some(trail) = &l.def.trail {
            effects.write(SpawnEffect {
                effect: trail.clone(),
                at,
                by: format!("{} ({})", l.def.class, a.by),
                rotation: a.rotation,
                follow: Some(e),
                ..default()
            });
        }
        state.made.push((a.by.clone(), l.def.class.clone()));
    }
    for (e, mut m, mut t) in &mut missiles {
        if gone.contains(&m.id) {
            gone.retain(|g| *g != m.id);
            log.line(format!(
                "{PART}: projectile {} of {} removed by the level script",
                m.def.class, m.by
            ));
            commands.entity(e).despawn();
            ended.write(EffectEnded {
                id: m.id,
                by: m.by.clone(),
            });
            continue;
        }
        let def = m.def.clone();
        let world = collision.as_deref();
        let mut hit = |a: [f32; 3], b: [f32; 3]| {
            let w = world?;
            let h = w.0.sweep(
                UVec3::ZERO,
                UVec3::new(a[0], a[1], a[2]),
                UVec3::new(b[0], b[1], b[2]),
            )?;
            (h.time < 1.0).then_some((h.time.max(0.0), [h.normal.x, h.normal.y, h.normal.z]))
        };
        let end = m.flight.step(&def, dt, &mut hit);
        let p = m.flight.location;
        t.translation = to_bevy(UVec3::new(p[0], p[1], p[2]));
        state.highest = Some(state.highest.map_or(p[2], |h| h.max(p[2])));
        let Some(end) = end else { continue };
        if let End::Blast {
            at,
            normal,
            hit_world,
        } = end
        {
            let point = to_bevy(UVec3::new(at[0], at[1], at[2]));
            if let Some(fx) = &def.hit_effect {
                // `HitWall` shows it, then `Explode` shows it again.
                for _ in 0..if hit_world { 2 } else { 1 } {
                    effects.write(SpawnEffect {
                        effect: fx.clone(),
                        at: point,
                        by: format!("{}'s blast", def.class),
                        rotation: rotator(normal),
                        ..default()
                    });
                }
            }
            if def.damage > 0.0 && def.damage_radius > 0.0 {
                explode.write(Explode {
                    point,
                    damage: def.damage,
                    radius: def.damage_radius,
                    damage_type: def.damage_type.clone(),
                    kind: DamageKind::Explosion,
                    instigator: None,
                    what: def.class.clone(),
                });
            }
            log.line(format!(
                "{PART}: projectile {} of {} blows up at ({:.0}, {:.0}, {:.0}) after {:.2} s{}",
                def.class,
                m.by,
                at[0],
                at[1],
                at[2],
                m.flight.age,
                if hit_world {
                    ", on the level"
                } else {
                    ", its fuse ran out"
                }
            ));
            state.blasts.push((def.class.clone(), at, hit_world));
        }
        commands.entity(e).despawn();
        ended.write(EffectEnded {
            id: m.id,
            by: m.by.clone(),
        });
    }
}

/// Forget the level's projectiles' records (classes stay read).
pub fn forget(mut state: ResMut<Projectiles>) {
    state.made.clear();
    state.blasts.clear();
    state.highest = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotator_points_along_a_direction() {
        assert_eq!(rotator([1.0, 0.0, 0.0]), [0, 0, 0]);
        assert_eq!(rotator([0.0, 1.0, 0.0]), [0, 16384, 0]);
        assert_eq!(rotator([0.0, 0.0, -1.0])[0], -16384);
        // And back: the forward axis of the rotation is the direction.
        let d = [0.3f32, -0.5, -0.8];
        let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let f = rc_effects::sim::rotation_axes(rotator(d))[0];
        for k in 0..3 {
            assert!((f[k] - d[k] / n).abs() < 1e-3, "{f:?}");
        }
    }

    #[test]
    fn the_model_turns_like_the_level_axes() {
        // Yaw 90 degrees: the model's forward (Unreal X) points along
        // Unreal Y, which is Bevy Z.
        let q = bevy_turn([0, 16384, 0]);
        let fwd = q * Vec3::X;
        assert!((fwd - Vec3::Z).length() < 1e-3, "{fwd:?}");
    }
}
