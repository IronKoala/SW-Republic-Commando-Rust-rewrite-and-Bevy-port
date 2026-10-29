//! Reads the header and the three tables of an Unreal Engine 2 package, the
//! container format behind every Republic Commando data file (`.u`, `.ctm`,
//! `.utx`, `.uax`, `.usx`, `.ukx`).
//!
//! Layout learned from:
//! - Unreal Wiki, "Legacy:Package File Format" (generic UE1/UE2 layout).
//! - UE Viewer (github.com/gildor2/UEViewer, MIT, Konstantin Nosov):
//!   `Unreal/UnrealPackage/UnPackage2.cpp` and `Unreal/GameDatabase.cpp` for
//!   the Republic Commando differences.
//!
//! Written from those descriptions; no code copied.
//!
//! This crate only reads bytes it is given. It never opens or writes files.

pub mod properties;
pub mod reader;
pub mod sound;
pub mod texture;

pub use reader::{ReadError, Reader, Result};

/// Every Unreal package starts with these four bytes (C1 83 2A 9E on disk).
pub const PACKAGE_TAG: u32 = 0x9E2A_83C1;

#[derive(Debug, Clone)]
pub struct Generation {
    pub export_count: i32,
    pub name_count: i32,
}

/// The fixed header at the start of the file.
#[derive(Debug, Clone)]
pub struct Summary {
    pub tag: u32,
    /// Low 16 bits of the stored version: the engine's package version.
    pub version: u16,
    /// High 16 bits: the "licensee" version, set by the game studio.
    pub licensee_version: u16,
    pub package_flags: u32,
    pub name_count: i32,
    pub name_offset: i32,
    pub export_count: i32,
    pub export_offset: i32,
    pub import_count: i32,
    pub import_offset: i32,
    pub guid: [u8; 16],
    pub generations: Vec<Generation>,
}

impl Summary {
    /// UE Viewer treats licensee version 1 with package version 133-148 or
    /// 154-159 as Republic Commando (`GameDatabase.cpp`).
    pub fn looks_like_republic_commando(&self) -> bool {
        self.licensee_version == 1
            && ((133..=148).contains(&self.version) || (154..=159).contains(&self.version))
    }

    /// Republic Commando changed the export table layout from version 151
    /// (UE Viewer, `FObjectExport::Serialize2`).
    fn uses_rc_export_layout(&self) -> bool {
        self.licensee_version == 1 && self.version >= 151
    }
}

#[derive(Debug, Clone)]
pub struct NameEntry {
    pub name: String,
    pub flags: u32,
}

/// A reference to an object in another package.
#[derive(Debug, Clone)]
pub struct Import {
    pub class_package: i32,
    pub class_name: i32,
    /// Object reference to the package that holds it (see `ObjectRef`).
    pub package_index: i32,
    pub object_name: i32,
}

/// An object stored in this package.
#[derive(Debug, Clone)]
pub struct Export {
    pub class_index: i32,
    pub super_index: i32,
    pub package_index: i32,
    /// Extra field Republic Commando adds from package version 159. Meaning
    /// unknown; UE Viewer reads and ignores it.
    pub rc_unknown: Option<i32>,
    pub object_name: i32,
    pub object_flags: u32,
    pub serial_size: i32,
    pub serial_offset: i32,
}

/// What an object index (`class_index`, `package_index`, ...) points at.
/// Positive numbers are exports (1-based), negative numbers are imports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectRef {
    None,
    Export(usize),
    Import(usize),
}

impl ObjectRef {
    pub fn from_index(i: i32) -> Self {
        match i {
            0 => ObjectRef::None,
            i if i > 0 => ObjectRef::Export((i - 1) as usize),
            i => ObjectRef::Import((-i - 1) as usize),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Package {
    pub summary: Summary,
    pub names: Vec<NameEntry>,
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
    /// Byte position where each table ended, for checking our reading.
    pub name_table_end: usize,
    pub import_table_end: usize,
    pub export_table_end: usize,
    pub file_size: usize,
}

fn read_summary(r: &mut Reader) -> Result<Summary> {
    let tag = r.u32()?;
    if tag != PACKAGE_TAG {
        r.seek(0)?;
        return r.invalid(format!(
            "not an Unreal package: tag {tag:#010X}, expected {PACKAGE_TAG:#010X}"
        ));
    }
    let file_version = r.u32()?;
    let version = (file_version & 0xFFFF) as u16;
    let licensee_version = (file_version >> 16) as u16;
    if version < 68 {
        // Versions before 68 use a different header ("heritage" table).
        // Republic Commando is far newer, so we don't support them.
        return r.invalid(format!(
            "package version {version} is older than 68, not supported"
        ));
    }
    let package_flags = r.u32()?;
    let name_count = r.i32()?;
    let name_offset = r.i32()?;
    let export_count = r.i32()?;
    let export_offset = r.i32()?;
    let import_count = r.i32()?;
    let import_offset = r.i32()?;
    let guid: [u8; 16] = r.bytes(16)?.try_into().unwrap();
    let generation_count = r.i32()?;
    if !(0..=10_000).contains(&generation_count) {
        return r.invalid(format!(
            "generation count {generation_count} is implausible"
        ));
    }
    let mut generations = Vec::with_capacity(generation_count as usize);
    for _ in 0..generation_count {
        generations.push(Generation {
            export_count: r.i32()?,
            name_count: r.i32()?,
        });
    }
    Ok(Summary {
        tag,
        version,
        licensee_version,
        package_flags,
        name_count,
        name_offset,
        export_count,
        export_offset,
        import_count,
        import_offset,
        guid,
        generations,
    })
}

fn table_start(r: &mut Reader, what: &str, count: i32, offset: i32) -> Result<usize> {
    if count < 0 || offset < 0 || offset as usize > r.len() {
        return r.invalid(format!(
            "{what} table has count {count} at offset {offset}, file is {} bytes",
            r.len()
        ));
    }
    // Every entry is at least one byte, so the count can't exceed the bytes left.
    if count as usize > r.len() - offset as usize {
        return r.invalid(format!("{what} table count {count} can't fit in the file"));
    }
    r.seek(offset as usize)?;
    Ok(count as usize)
}

impl Package {
    /// Parse the header and the name, import and export tables.
    /// Object data itself (the bytes each export points to) is not read yet.
    pub fn parse(data: &[u8]) -> Result<Package> {
        let mut r = Reader::new(data);
        let summary = read_summary(&mut r)?;

        let n = table_start(&mut r, "name", summary.name_count, summary.name_offset)?;
        let mut names = Vec::with_capacity(n);
        for _ in 0..n {
            names.push(NameEntry {
                name: r.fstring()?,
                flags: r.u32()?,
            });
        }
        let name_table_end = r.pos();

        let n = table_start(
            &mut r,
            "import",
            summary.import_count,
            summary.import_offset,
        )?;
        let mut imports = Vec::with_capacity(n);
        for _ in 0..n {
            imports.push(Import {
                class_package: r.compact_index()?,
                class_name: r.compact_index()?,
                package_index: r.i32()?,
                object_name: r.compact_index()?,
            });
        }
        let import_table_end = r.pos();

        let n = table_start(
            &mut r,
            "export",
            summary.export_count,
            summary.export_offset,
        )?;
        let rc_layout = summary.uses_rc_export_layout();
        let mut exports = Vec::with_capacity(n);
        for _ in 0..n {
            let class_index = r.compact_index()?;
            let super_index = r.compact_index()?;
            let package_index = r.i32()?;
            let export = if rc_layout {
                let rc_unknown = if summary.version >= 159 {
                    Some(r.compact_index()?)
                } else {
                    None
                };
                Export {
                    class_index,
                    super_index,
                    package_index,
                    rc_unknown,
                    object_name: r.compact_index()?,
                    object_flags: r.u32()?,
                    serial_size: r.i32()?,
                    serial_offset: r.i32()?,
                }
            } else {
                let object_name = r.compact_index()?;
                let object_flags = r.u32()?;
                let serial_size = r.compact_index()?;
                let serial_offset = if serial_size != 0 {
                    r.compact_index()?
                } else {
                    0
                };
                Export {
                    class_index,
                    super_index,
                    package_index,
                    rc_unknown: None,
                    object_name,
                    object_flags,
                    serial_size,
                    serial_offset,
                }
            };
            exports.push(export);
        }
        let export_table_end = r.pos();

        Ok(Package {
            summary,
            names,
            imports,
            exports,
            name_table_end,
            import_table_end,
            export_table_end,
            file_size: data.len(),
        })
    }

    /// The text of a name index, or a marker if it is out of range.
    pub fn name(&self, index: i32) -> &str {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.names.get(i))
            .map(|n| n.name.as_str())
            .unwrap_or("<bad name index>")
    }

    /// The object name an object index points at.
    pub fn object_name(&self, index: i32) -> &str {
        match ObjectRef::from_index(index) {
            ObjectRef::None => "None",
            ObjectRef::Export(i) => self
                .exports
                .get(i)
                .map_or("<bad export>", |e| self.name(e.object_name)),
            ObjectRef::Import(i) => self
                .imports
                .get(i)
                .map_or("<bad import>", |m| self.name(m.object_name)),
        }
    }

    /// The class name of an export. Class index 0 means the export is itself
    /// a class ("Class").
    pub fn export_class_name(&self, export: &Export) -> &str {
        if export.class_index == 0 {
            "Class"
        } else {
            self.object_name(export.class_index)
        }
    }

    /// Consistency checks that would fail if we read the format wrongly.
    /// Returns one line per problem; an empty list means every check passed.
    pub fn check(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let names = self.names.len() as i32;
        let in_names = |i: i32| (0..names).contains(&i);
        let in_objects = |i: i32| match ObjectRef::from_index(i) {
            ObjectRef::None => true,
            ObjectRef::Export(e) => e < self.exports.len(),
            ObjectRef::Import(m) => m < self.imports.len(),
        };

        for (i, n) in self.names.iter().enumerate() {
            if n.name.is_empty() || n.name.chars().any(|c| c.is_control()) {
                problems.push(format!("name {i} looks wrong: {:?}", n.name));
            }
        }
        for (i, m) in self.imports.iter().enumerate() {
            if !in_names(m.class_package) || !in_names(m.class_name) || !in_names(m.object_name) {
                problems.push(format!("import {i} has a name index out of range: {m:?}"));
            }
            if !in_objects(m.package_index) {
                problems.push(format!(
                    "import {i} has a package index out of range: {}",
                    m.package_index
                ));
            }
        }
        for (i, e) in self.exports.iter().enumerate() {
            if !in_names(e.object_name) {
                problems.push(format!(
                    "export {i} has a name index out of range: {}",
                    e.object_name
                ));
            }
            if !in_objects(e.class_index)
                || !in_objects(e.super_index)
                || !in_objects(e.package_index)
            {
                problems.push(format!(
                    "export {i} has an object index out of range: {e:?}"
                ));
            }
            let end = e.serial_offset as i64 + e.serial_size as i64;
            if e.serial_size < 0 || e.serial_offset < 0 || end > self.file_size as i64 {
                problems.push(format!(
                    "export {i} data range {}..{end} is outside the file ({} bytes)",
                    e.serial_offset, self.file_size
                ));
            }
        }
        if let Some(last) = self.summary.generations.last() {
            if last.name_count != self.summary.name_count
                || last.export_count != self.summary.export_count
            {
                problems.push(format!(
                    "last generation says {} names / {} exports, header says {} / {}",
                    last.name_count,
                    last.export_count,
                    self.summary.name_count,
                    self.summary.export_count
                ));
            }
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a compact index, the inverse of `Reader::compact_index`.
    fn ci(out: &mut Vec<u8>, v: i32) {
        let mut a = v.unsigned_abs();
        let mut b0 = (a & 0x3F) as u8;
        if v < 0 {
            b0 |= 0x80;
        }
        a >>= 6;
        if a > 0 {
            b0 |= 0x40;
        }
        out.push(b0);
        while a > 0 {
            let mut b = (a & 0x7F) as u8;
            a >>= 7;
            if a > 0 {
                b |= 0x80;
            }
            out.push(b);
        }
    }

    fn fstr(out: &mut Vec<u8>, s: &str) {
        ci(out, s.len() as i32 + 1);
        out.extend_from_slice(s.as_bytes());
        out.push(0);
    }

    /// Builds a tiny made-up package in memory. No game data involved.
    fn synthetic(version: u16, licensee: u16) -> Vec<u8> {
        let names = ["Core", "Class", "Object", "MyThing"];
        let mut body = Vec::new();
        // Header is 4*9 + 16 + 4 + 8 = 64 bytes with one generation.
        let header_len = 64usize;
        let name_offset = header_len;
        for n in names {
            fstr(&mut body, n);
            body.extend_from_slice(&0x0007_0010u32.to_le_bytes());
        }
        let import_offset = header_len + body.len();
        // Import: Core.Class "Object" inside package Core... keep it simple:
        // import 0 = package "Core", import 1 = class "Object" in Core.
        ci(&mut body, 0);
        ci(&mut body, 1);
        body.extend_from_slice(&0i32.to_le_bytes());
        ci(&mut body, 0);
        ci(&mut body, 0);
        ci(&mut body, 1);
        body.extend_from_slice(&(-1i32).to_le_bytes());
        ci(&mut body, 2);
        let export_offset = header_len + body.len();
        let data_offset = 0i32; // points at the header, fine for a range check
        ci(&mut body, -2); // class = import 1 ("Object")
        ci(&mut body, 0);
        body.extend_from_slice(&0i32.to_le_bytes());
        if licensee == 1 && version >= 151 {
            if version >= 159 {
                ci(&mut body, 0);
            }
            ci(&mut body, 3);
            body.extend_from_slice(&0u32.to_le_bytes());
            body.extend_from_slice(&8i32.to_le_bytes());
            body.extend_from_slice(&data_offset.to_le_bytes());
        } else {
            ci(&mut body, 3);
            body.extend_from_slice(&0u32.to_le_bytes());
            ci(&mut body, 8);
            ci(&mut body, data_offset);
        }

        let mut out = Vec::new();
        out.extend_from_slice(&PACKAGE_TAG.to_le_bytes());
        out.extend_from_slice(&(version as u32 | (licensee as u32) << 16).to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        for v in [names.len(), name_offset, 1, export_offset, 2, import_offset] {
            out.extend_from_slice(&(v as i32).to_le_bytes());
        }
        out.extend_from_slice(&[0xAB; 16]);
        out.extend_from_slice(&1i32.to_le_bytes());
        out.extend_from_slice(&1i32.to_le_bytes());
        out.extend_from_slice(&(names.len() as i32).to_le_bytes());
        assert_eq!(out.len(), header_len);
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn parses_generic_ue2_layout() {
        let pkg = Package::parse(&synthetic(128, 0)).unwrap();
        assert!(!pkg.summary.looks_like_republic_commando());
        assert_eq!(pkg.names.len(), 4);
        assert_eq!(pkg.export_class_name(&pkg.exports[0]), "Object");
        assert_eq!(pkg.name(pkg.exports[0].object_name), "MyThing");
        assert_eq!(pkg.check(), Vec::<String>::new());
    }

    #[test]
    fn parses_republic_commando_layouts() {
        for version in [148, 151, 159] {
            let pkg = Package::parse(&synthetic(version, 1)).unwrap();
            assert_eq!(pkg.summary.version, version);
            assert!(pkg.summary.looks_like_republic_commando() || version == 151);
            assert_eq!(pkg.name(pkg.exports[0].object_name), "MyThing");
            assert_eq!(pkg.exports[0].serial_size, 8);
            assert_eq!(pkg.exports[0].rc_unknown.is_some(), version >= 159);
            assert_eq!(pkg.check(), Vec::<String>::new(), "version {version}");
        }
    }

    #[test]
    fn rejects_non_package() {
        let err = Package::parse(b"this is not a package at all, just text").unwrap_err();
        assert!(err.to_string().contains("not an Unreal package"));
    }

    #[test]
    fn truncated_file_is_an_error() {
        let data = synthetic(159, 1);
        assert!(Package::parse(&data[..data.len() - 3]).is_err());
    }
}
