//! Reads a `Model` object: the level's BSP ("binary space partition"), which
//! holds the level's own walls, floors and ceilings as flat polygons.
//!
//! Only the arrays needed to rebuild the polygons are read: vectors, points,
//! nodes, surfaces and the vertex pool. The zones, render sections and
//! lightmaps that follow them are read by `lightmap.rs`.
//!
//! Field names and meanings come from the public Unreal Engine 1/2 headers as
//! mirrored, read-only, in SWRC-Modding/CT (`Engine/Inc/UnObj.h`,
//! `Engine/Inc/UnModel.h`; no license, nothing copied). The byte layouts for
//! Republic Commando were worked out by reading `geo_01a.ctm` and checked on
//! other maps: every node, surface and vertex index has to land inside its
//! array, every polygon has to lie on its node's plane, and the arrays have to
//! end exactly where the next one starts.

use rc_package::properties::read_properties;
use rc_package::{Export, Package, Reader, Result};

/// Surface flags ("PolyFlags") that matter for drawing and collision.
/// Values from the UE2 `EPolyFlags` list.
pub mod poly_flags {
    pub const INVISIBLE: u32 = 0x0000_0001;
    pub const MASKED: u32 = 0x0000_0002;
    pub const TRANSLUCENT: u32 = 0x0000_0004;
    pub const NOT_SOLID: u32 = 0x0000_0008;
    pub const SEMISOLID: u32 = 0x0000_0020;
    pub const FAKE_BACKDROP: u32 = 0x0000_0080;
    pub const TWO_SIDED: u32 = 0x0000_0100;
    pub const UNLIT: u32 = 0x0040_0000;
    pub const PORTAL: u32 = 0x0400_0000;
}

pub type Vec3 = [f32; 3];

/// One node of the BSP tree. A node with vertices is one convex polygon.
#[derive(Debug, Clone)]
pub struct Node {
    /// Plane the polygon lies on: normal (x, y, z) and distance w, so that
    /// points on it satisfy `normal . p == w`.
    pub plane: [f32; 4],
    pub zone_mask: u64,
    pub node_flags: u8,
    /// First entry in the vertex pool (`Bsp::verts`).
    pub vert_pool: i32,
    /// Index into `Bsp::surfs`.
    pub surf: i32,
    pub back: i32,
    pub front: i32,
    /// Next polygon on the same plane, or -1.
    pub coplanar: i32,
    pub collision_bound: i32,
    pub render_bound: i32,
    /// Bounding sphere: centre and radius.
    pub sphere: [f32; 4],
    pub zone: [u8; 2],
    pub num_vertices: u8,
    pub leaf: [i32; 2],
    /// Three numbers Republic Commando stores after the leaves. From their
    /// values they look like the render section, the first vertex in that
    /// section, and the lightmap index. Not used yet.
    pub section: i32,
    pub section_first_vertex: i32,
    pub light_map: i32,
}

/// The settings shared by every polygon on one surface.
#[derive(Debug, Clone)]
pub struct Surf {
    /// Object reference to the material (texture or shader).
    pub material: i32,
    pub poly_flags: u32,
    /// Index into `Bsp::points` of the point where texture U and V are 0.
    pub base: i32,
    /// Indexes into `Bsp::vectors`.
    pub normal: i32,
    pub texture_u: i32,
    pub texture_v: i32,
    pub brush_poly: i32,
    /// The brush actor this surface came from.
    pub actor: i32,
    pub plane: [f32; 4],
    pub light_map_scale: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct Vert {
    /// Index into `Bsp::points`.
    pub point: i32,
    pub side: i32,
}

#[derive(Debug, Clone, Default)]
pub struct Bsp {
    pub vectors: Vec<Vec3>,
    pub points: Vec<Vec3>,
    pub nodes: Vec<Node>,
    pub surfs: Vec<Surf>,
    pub verts: Vec<Vert>,
    pub num_shared_sides: i32,
    /// Byte position (inside the object) where the vertex pool ended.
    pub read_end: usize,
    pub object_size: usize,
}

/// One drawable polygon, in Unreal coordinates (X forward, Y right, Z up).
#[derive(Debug, Clone)]
pub struct Polygon {
    pub node: usize,
    pub surf: usize,
    pub points: Vec<Vec3>,
    /// Texture coordinates in texels (divide by the texture's size to get
    /// the usual 0..1 range).
    pub uvs: Vec<[f32; 2]>,
    pub normal: Vec3,
    pub material: i32,
    pub poly_flags: u32,
}

pub(crate) fn f32(r: &mut Reader) -> Result<f32> {
    Ok(f32::from_bits(r.u32()?))
}

fn vec3(r: &mut Reader) -> Result<Vec3> {
    Ok([f32(r)?, f32(r)?, f32(r)?])
}

fn vec4(r: &mut Reader) -> Result<[f32; 4]> {
    Ok([f32(r)?, f32(r)?, f32(r)?, f32(r)?])
}

/// Reads an array length and refuses lengths that can't fit in what's left.
pub(crate) fn count(r: &mut Reader, what: &str, min_item_size: usize) -> Result<usize> {
    let n = r.compact_index()?;
    if n < 0 || n as usize * min_item_size > r.len() - r.pos() {
        return r.invalid(format!("{what} count {n} doesn't fit in the object"));
    }
    Ok(n as usize)
}

pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Read a `Model` export. `data` is the whole package file.
pub fn read_model(pkg: &Package, data: &[u8], export: &Export) -> Result<Bsp> {
    let start = export.serial_offset as usize;
    let end = start + export.serial_size as usize;
    let Some(bytes) = data.get(start..end) else {
        return Reader::new(data).invalid("model data lies outside the file");
    };
    let mut r = Reader::new(bytes);
    read_properties(pkg, &mut r)?;
    // UPrimitive: bounding box (min, max, "is valid" byte) and bounding
    // sphere. Zero for the level model, so not kept.
    r.bytes(12 + 12 + 1 + 16)?;

    let n = count(&mut r, "vector", 12)?;
    let vectors = (0..n).map(|_| vec3(&mut r)).collect::<Result<Vec<_>>>()?;
    let n = count(&mut r, "point", 12)?;
    let points = (0..n).map(|_| vec3(&mut r)).collect::<Result<Vec<_>>>()?;

    let n = count(&mut r, "node", 50)?;
    let mut nodes = Vec::with_capacity(n);
    for _ in 0..n {
        let plane = vec4(&mut r)?;
        let zone_mask = u64::from_le_bytes(r.bytes(8)?.try_into().unwrap());
        let node_flags = r.u8()?;
        let vert_pool = r.compact_index()?;
        let surf = r.compact_index()?;
        let back = r.compact_index()?;
        let front = r.compact_index()?;
        let coplanar = r.compact_index()?;
        let collision_bound = r.compact_index()?;
        let render_bound = r.compact_index()?;
        let sphere = vec4(&mut r)?;
        let zone = [r.u8()?, r.u8()?];
        let num_vertices = r.u8()?;
        let leaf = [r.i32()?, r.i32()?];
        nodes.push(Node {
            plane,
            zone_mask,
            node_flags,
            vert_pool,
            surf,
            back,
            front,
            coplanar,
            collision_bound,
            render_bound,
            sphere,
            zone,
            num_vertices,
            leaf,
            section: r.i32()?,
            section_first_vertex: r.i32()?,
            light_map: r.i32()?,
        });
    }

    let n = count(&mut r, "surface", 27)?;
    let mut surfs = Vec::with_capacity(n);
    for _ in 0..n {
        surfs.push(Surf {
            material: r.compact_index()?,
            poly_flags: r.u32()?,
            base: r.compact_index()?,
            normal: r.compact_index()?,
            texture_u: r.compact_index()?,
            texture_v: r.compact_index()?,
            brush_poly: r.compact_index()?,
            actor: r.compact_index()?,
            plane: vec4(&mut r)?,
            light_map_scale: f32(&mut r)?,
        });
    }

    let n = count(&mut r, "vertex", 2)?;
    let mut verts = Vec::with_capacity(n);
    for _ in 0..n {
        verts.push(Vert {
            point: r.compact_index()?,
            side: r.compact_index()?,
        });
    }
    let read_end = r.pos();
    let num_shared_sides = r.i32()?;

    Ok(Bsp {
        vectors,
        points,
        nodes,
        surfs,
        verts,
        num_shared_sides,
        read_end,
        object_size: bytes.len(),
    })
}

fn get<T>(v: &[T], i: i32) -> Option<&T> {
    usize::try_from(i).ok().and_then(|i| v.get(i))
}

impl Bsp {
    /// The polygon of one node, or `None` if the node has no vertices or an
    /// index is out of range (`check` reports those).
    pub fn polygon(&self, node_index: usize) -> Option<Polygon> {
        let node = self.nodes.get(node_index)?;
        if node.num_vertices == 0 {
            return None;
        }
        let surf = get(&self.surfs, node.surf)?;
        let mut points = Vec::with_capacity(node.num_vertices as usize);
        for k in 0..node.num_vertices as i32 {
            let v = get(&self.verts, node.vert_pool + k)?;
            points.push(*get(&self.points, v.point)?);
        }
        let base = *get(&self.points, surf.base)?;
        let tu = *get(&self.vectors, surf.texture_u)?;
        let tv = *get(&self.vectors, surf.texture_v)?;
        // Unreal's texture mapping: U = (P - Base) . TextureU, same for V,
        // measured in texels.
        let uvs = points
            .iter()
            .map(|&p| {
                let d = sub(p, base);
                [dot(d, tu), dot(d, tv)]
            })
            .collect();
        Some(Polygon {
            node: node_index,
            surf: node.surf as usize,
            points,
            uvs,
            normal: [node.plane[0], node.plane[1], node.plane[2]],
            material: surf.material,
            poly_flags: surf.poly_flags,
        })
    }

    /// Which zone a point is in, by walking the tree from the root: at each
    /// node go to the front child if the point is on the front of the
    /// plane, else the back; where there is no child, the node's zone on
    /// that side is the answer. Same walk as UE2's `UModel::PointRegion`
    /// (public UE2 headers). `None` for an empty tree.
    pub fn point_zone(&self, p: Vec3) -> Option<u8> {
        let mut i = 0usize;
        for _ in 0..self.nodes.len() {
            let n = self.nodes.get(i)?;
            let front = dot([n.plane[0], n.plane[1], n.plane[2]], p) - n.plane[3] >= 0.0;
            let child = if front { n.front } else { n.back };
            if child < 0 || child as usize >= self.nodes.len() {
                return Some(n.zone[front as usize]);
            }
            i = child as usize;
        }
        None
    }

    /// Every polygon in the BSP.
    pub fn polygons(&self) -> Vec<Polygon> {
        (0..self.nodes.len())
            .filter_map(|i| self.polygon(i))
            .collect()
    }

    /// Consistency checks that would fail if we read the format wrongly.
    /// Returns one line per problem (at most 20, then a count).
    pub fn check(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut more = 0;
        let mut add = |s: String| {
            if problems.len() < 20 {
                problems.push(s);
            } else {
                more += 1;
            }
        };
        let in_nodes = |i: i32| i == -1 || get(&self.nodes, i).is_some();
        for (i, n) in self.nodes.iter().enumerate() {
            let len = dot(
                [n.plane[0], n.plane[1], n.plane[2]],
                [n.plane[0], n.plane[1], n.plane[2]],
            );
            if (len - 1.0).abs() > 0.01 {
                add(format!(
                    "node {i}: plane normal length {:.3}, expected 1",
                    len.sqrt()
                ));
            }
            if !in_nodes(n.back) || !in_nodes(n.front) || !in_nodes(n.coplanar) {
                add(format!(
                    "node {i}: child index out of range (back {}, front {}, coplanar {})",
                    n.back, n.front, n.coplanar
                ));
            }
            if n.num_vertices == 0 {
                continue;
            }
            if get(&self.surfs, n.surf).is_none() {
                add(format!("node {i}: surface {} out of range", n.surf));
                continue;
            }
            let Some(poly) = self.polygon(i) else {
                add(format!(
                    "node {i}: vertex pool {}..+{} or a point index is out of range",
                    n.vert_pool, n.num_vertices
                ));
                continue;
            };
            // Every corner must lie on the node's plane.
            for p in &poly.points {
                let d = dot(*p, poly.normal) - n.plane[3];
                if d.abs() > 1.0 {
                    add(format!("node {i}: a corner is {d:.2} units off its plane"));
                    break;
                }
            }
        }
        for (i, s) in self.surfs.iter().enumerate() {
            if get(&self.points, s.base).is_none()
                || get(&self.vectors, s.normal).is_none()
                || get(&self.vectors, s.texture_u).is_none()
                || get(&self.vectors, s.texture_v).is_none()
            {
                add(format!(
                    "surface {i}: point or vector index out of range: {s:?}"
                ));
            }
        }
        for (i, v) in self.verts.iter().enumerate() {
            if v.side < -1 || v.side >= self.num_shared_sides {
                add(format!(
                    "vertex {i}: side {} out of range (shared sides {})",
                    v.side, self.num_shared_sides
                ));
            }
        }
        if more > 0 {
            problems.push(format!("... and {more} more problems"));
        }
        problems
    }

    /// Which way the polygon's corners go around, compared with its normal.
    /// +1 if the corner order, read with the right-hand rule on the raw
    /// numbers, points the same way as the normal; -1 if the opposite way; 0
    /// if the polygon is too small to tell. Swapping two axes (as the viewer
    /// does to go from Unreal to Bevy) flips this, so the viewer needs this
    /// to know which side of each triangle is the front.
    pub fn winding(poly: &Polygon) -> i32 {
        let mut n = [0.0f32; 3];
        for k in 1..poly.points.len().saturating_sub(1) {
            let c = cross(
                sub(poly.points[k], poly.points[0]),
                sub(poly.points[k + 1], poly.points[0]),
            );
            n = [n[0] + c[0], n[1] + c[1], n[2] + c[2]];
        }
        let d = dot(n, poly.normal);
        if d.abs() < 1e-3 {
            0
        } else if d > 0.0 {
            1
        } else {
            -1
        }
    }
}
