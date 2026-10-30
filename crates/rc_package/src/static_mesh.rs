//! Reads `StaticMesh` objects (`.usx` packages): a fixed 3D model made of
//! vertices (corner points), triangles and texture coordinates.
//!
//! Layout learned from UE Viewer (github.com/gildor2/UEViewer, MIT, Konstantin
//! Nosov): `Unreal/UnrealMesh/UnMesh2.h` and `UnMesh2.cpp` (UStaticMesh,
//! UPrimitive, the stream structs) and `Unreal/UnCore.h` (FBox, FSphere).
//! Republic Commando adds two 4-byte numbers after the vertex stream (one from
//! package version 149, one more from 155). Written from those descriptions;
//! no code copied.
//!
//! The collision data after the streams is not read.

use crate::properties::{self, Property, PropertyType, Value};
use crate::{Export, Package, ReadError, Reader, Result};

/// Object flag meaning the data starts with a script state frame.
const RF_HAS_STACK: u32 = 0x0200_0000;

/// An axis-aligned box: the smallest and largest x, y, z.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub is_valid: bool,
}

/// A part of the mesh drawn with one material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
    /// Position of the section's first index in `indices`.
    pub first_index: u16,
    pub first_vertex: u16,
    pub last_vertex: u16,
    /// Number of triangles.
    pub num_faces: u16,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

#[derive(Debug, Clone)]
pub struct StaticMesh {
    pub properties: Vec<Property>,
    /// Object reference of each section's material, from the `Materials`
    /// property (0 means none).
    pub materials: Vec<i32>,
    /// Whether each section blocks movement (`EnableCollision` in the
    /// `Materials` property; true when not stored). Decorations such as
    /// hanging cables or light shafts have it off. Same order as
    /// `materials`; may be empty.
    pub section_collision: Vec<bool>,
    pub bounding_box: BoundingBox,
    /// Centre (x, y, z) and radius.
    pub bounding_sphere: [f32; 4],
    pub sections: Vec<Section>,
    pub vertices: Vec<Vertex>,
    /// One colour per vertex, stored blue, green, red, alpha. Often empty.
    pub colors: Vec<[u8; 4]>,
    pub alphas: Vec<[u8; 4]>,
    /// Texture coordinate sets. Set 0 is the one textures use.
    pub uv_sets: Vec<Vec<[f32; 2]>>,
    /// Triangle list: every three numbers are one triangle's vertices.
    pub indices: Vec<u16>,
    /// Second index list (wireframe lines in UE2). Not used for drawing.
    pub indices2: Vec<u16>,
    /// Bytes after the part we read (collision data). Not checked.
    pub unread: usize,
}

/// Read a TArray's element count, rejecting counts that can't fit in the
/// bytes left (each element is at least `min_size` bytes).
fn array_len(r: &mut Reader, min_size: usize, what: &str) -> Result<usize> {
    let n = r.compact_index()?;
    if n < 0 || (n as usize).saturating_mul(min_size) > r.len() - r.pos() {
        return r.invalid(format!("{what} count {n} doesn't fit in the object"));
    }
    Ok(n as usize)
}

fn vector(r: &mut Reader) -> Result<[f32; 3]> {
    Ok([r.f32()?, r.f32()?, r.f32()?])
}

fn read_box(r: &mut Reader) -> Result<BoundingBox> {
    Ok(BoundingBox {
        min: vector(r)?,
        max: vector(r)?,
        is_valid: r.u8()? != 0,
    })
}

fn colors(r: &mut Reader, what: &str) -> Result<Vec<[u8; 4]>> {
    let n = array_len(r, 4, what)?;
    let out = (0..n)
        .map(|_| r.bytes(4).map(|c| [c[0], c[1], c[2], c[3]]))
        .collect::<Result<Vec<_>>>()?;
    r.i32()?; // revision
    Ok(out)
}

fn index_buffer(r: &mut Reader, what: &str) -> Result<Vec<u16>> {
    let n = array_len(r, 2, what)?;
    let out = (0..n).map(|_| r.u16()).collect::<Result<Vec<_>>>()?;
    r.i32()?; // revision
    Ok(out)
}

/// The `Materials` property is an array of structs; each struct is its own
/// property list (`Material`, `EnableCollision`) ending in "None". Returns
/// each section's material and whether it blocks movement.
fn material_refs(pkg: &Package, props: &[Property]) -> Result<(Vec<i32>, Vec<bool>)> {
    let Some(p) = properties::find(pkg, props, "Materials") else {
        return Ok((Vec::new(), Vec::new()));
    };
    let Value::Raw(raw) = &p.value else {
        return Ok((Vec::new(), Vec::new()));
    };
    if p.kind != PropertyType::Array {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut r = Reader::new(raw);
    let n = array_len(&mut r, 1, "material")?;
    let mut out = Vec::with_capacity(n);
    let mut collide = Vec::with_capacity(n);
    for _ in 0..n {
        let fields = properties::read_properties(pkg, &mut r)?;
        out.push(
            match properties::find(pkg, &fields, "Material").map(|f| &f.value) {
                Some(Value::Object(i)) => *i,
                _ => 0,
            },
        );
        collide.push(!matches!(
            properties::find(pkg, &fields, "EnableCollision").map(|f| &f.value),
            Some(Value::Bool(false))
        ));
    }
    if r.pos() != raw.len() {
        return r.invalid(format!(
            "{} bytes left over in the Materials property",
            raw.len() - r.pos()
        ));
    }
    Ok((out, collide))
}

/// Read a `StaticMesh` export. `data` is the whole package file.
pub fn read_static_mesh(pkg: &Package, data: &[u8], export: &Export) -> Result<StaticMesh> {
    let start = export.serial_offset as usize;
    let Some(object) = data.get(start..start + export.serial_size as usize) else {
        return Err(ReadError::Invalid {
            offset: start,
            what: "export data is outside the file".into(),
        });
    };
    let mut r = Reader::new(object);
    if export.object_flags & RF_HAS_STACK != 0 {
        return r.invalid("object has a script state frame, not supported for meshes");
    }
    let version = pkg.summary.version;
    if version < 112 {
        return r.invalid(format!(
            "static mesh in package version {version}, not supported"
        ));
    }
    let properties = properties::read_properties(pkg, &mut r)?;
    let (materials, section_collision) = material_refs(pkg, &properties)?;

    // UPrimitive: bounding box and sphere.
    let bounding_box = read_box(&mut r)?;
    let bounding_sphere = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];

    let n = array_len(&mut r, 14, "section")?;
    let mut sections = Vec::with_capacity(n);
    for _ in 0..n {
        r.i32()?; // always 0 in UE Viewer's notes
        let first_index = r.u16()?;
        let first_vertex = r.u16()?;
        let last_vertex = r.u16()?;
        r.u16()?; // unknown
        let num_faces = r.u16()?;
        sections.push(Section {
            first_index,
            first_vertex,
            last_vertex,
            num_faces,
        });
    }
    // The bounding box is stored a second time here.
    read_box(&mut r)?;

    let n = array_len(&mut r, 24, "vertex")?;
    let mut vertices = Vec::with_capacity(n);
    for _ in 0..n {
        vertices.push(Vertex {
            position: vector(&mut r)?,
            normal: vector(&mut r)?,
        });
    }
    r.i32()?; // revision
              // Republic Commando extras, meaning unknown.
    if version >= 155 {
        r.i32()?;
    }
    if version >= 149 {
        r.i32()?;
    }

    let colors_ = colors(&mut r, "colour")?;
    let alphas = colors(&mut r, "alpha")?;

    let n = array_len(&mut r, 9, "UV set")?;
    let mut uv_sets = Vec::with_capacity(n);
    for _ in 0..n {
        let count = array_len(&mut r, 8, "UV")?;
        let uvs = (0..count)
            .map(|_| Ok([r.f32()?, r.f32()?]))
            .collect::<Result<Vec<_>>>()?;
        r.i32()?; // unknown
        r.i32()?; // unknown
        uv_sets.push(uvs);
    }
    let indices = index_buffer(&mut r, "index")?;
    let indices2 = index_buffer(&mut r, "second index")?;
    r.compact_index()?; // object reference, unknown (collision?)

    Ok(StaticMesh {
        properties,
        materials,
        section_collision,
        bounding_box,
        bounding_sphere,
        sections,
        vertices,
        colors: colors_,
        alphas,
        uv_sets,
        indices,
        indices2,
        unread: object.len() - r.pos(),
    })
}

impl StaticMesh {
    /// Whether triangle number `k` (its corners at `indices[3k..3k+3]`)
    /// blocks movement: false when it is in a section with collision off.
    pub fn triangle_collides(&self, k: usize) -> bool {
        let at = k * 3;
        for (s, sec) in self.sections.iter().enumerate() {
            let first = sec.first_index as usize;
            if at >= first && at < first + sec.num_faces as usize * 3 {
                return self.section_collision.get(s).copied().unwrap_or(true);
            }
        }
        true
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// The box around the vertices we actually read.
    pub fn vertex_bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let first = self.vertices.first()?.position;
        let (mut lo, mut hi) = (first, first);
        for v in &self.vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(v.position[k]);
                hi[k] = hi[k].max(v.position[k]);
            }
        }
        Some((lo, hi))
    }

    /// How many vertices have a normal of exactly (0, 0, 0).
    pub fn zero_normals(&self) -> usize {
        self.vertices
            .iter()
            .filter(|v| v.normal == [0.0; 3])
            .count()
    }

    /// Smallest and largest U and V of a UV set.
    pub fn uv_range(&self, set: usize) -> Option<([f32; 2], [f32; 2])> {
        let uvs = self.uv_sets.get(set)?;
        let first = *uvs.first()?;
        let (mut lo, mut hi) = (first, first);
        for uv in uvs {
            for k in 0..2 {
                lo[k] = lo[k].min(uv[k]);
                hi[k] = hi[k].max(uv[k]);
            }
        }
        Some((lo, hi))
    }

    /// Problems that would show up if we read the layout wrongly.
    pub fn check(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let nv = self.vertices.len();
        if nv == 0 {
            problems.push("no vertices".into());
        }
        if !self.indices.len().is_multiple_of(3) {
            problems.push(format!(
                "{} indices is not a whole number of triangles",
                self.indices.len()
            ));
        }
        if let Some(i) = self.indices.iter().find(|&&i| i as usize >= nv) {
            problems.push(format!("index {i} points past the {nv} vertices"));
        }
        let faces: usize = self.sections.iter().map(|s| s.num_faces as usize).sum();
        if faces != self.triangle_count() {
            problems.push(format!(
                "sections hold {faces} triangles but the index list has {}",
                self.triangle_count()
            ));
        }
        for (k, s) in self.sections.iter().enumerate() {
            let end = s.first_index as usize + s.num_faces as usize * 3;
            if end > self.indices.len() {
                problems.push(format!("section {k} ends at index {end}, past the list"));
                continue;
            }
            let idx = &self.indices[s.first_index as usize..end];
            if idx.iter().any(|&i| i < s.first_vertex || i > s.last_vertex) {
                problems.push(format!(
                    "section {k} uses vertices outside its range {}..={}",
                    s.first_vertex, s.last_vertex
                ));
            }
        }
        if !self.materials.is_empty() && self.materials.len() != self.sections.len() {
            problems.push(format!(
                "{} materials for {} sections",
                self.materials.len(),
                self.sections.len()
            ));
        }
        for (k, uvs) in self.uv_sets.iter().enumerate() {
            if uvs.len() != nv {
                problems.push(format!(
                    "UV set {k} has {} entries for {nv} vertices",
                    uvs.len()
                ));
            }
        }
        if self.uv_sets.is_empty() {
            problems.push("no texture coordinates".into());
        }
        for (what, c) in [("colour", &self.colors), ("alpha", &self.alphas)] {
            if !c.is_empty() && c.len() != nv {
                problems.push(format!("{} {what} entries for {nv} vertices", c.len()));
            }
        }
        // Normals should be about 1 long. Exactly zero is allowed: 11 of the
        // game's 2,447 static meshes have a few, probably from triangles with
        // no area in the artists' tools (see `zero_normals`).
        let bad_normals = self
            .vertices
            .iter()
            .filter(|v| {
                let l = v.normal.iter().map(|n| n * n).sum::<f32>().sqrt();
                l != 0.0 && !(0.9..=1.1).contains(&l)
            })
            .count();
        if bad_normals > 0 {
            problems.push(format!("{bad_normals} normals are not about 1 long"));
        }
        // Every vertex should be inside the stored bounding box (with a
        // little slack for rounding).
        if let Some((lo, hi)) = self.vertex_bounds() {
            let b = &self.bounding_box;
            let slack = 0.01;
            if (0..3).any(|k| lo[k] < b.min[k] - slack || hi[k] > b.max[k] + slack) {
                problems.push(format!(
                    "vertices span {lo:?}..{hi:?}, outside the stored box {:?}..{:?}",
                    b.min, b.max
                ));
            }
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Triangles in a section with collision off don't collide; the rest
    /// (and meshes that store no flags) do.
    #[test]
    fn section_collision_by_triangle() {
        let sec = |first_index, num_faces| Section {
            first_index,
            first_vertex: 0,
            last_vertex: 0,
            num_faces,
        };
        let mut m = StaticMesh {
            properties: Vec::new(),
            materials: vec![0, 0],
            section_collision: vec![true, false],
            bounding_box: BoundingBox {
                min: [0.0; 3],
                max: [0.0; 3],
                is_valid: false,
            },
            bounding_sphere: [0.0; 4],
            sections: vec![sec(0, 2), sec(6, 1)],
            vertices: Vec::new(),
            colors: Vec::new(),
            alphas: Vec::new(),
            uv_sets: Vec::new(),
            indices: vec![0; 9],
            indices2: Vec::new(),
            unread: 0,
        };
        assert!(m.triangle_collides(0));
        assert!(m.triangle_collides(1));
        assert!(!m.triangle_collides(2));
        m.section_collision.clear();
        assert!(m.triangle_collides(2));
    }

    fn f(out: &mut Vec<u8>, v: f32) {
        out.extend_from_slice(&v.to_le_bytes());
    }

    /// One triangle, built byte by byte in the layout described above.
    fn one_triangle(version: u16) -> (Package, Vec<u8>) {
        let mut b = Vec::new();
        b.push(0); // property list: "None" straight away
        for v in [0.0, 0.0, 0.0, 1.0, 1.0, 0.0] {
            f(&mut b, v); // bounding box min, max
        }
        b.push(1); // box is valid
        for v in [0.5, 0.5, 0.0, 0.8] {
            f(&mut b, v); // sphere
        }
        b.push(1); // 1 section
        b.extend_from_slice(&0i32.to_le_bytes());
        for v in [0u16, 0, 2, 0, 1] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for v in [0.0, 0.0, 0.0, 1.0, 1.0, 0.0] {
            f(&mut b, v);
        }
        b.push(1); // second box valid
        b.push(3); // 3 vertices
        for p in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for v in p {
                f(&mut b, v);
            }
            for v in [0.0, 0.0, 1.0] {
                f(&mut b, v);
            }
        }
        b.extend_from_slice(&[0; 4]); // revision
        b.extend_from_slice(&[0; 8]); // the two Republic Commando extras
        b.extend_from_slice(&[0, 0, 0, 0, 0]); // no colours + revision
        b.extend_from_slice(&[0, 0, 0, 0, 0]); // no alphas + revision
        b.push(1); // 1 UV set
        b.push(3);
        for v in [0.0, 0.0, 1.0, 0.0, 0.0, 1.0] {
            f(&mut b, v);
        }
        b.extend_from_slice(&[0; 8]);
        b.push(3); // 3 indices
        for v in [0u16, 1, 2] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&[0, 0, 0, 0, 0]); // empty second index list
        b.push(0); // object reference: none
        b.extend_from_slice(&[0xAA; 7]); // stands in for collision data

        let mut pkg = crate::properties::tests::package_with_names(&["None"]);
        pkg.summary.version = version;
        let export = Export {
            class_index: 0,
            super_index: 0,
            package_index: 0,
            rc_unknown: None,
            object_name: 0,
            object_flags: 0,
            serial_size: b.len() as i32,
            serial_offset: 0,
        };
        pkg.exports.push(export);
        (pkg, b)
    }

    #[test]
    fn reads_one_triangle() {
        let (pkg, data) = one_triangle(159);
        let m = read_static_mesh(&pkg, &data, &pkg.exports[0]).unwrap();
        assert_eq!(m.vertices.len(), 3);
        assert_eq!(m.indices, vec![0, 1, 2]);
        assert_eq!(m.uv_sets[0][1], [1.0, 0.0]);
        assert_eq!(m.unread, 7);
        assert!(m.check().is_empty(), "{:?}", m.check());
    }
}
