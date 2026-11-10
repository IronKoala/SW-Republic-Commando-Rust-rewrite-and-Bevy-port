//! World-space triangles for collision: the BSP's solid polygons plus every
//! placed static mesh, in Unreal coordinates (X forward, Y right, Z up).
//!
//! This crate opens no files, so the caller reads the static meshes (they
//! live in other packages) and hands them in with their placements.

use crate::actors::Placement;
use crate::bsp::{poly_flags, Bsp, Vec3};
use crate::volumes::BlockingVolume;
use rc_package::static_mesh::StaticMesh;
use rc_package::{ObjectRef, Package};

/// Where a collision triangle came from, for logging what was hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// BSP node index.
    Bsp(usize),
    /// Export index of the actor (0-based).
    Actor(usize),
    /// Export index of a blocking volume (0-based).
    Volume(usize),
    /// Export index of a terrain (outdoor ground, `crate::terrain`).
    Terrain(usize),
}

#[derive(Debug, Clone, Default)]
pub struct CollisionMesh {
    pub triangles: Vec<[Vec3; 3]>,
    /// One entry per triangle, same order.
    pub sources: Vec<Source>,
}

/// Build the collision triangles. BSP polygons marked not solid or portal
/// are left out. Invisible ones are kept: in Unreal 2 invisible walls still
/// block. Hidden actors are kept too (hidden blocking props still collide).
///
/// Mesh sections with collision switched off (`EnableCollision`, such as a
/// lamp's glow or hanging cables) are left out. A mesh with collision off
/// on every section collides with its simple shape instead, if it has one
/// (see `solid_local_triangles`). Props that don't block at
/// all (`bCollideActors` or `bBlockPlayers` off) are for the caller to
/// leave out; see `crate::class_look::blocks_players`.
///
/// Not handled yet: moving movers.
/// Blocking volumes are added separately with `add_blocking_volumes`.
pub fn collision_triangles(bsp: &Bsp, meshes: &[(&Placement, &StaticMesh)]) -> CollisionMesh {
    let mut out = CollisionMesh::default();
    for poly in bsp.polygons() {
        if poly.poly_flags & (poly_flags::NOT_SOLID | poly_flags::PORTAL) != 0 {
            continue;
        }
        for k in 1..poly.points.len().saturating_sub(1) {
            out.triangles
                .push([poly.points[0], poly.points[k], poly.points[k + 1]]);
            out.sources.push(Source::Bsp(poly.node));
        }
    }
    add_meshes(&mut out, meshes);
    out
}

/// Add placed props' triangles (as `collision_triangles` does for its
/// props), for props placed after the rest, such as settled physics props
/// (`crate::settle`).
pub fn add_meshes(out: &mut CollisionMesh, meshes: &[(&Placement, &StaticMesh)]) {
    for (p, mesh) in meshes {
        for t in solid_triangles(p, mesh) {
            out.triangles.push(t);
            out.sources.push(Source::Actor(p.export));
        }
    }
}

/// What a mesh collides with, in its own coordinates:
///
/// - the triangles of its sections with collision on, if there are any;
/// - otherwise its simple collision shape (`StaticMesh::simple_shape`,
///   filled in by `read_simple_shape`). The artists switched collision
///   off on every section of meshes such as the explosive barrels and the
///   Geonosian shield walls and gave them a few blocky boxes instead. The
///   game's own collision triangle list for those meshes is empty, so the
///   simple shape is all it can collide with;
/// - otherwise nothing (decorations).
///
/// Meshes with both (most walls and floors) keep their triangles. The
/// original may use the simple shape for walking against those too; we
/// can't tell that from the files, so they stay as they were.
pub fn solid_local_triangles(mesh: &StaticMesh) -> Vec<[Vec3; 3]> {
    let get = |i: u16| mesh.vertices.get(i as usize).map(|v| v.position);
    let own: Vec<[Vec3; 3]> = mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .enumerate()
        .filter(|(k, _)| mesh.triangle_collides(*k))
        .filter_map(|(_, t)| Some([get(t[0])?, get(t[1])?, get(t[2])?]))
        .collect();
    if own.is_empty() {
        mesh.simple_shape.clone()
    } else {
        own
    }
}

/// `solid_local_triangles` placed in the world.
pub fn solid_triangles(p: &Placement, mesh: &StaticMesh) -> Vec<[Vec3; 3]> {
    solid_local_triangles(mesh)
        .into_iter()
        .map(|t| t.map(|c| p.transform_point(c)))
        .collect()
}

/// Read a mesh's simple collision shape (`StaticMesh::collision_model`, a
/// `Model` in the package the mesh came from) into `mesh.simple_shape`, as
/// triangles in the mesh's own coordinates. Returns how many triangles;
/// 0 if the mesh has no shape. `pkg` and `data` must be the package the
/// mesh was read from.
pub fn read_simple_shape(
    pkg: &Package,
    data: &[u8],
    mesh: &mut StaticMesh,
) -> Result<usize, String> {
    mesh.simple_shape.clear();
    let k = match ObjectRef::from_index(mesh.collision_model) {
        ObjectRef::None => return Ok(0),
        ObjectRef::Export(k) => k,
        ObjectRef::Import(_) => return Err("simple shape is in another package".into()),
    };
    let export = pkg
        .exports
        .get(k)
        .ok_or_else(|| format!("simple shape export {k} out of range"))?;
    if pkg.export_class_name(export) != "Model" {
        return Err(format!(
            "simple shape is a {}, not a Model",
            pkg.export_class_name(export)
        ));
    }
    let model = crate::bsp::read_model(pkg, data, export).map_err(|e| e.to_string())?;
    for poly in model.polygons() {
        if poly.poly_flags & (poly_flags::NOT_SOLID | poly_flags::PORTAL) != 0 {
            continue;
        }
        for k in 1..poly.points.len().saturating_sub(1) {
            mesh.simple_shape
                .push([poly.points[0], poly.points[k], poly.points[k + 1]]);
        }
    }
    Ok(mesh.simple_shape.len())
}

/// Every triangle of a placed mesh, in the world, collision switched on or
/// not.
pub fn drawn_triangles(p: &Placement, mesh: &StaticMesh) -> Vec<[Vec3; 3]> {
    let world: Vec<Vec3> = mesh
        .vertices
        .iter()
        .map(|v| p.transform_point(v.position))
        .collect();
    let get = |i: u16| world.get(i as usize).copied();
    mesh.indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter_map(|t| Some([get(t[0])?, get(t[1])?, get(t[2])?]))
        .collect()
}

/// Add the drawn triangles of the map's terrains (outdoor ground). Returns
/// how many triangles were added.
pub fn add_terrains(mesh: &mut CollisionMesh, terrains: &[crate::terrain::Terrain]) -> usize {
    let before = mesh.triangles.len();
    for t in terrains {
        for tri in t.triangle_points() {
            mesh.triangles.push(tri);
            mesh.sources.push(Source::Terrain(t.export));
        }
    }
    mesh.triangles.len() - before
}

/// Add the faces of the blocking volumes that stop players (or, with
/// `players` false, the ones that stop other characters). Returns how many
/// volumes were added.
pub fn add_blocking_volumes(
    out: &mut CollisionMesh,
    volumes: &[BlockingVolume],
    players: bool,
) -> usize {
    let mut added = 0;
    for v in volumes {
        if !(if players {
            v.block_players
        } else {
            v.block_actors
        }) {
            continue;
        }
        added += 1;
        for t in v.triangles() {
            out.triangles.push(t);
            out.sources.push(Source::Volume(v.export));
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_package::static_mesh::{BoundingBox, Section, Vertex};

    /// A mesh of one section of one triangle, collision on or off, with a
    /// one-triangle simple shape somewhere else.
    fn mesh(section_on: bool) -> StaticMesh {
        let v = |x: f32| Vertex {
            position: [x, 0.0, 0.0],
            normal: [0.0; 3],
        };
        StaticMesh {
            properties: Vec::new(),
            materials: vec![0],
            section_collision: vec![section_on],
            bounding_box: BoundingBox {
                min: [0.0; 3],
                max: [0.0; 3],
                is_valid: false,
            },
            bounding_sphere: [0.0; 4],
            sections: vec![Section {
                first_index: 0,
                first_vertex: 0,
                last_vertex: 2,
                num_faces: 1,
            }],
            vertices: vec![v(0.0), v(1.0), v(2.0)],
            colors: Vec::new(),
            alphas: Vec::new(),
            uv_sets: Vec::new(),
            indices: vec![0, 1, 2],
            indices2: Vec::new(),
            collision_model: 5,
            simple_shape: vec![[[9.0; 3], [8.0; 3], [7.0; 3]]],
            unread: 0,
        }
    }

    /// A mesh with collision on keeps its own triangles; one with every
    /// section off collides with its simple shape; with neither, nothing.
    #[test]
    fn simple_shape_only_when_no_section_collides() {
        assert_eq!(solid_local_triangles(&mesh(true))[0][1], [1.0, 0.0, 0.0]);
        assert_eq!(
            solid_local_triangles(&mesh(false)),
            vec![[[9.0; 3], [8.0; 3], [7.0; 3]]]
        );
        let mut bare = mesh(false);
        bare.simple_shape.clear();
        assert!(solid_local_triangles(&bare).is_empty());
    }
}
