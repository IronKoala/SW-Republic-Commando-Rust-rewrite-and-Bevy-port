//! Reads the level's baked lighting: the "lightmaps" stored at the end of the
//! BSP `Model` object, after the vertex pool that `bsp::read_model` stops at.
//!
//! When a level was built, the editor worked out how much light reaches each
//! spot on each wall and saved it as small pictures (one rectangle per
//! surface), packed together onto 512x512 "pages". Drawing a wall's texture
//! multiplied by its lightmap gives the lit look of the original game.
//!
//! Layout, in order (all measured on `geo_01a`, `pro`, `ras_01a` and
//! `yyy_01b`; every object is read exactly to its end):
//!
//! - `NumZones` and the zones (an actor, 16 bytes, 4 bytes each), the
//!   `Polys` object, `Bounds` (25 bytes each), `LeafHulls` (ints),
//!   `Leaves` (two indices and 8 bytes each), `Lights` (actor indices),
//!   `RootOutside`, `Linked`;
//! - render sections: vertices of 40 bytes, a node count, a material and
//!   12 bytes each;
//! - lightmap entries: page, surface, zone, offset X/Y and size X/Y on the
//!   page, a 4x4 world-to-lightmap matrix and 9 more floats, then the shadow
//!   bitmaps of the dynamic lights that touch it, then 5 indices;
//! - pages: the lightmap entries on it, 3 ints, two mip levels of pixels
//!   (stored as "lazy arrays" that start with the absolute file offset of
//!   their end), then the pixel format, width, height and a revision number.
//!
//! The names (`FLightMap`, `FLightMapTexture`, `WorldToLightMap`) follow the
//! public UE2 headers, read in SWRC-Modding/CT (no license, nothing copied).
//! The byte layout was worked out from the game's maps and checked two ways:
//! every polygon corner maps inside its own lightmap rectangle, and every
//! lightmap names the same surface as the BSP node that uses it.

use crate::bsp::{self, count, Bsp, Vec3};
use rc_package::{Reader, Result};

/// One surface's rectangle of baked light on a page.
#[derive(Debug, Clone)]
pub struct LightMap {
    pub page: i32,
    pub surf: i32,
    pub zone: i32,
    /// Top-left corner of the rectangle on its page, in texels.
    pub offset: [i32; 2],
    /// Size of the rectangle in texels.
    pub size: [i32; 2],
    /// Turns a world position (Unreal units) into texels inside the
    /// rectangle. Stored the UE way: `matrix[row * 4 + column]`, with a point
    /// as a row vector on the left, so the translation is in 12..15.
    pub world_to_lightmap: [f32; 16],
    /// The 9 floats after the matrix (an origin, then pairs whose meaning is
    /// not known). Kept for later study.
    pub extra: [f32; 9],
    /// How many dynamic lights have a shadow bitmap for this rectangle.
    pub shadow_bitmaps: usize,
}

/// A 512x512 picture holding many lightmap rectangles.
#[derive(Debug, Clone)]
pub struct Page {
    /// Indices into `LightMaps::entries`.
    pub lightmaps: Vec<i32>,
    /// Texture format number, as in `Texture` objects (7 = DXT3 was seen).
    pub format: u8,
    pub width: usize,
    pub height: usize,
    /// The full-size pixels, still compressed in `format`.
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct LightMaps {
    pub entries: Vec<LightMap>,
    pub pages: Vec<Page>,
}

fn i32s<const N: usize>(r: &mut Reader) -> Result<[i32; N]> {
    let mut out = [0; N];
    for v in &mut out {
        *v = r.compact_index()?;
    }
    Ok(out)
}

/// Read the lightmaps. `object` is the `Model` object's bytes, and
/// `object_offset` is where they start in the file (lazy arrays store
/// absolute file offsets, which are checked).
pub fn read_lightmaps(bsp: &Bsp, object: &[u8], object_offset: usize) -> Result<LightMaps> {
    let mut r = Reader::new(object);
    r.seek(bsp.read_end)?;
    let _num_shared_sides = r.i32()?;
    let zones = r.i32()?;
    if !(0..=64).contains(&zones) {
        return r.invalid(format!("zone count {zones}"));
    }
    for _ in 0..zones {
        r.compact_index()?;
        r.bytes(20)?;
    }
    r.compact_index()?; // Polys
    let n = count(&mut r, "bounds", 25)?;
    r.bytes(n * 25)?;
    let n = count(&mut r, "leaf hulls", 4)?;
    r.bytes(n * 4)?;
    let n = count(&mut r, "leaves", 10)?;
    for _ in 0..n {
        r.compact_index()?;
        r.compact_index()?;
        r.bytes(8)?;
    }
    let n = count(&mut r, "lights", 1)?;
    for _ in 0..n {
        r.compact_index()?;
    }
    r.i32()?; // RootOutside
    r.i32()?; // Linked
    let n = count(&mut r, "sections", 18)?;
    for _ in 0..n {
        let vertices = count(&mut r, "section vertices", 40)?;
        r.bytes(vertices * 40)?;
        r.i32()?;
        r.compact_index()?;
        r.bytes(12)?;
    }

    let n = count(&mut r, "lightmaps", 107)?;
    let mut entries = Vec::with_capacity(n);
    for _ in 0..n {
        let [page, surf, zone, ox, oy, sx, sy] = i32s::<7>(&mut r)?;
        let mut m = [0.0; 25];
        for v in &mut m {
            *v = bsp::f32(&mut r)?;
        }
        let bitmaps = count(&mut r, "shadow bitmaps", 30)?;
        for _ in 0..bitmaps {
            r.compact_index()?; // the light
            let bits = count(&mut r, "shadow bits", 1)?;
            r.bytes(bits)?;
            r.bytes(28)?; // size, stride, min and max
        }
        i32s::<5>(&mut r)?;
        entries.push(LightMap {
            page,
            surf,
            zone,
            offset: [ox, oy],
            size: [sx, sy],
            world_to_lightmap: m[..16].try_into().unwrap(),
            extra: m[16..].try_into().unwrap(),
            shadow_bitmaps: bitmaps,
        });
    }

    let n = count(&mut r, "lightmap pages", 30)?;
    let mut pages = Vec::with_capacity(n);
    for _ in 0..n {
        r.compact_index()?; // the level
        let k = count(&mut r, "page lightmaps", 4)?;
        let mut lightmaps = Vec::with_capacity(k);
        for _ in 0..k {
            lightmaps.push(r.i32()?);
        }
        r.bytes(12)?;
        let mut mips = Vec::new();
        for _ in 0..2 {
            let end = r.i32()? as usize;
            let len = count(&mut r, "lightmap pixels", 1)?;
            let pixels = r.bytes(len)?;
            if end != object_offset + r.pos() {
                return r.invalid("lightmap pixels don't end where their header says");
            }
            mips.push(pixels);
        }
        let format = r.u8()?;
        let width = r.i32()?;
        let height = r.i32()?;
        r.i32()?; // revision
        if !(1..=4096).contains(&width) || !(1..=4096).contains(&height) {
            return r.invalid(format!("lightmap page size {width}x{height}"));
        }
        pages.push(Page {
            lightmaps,
            format,
            width: width as usize,
            height: height as usize,
            pixels: mips[0].to_vec(),
        });
    }
    if r.pos() != object.len() {
        return r.invalid(format!(
            "{} bytes left after the lightmap pages",
            object.len() - r.pos()
        ));
    }
    Ok(LightMaps { entries, pages })
}

impl LightMap {
    /// Texel position of a world point inside this rectangle (0..size).
    pub fn texel(&self, p: Vec3) -> [f32; 2] {
        let m = &self.world_to_lightmap;
        [
            p[0] * m[0] + p[1] * m[4] + p[2] * m[8] + m[12],
            p[0] * m[1] + p[1] * m[5] + p[2] * m[9] + m[13],
        ]
    }
}

impl LightMaps {
    /// The lightmap entry a BSP node uses, if any.
    pub fn for_node(&self, node: &bsp::Node) -> Option<&LightMap> {
        usize::try_from(node.light_map)
            .ok()
            .and_then(|i| self.entries.get(i))
    }

    /// Where a world point on a node's polygon falls on its lightmap page,
    /// in the usual 0..1 texture range. `None` when the node has no lightmap
    /// or its page is missing.
    pub fn page_uv(&self, node: &bsp::Node, p: Vec3) -> Option<(usize, [f32; 2])> {
        let lm = self.for_node(node)?;
        let page = usize::try_from(lm.page).ok()?;
        let pg = self.pages.get(page)?;
        let t = lm.texel(p);
        Some((
            page,
            [
                (lm.offset[0] as f32 + t[0]) / pg.width as f32,
                (lm.offset[1] as f32 + t[1]) / pg.height as f32,
            ],
        ))
    }

    /// Checks that only make sense if the layout was read correctly. Returns
    /// one line per problem; empty means all passed.
    pub fn check(&self, bsp: &Bsp) -> Vec<String> {
        let mut problems = Vec::new();
        let (mut corners, mut outside, mut wrong_surf, mut lit_nodes) = (0, 0, 0, 0);
        for (i, node) in bsp.nodes.iter().enumerate() {
            let Some(lm) = self.for_node(node) else {
                continue;
            };
            lit_nodes += 1;
            if lm.surf != node.surf {
                wrong_surf += 1;
            }
            let Some(poly) = bsp.polygon(i) else { continue };
            for p in &poly.points {
                corners += 1;
                let [u, v] = lm.texel(*p);
                // Corners land on texel edges, give or take rounding.
                let tol = 0.6;
                if u < -tol
                    || v < -tol
                    || u > lm.size[0] as f32 + tol
                    || v > lm.size[1] as f32 + tol
                {
                    outside += 1;
                }
            }
        }
        if outside > 0 {
            problems.push(format!(
                "{outside} of {corners} polygon corners fall outside their lightmap rectangle"
            ));
        }
        if wrong_surf > 0 {
            problems.push(format!(
                "{wrong_surf} of {lit_nodes} lit nodes use a lightmap made for another surface"
            ));
        }
        for (k, lm) in self.entries.iter().enumerate() {
            let Some(pg) = usize::try_from(lm.page)
                .ok()
                .and_then(|p| self.pages.get(p))
            else {
                problems.push(format!("lightmap {k} names missing page {}", lm.page));
                break;
            };
            if lm.offset[0] < 0
                || lm.offset[1] < 0
                || (lm.offset[0] + lm.size[0]) as usize > pg.width
                || (lm.offset[1] + lm.size[1]) as usize > pg.height
            {
                problems.push(format!("lightmap {k} doesn't fit on its page"));
                break;
            }
        }
        problems
    }
}
