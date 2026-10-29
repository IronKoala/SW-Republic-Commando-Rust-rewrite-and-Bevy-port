//! A stand-in level loader, so the game can be played before the level
//! part (the "all levels" work) plugs in its own `LevelPlugin`. It does what
//! `rc_walk --map` does: draws the map with the level viewer's code and makes
//! its BSP and static meshes solid. It is switched off when a real level
//! plugin is added (see `main.rs`), and should be deleted then.
//!
//! The collision loading below is copied from `rc_walk/src/level.rs`.

use bevy::prelude::*;
use rc_game_core::{
    CurrentLevel, GameLog, GamePaths, GameState, LevelCollision, LoadingGate, PlayerStart,
};
use rc_level::actors::{self, Placement};
use rc_level::bsp;
use rc_level::collision::collision_triangles;
use rc_level_viewer::lighting::LightingSettings;
use rc_level_viewer::{draw, materials, scene};
use rc_movement::{TriangleWorld, Vec3};
use rc_package::static_mesh::{self, StaticMesh};
use rc_package::{ObjectRef, Package};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

const PART: &str = "level (stand-in)";

pub struct StandInLevelPlugin;

impl Plugin for StandInLevelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Loading), begin)
            .add_systems(Update, load.run_if(in_state(GameState::Loading)));
    }
}

/// Frames to wait in the loading screen before the (slow, blocking) load,
/// so the loading screen is drawn first.
#[derive(Resource)]
struct Countdown(u32);

fn begin(mut commands: Commands, mut gate: ResMut<LoadingGate>) {
    gate.wait(PART);
    commands.insert_resource(Countdown(3));
}

fn load(world: &mut World) {
    {
        let Some(mut c) = world.get_resource_mut::<Countdown>() else {
            return;
        };
        if c.0 > 0 {
            c.0 -= 1;
            return;
        }
    }
    world.remove_resource::<Countdown>();
    let log = world.resource::<GameLog>().clone();
    let paths = world.resource::<GamePaths>().clone();
    let Some(map) = world.resource::<CurrentLevel>().map.clone() else {
        world
            .resource_mut::<LoadingGate>()
            .fail(PART, "no map chosen");
        return;
    };
    let path = paths.map_path(&map);
    let game_data = paths.game_data();
    let t = Instant::now();
    let lvl = match load_collision(&path, &game_data, 84.0) {
        Ok(l) => l,
        Err(e) => {
            log.line(format!("{PART}: ERROR loading {}: {e}", path.display()));
            world.resource_mut::<LoadingGate>().fail(PART, e);
            return;
        }
    };
    log.line(format!(
        "{PART}: {} loaded: {} collision triangles, {} props ({} unreadable), {:.0} ms",
        path.display(),
        lvl.triangles.len(),
        lvl.mesh_actors,
        lvl.mesh_failures,
        t.elapsed().as_secs_f64() * 1000.0
    ));
    let tw = TriangleWorld::new(lvl.triangles);
    let (feet, yaw) = lvl.start.unwrap_or_else(|| {
        let (lo, hi) = tw.bounds();
        log.line(format!("{PART}: no PlayerStart, starting above the middle"));
        (
            Vec3::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0, hi.z),
            0.0,
        )
    });
    world.insert_resource(PlayerStart { feet, yaw });
    world.insert_resource(LevelCollision(Arc::new(tw)));

    // Draw it, and mark everything drawn to be cleared when the map ends.
    let before: HashSet<Entity> = world.query::<Entity>().iter(world).collect();
    let t = Instant::now();
    match build_look(&path, &game_data) {
        Ok((sc, settings)) => {
            let report = world.resource_scope(|world, mut meshes: Mut<Assets<Mesh>>| {
                world.resource_scope(|world, mut images: Mut<Assets<Image>>| {
                    world.resource_scope(|world, mut mats: Mut<Assets<StandardMaterial>>| {
                        let mut commands = world.commands();
                        draw::spawn_level(
                            &mut commands,
                            &mut meshes,
                            &mut images,
                            &mut mats,
                            &sc,
                            &settings,
                        )
                    })
                })
            });
            world.flush();
            log.line(format!(
                "{PART}: drawn in {:.0} ms: {} triangles, {} textured batches, {} lightmapped, {} lights",
                t.elapsed().as_secs_f64() * 1000.0,
                report.triangles,
                report.textured,
                report.lightmapped,
                report.lights.point + report.lights.spot + report.lights.directional
            ));
        }
        Err(e) => log.line(format!(
            "{PART}: map look not built ({e}); it will be invisible"
        )),
    }
    let new: Vec<Entity> = world
        .query::<Entity>()
        .iter(world)
        .filter(|e| !before.contains(e))
        .collect();
    for e in &new {
        world
            .entity_mut(*e)
            .insert(DespawnOnExit(GameState::InGame));
    }
    world.resource_mut::<LoadingGate>().done(PART);
}

fn build_look(
    map_path: &Path,
    game_data: &Path,
) -> Result<(scene::Scene, LightingSettings), String> {
    let (settings, _) = LightingSettings::load(Path::new("lighting.cfg"));
    let data = std::fs::read(map_path).map_err(|e| e.to_string())?;
    let pkg = Package::parse(&data).map_err(|e| e.to_string())?;
    let mut mats = materials::Materials::new(game_data);
    let sc = scene::build(Rc::new((pkg, data)), &mut mats)?;
    Ok((sc, settings))
}

struct Level {
    triangles: Vec<[Vec3; 3]>,
    start: Option<(Vec3, f32)>,
    mesh_actors: usize,
    mesh_failures: usize,
}

fn index_packages(dir: &Path, out: &mut HashMap<String, PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            index_packages(&p, out);
        } else if let Some(stem) = p.file_stem() {
            out.entry(stem.to_string_lossy().to_ascii_lowercase())
                .or_insert(p);
        }
    }
}

fn object_path(pkg: &Package, index: i32) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = index;
    for _ in 0..16 {
        match ObjectRef::from_index(i) {
            ObjectRef::None => break,
            ObjectRef::Import(k) => {
                let Some(m) = pkg.imports.get(k) else { break };
                out.push(pkg.name(m.object_name).to_string());
                i = m.package_index;
            }
            ObjectRef::Export(k) => {
                let Some(e) = pkg.exports.get(k) else { break };
                out.push(pkg.name(e.object_name).to_string());
                i = e.package_index;
            }
        }
    }
    out.reverse();
    out
}

struct Packages {
    files: HashMap<String, PathBuf>,
    open: HashMap<String, Option<(Package, Vec<u8>)>>,
}

impl Packages {
    fn get(&mut self, name: &str) -> Option<&(Package, Vec<u8>)> {
        let key = name.to_ascii_lowercase();
        if !self.open.contains_key(&key) {
            let loaded = self.files.get(&key).and_then(|p| {
                let data = std::fs::read(p).ok()?;
                let pkg = Package::parse(&data).ok()?;
                Some((pkg, data))
            });
            self.open.insert(key.clone(), loaded);
        }
        self.open.get(&key).and_then(|o| o.as_ref())
    }
}

fn load_mesh(pkgs: &mut Packages, map: &(Package, Vec<u8>), index: i32) -> Option<StaticMesh> {
    match ObjectRef::from_index(index) {
        ObjectRef::Export(k) => {
            static_mesh::read_static_mesh(&map.0, &map.1, &map.0.exports[k]).ok()
        }
        ObjectRef::Import(_) => {
            let path = object_path(&map.0, index);
            let (owner, data) = pkgs.get(&path[0])?;
            let rest = &path[1..];
            let k = (0..owner.exports.len()).find(|&i| {
                let p = object_path(owner, i as i32 + 1);
                p.len() == rest.len() && p.iter().zip(rest).all(|(a, b)| a.eq_ignore_ascii_case(b))
            })?;
            static_mesh::read_static_mesh(owner, data, &owner.exports[k]).ok()
        }
        ObjectRef::None => None,
    }
}

fn load_collision(
    map_path: &Path,
    game_data: &Path,
    collision_height: f32,
) -> Result<Level, String> {
    let data =
        std::fs::read(map_path).map_err(|e| format!("can't read {}: {e}", map_path.display()))?;
    let pkg = Package::parse(&data).map_err(|e| e.to_string())?;
    let level = actors::read_level(&pkg, &data).map_err(|e| format!("level: {e}"))?;
    let model = level.model.ok_or("the level has no BSP model")?;
    let bsp = bsp::read_model(&pkg, &data, &pkg.exports[model]).map_err(|e| format!("BSP: {e}"))?;
    let mut files = HashMap::new();
    index_packages(game_data, &mut files);
    let mut pkgs = Packages {
        files,
        open: HashMap::new(),
    };
    let map = (pkg, data);
    let mut cache: HashMap<i32, Option<StaticMesh>> = HashMap::new();
    let mut placed: Vec<(Placement, i32)> = Vec::new();
    let mut start = None;
    let mut mesh_failures = 0;
    for &a in level.actors.iter().flatten() {
        let Ok(p) = actors::read_placement(&map.0, &map.1, a) else {
            continue;
        };
        if p.class == "PlayerStart" && start.is_none() {
            let feet = Vec3::new(
                p.location[0],
                p.location[1],
                p.location[2] - collision_height,
            );
            let yaw = p.rotation[1] as f32 / 65536.0 * std::f32::consts::TAU;
            start = Some((feet, yaw));
        }
        if p.static_mesh == 0 || p.hidden {
            continue;
        }
        let mesh = cache
            .entry(p.static_mesh)
            .or_insert_with(|| load_mesh(&mut pkgs, &map, p.static_mesh));
        if mesh.is_some() {
            let index = p.static_mesh;
            placed.push((p, index));
        } else {
            mesh_failures += 1;
        }
    }
    let pairs: Vec<(&Placement, &StaticMesh)> = placed
        .iter()
        .filter_map(|(p, i)| cache.get(i).and_then(|m| m.as_ref()).map(|m| (p, m)))
        .collect();
    let mesh = collision_triangles(&bsp, &pairs);
    let v = |p: [f32; 3]| Vec3::new(p[0], p[1], p[2]);
    Ok(Level {
        triangles: mesh
            .triangles
            .iter()
            .map(|t| [v(t[0]), v(t[1]), v(t[2])])
            .collect(),
        start,
        mesh_actors: pairs.len(),
        mesh_failures,
    })
}
