//! Reads `SkeletalMesh` objects: a character's surface (points, triangles,
//! texture coordinates), its skeleton ("bones"), and how strongly each bone
//! pulls on each point ("influences" or "weights").
//!
//! Layout learned from UE Viewer (github.com/gildor2/UEViewer, MIT, Konstantin
//! Nosov): `Unreal/UnrealMesh/UnMesh2.h` and `UnMesh2.cpp` (UPrimitive,
//! ULodMesh, USkeletalMesh, FStaticLODModel and their `SWRC` branches) and
//! `UnMesh.h` (FMeshBone, VJointPos). Written from those descriptions; no
//! code copied.
//!
//! Republic Commando differences (all from UE Viewer):
//! - ULodMesh version 7 adds four unknown 4-byte numbers at the end.
//! - The skeleton is followed by a list of linked animations (version 5+),
//!   each with a flags number from package version 151.
//! - Attachment points ("sockets") are one list of (alias, bone, 4x4 matrix)
//!   from package version 140.
//! - Each LOD model starts with one unknown 4-byte number from version 146.
//! - After the LOD models come the base mesh's points, wedges, triangles and
//!   influences, the data we use. Whatever follows is not described anywhere
//!   we found; we count those bytes as "leftover".
//!
//! Quaternions are returned exactly as stored. How to turn them into
//! rotations is in `rc_anim` (the game stores them mirrored, see there).

use crate::properties::{self, Property};
use crate::{Export, Package, ReadError, Reader, Result};

/// Object flag meaning the data starts with a script state frame.
const RF_HAS_STACK: u32 = 0x0200_0000;

/// Upper limit for array lengths. Real meshes have a few thousand entries;
/// anything near this means we are reading from the wrong place.
const MAX_ARRAY: i32 = 4_000_000;

/// How far a point's weights may add up from 1. Measured on the real files:
/// 17 meshes have a few points whose weights add up to 0.979-0.990, which
/// looks like rounding in the game's export tools; we spread the difference
/// when skinning (`rc_anim::weights_per_point`).
const WEIGHT_SUM_TOLERANCE: f32 = 0.025;

pub(crate) fn vec3(r: &mut Reader) -> Result<[f32; 3]> {
    Ok([r.f32()?, r.f32()?, r.f32()?])
}

pub(crate) fn quat(r: &mut Reader) -> Result<[f32; 4]> {
    Ok([r.f32()?, r.f32()?, r.f32()?, r.f32()?])
}

/// A `TArray`: a compact index count, then the items.
pub(crate) fn array<T>(
    r: &mut Reader,
    mut item: impl FnMut(&mut Reader) -> Result<T>,
) -> Result<Vec<T>> {
    let count = r.compact_index()?;
    if !(0..=MAX_ARRAY).contains(&count) {
        return r.invalid(format!("array length {count} is implausible"));
    }
    // Each item is at least one byte, so a count bigger than what's left is wrong.
    if count as usize > r.len() - r.pos() {
        return r.invalid(format!("array length {count} can't fit in the data left"));
    }
    (0..count).map(|_| item(r)).collect()
}

/// A "lazy array": the file offset just after the array, then a `TArray`.
/// `base` is the file offset where the reader's data starts, so the stored
/// offset can be checked against where the array really ended.
pub(crate) fn lazy_array<T>(
    r: &mut Reader,
    base: usize,
    item: impl FnMut(&mut Reader) -> Result<T>,
) -> Result<Vec<T>> {
    let skip_pos = r.i32()?;
    let out = array(r, item)?;
    if skip_pos as i64 != (base + r.pos()) as i64 {
        return r.invalid(format!(
            "lazy array says it ends at {skip_pos}, it really ends at {}",
            base + r.pos()
        ));
    }
    Ok(out)
}

/// A bounding box: lowest corner, highest corner, and a "valid" byte.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub valid: u8,
}

pub(crate) fn bounding_box(r: &mut Reader) -> Result<BoundingBox> {
    Ok(BoundingBox {
        min: vec3(r)?,
        max: vec3(r)?,
        valid: r.u8()?,
    })
}

/// One bone of the skeleton, in the "reference pose" (the pose the mesh was
/// modelled in). Position and orientation are relative to the parent bone.
#[derive(Debug, Clone)]
pub struct Bone {
    pub name: String,
    pub flags: u32,
    /// Quaternion x, y, z, w exactly as stored.
    pub orientation: [f32; 4],
    pub position: [f32; 3],
    pub length: f32,
    pub size: [f32; 3],
    pub num_children: i32,
    /// Index of the parent bone. -1 for the root bone (index 0).
    pub parent: i32,
}

/// A mesh "material slot": which entry of `textures` a triangle uses.
#[derive(Debug, Clone, Copy)]
pub struct MeshMaterial {
    pub poly_flags: u32,
    pub texture_index: i32,
}

/// A "wedge" is a corner of a triangle: a point plus texture coordinates.
/// Two triangles sharing a point can still use different texture spots.
#[derive(Debug, Clone, Copy)]
pub struct Wedge {
    pub point: u16,
    pub u: f32,
    pub v: f32,
}

fn wedge(r: &mut Reader) -> Result<Wedge> {
    Ok(Wedge {
        point: r.u16()?,
        u: r.f32()?,
        v: r.f32()?,
    })
}

#[derive(Debug, Clone, Copy)]
pub struct Triangle {
    pub wedges: [u16; 3],
    pub material: u8,
    pub aux_material: u8,
    pub smoothing_groups: u32,
}

/// How much one bone moves one point (0 to 1).
#[derive(Debug, Clone, Copy)]
pub struct Influence {
    pub weight: f32,
    pub point: u16,
    pub bone: u16,
}

fn influence(r: &mut Reader) -> Result<Influence> {
    Ok(Influence {
        weight: r.f32()?,
        point: r.u16()?,
        bone: r.u16()?,
    })
}

/// A named attachment point on a bone (for weapons, effects, ...).
#[derive(Debug, Clone)]
pub struct Socket {
    pub alias: String,
    pub bone: String,
    pub matrix: [f32; 16],
}

/// A simplified copy of the mesh, for distant characters. We read these to
/// get past them, and keep only their sizes for the log.
#[derive(Debug, Clone)]
pub struct LodSummary {
    pub soft_sections: usize,
    pub rigid_sections: usize,
    pub points: usize,
    pub wedges: usize,
    pub faces: usize,
    pub influences: usize,
}

#[derive(Debug, Clone)]
pub struct SkeletalMesh {
    pub properties: Vec<Property>,
    pub bounding_box: BoundingBox,
    /// Centre x, y, z and radius.
    pub bounding_sphere: [f32; 4],
    /// ULodMesh format version (7 in Republic Commando's newer files).
    pub lod_mesh_version: i32,
    pub vertex_count: i32,
    /// Object references to the materials (skins), one per slot.
    pub textures: Vec<i32>,
    pub mesh_scale: [f32; 3],
    pub mesh_origin: [f32; 3],
    /// Pitch, yaw, roll in Unreal angle units (65536 = full turn).
    pub rot_origin: [i32; 3],
    pub materials: Vec<MeshMaterial>,
    pub bones: Vec<Bone>,
    pub skeletal_depth: i32,
    /// Object references to the `MeshAnimation`s linked to this mesh.
    pub animations: Vec<i32>,
    pub sockets: Vec<Socket>,
    pub lods: Vec<LodSummary>,
    pub points: Vec<[f32; 3]>,
    pub wedges: Vec<Wedge>,
    pub triangles: Vec<Triangle>,
    pub influences: Vec<Influence>,
    /// Bytes after the part we understand. Not an error; see module notes.
    pub leftover: usize,
}

fn face(r: &mut Reader) -> Result<[u16; 4]> {
    Ok([r.u16()?, r.u16()?, r.u16()?, r.u16()?])
}

fn section(r: &mut Reader) -> Result<[u16; 9]> {
    let mut s = [0u16; 9];
    for v in &mut s {
        *v = r.u16()?;
    }
    Ok(s)
}

fn index_buffer(r: &mut Reader) -> Result<usize> {
    let n = array(r, |r| r.u16())?.len();
    r.i32()?; // revision
    Ok(n)
}

fn lod_model(r: &mut Reader, base: usize, version: u16) -> Result<LodSummary> {
    if version >= 146 {
        r.i32()?; // unknown, Republic Commando only
    }
    array(r, |r| r.u32())?; // skinning data
    array(r, |r| {
        vec3(r)?;
        r.u32() // packed normal
    })?;
    r.i32()?; // number of soft wedges
    let soft_sections = array(r, section)?.len();
    let rigid_sections = array(r, section)?.len();
    index_buffer(r)?;
    index_buffer(r)?;
    // Vertex stream for the rigid parts: three numbers, then position,
    // normal and texture coordinate per vertex (8 floats).
    r.i32()?;
    r.i32()?;
    r.i32()?;
    array(r, |r| r.bytes(32).map(|_| ()))?;
    let influences = lazy_array(r, base, influence)?.len();
    let wedges = lazy_array(r, base, wedge)?.len();
    let faces = lazy_array(r, base, face)?.len();
    let points = lazy_array(r, base, vec3)?.len();
    r.f32()?; // LOD distance factor
    r.f32()?; // LOD hysteresis
    r.i32()?; // shared vertex count
    r.i32()?; // max influences
    r.i32()?;
    r.i32()?;
    Ok(LodSummary {
        soft_sections,
        rigid_sections,
        points,
        wedges,
        faces,
        influences,
    })
}

fn name_text(pkg: &Package, r: &mut Reader) -> Result<String> {
    let i = r.compact_index()?;
    if !(0..pkg.names.len() as i32).contains(&i) {
        return r.invalid(format!("name index {i} is out of range"));
    }
    Ok(pkg.name(i).to_string())
}

/// The bytes of an export's data inside the whole file.
pub(crate) fn export_data<'a>(data: &'a [u8], export: &Export) -> Result<(usize, &'a [u8])> {
    let start = export.serial_offset as usize;
    match data.get(start..start + export.serial_size as usize) {
        Some(object) => Ok((start, object)),
        None => Err(ReadError::Invalid {
            offset: start,
            what: "export data is outside the file".into(),
        }),
    }
}

/// Read a `SkeletalMesh` export. `data` is the whole package file.
pub fn read_skeletal_mesh(pkg: &Package, data: &[u8], export: &Export) -> Result<SkeletalMesh> {
    let (base, object) = export_data(data, export)?;
    let version = pkg.summary.version;
    let mut r = Reader::new(object);
    if export.object_flags & RF_HAS_STACK != 0 {
        return r.invalid("object has a script state frame, not supported for meshes");
    }
    let properties = properties::read_properties(pkg, &mut r)?;
    let r = &mut r;

    // UPrimitive
    let bounding_box = bounding_box(r)?;
    let bounding_sphere = quat(r)?;

    // ULodMesh
    let lod_mesh_version = r.i32()?;
    let vertex_count = r.i32()?;
    array(r, |r| r.u32())?; // packed vertex-animation points, empty here
    if lod_mesh_version <= 1 {
        return r.invalid(format!(
            "ULodMesh version {lod_mesh_version} is the old layout, not supported"
        ));
    }
    let textures = array(r, |r| r.compact_index())?;
    let mesh_scale = vec3(r)?;
    let mesh_origin = vec3(r)?;
    let rot_origin = [r.i32()?, r.i32()?, r.i32()?];
    array(r, |r| r.u16())?; // face level
    array(r, face)?; // faces (empty for skeletal meshes)
    array(r, |r| r.u16())?; // collapse wedge
    array(r, wedge)?; // wedges of the ULodMesh part (empty here)
    let materials = array(r, |r| {
        Ok(MeshMaterial {
            poly_flags: r.u32()?,
            texture_index: r.i32()?,
        })
    })?;
    r.f32()?; // mesh scale max
    r.f32()?; // LOD hysteresis
    r.f32()?; // LOD strength
    r.i32()?; // LOD min verts
    r.f32()?; // LOD morph
    r.f32()?; // LOD Z displace
    if lod_mesh_version >= 3 {
        // Impostor: a flat picture used instead of the mesh far away.
        r.i32()?;
        r.compact_index()?;
        vec3(r)?;
        r.bytes(12)?; // rotator
        vec3(r)?;
        r.u32()?; // colour
        r.i32()?;
        r.i32()?;
        r.i32()?;
    }
    if lod_mesh_version >= 4 {
        r.f32()?; // skin tesselation factor
    }
    if lod_mesh_version >= 7 {
        r.bytes(16)?; // four unknown numbers, Republic Commando only
    }

    // USkeletalMesh
    array(r, vec3)?; // old-format points, empty here
    let bones = array(r, |r| {
        Ok(Bone {
            name: name_text(pkg, r)?,
            flags: r.u32()?,
            orientation: quat(r)?,
            position: vec3(r)?,
            length: r.f32()?,
            size: vec3(r)?,
            num_children: r.i32()?,
            parent: r.i32()?,
        })
    })?;
    let animations = if lod_mesh_version >= 5 {
        array(r, |r| {
            if version >= 151 {
                r.i32()?; // flags
            }
            r.compact_index()
        })?
    } else {
        vec![r.compact_index()?]
    };
    let skeletal_depth = r.i32()?;
    array(r, |r| {
        array(r, |r| r.u16())?;
        r.i32()
    })?; // weight indices (old format)
    array(r, |r| r.u32())?; // bone influences (old format)
    let sockets = if version >= 140 {
        array(r, |r| {
            let alias = name_text(pkg, r)?;
            let bone = name_text(pkg, r)?;
            let mut matrix = [0f32; 16];
            for m in &mut matrix {
                *m = r.f32()?;
            }
            Ok(Socket {
                alias,
                bone,
                matrix,
            })
        })?
    } else {
        return r.invalid(format!(
            "package version {version} uses the older socket layout, not supported yet"
        ));
    };
    if lod_mesh_version >= 6 {
        r.i32()?;
    }
    let lods = array(r, |r| lod_model(r, base, version))?;
    if lod_mesh_version < 5 {
        r.compact_index()?;
    }
    let points = lazy_array(r, base, vec3)?;
    let wedges = lazy_array(r, base, wedge)?;
    let triangles = lazy_array(r, base, |r| {
        Ok(Triangle {
            wedges: [r.u16()?, r.u16()?, r.u16()?],
            material: r.u8()?,
            aux_material: r.u8()?,
            smoothing_groups: r.u32()?,
        })
    })?;
    let influences = lazy_array(r, base, influence)?;
    lazy_array(r, base, |r| r.u16())?; // collapse wedge
    lazy_array(r, base, |r| r.u16())?; // unknown, one per triangle

    Ok(SkeletalMesh {
        properties,
        bounding_box,
        bounding_sphere,
        lod_mesh_version,
        vertex_count,
        textures,
        mesh_scale,
        mesh_origin,
        rot_origin,
        materials,
        bones,
        skeletal_depth,
        animations,
        sockets,
        lods,
        points,
        wedges,
        triangles,
        influences,
        leftover: object.len() - r.pos(),
    })
}

impl SkeletalMesh {
    /// Checks that would fail if we read the format wrongly. One line per
    /// problem; an empty list means every check passed.
    pub fn check(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let (np, nw, nb) = (self.points.len(), self.wedges.len(), self.bones.len());
        if np == 0 || nw == 0 || self.triangles.is_empty() || nb == 0 {
            problems.push(format!(
                "empty mesh: {np} points, {nw} wedges, {} triangles, {nb} bones",
                self.triangles.len()
            ));
        }
        for (i, b) in self.bones.iter().enumerate() {
            let p = b.parent;
            // Parents come before children. The root's parent is stored as
            // -1 in this game (0, pointing at itself, in some other UE2 games).
            let ok = if i == 0 {
                p == 0 || p == -1
            } else {
                p >= 0 && (p as usize) < i
            };
            if !ok {
                problems.push(format!("bone {i} ({}) has parent {p}", b.name));
            }
            let q = b.orientation;
            let len = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
            if (len - 1.0).abs() > 0.01 {
                problems.push(format!("bone {i} ({}) quaternion length {len}", b.name));
            }
        }
        // `num_children` counts all bones below this one (children,
        // grandchildren, ...), not only direct children: measured on the
        // real files, the root says 64 for a 65-bone skeleton.
        let mut below = vec![0i32; nb];
        for i in (1..nb).rev() {
            let p = self.bones[i].parent;
            if p >= 0 && (p as usize) < i {
                below[p as usize] += below[i] + 1;
            }
        }
        for (i, b) in self.bones.iter().enumerate() {
            if b.num_children != below[i] {
                problems.push(format!(
                    "bone {i} ({}) says {} bones below it, found {}",
                    b.name, b.num_children, below[i]
                ));
            }
        }
        if let Some(w) = self.wedges.iter().find(|w| w.point as usize >= np) {
            problems.push(format!("a wedge points at point {} of {np}", w.point));
        }
        let bad_tris = self
            .triangles
            .iter()
            .filter(|t| t.wedges.iter().any(|&w| w as usize >= nw))
            .count();
        if bad_tris > 0 {
            problems.push(format!("{bad_tris} triangles point past the wedge list"));
        }
        let bad_mat = self
            .triangles
            .iter()
            .filter(|t| t.material as usize >= self.materials.len().max(1))
            .count();
        if bad_mat > 0 {
            problems.push(format!(
                "{bad_mat} triangles use a material slot that doesn't exist"
            ));
        }
        // Every point should be moved by at least one bone, and the weights
        // on each point should add up to 1.
        let mut sum = vec![0f32; np];
        for inf in &self.influences {
            if inf.point as usize >= np || inf.bone as usize >= nb {
                problems.push(format!("influence out of range: {inf:?}"));
                break;
            }
            sum[inf.point as usize] += inf.weight;
        }
        let unweighted = sum.iter().filter(|&&s| s == 0.0).count();
        let off = sum
            .iter()
            .filter(|&&s| s != 0.0 && (s - 1.0).abs() > WEIGHT_SUM_TOLERANCE)
            .count();
        if unweighted > 0 {
            problems.push(format!("{unweighted} points have no bone influence"));
        }
        if off > 0 {
            let bad = sum.iter().filter(|&&s| s != 0.0 && (s - 1.0).abs() > WEIGHT_SUM_TOLERANCE);
            let lo = bad.clone().fold(f32::INFINITY, |a, &b| a.min(b));
            let hi = bad.fold(f32::NEG_INFINITY, |a, &b| a.max(b));
            problems.push(format!(
                "{off} points have weights that don't add up to 1 (sums from {lo:.3} to {hi:.3})"
            ));
        }
        if np as i32 != self.vertex_count && self.vertex_count != 0 {
            problems.push(format!(
                "header says {} vertices, found {np} points",
                self.vertex_count
            ));
        }
        problems
    }

    /// Lowest and highest corner of the points, in the mesh's own units.
    pub fn points_bounds(&self) -> ([f32; 3], [f32; 3]) {
        bounds(&self.points)
    }
}

/// Lowest and highest corner of a list of points.
pub fn bounds(points: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for p in points {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (lo, hi)
}
