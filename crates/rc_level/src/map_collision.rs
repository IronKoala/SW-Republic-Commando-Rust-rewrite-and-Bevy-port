//! Load a map's collision from disk: its BSP walls and floors plus every
//! placed static mesh and the blocking volumes that stop players, as
//! triangles (via `crate::collision`), and where the player starts. Unlike the rest of this crate it opens files, because the
//! static meshes live in other packages under `GameData`. Coordinates stay
//! in Unreal units (X forward, Y right, Z up).
//!
//! Moved here from `rc_walk` so other programs (the droid AI tests, the
//! game) can load the same collision without Bevy.

use crate::actors::{self, Placement};
use crate::bsp::{self, Vec3};
use crate::collision::{add_blocking_volumes, collision_triangles, Source};
use rc_package::static_mesh::{self, StaticMesh};
use rc_package::{ObjectRef, Package};
use crate::class_look::{apply_class_look, ClassLooks, Loaded};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct MapCollision {
    pub triangles: Vec<[Vec3; 3]>,
    /// Feet position and facing (radians, Unreal yaw) of the first
    /// PlayerStart.
    pub start: Option<(Vec3, f32)>,
    pub bsp_nodes: usize,
    pub mesh_actors: usize,
    pub mesh_failures: usize,
    /// Props whose model comes from their class, not the map.
    pub class_mesh_actors: usize,
    /// Blocking volumes added (the ones that stop players), and ones that
    /// couldn't be read.
    pub blocking_volumes: usize,
    pub volume_failures: usize,
    /// The map's doors, lifts and their triggers (`crate::movers`). Their
    /// triangles are not in `triangles`: they are in `mover_triangles`,
    /// by the mover's export index, placed where the map puts them, so
    /// they can move.
    pub movers: crate::movers::MapMovers,
    pub mover_triangles: HashMap<usize, Vec<[Vec3; 3]>>,
    /// Props and blocking volumes with a `Tag` of their own (barricades,
    /// bunker walls): a level script may switch them off or blow them up,
    /// so their triangles are kept apart too, by export index, with their
    /// (class, tag). See `crate::scripts::tagged_actors`.
    pub part_triangles: HashMap<usize, Vec<[Vec3; 3]>>,
    pub parts: HashMap<usize, (String, String)>,
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
        } else if let (Some(stem), Some(ext)) = (p.file_stem(), p.extension()) {
            // Only packages: `Engine.int` (text) sits next to `Engine.u`
            // in GameData/System, and must not stand in for it.
            let ext = ext.to_string_lossy().to_ascii_lowercase();
            if ["u", "usx", "utx", "ukx", "uax", "ctm"].contains(&ext.as_str()) {
                out.entry(stem.to_string_lossy().to_ascii_lowercase()).or_insert(p);
            }
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
    open: HashMap<String, Option<Loaded>>,
}

impl Packages {
    fn get(&mut self, name: &str) -> Option<Loaded> {
        let key = name.to_ascii_lowercase();
        if !self.open.contains_key(&key) {
            let loaded = self.files.get(&key).and_then(|p| {
                let data = std::fs::read(p).ok()?;
                let pkg = Package::parse(&data).ok()?;
                Some(Rc::new((pkg, data)))
            });
            self.open.insert(key.clone(), loaded);
        }
        self.open.get(&key).cloned().flatten()
    }
}

/// Read the static mesh that `index` (a reference inside package `map`,
/// the map itself or a class's script package) points at.
fn load_mesh(pkgs: &mut Packages, map: &Loaded, index: i32) -> Option<StaticMesh> {
    match ObjectRef::from_index(index) {
        ObjectRef::Export(k) => static_mesh::read_static_mesh(&map.0, &map.1, &map.0.exports[k]).ok(),
        ObjectRef::Import(_) => {
            let path = object_path(&map.0, index);
            let loaded = pkgs.get(&path[0])?;
            let (owner, data) = (&loaded.0, &loaded.1);
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
    let map: Loaded = Rc::new((pkg, data));
    // Keyed by the package the reference is in (by address) and the reference.
    let mut cache: HashMap<(usize, i32), Option<StaticMesh>> = HashMap::new();
    let mut placed: Vec<(Placement, (usize, i32))> = Vec::new();
    let mut looks = ClassLooks::new();
    let mut start = None;
    let mut mesh_failures = 0;
    let mut class_mesh_actors = 0;
    for &a in level.actors.iter().flatten() {
        let Ok(mut p) = actors::read_placement(&map.0, &map.1, a) else {
            continue;
        };
        if p.class == "PlayerStart" && start.is_none() {
            // Unreal stands the pawn with its centre at the PlayerStart.
            let feet = [p.location[0], p.location[1], p.location[2] - collision_height];
            let yaw = p.rotation[1] as f32 / 65536.0 * std::f32::consts::TAU;
            start = Some((feet, yaw));
        }
        // No mesh in the map: the class may give one (props, double doors).
        let mut owner = map.clone();
        if p.static_mesh == 0 {
            match apply_class_look(&mut looks, &map.0, &map.1, &mut p, &mut |n| pkgs.get(n)) {
                Some(o) if !p.hidden => {
                    owner = o;
                    class_mesh_actors += 1;
                }
                _ => continue,
            }
        }
        // Hidden props are left out, as in the level viewer.
        if p.hidden {
            continue;
        }
        let key = (Rc::as_ptr(&owner) as usize, p.static_mesh);
        let mesh = cache
            .entry(key)
            .or_insert_with(|| load_mesh(&mut pkgs, &owner, p.static_mesh));
        if mesh.is_some() {
            placed.push((p, key));
        } else {
            mesh_failures += 1;
        }
    }
    let pairs: Vec<(&Placement, &StaticMesh)> = placed
        .iter()
        .filter_map(|(p, i)| cache.get(i).and_then(|m| m.as_ref()).map(|m| (p, m)))
        .collect();
    let mut mesh = collision_triangles(&bsp, &pairs);
    let volumes = crate::volumes::blocking_volumes(&map.0, &map.1, &level);
    let blocking_volumes = add_blocking_volumes(&mut mesh, &volumes.volumes, true);
    // Take the movers' triangles out, so they can move.
    let movers = crate::movers::read_movers(&mut looks, &map.0, &map.1, &level.actors, &mut |n| pkgs.get(n));
    let is_mover: std::collections::HashSet<usize> = movers.movers.iter().map(|m| m.place.export).collect();
    let mut mover_triangles: HashMap<usize, Vec<[Vec3; 3]>> = HashMap::new();
    // And the parts scripts can switch off.
    let mut parts = crate::scripts::tagged_actors(&map.0, &map.1, &level);
    parts.retain(|a, _| !is_mover.contains(a));
    let mut part_triangles: HashMap<usize, Vec<[Vec3; 3]>> = HashMap::new();
    let mut triangles = Vec::with_capacity(mesh.triangles.len());
    for (t, src) in mesh.triangles.into_iter().zip(mesh.sources) {
        match src {
            Source::Actor(a) if is_mover.contains(&a) => mover_triangles.entry(a).or_default().push(t),
            Source::Actor(a) | Source::Volume(a) if parts.contains_key(&a) => {
                part_triangles.entry(a).or_default().push(t)
            }
            _ => triangles.push(t),
        }
    }
    Ok(MapCollision {
        triangles,
        movers,
        mover_triangles,
        part_triangles,
        parts,
        start,
        bsp_nodes: bsp.nodes.len(),
        mesh_actors: pairs.len(),
        mesh_failures,
        class_mesh_actors,
        blocking_volumes,
        volume_failures: volumes.failures.len(),
    })
}
