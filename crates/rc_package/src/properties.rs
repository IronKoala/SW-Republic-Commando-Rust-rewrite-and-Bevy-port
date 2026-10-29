//! Reads the "tagged property" list at the start of most objects' data.
//!
//! Each property is a name, a one-byte info field (type, size, array flag),
//! and the value. The list ends with the name "None".
//!
//! Layout learned from UE Viewer (github.com/gildor2/UEViewer, MIT, Konstantin
//! Nosov), `Unreal/UnObject.cpp`, the UE1/UE2 branch of `FPropertyTag`.
//! UE Viewer has no Republic Commando changes here. Written from that
//! description; no code copied.

use crate::{Package, Reader, Result};

/// Property type numbers stored in the low 4 bits of the info byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyType {
    Byte,
    Int,
    Bool,
    Float,
    Object,
    Name,
    String,
    Class,
    Array,
    Struct,
    Vector,
    Rotator,
    Str,
    Map,
    FixedArray,
    Unknown(u8),
}

impl PropertyType {
    fn from_info(t: u8) -> Self {
        use PropertyType::*;
        match t {
            1 => Byte,
            2 => Int,
            3 => Bool,
            4 => Float,
            5 => Object,
            6 => Name,
            7 => String,
            8 => Class,
            9 => Array,
            10 => Struct,
            11 => Vector,
            12 => Rotator,
            13 => Str,
            14 => Map,
            15 => FixedArray,
            other => Unknown(other),
        }
    }
}

/// A decoded value, for the simple types. Everything else keeps its raw bytes.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Byte(u8),
    Int(i32),
    Bool(bool),
    Float(f32),
    /// Object reference (see `ObjectRef`).
    Object(i32),
    /// Name table index.
    Name(i32),
    Raw(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct Property {
    /// Name table index of the property's name.
    pub name: i32,
    pub kind: PropertyType,
    /// Name table index of the struct's type, for struct properties.
    pub struct_name: Option<i32>,
    pub array_index: u32,
    pub size: usize,
    pub value: Value,
}

/// Read the property list from the reader's current position up to and
/// including the closing "None".
pub fn read_properties(pkg: &Package, r: &mut Reader) -> Result<Vec<Property>> {
    let mut out = Vec::new();
    loop {
        let name = r.compact_index()?;
        let name_text = pkg.name(name);
        if name_text.eq_ignore_ascii_case("None") {
            return Ok(out);
        }
        if name_text == "<bad name index>" {
            return r.invalid(format!("property name index {name} is out of range"));
        }
        let info = r.u8()?;
        let is_array = info & 0x80 != 0;
        let kind = PropertyType::from_info(info & 0x0F);
        let struct_name = if kind == PropertyType::Struct {
            Some(r.compact_index()?)
        } else {
            None
        };
        let size = match (info >> 4) & 7 {
            0 => 1,
            1 => 2,
            2 => 4,
            3 => 12,
            4 => 16,
            5 => r.u8()? as usize,
            6 => r.u16()? as usize,
            _ => {
                let s = r.i32()?;
                if s < 0 {
                    return r.invalid(format!("property size {s} is negative"));
                }
                s as usize
            }
        };
        // For bools the "array" bit is the value itself.
        let mut array_index = 0;
        if is_array && kind != PropertyType::Bool {
            let b = r.u8()? as u32;
            array_index = if b < 0x80 {
                b
            } else if b & 0x40 == 0 {
                ((b << 8) | r.u8()? as u32) & 0x3FFF
            } else {
                let rest = r.bytes(3)?;
                ((b << 24) | (rest[0] as u32) << 16 | (rest[1] as u32) << 8 | rest[2] as u32)
                    & 0x3F_FFFF
            };
        }
        let raw = r.bytes(if kind == PropertyType::Bool { 0 } else { size })?;
        let mut v = Reader::new(raw);
        let value = match kind {
            PropertyType::Bool => Value::Bool(is_array),
            PropertyType::Byte if size == 1 => Value::Byte(raw[0]),
            PropertyType::Int if size == 4 => Value::Int(v.i32()?),
            PropertyType::Float if size == 4 => Value::Float(f32::from_bits(v.u32()?)),
            PropertyType::Object | PropertyType::Class => Value::Object(v.compact_index()?),
            PropertyType::Name => Value::Name(v.compact_index()?),
            _ => Value::Raw(raw.to_vec()),
        };
        out.push(Property {
            name,
            kind,
            struct_name,
            array_index,
            size,
            value,
        });
    }
}

/// Find a property by name (first array element).
pub fn find<'a>(pkg: &Package, props: &'a [Property], name: &str) -> Option<&'a Property> {
    props
        .iter()
        .find(|p| p.array_index == 0 && pkg.name(p.name).eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package_with_names(names: &[&str]) -> Package {
        Package {
            summary: crate::Summary {
                tag: crate::PACKAGE_TAG,
                version: 159,
                licensee_version: 1,
                package_flags: 0,
                name_count: names.len() as i32,
                name_offset: 0,
                export_count: 0,
                export_offset: 0,
                import_count: 0,
                import_offset: 0,
                guid: [0; 16],
                generations: vec![],
            },
            names: names
                .iter()
                .map(|n| crate::NameEntry {
                    name: n.to_string(),
                    flags: 0,
                })
                .collect(),
            imports: vec![],
            exports: vec![],
            name_table_end: 0,
            import_table_end: 0,
            export_table_end: 0,
            file_size: 0,
        }
    }

    #[test]
    fn reads_simple_properties() {
        let pkg = package_with_names(&["None", "Format", "USize", "bMasked", "Palette"]);
        let bytes = [
            1, 0x01, 3, // Format: byte, size 1, value 3
            2, 0x22, 0, 1, 0, 0, // USize: int, size 4, value 256
            3, 0x83, // bMasked: bool, true
            4, 0x05, 0x85, // Palette: object, size 1, import -5
            0,    // None
        ];
        let mut r = Reader::new(&bytes);
        let props = read_properties(&pkg, &mut r).unwrap();
        assert_eq!(r.pos(), bytes.len());
        assert_eq!(find(&pkg, &props, "Format").unwrap().value, Value::Byte(3));
        assert_eq!(find(&pkg, &props, "USize").unwrap().value, Value::Int(256));
        assert_eq!(
            find(&pkg, &props, "bMasked").unwrap().value,
            Value::Bool(true)
        );
        assert_eq!(
            find(&pkg, &props, "Palette").unwrap().value,
            Value::Object(-5)
        );
    }
}
