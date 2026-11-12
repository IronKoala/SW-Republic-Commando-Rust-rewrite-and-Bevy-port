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
use crate::class_look::{anim_mesh_of, apply_class_look, blocks, ClassLooks, Loaded};
use crate::collision::{add_blocking_volumes, collision_triangles, Source};
use rc_package::static_mesh::{self, StaticMesh};
use rc_package::{ObjectRef, Package};
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
    /// Props left out of collision because the game doesn't let them
    /// block (`bCollideActors` or `bBlockPlayers` off). Still drawn.
    /// Includes the `shot_only_props`.
    pub not_solid_props: usize,
    /// Props people walk through but shots stop at (`Blocks::shots`
    /// without `Blocks::players`), such as the shield wall pieces of
    /// geo_04c. Their triangles are in `shot_triangles`, or in
    /// `shot_part_triangles` when they are parts.
    pub shot_only_props: usize,
    /// Triangles that stop shots (lines with no thickness) but not
    /// movement.
    pub shot_triangles: Vec<[Vec3; 3]>,
    /// The same for shots-only props that are parts (`parts`), by export
    /// index, so they can go when they break. Not in `part_triangles`.
    pub shot_part_triangles: HashMap<usize, Vec<[Vec3; 3]>>,
    /// The faces of blocking volumes in `triangles` that a line with no
    /// thickness passes through in the original (`bBlockZeroExtentTraces`
    /// off): its sight checks, such as the objective tracker's, ignore
    /// them. Kept apart so a line check can tell them from the rest.
    pub line_through_triangles: Vec<[Vec3; 3]>,
    /// Placed meshes that collide through their simple collision shape
    /// (collision off on every section; see
    /// `crate::collision::solid_local_triangles`).
    pub simple_shape_props: usize,
    /// Blocking volumes added (the ones that stop players), and ones that
    /// couldn't be read.
    pub blocking_volumes: usize,
    pub volume_failures: usize,
    /// Terrains (outdoor ground) read, their triangles added, and the ones
    /// that couldn't be read with why.
    pub terrains: usize,
    pub terrain_triangles: usize,
    pub terrain_failures: Vec<String>,
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
    /// Props weapons can break (`crate::breakables`). They are parts too,
    /// so their triangles can go when they break.
    pub breakables: Vec<crate::breakables::Breakable>,
    /// Places to arrive at from another map: each `Teleporter`'s `Tag`,
    /// feet position and facing, as for `start`. A level address like
    /// `RAS_03B.ctm#FromA` names one by its tag.
    pub arrivals: Vec<(String, Vec3, f32)>,
    /// Props drawn with an animated model from their class (bacta
    /// dispensers and the like, `class_look::anim_mesh_of`): where each
    /// stands (moved down by its collision height, so the model's base is
    /// on the floor), and the package and skeletal mesh. Not solid yet.
    pub anim_props: Vec<(Placement, String, String)>,
    /// Ammo and grenade boxes (`crate::pickups`), resting on the ground
    /// under them as the game drops them at the start. Pickups are left
    /// out of `triangles`: the player walks into them to take them.
    pub ammo_boxes: Vec<crate::pickups::AmmoBox>,
    /// Weapons lying in the map (`crate::pickups::PlacedWeapon`), resting
    /// on the ground the same way.
    pub weapons: Vec<crate::pickups::PlacedWeapon>,
    /// Pickups of every kind left out of collision.
    pub pickups_not_solid: usize,
}

impl MapCollision {
    /// Feet and facing for arriving through `portal` (a teleporter's tag,
    /// any case), if the map has one by that name.
    pub fn arrival(&self, portal: &str) -> Option<(Vec3, f32)> {
        self.arrivals
            .iter()
            .find(|(t, _, _)| t.eq_ignore_ascii_case(portal))
            .map(|(_, f, y)| (*f, *y))
    }
}

/// An actor's own `Tag` (the class name if the map gives none, as in UE2).
fn actor_tag(pkg: &Package, data: &[u8], export: usize, class: &str) -> String {
    actors::read_actor_properties(pkg, data, export)
        .ok()
        .and_then(|ps| {
            ps.into_iter().find_map(|p| match p.value {
                rc_package::properties::Value::Name(n) if pkg.name(p.name) == "Tag" => {
                    Some(pkg.name(n).to_string())
                }
                _ => None,
            })
        })
        .unwrap_or_else(|| class.to_string())
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
                out.entry(stem.to_string_lossy().to_ascii_lowercase())
                    .or_insert(p);
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
    let (owner, k) = match ObjectRef::from_index(index) {
        ObjectRef::Export(k) => (map.clone(), k),
        ObjectRef::Import(_) => {
            let path = object_path(&map.0, index);
            let owner = pkgs.get(&path[0])?;
            let rest = &path[1..];
            let k = (0..owner.0.exports.len()).find(|&i| {
                let p = object_path(&owner.0, i as i32 + 1);
                p.len() == rest.len() && p.iter().zip(rest).all(|(a, b)| a.eq_ignore_ascii_case(b))
            })?;
            (owner, k)
        }
        ObjectRef::None => return None,
    };
    let mut mesh = static_mesh::read_static_mesh(&owner.0, &owner.1, &owner.0.exports[k]).ok()?;
    // A shape that can't be read leaves the mesh as it was (its own
    // triangles, or nothing).
    let _ = crate::collision::read_simple_shape(&owner.0, &owner.1, &mut mesh);
    Some(mesh)
}

pub fn load(
    map_path: &Path,
    game_data: &Path,
    collision_height: f32,
) -> Result<MapCollision, String> {
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
    let map: Loaded = Rc::new((pkg, data));
    // Keyed by the package the reference is in (by address) and the reference.
    let mut cache: HashMap<(usize, i32), Option<StaticMesh>> = HashMap::new();
    let mut placed: Vec<(Placement, (usize, i32))> = Vec::new();
    let mut shot_only: Vec<(Placement, (usize, i32))> = Vec::new();
    let mut looks = ClassLooks::new();
    let mut start = None;
    let mut arrivals = Vec::new();
    let mut mesh_failures = 0;
    let mut class_mesh_actors = 0;
    let mut anim_props = Vec::new();
    let mut not_solid = 0;
    let mut pickups_not_solid = 0;
    // Physics props that fall as the level starts, moved down onto the
    // ground below (`crate::settle`).
    let mut karma = std::collections::HashSet::new();
    for &a in level.actors.iter().flatten() {
        let Ok(mut p) = actors::read_placement(&map.0, &map.1, a) else {
            continue;
        };
        if p.class == "PlayerStart" && start.is_none() {
            // Unreal stands the pawn with its centre at the PlayerStart.
            let feet = [
                p.location[0],
                p.location[1],
                p.location[2] - collision_height,
            ];
            let yaw = p.rotation[1] as f32 / 65536.0 * std::f32::consts::TAU;
            start = Some((feet, yaw));
        }
        if p.class == "Teleporter" {
            // Arriving pawns stand with their centre on it, as at a
            // PlayerStart.
            let feet = [
                p.location[0],
                p.location[1],
                p.location[2] - collision_height,
            ];
            let yaw = p.rotation[1] as f32 / 65536.0 * std::f32::consts::TAU;
            arrivals.push((actor_tag(&map.0, &map.1, a, &p.class), feet, yaw));
        }
        // No mesh in the map: the class may give one (props, double doors).
        let mut owner = map.clone();
        if p.static_mesh == 0 {
            match apply_class_look(&mut looks, &map.0, &map.1, &mut p, &mut |n| pkgs.get(n)) {
                Some(o) if !p.hidden => {
                    owner = o;
                    class_mesh_actors += 1;
                }
                Some(_) => continue,
                None => {
                    if let Some((package, mesh, height)) =
                        anim_mesh_of(&mut looks, &map.0, &map.1, &p, &mut |n| pkgs.get(n))
                    {
                        let mut p = p;
                        p.location[2] -= height;
                        anim_props.push((p, package, mesh));
                    }
                    continue;
                }
            }
        }
        // Hidden props are left out, as in the level viewer.
        if p.hidden {
            continue;
        }
        // Pickups don't block: touching one is how you take it.
        if crate::pickups::is_pickup(&mut looks, &map.0, a, &mut |n| pkgs.get(n)) {
            pickups_not_solid += 1;
            continue;
        }
        // So are props the game lets you walk through; some of those
        // still stop shots.
        let stops = blocks(&mut looks, &map.0, &map.1, a, &mut |n| pkgs.get(n));
        if !stops.players {
            not_solid += 1;
            if stops.shots {
                let key = (Rc::as_ptr(&owner) as usize, p.static_mesh);
                let mesh = cache
                    .entry(key)
                    .or_insert_with(|| load_mesh(&mut pkgs, &owner, p.static_mesh));
                if mesh.is_some() {
                    shot_only.push((p, key));
                }
            }
            continue;
        }
        if crate::settle::starts_awake(&map.0, &map.1, a)
            && looks
                .for_actor(&map.0, a, &mut |n| pkgs.get(n))
                .is_some_and(|l| l.is_a("KarmaProp"))
        {
            karma.insert(a);
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
    let simple_shape_props = pairs
        .iter()
        .filter(|(_, m)| {
            !m.simple_shape.is_empty() && (0..m.triangle_count()).all(|k| !m.triangle_collides(k))
        })
        .count();
    let (loose, fixed): (Vec<_>, Vec<_>) =
        pairs.iter().partition(|(p, _)| karma.contains(&p.export));
    let mut mesh = collision_triangles(&bsp, &fixed);
    let (terrains, terrain_failures) =
        crate::terrain::read_terrains(&map.0, &map.1, level.actors.iter().flatten().copied());
    let terrain_triangles = crate::collision::add_terrains(&mut mesh, &terrains);
    let settled: Vec<(Placement, &StaticMesh)> = {
        let ground = crate::settle::Ground::new(&mesh.triangles);
        loose
            .iter()
            .map(|(p, m)| {
                let mut p = (*p).clone();
                crate::settle::settle(&mut p, m, &ground);
                (p, *m)
            })
            .collect()
    };
    let settled_refs: Vec<(&Placement, &StaticMesh)> =
        settled.iter().map(|(p, m)| (p, *m)).collect();
    crate::collision::add_meshes(&mut mesh, &settled_refs);
    let volumes = crate::volumes::blocking_volumes(&map.0, &map.1, &level);
    let blocking_volumes = add_blocking_volumes(&mut mesh, &volumes.volumes, true);
    // Take the movers' triangles out, so they can move.
    let movers = crate::movers::read_movers(&mut looks, &map.0, &map.1, &level.actors, &mut |n| {
        pkgs.get(n)
    });
    let is_mover: std::collections::HashSet<usize> =
        movers.movers.iter().map(|m| m.place.export).collect();
    let mut mover_triangles: HashMap<usize, Vec<[Vec3; 3]>> = HashMap::new();
    // And the parts scripts can switch off.
    let mut parts = crate::scripts::tagged_actors(&map.0, &map.1, &level);
    let breakables =
        crate::breakables::read(&mut looks, &map.0, &map.1, &level, &mut |n| pkgs.get(n));
    for b in &breakables {
        parts
            .entry(b.export)
            .or_insert_with(|| (b.class.clone(), b.tag.clone()));
    }
    parts.retain(|a, _| !is_mover.contains(a));
    let mut part_triangles: HashMap<usize, Vec<[Vec3; 3]>> = HashMap::new();
    // Ammo boxes, dropped onto what is below them.
    let mut ammo_boxes = crate::pickups::read_boxes(
        &mut looks,
        &map.0,
        &map.1,
        level.actors.iter().flatten().copied(),
        &mut |n| pkgs.get(n),
    );
    let mut weapons = crate::pickups::read_weapons(
        &mut looks,
        &map.0,
        &map.1,
        level.actors.iter().flatten().copied(),
        &mut |n| pkgs.get(n),
    );
    {
        let ground = crate::settle::Ground::new(&mesh.triangles);
        for b in &mut ammo_boxes {
            crate::pickups::rest_on_ground(b, |p| ground.below(p));
        }
        for w in &mut weapons {
            crate::pickups::rest_weapon_on_ground(w, |p| ground.below(p));
        }
    }
    let lines_pass: std::collections::HashSet<usize> = volumes
        .volumes
        .iter()
        .filter(|v| !v.block_traces)
        .map(|v| v.export)
        .collect();
    let mut line_through_triangles = Vec::new();
    let mut triangles = Vec::with_capacity(mesh.triangles.len());
    for (t, src) in mesh.triangles.into_iter().zip(mesh.sources) {
        if let Source::Volume(a) = src {
            if lines_pass.contains(&a) && !parts.contains_key(&a) {
                line_through_triangles.push(t);
            }
        }
        match src {
            Source::Actor(a) if is_mover.contains(&a) => {
                mover_triangles.entry(a).or_default().push(t)
            }
            Source::Actor(a) | Source::Volume(a) if parts.contains_key(&a) => {
                part_triangles.entry(a).or_default().push(t)
            }
            _ => triangles.push(t),
        }
    }
    // A breakable prop with no collision at all (every section's
    // collision off and no simple shape) could never be hit, so it gets
    // its drawn triangles.
    let shapes: HashMap<usize, (&Placement, &StaticMesh)> = fixed
        .iter()
        .copied()
        .chain(settled_refs.iter().copied())
        .map(|(p, m)| (p.export, (p, m)))
        .collect();
    for b in &breakables {
        if part_triangles.contains_key(&b.export) || is_mover.contains(&b.export) {
            continue;
        }
        let Some(&(p, m)) = shapes.get(&b.export) else {
            continue;
        };
        let tris = crate::collision::drawn_triangles(p, m);
        if !tris.is_empty() {
            part_triangles.insert(b.export, tris);
        }
    }
    // Props that stop shots but not people.
    let mut shot_triangles = Vec::new();
    let mut shot_part_triangles: HashMap<usize, Vec<[Vec3; 3]>> = HashMap::new();
    for (p, key) in &shot_only {
        let Some(Some(m)) = cache.get(key) else {
            continue;
        };
        let tris = crate::collision::solid_triangles(p, m);
        if parts.contains_key(&p.export) {
            shot_part_triangles
                .entry(p.export)
                .or_default()
                .extend(tris);
        } else {
            shot_triangles.extend(tris);
        }
    }
    Ok(MapCollision {
        triangles,
        shot_only_props: shot_only.len(),
        shot_triangles,
        shot_part_triangles,
        line_through_triangles,
        simple_shape_props,
        movers,
        mover_triangles,
        part_triangles,
        parts,
        breakables,
        start,
        arrivals,
        ammo_boxes,
        weapons,
        pickups_not_solid,
        bsp_nodes: bsp.nodes.len(),
        mesh_actors: pairs.len(),
        mesh_failures,
        class_mesh_actors,
        not_solid_props: not_solid,
        blocking_volumes,
        volume_failures: volumes.failures.len(),
        terrains: terrains.len(),
        terrain_triangles,
        terrain_failures,
        anim_props,
    })
}
