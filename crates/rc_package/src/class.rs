//! Reads the parts of UnrealScript classes that are plain data: a class's
//! default values, the values inside struct properties, and the names of an
//! enum. The original game's menus keep their whole layout in default
//! values, so this is enough to rebuild them (see `docs/ORIGINAL_MENUS.md`).
//!
//! We never read or keep the script itself (bytecode or source text).
//!
//! A class's data is its script structure (compiled code and tables) followed
//! by the default values as an ordinary tagged property list that runs to the
//! very end of the object. Rather than decode the code (whose layout changes
//! between engine versions), we look for the start of a property list that
//! ends exactly at the end of the object. The tagged property layout is the
//! one in `properties.rs`; that classes end with their defaults is described
//! by UE Viewer (github.com/gildor2/UEViewer, MIT, Konstantin Nosov) and the
//! Unreal Wiki ("Legacy:Package File Format"). Written from those
//! descriptions; no code copied.

use crate::properties::{self, Property, PropertyType, Value};
use crate::{Export, ObjectRef, Package, ReadError, Reader, Result};

/// The bytes of an export's data.
pub fn export_data<'a>(data: &'a [u8], export: &Export) -> Result<&'a [u8]> {
    let start = export.serial_offset.max(0) as usize;
    let end = start + export.serial_size.max(0) as usize;
    data.get(start..end).ok_or(ReadError::Invalid {
        offset: start,
        what: "export data is outside the file".into(),
    })
}

/// Find an export by object name and class name (both ignoring case).
pub fn find_export<'a>(pkg: &'a Package, name: &str, class: &str) -> Option<&'a Export> {
    pkg.exports.iter().find(|e| {
        pkg.name(e.object_name).eq_ignore_ascii_case(name)
            && pkg.export_class_name(e).eq_ignore_ascii_case(class)
    })
}

/// The full dotted path of an object, like `GUIContent.Menu.Kamino` for an
/// import, or `XInterfaceCTMenus.CTMenuMain` for an export (the package's own
/// name isn't stored in it, so exports start at their outermost object).
pub fn object_path(pkg: &Package, index: i32) -> String {
    let mut parts = Vec::new();
    let mut i = index;
    // Packages nest only a few levels; the limit guards against loops in a
    // damaged file.
    for _ in 0..16 {
        let outer = match ObjectRef::from_index(i) {
            ObjectRef::None => break,
            ObjectRef::Export(e) => match pkg.exports.get(e) {
                Some(x) => {
                    parts.push(pkg.name(x.object_name).to_string());
                    x.package_index
                }
                None => break,
            },
            ObjectRef::Import(m) => match pkg.imports.get(m) {
                Some(x) => {
                    parts.push(pkg.name(x.object_name).to_string());
                    x.package_index
                }
                None => break,
            },
        };
        i = outer;
    }
    parts.reverse();
    parts.join(".")
}

/// The name of the class an object index refers to, for imports (the class
/// is stored on the import) and exports.
pub fn object_class(pkg: &Package, index: i32) -> String {
    match ObjectRef::from_index(index) {
        ObjectRef::None => String::new(),
        ObjectRef::Export(e) => pkg
            .exports
            .get(e)
            .map(|x| pkg.export_class_name(x).to_string())
            .unwrap_or_default(),
        ObjectRef::Import(m) => pkg
            .imports
            .get(m)
            .map(|x| pkg.name(x.class_name).to_string())
            .unwrap_or_default(),
    }
}

/// A class's default values and where in the object they started.
#[derive(Debug, Clone)]
pub struct ClassDefaults {
    pub properties: Vec<Property>,
    /// Offset inside the object where the list starts.
    pub start: usize,
    /// The parent class (an object index), from the class's `SuperField`.
    /// Zero when not found.
    pub super_class: i32,
    /// True when the bytes before the list matched the expected class
    /// layout. False means we fell back to the earliest list that reads,
    /// which may begin with a few bytes of junk.
    pub layout_checked: bool,
}

/// Read a class export's default values.
pub fn read_class_defaults(pkg: &Package, data: &[u8], export: &Export) -> Result<ClassDefaults> {
    let object = export_data(data, export)?;
    let (start, layout_checked) = find_trailing_list(pkg, object).ok_or(ReadError::Invalid {
        offset: export.serial_offset.max(0) as usize,
        what: format!(
            "no default value list ending at the end of class {}",
            pkg.name(export.object_name)
        ),
    })?;
    let mut r = Reader::new(object);
    r.seek(start)?;
    let properties = properties::read_properties(pkg, &mut r)?;
    Ok(ClassDefaults {
        properties,
        start,
        super_class: export.super_index,
        layout_checked,
    })
}

/// Where the default values start: the earliest offset from which a tagged
/// property list reads cleanly and ends exactly at the end of `object`, and
/// that comes straight after the class's list of package imports.
///
/// Any later property boundary in the same list also ends at the end, so
/// the earliest is the real start, unless bytes just before it happen to
/// read as more properties (they do: the package import names can read as a
/// name property). So we also demand what Unreal stores just before the
/// defaults: the number of packages the class uses, their names, the class
/// it must live inside, its config file name (UE2 `UClass` layout, Unreal
/// Wiki and UE Viewer), and one more list of names that we found by
/// measuring (always empty here). Checked on all 601 classes in the game's script
/// packages.
fn find_trailing_list(pkg: &Package, object: &[u8]) -> Option<(usize, bool)> {
    let package_names: Vec<i32> = pkg
        .imports
        .iter()
        .filter(|m| pkg.name(m.class_name).eq_ignore_ascii_case("Package"))
        .map(|m| m.object_name)
        .collect();
    let mut fallback = None;
    for start in 0..object.len() {
        let mut r = Reader::new(object);
        if r.seek(start).is_err() {
            break;
        }
        let Ok(props) = properties::read_properties(pkg, &mut r) else {
            continue;
        };
        if r.pos() != object.len() || !props.iter().all(|p| plausible(pkg, p)) {
            continue;
        }
        if follows_package_imports(pkg, object, start, &package_names) {
            return Some((start, true));
        }
        fallback.get_or_insert((start, false));
    }
    fallback
}

/// True when some position before `start` reads as: a count, that many
/// names of packages, then two more compact indexes, ending at `start`.
fn follows_package_imports(pkg: &Package, object: &[u8], start: usize, packages: &[i32]) -> bool {
    // Each entry is 1 to 5 bytes; 32 packages is far more than any class uses.
    for back in 3..=start.min(32 * 5 + 11) {
        let mut r = Reader::new(object);
        if r.seek(start - back).is_err() {
            continue;
        }
        let Ok(count) = r.compact_index() else {
            continue;
        };
        if !(1..=32).contains(&count) {
            continue;
        }
        let mut ok = true;
        for _ in 0..count {
            match r.compact_index() {
                // The class's own package isn't an import, so allow names
                // that aren't import packages only if they are valid names.
                Ok(n) if packages.contains(&n) => {}
                Ok(n) if n == 0 || pkg.name(n) == "<bad name index>" => ok = false,
                Ok(_) => {}
                Err(_) => ok = false,
            }
            if !ok {
                break;
            }
        }
        if !ok {
            continue;
        }
        let within = r.compact_index();
        let config = r.compact_index();
        if within.is_err() || !config.is_ok_and(|c| pkg.name(c) != "<bad name index>") {
            continue;
        }
        // Then a list of names (in the game's classes always empty; probably
        // the editor's hidden categories).
        let Ok(hidden) = r.compact_index() else {
            continue;
        };
        if !(0..=16).contains(&hidden) {
            continue;
        }
        if (0..hidden).all(|_| r.compact_index().is_ok()) && r.pos() == start {
            return true;
        }
    }
    false
}

fn plausible(pkg: &Package, p: &Property) -> bool {
    let name = pkg.name(p.name);
    let starts_well = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    let fixed_size = match p.kind {
        PropertyType::Int | PropertyType::Float => p.size == 4,
        PropertyType::Byte => p.size == 1,
        PropertyType::Vector => p.size == 12,
        PropertyType::Rotator => p.size == 12,
        PropertyType::Unknown(_) => false,
        _ => true,
    };
    starts_well && fixed_size && p.array_index < 4096
}

/// The values inside a struct property, when the struct is stored as its own
/// tagged list (most structs are; a few small ones such as `Color` are
/// stored as plain bytes and return `None`).
pub fn struct_properties(pkg: &Package, p: &Property) -> Option<Vec<Property>> {
    let Value::Raw(raw) = &p.value else {
        return None;
    };
    if p.kind != PropertyType::Struct {
        return None;
    }
    let mut r = Reader::new(raw);
    let props = properties::read_properties(pkg, &mut r).ok()?;
    (r.pos() == raw.len()).then_some(props)
}

/// The names of an `Enum` export, in order. Value 0 is the first name.
pub fn read_enum(pkg: &Package, data: &[u8], export: &Export) -> Result<Vec<String>> {
    let object = export_data(data, export)?;
    let mut r = Reader::new(object);
    properties::read_properties(pkg, &mut r)?;
    // Generic UE2 stores the field chain (super field, next field) here;
    // Republic Commando's enums go straight to the name count (checked on
    // all 133 enums in the game's script packages).
    let count = r.compact_index()?;
    if !(0..=256).contains(&count) {
        return r.invalid(format!("enum name count {count} is implausible"));
    }
    let mut names = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let n = r.compact_index()?;
        names.push(pkg.name(n).to_string());
    }
    if r.pos() != object.len() {
        return r.invalid(format!(
            "enum has {} bytes left after its names",
            object.len() - r.pos()
        ));
    }
    Ok(names)
}

/// The editor category of a property export (`IntProperty`,
/// `StructProperty`, ...), such as "MenuDefault". A class's properties are
/// the exports whose outer object is the class.
///
/// Republic Commando stores, after an empty tagged list: the array size
/// (2 bytes), the property flags (4 bytes) and the category name. Worked
/// out from the bytes of the game's `XInterface.u` (generic UE2 also stores
/// the field chain and a 4-byte array size).
pub fn read_property_category(pkg: &Package, data: &[u8], export: &Export) -> Result<String> {
    read_property_info(pkg, data, export).map(|(_, c)| c)
}

/// A property export's fixed array size (1 for a plain value) and its
/// editor category; see [`read_property_category`].
pub fn read_property_info(pkg: &Package, data: &[u8], export: &Export) -> Result<(u16, String)> {
    let object = export_data(data, export)?;
    let mut r = Reader::new(object);
    properties::read_properties(pkg, &mut r)?;
    let dim = r.u16()?;
    r.u32()?;
    let category = r.compact_index()?;
    Ok((dim, pkg.name(category).to_string()))
}

/// A colour stored as plain bytes (`Color` struct): blue, green, red, alpha
/// on disk. Returned as red, green, blue, alpha.
pub fn color_value(p: &Property) -> Option<[u8; 4]> {
    match &p.value {
        Value::Raw(b) if b.len() == 4 => Some([b[2], b[1], b[0], b[3]]),
        _ => None,
    }
}

/// A string property's text. Unreal stores it as a length-prefixed string.
pub fn string_value(p: &Property) -> Option<String> {
    match (&p.value, p.kind) {
        (Value::Raw(b), PropertyType::Str | PropertyType::String) => {
            let mut r = Reader::new(b);
            r.fstring().ok()
        }
        _ => None,
    }
}

/// A dynamic array of strings (`array<string>`, such as a menu option's
/// choices): a count, then each string. `None` when the bytes aren't
/// exactly that, so other arrays aren't misread.
pub fn string_list_value(p: &Property) -> Option<Vec<String>> {
    let (Value::Raw(b), PropertyType::Array) = (&p.value, p.kind) else {
        return None;
    };
    let mut r = Reader::new(b);
    let n = r.compact_index().ok()?;
    if !(0..=1024).contains(&n) {
        return None;
    }
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        out.push(r.fstring().ok()?);
    }
    (r.pos() == r.len()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::properties::tests::package_with_names;

    #[test]
    fn reads_a_string_list() {
        let p = Property {
            name: 0,
            kind: PropertyType::Array,
            struct_name: None,
            array_index: 0,
            size: 10,
            value: Value::Raw(vec![2, 4, b'Y', b'E', b'S', 0, 3, b'N', b'O', 0]),
        };
        assert_eq!(string_list_value(&p), Some(vec!["YES".into(), "NO".into()]));
        let bad = Property {
            value: Value::Raw(vec![2, 4, b'Y', b'E', b'S', 0]),
            ..p
        };
        assert_eq!(string_list_value(&bad), None);
    }

    #[test]
    fn finds_list_at_end_after_junk() {
        let pkg = package_with_names(&["None", "PosX", "Label"]);
        // Some made-up code bytes, then PosX = 0.5f, then None.
        let mut object = vec![0xFFu8, 0x07, 0x99, 0x42];
        let list = [1u8, 0x24, 0, 0, 0, 0x3F, 0];
        object.extend_from_slice(&list);
        let (start, _) = find_trailing_list(&pkg, &object).unwrap();
        assert_eq!(start, 4);
    }

    #[test]
    fn reads_nested_struct() {
        let pkg = package_with_names(&["None", "PosX", "Sprite", "MenuSprite"]);
        // Sprite: struct MenuSprite, size 7, holding PosX = 0.5f.
        let bytes = [2u8, 0x5A, 3, 7, 1, 0x24, 0, 0, 0, 0x3F, 0, 0];
        let mut r = Reader::new(&bytes);
        let props = properties::read_properties(&pkg, &mut r).unwrap();
        assert_eq!(props.len(), 1);
        let inner = struct_properties(&pkg, &props[0]).unwrap();
        assert_eq!(inner[0].value, Value::Float(0.5));
    }
}
