//! World-space triangles for collision: the BSP's solid polygons plus every
//! placed static mesh, in Unreal coordinates (X forward, Y right, Z up).
//!
//! This crate opens no files, so the caller reads the static meshes (they
//! live in other packages) and hands them in with their placements.

use crate::actors::Placement;
use crate::bsp::{poly_flags, Bsp, Vec3};
use crate::volumes::BlockingVolume;
use rc_package::static_mesh::StaticMesh;

/// Where a collision triangle came from, for logging what was hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// BSP node index.
    Bsp(usize),
    /// Export index of the actor (0-based).
    Actor(usize),
    /// Export index of a blocking volume (0-based).
    Volume(usize),
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
/// lamp's glow or hanging cables) are left out. Props that don't block at
/// all (`bCollideActors` or `bBlockPlayers` off) are for the caller to
/// leave out; see `crate::class_look::blocks_players`.
///
/// Not handled yet: simplified collision models and moving movers.
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
    for (p, mesh) in meshes {
        let world: Vec<Vec3> = mesh
            .vertices
            .iter()
            .map(|v| p.transform_point(v.position))
            .collect();
        for (k, t) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
            if !mesh.triangle_collides(k) {
                continue;
            }
            let get = |i: u16| world.get(i as usize).copied();
            if let (Some(a), Some(b), Some(c)) = (get(t[0]), get(t[1]), get(t[2])) {
                out.triangles.push([a, b, c]);
                out.sources.push(Source::Actor(p.export));
            }
        }
    }
    out
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
