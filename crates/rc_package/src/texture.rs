//! Reads `Texture` objects: their properties and the list of mipmaps
//! (the picture at full size, half size, quarter size, ...).
//!
//! Layout learned from UE Viewer (github.com/gildor2/UEViewer, MIT, Konstantin
//! Nosov): `Unreal/UnrealMaterial/UnTexture2.cpp`, `UnMaterial2.h` (FMipmap,
//! ETextureFormat) and `UnCoreSerialize.cpp` (lazy arrays). Republic Commando
//! has no layout change there; UE Viewer guesses its format 14 is CxV8U8.
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
        14 => "CxV8U8?",
        _ => "unknown",
    }
}

/// How many bytes a mip of this size should take, if we know the format.
pub fn expected_mip_bytes(format: u8, width: i32, height: i32) -> Option<usize> {
    let (w, h) = (width.max(1) as usize, height.max(1) as usize);
    let blocks = w.div_ceil(4) * h.div_ceil(4);
    match format {
        0 | 9 => Some(w * h),
        2 | 10 | 14 => Some(w * h * 2),
        4 => Some(w * h * 3),
        5 => Some(w * h * 4),
        3 => Some(blocks * 8),
        7 | 8 => Some(blocks * 16),
        _ => None,
    }
}

impl Texture {
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
}
