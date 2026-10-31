//! Reads `Texture` objects: their properties and the list of mipmaps
//! (the picture at full size, half size, quarter size, ...).
//!
//! Layout learned from UE Viewer (github.com/gildor2/UEViewer, MIT, Konstantin
//! Nosov): `Unreal/UnrealMaterial/UnTexture2.cpp`, `UnMaterial2.h` (FMipmap,
//! ETextureFormat) and `UnCoreSerialize.cpp` (lazy arrays). Republic Commando
//! has no layout change there; UE Viewer guesses (unverified) its format 14 is
//! CxV8U8. We measured instead that 12 is V8U8 and 14 is L6V5U5, two bump-map
//! layouts from Direct3D (see `docs/DESIGN.md`, "Bump maps").
//! Written from those descriptions; no code copied.

use crate::properties::{self, Property, Value};
use crate::{Export, Package, ReadError, Reader, Result};

/// Object flag meaning the data starts with a script state frame.
const RF_HAS_STACK: u32 = 0x0200_0000;

#[derive(Debug, Clone)]
pub struct Mip {
    pub width: i32,
    pub height: i32,
    pub u_bits: u8,
    pub v_bits: u8,
    /// Where the pixel bytes start, counted from the start of the file.
    pub data_offset: usize,
    pub data_len: usize,
    /// The "skip position" stored before the pixels: the file offset just
    /// after them. Checked against what we actually read.
    pub skip_pos: i32,
}

#[derive(Debug, Clone)]
pub struct Texture {
    pub properties: Vec<Property>,
    /// Format number from the `Format` property (0 if absent, the default).
    pub format: u8,
    pub mips: Vec<Mip>,
    /// Bytes left in the object after the mip list. Expected to be 0.
    pub leftover: usize,
}

/// Which group of pictures a texture belongs to, for the TEXTURE QUALITY
/// and CHARACTER DETAIL options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    /// Walls, floors, props, the sky.
    World = 0,
    /// The squad, enemies, the player's arms and weapons.
    Characters = 1,
}

static SKIP_MIPS: [std::sync::atomic::AtomicU8; 2] =
    [std::sync::atomic::AtomicU8::new(0), std::sync::atomic::AtomicU8::new(0)];

/// Never go below this size when skipping mipmaps (and never below the
/// 4x4 block the compressed formats need).
const SMALLEST_SKIPPED_TO: i32 = 32;

/// Use a smaller picture for this group: 0 is full size, 1 is half size,
/// 2 is quarter size. Applies to pictures decoded from now on (the game
/// sets it from the options and it takes effect on the next level load).
pub fn set_skip_mips(group: Detail, skip: u8) {
    SKIP_MIPS[group as usize].store(skip.min(4), std::sync::atomic::Ordering::Relaxed);
}

pub fn skip_mips(group: Detail) -> u8 {
    SKIP_MIPS[group as usize].load(std::sync::atomic::Ordering::Relaxed)
}

impl Texture {
    /// The picture to draw for this group's detail setting: the full-size
    /// one, or a smaller mipmap if the setting asks for less detail and the
    /// texture has one big enough.
    pub fn mip_for(&self, group: Detail) -> Option<&Mip> {
        pick_mip(&self.mips, skip_mips(group) as usize)
    }
}

fn pick_mip(mips: &[Mip], skip: usize) -> Option<&Mip> {
    let mut best = mips.first()?;
    for m in mips.iter().skip(1).take(skip) {
        if m.width.min(m.height) < SMALLEST_SKIPPED_TO || m.data_len == 0 {
            break;
        }
        best = m;
    }
    Some(best)
}

/// Name of a texture format number (UE2's ETextureFormat).
pub fn format_name(format: u8) -> &'static str {
    match format {
        0 => "P8",
        1 => "RGBA7",
        2 => "RGB16",
        3 => "DXT1",
        4 => "RGB8",
        5 => "RGBA8",
        6 => "NODATA",
        7 => "DXT3",
        8 => "DXT5",
        9 => "L8",
        10 => "G16",
        11 => "RRRGGGBBB",
        12 => "V8U8",
        14 => "L6V5U5",
        _ => "unknown",
    }
}

/// How many bytes a mip of this size should take, if we know the format.
pub fn expected_mip_bytes(format: u8, width: i32, height: i32) -> Option<usize> {
    let (w, h) = (width.max(1) as usize, height.max(1) as usize);
    let blocks = w.div_ceil(4) * h.div_ceil(4);
    match format {
        0 | 9 => Some(w * h),
        2 | 10 | 12 | 14 => Some(w * h * 2),
        4 => Some(w * h * 3),
        5 => Some(w * h * 4),
        3 => Some(blocks * 8),
        7 | 8 => Some(blocks * 16),
        _ => None,
    }
}

impl Texture {
    /// The `Palette` property: an object reference, or `None` if absent.
    pub fn palette_ref(&self, pkg: &Package) -> Option<i32> {
        match self.property(pkg, "Palette").map(|p| &p.value) {
            Some(Value::Object(i)) if *i != 0 => Some(*i),
            _ => None,
        }
    }

    /// Whether the `bMasked` property is set: palette colour 0 is see-through.
    pub fn masked(&self, pkg: &Package) -> bool {
        matches!(
            self.property(pkg, "bMasked").map(|p| &p.value),
            Some(Value::Bool(true))
        )
    }

    pub fn property<'a>(&'a self, pkg: &Package, name: &str) -> Option<&'a Property> {
        properties::find(pkg, &self.properties, name)
    }

    /// Problems found by comparing the mips to each other and to the format.
    pub fn check(&self, pkg: &Package) -> Vec<String> {
        let mut problems = Vec::new();
        if self.mips.is_empty() {
            problems.push("no mips".into());
        }
        let int_prop = |n| match self.property(pkg, n).map(|p| &p.value) {
            Some(Value::Int(v)) => Some(*v),
            _ => None,
        };
        if let (Some(u), Some(m)) = (int_prop("USize"), self.mips.first()) {
            if u != m.width {
                problems.push(format!("USize property {u} but mip 0 is {} wide", m.width));
            }
        }
        if let (Some(v), Some(m)) = (int_prop("VSize"), self.mips.first()) {
            if v != m.height {
                problems.push(format!("VSize property {v} but mip 0 is {} high", m.height));
            }
        }
        for (i, m) in self.mips.iter().enumerate() {
            if m.skip_pos as i64 != (m.data_offset + m.data_len) as i64 {
                problems.push(format!(
                    "mip {i}: skip position {} but data ends at {}",
                    m.skip_pos,
                    m.data_offset + m.data_len
                ));
            }
            if let Some(want) = expected_mip_bytes(self.format, m.width, m.height) {
                if want != m.data_len {
                    problems.push(format!(
                        "mip {i}: {}x{} {} should be {want} bytes, found {}",
                        m.width,
                        m.height,
                        format_name(self.format),
                        m.data_len
                    ));
                }
            }
        }
        if self.leftover != 0 {
            problems.push(format!("{} bytes left over after the mips", self.leftover));
        }
        problems
    }
}

/// Read a `Palette` export: its properties, then 256 colours stored as red,
/// green, blue, alpha (UE Viewer, `UnMaterial2.h`, UPalette; FColor in
/// `UnCore.h` uses RGBA order before UE3).
pub fn read_palette(pkg: &Package, data: &[u8], export: &Export) -> Result<Vec<[u8; 4]>> {
    let start = export.serial_offset as usize;
    let Some(object) = data.get(start..start + export.serial_size as usize) else {
        return Err(ReadError::Invalid {
            offset: start,
            what: "export data is outside the file".into(),
        });
    };
    let mut r = Reader::new(object);
    properties::read_properties(pkg, &mut r)?;
    let count = r.compact_index()?;
    if count != 256 {
        return r.invalid(format!("palette has {count} colours, expected 256"));
    }
    let colors = (0..count)
        .map(|_| r.bytes(4).map(|c| [c[0], c[1], c[2], c[3]]))
        .collect::<Result<Vec<_>>>()?;
    if r.pos() != object.len() {
        return r.invalid(format!(
            "{} bytes left over after the palette",
            object.len() - r.pos()
        ));
    }
    Ok(colors)
}

/// Read a `Texture` export. `data` is the whole package file.
pub fn read_texture(pkg: &Package, data: &[u8], export: &Export) -> Result<Texture> {
    let start = export.serial_offset as usize;
    let end = start + export.serial_size as usize;
    let Some(object) = data.get(start..end) else {
        return Err(ReadError::Invalid {
            offset: start,
            what: "export data is outside the file".into(),
        });
    };
    let mut r = Reader::new(object);
    if export.object_flags & RF_HAS_STACK != 0 {
        return r.invalid("object has a script state frame, not supported for textures");
    }
    let properties = properties::read_properties(pkg, &mut r)?;
    let format = match properties::find(pkg, &properties, "Format").map(|p| &p.value) {
        Some(Value::Byte(f)) => *f,
        _ => 0,
    };

    let count = r.compact_index()?;
    if !(0..=32).contains(&count) {
        return r.invalid(format!("mip count {count} is implausible"));
    }
    let mut mips = Vec::with_capacity(count as usize);
    for _ in 0..count {
        // Lazy array: a skip position (package version > 61), then a byte array.
        let skip_pos = r.i32()?;
        let len = r.compact_index()?;
        if len < 0 {
            return r.invalid(format!("mip data length {len} is negative"));
        }
        let data_offset = start + r.pos();
        r.bytes(len as usize)?;
        mips.push(Mip {
            width: r.i32()?,
            height: r.i32()?,
            u_bits: r.u8()?,
            v_bits: r.u8()?,
            data_offset,
            data_len: len as usize,
            skip_pos,
        });
    }
    Ok(Texture {
        properties,
        format,
        mips,
        leftover: object.len() - r.pos(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_sizes() {
        assert_eq!(expected_mip_bytes(3, 16, 16), Some(128)); // DXT1: 16 blocks * 8
        assert_eq!(expected_mip_bytes(3, 1, 1), Some(8)); // a DXT block is never smaller than 4x4
        assert_eq!(expected_mip_bytes(8, 256, 256), Some(65536));
        assert_eq!(expected_mip_bytes(0, 32, 32), Some(1024));
        assert_eq!(expected_mip_bytes(5, 32, 32), Some(4096));
        assert_eq!(expected_mip_bytes(6, 32, 32), None);
    }

    fn mip(size: i32) -> Mip {
        Mip {
            width: size,
            height: size,
            u_bits: 0,
            v_bits: 0,
            data_offset: 0,
            data_len: (size * size) as usize,
            skip_pos: 0,
        }
    }

    #[test]
    fn lower_detail_picks_smaller_mipmaps_but_not_tiny_ones() {
        let mips = [mip(256), mip(128), mip(64), mip(32), mip(16)];
        assert_eq!(pick_mip(&mips, 0).unwrap().width, 256);
        assert_eq!(pick_mip(&mips, 1).unwrap().width, 128);
        assert_eq!(pick_mip(&mips, 2).unwrap().width, 64);
        assert_eq!(pick_mip(&mips, 9).unwrap().width, 32);
        // A small texture keeps its only usable picture.
        assert_eq!(pick_mip(&mips[3..], 2).unwrap().width, 32);
        assert!(pick_mip(&[], 1).is_none());
    }
}
