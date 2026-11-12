//! Reads `KMeshProps` objects: the physics shape and mass settings a
//! static mesh carries for the game's Karma physics (its `KPhysicsProps`
//! property points at one). Loose props such as geo_04's falling rocks
//! collide using these shapes, not their drawn triangles.
//!
//! Layout read from the game's own `Engine.dll` (`UKMeshProps::Serialize`
//! at 0x10386d10 and the `FKAggregateGeom` reader at 0x10386ad0, looked at
//! locally with a disassembler; nothing of it is copied here): after the
//! (empty) property list come 6 floats of inertia tensor, a 3-float
//! centre-of-mass nudge, then five lists, each a compact count followed
//! by its elements. Every element starts with a 4x4 matrix (16 floats,
//! the element's placement inside the mesh):
//!
//! - spheres: matrix + radius
//! - boxes: matrix + X, Y, Z sizes (full lengths, not half)
//! - cylinders: matrix + radius, height
//! - convex hulls: matrix + a list of corner points (compact count + 3 floats each)
//! - sphyls (capsules): matrix + radius, length (only from package
//!   version 157; Republic Commando's are 159)
//!
//! All sizes are in the mesh's own units (Unreal units before the actor's
//! `DrawScale`).

use crate::properties::{self, Property};
use crate::{Export, Package, ReadError, Reader, Result};

/// A 4x4 matrix as Unreal stores it: rows X axis, Y axis, Z axis, origin.
pub type Matrix = [f32; 16];

#[derive(Debug, Clone, PartialEq)]
pub enum KElement {
    Sphere { tm: Matrix, radius: f32 },
    Box { tm: Matrix, size: [f32; 3] },
    Cylinder { tm: Matrix, radius: f32, height: f32 },
    Convex { tm: Matrix, points: Vec<[f32; 3]> },
    Sphyl { tm: Matrix, radius: f32, length: f32 },
}

impl KElement {
    pub fn tm(&self) -> &Matrix {
        match self {
            KElement::Sphere { tm, .. }
            | KElement::Box { tm, .. }
            | KElement::Cylinder { tm, .. }
            | KElement::Convex { tm, .. }
            | KElement::Sphyl { tm, .. } => tm,
        }
    }
}

#[derive(Debug, Clone)]
pub struct KMeshProps {
    pub properties: Vec<Property>,
    /// Inertia tensor (xx, xy, xz, yy, yz, zz) for a mass of 1, in Karma
    /// units.
    pub inertia: [f32; 6],
    /// Centre-of-mass offset, in Karma units.
    pub com_nudge: [f32; 3],
    pub elements: Vec<KElement>,
}

/// Package version from which the capsule list is stored (0x9d, the check
/// in `Engine.dll` at 0x10386b23).
const SPHYL_VERSION: u16 = 157;

fn matrix(r: &mut Reader) -> Result<Matrix> {
    let mut m = [0.0; 16];
    for v in &mut m {
        *v = r.f32()?;
    }
    Ok(m)
}

fn count(r: &mut Reader, what: &str) -> Result<usize> {
    let n = r.compact_index()?;
    match usize::try_from(n) {
        Ok(n) if n <= 100_000 => Ok(n),
        _ => r.invalid(format!("bad {what} count {n}")),
    }
}

/// Read a `KMeshProps` export.
pub fn read_kmesh_props(pkg: &Package, data: &[u8], export: &Export) -> Result<KMeshProps> {
    let start = export.serial_offset as usize;
    let Some(object) = data.get(start..start + export.serial_size as usize) else {
        return Err(ReadError::Invalid {
            offset: start,
            what: "export data is outside the file".into(),
        });
    };
    let mut r = Reader::new(object);
    let properties = properties::read_properties(pkg, &mut r)?;
    let mut inertia = [0.0; 6];
    for v in &mut inertia {
        *v = r.f32()?;
    }
    let com_nudge = [r.f32()?, r.f32()?, r.f32()?];
    let mut elements = Vec::new();
    for _ in 0..count(&mut r, "sphere")? {
        let tm = matrix(&mut r)?;
        elements.push(KElement::Sphere { tm, radius: r.f32()? });
    }
    for _ in 0..count(&mut r, "box")? {
        let tm = matrix(&mut r)?;
        elements.push(KElement::Box {
            tm,
            size: [r.f32()?, r.f32()?, r.f32()?],
        });
    }
    for _ in 0..count(&mut r, "cylinder")? {
        let tm = matrix(&mut r)?;
        elements.push(KElement::Cylinder {
            tm,
            radius: r.f32()?,
            height: r.f32()?,
        });
    }
    for _ in 0..count(&mut r, "convex")? {
        let tm = matrix(&mut r)?;
        let n = count(&mut r, "convex point")?;
        let mut points = Vec::with_capacity(n);
        for _ in 0..n {
            points.push([r.f32()?, r.f32()?, r.f32()?]);
        }
        elements.push(KElement::Convex { tm, points });
    }
    if pkg.summary.version >= SPHYL_VERSION {
        for _ in 0..count(&mut r, "sphyl")? {
            let tm = matrix(&mut r)?;
            elements.push(KElement::Sphyl {
                tm,
                radius: r.f32()?,
                length: r.f32()?,
            });
        }
    }
    if r.pos() != r.len() {
        return r.invalid(format!(
            "KMeshProps has {} bytes left over",
            r.len() - r.pos()
        ));
    }
    Ok(KMeshProps {
        properties,
        inertia,
        com_nudge,
        elements,
    })
}
