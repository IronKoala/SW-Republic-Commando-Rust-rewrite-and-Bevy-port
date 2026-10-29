//! Load a map's collision from disk: its BSP walls and floors plus every
//! placed static mesh, as triangles (via `crate::collision`), and where the
//! player starts. Unlike the rest of this crate it opens files, because the
//! static meshes live in other packages under `GameData`. Coordinates stay
//! in Unreal units (X forward, Y right, Z up).
//!
//! Moved here from `rc_walk` so other programs (the droid AI tests, the
//! game) can load the same collision without Bevy.

use crate::actors::{self, Placement};
use crate::bsp::{self, Vec3};
use crate::collision::collision_triangles;
use rc_package::static_mesh::{self, StaticMesh};
use rc_package::{ObjectRef, Package};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct MapCollision {
    pub triangles: Vec<[Vec3; 3]>,
    /// Feet position and facing (radians, Unreal yaw) of the first
    /// PlayerStart.
    pub start: Option<(Vec3, f32)>,
    pub bsp_nodes: usize,
    pub mesh_actors: usize,
    pub mesh_failures: usize,
}

/// Every package file under the game's data folder, by lower-case name.
fn index_packages(dir: &Path, out: &mut HashMap<String, PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            index_packages(&p, out);
        } else if let Some(stem) = p.file_stem() {
            out.entry(stem.to_string_lossy().to_ascii_lowercase()).or_insert(p);
        }
    }
}

/// Names from the outermost package down to the object, e.g.
/// `["weapons", "CloneWeapons", "DC15Blaster"]`.
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

/// Read the static mesh an actor points at, from the map or another package.
fn load_mesh(pkgs: &mut Packages, map: &(Package, Vec<u8>), index: i32) -> Option<StaticMesh> {
    match ObjectRef::from_index(index) {
        ObjectRef::Export(k) => static_mesh::read_static_mesh(&map.0, &map.1, &map.0.exports[k]).ok(),
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

pub fn load(map_path: &Path, game_data: &Path, collision_height: f32) -> Result<MapCollision, String> {
    let data = std::fs::read(map_path).map_err(|e| format!("can't read {}: {e}", map_path.display()))?;
    let pkg = Package::parse(&data).map_err(|e| e.to_string())?;
    let level = actors::read_level(&pkg, &data).map_err(|e| format!("level: {e}"))?;
    let model = level.model.ok_or("the level has no BSP model")?;
    let bsp = bsp::read_model(&pkg, &data, &pkg.exports[model]).map_err(|e| format!("BSP: {e}"))?;

    let mut files = HashMap::new();
    index_packages(game_data, &mut files);
    let mut pkgs = Packages { files, open: HashMap::new() };
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
            // Unreal stands the pawn with its centre at the PlayerStart.
            let feet = [p.location[0], p.location[1], p.location[2] - collision_height];
            let yaw = p.rotation[1] as f32 / 65536.0 * std::f32::consts::TAU;
            start = Some((feet, yaw));
        }
        // Hidden props are left out, as in the level viewer.
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
    Ok(MapCollision {
        triangles: mesh.triangles,
        start,
        bsp_nodes: bsp.nodes.len(),
        mesh_actors: pairs.len(),
        mesh_failures,
    })
}
