//! Finds a script class's default values ("defaultproperties").
//!
//! A class's data in a script package (`.u`) ends with its default values,
//! stored as the same tagged property list textures use (see
//! `properties.rs`). What comes before them (bytecode, flags, dependency
//! lists) has Republic Commando changes we don't read. So we find the start
//! of the list the other way round: try every position near the end of the
//! class's data and keep the one whose list reads cleanly to the exact end,
//! using only names of real properties. The earliest such start gives the
//! whole list.
//!
//! The method was first used by hand for the movement numbers (see
//! docs/RESEARCH.md, "Player movement"); this is the same idea as code.

use crate::properties::{read_properties, Property, PropertyType};
use crate::{Export, Package, Reader};

/// How far back from the end of a class's data we look for the start of
/// its defaults, in bytes. The biggest weapon defaults seen are under 2 KB.
const SEARCH_BYTES: usize = 16 * 1024;

/// Largest array position accepted in a defaults list. The game's fixed
/// arrays are far smaller (the biggest seen is 16); a false start reads
/// junk like `Tag[14081]` in CTBot's data, and this rejects it.
const MAX_ARRAY_INDEX: u32 = 256;

/// A class's own default values and where they were found.
#[derive(Debug, Clone)]
pub struct Defaults {
    pub properties: Vec<Property>,
    /// Where the list starts, in bytes from the start of the class's data.
    pub start: usize,
    /// Length of the class's data in bytes.
    pub data_len: usize,
}

/// The default values a class sets itself (not the ones it inherits), or
/// `None` if no clean list was found.
///
/// `is_property` answers "is this the name of a real script property?".
/// Build it from the property exports of every package the class can see
/// (see `property_names`).
pub fn class_defaults(
    pkg: &Package,
    data: &[u8],
    export: &Export,
    is_property: &dyn Fn(&str) -> bool,
) -> Option<Defaults> {
    let start = usize::try_from(export.serial_offset).ok()?;
    let len = usize::try_from(export.serial_size).ok()?;
    let body = data.get(start..start.checked_add(len)?)?;
    let first = body.len().saturating_sub(SEARCH_BYTES);
    // An empty list is just "None" at the very end; any longer clean list
    // found earlier wins.
    for pos in first..body.len() {
        let mut r = Reader::new(&body[pos..]);
        let Ok(props) = read_properties(pkg, &mut r) else {
            continue;
        };
        if r.pos() != r.len() {
            continue;
        }
        let clean = props.iter().all(|p| {
            !matches!(p.kind, PropertyType::Unknown(_) | PropertyType::Map)
                && p.array_index < MAX_ARRAY_INDEX
                && is_property(pkg.name(p.name))
        });
        if clean {
            return Some(Defaults {
                properties: props,
                start: pos,
                data_len: body.len(),
            });
        }
    }
    None
}

/// Names of every property declared in a package (exports whose class ends
/// in "Property", like `FloatProperty` or `ObjectProperty`), lower-cased.
pub fn property_names(pkg: &Package) -> Vec<String> {
    pkg.exports
        .iter()
        .filter(|e| pkg.export_class_name(e).ends_with("Property"))
        .map(|e| pkg.name(e.object_name).to_ascii_lowercase())
        .collect()
}

/// The class export with this name, if the package has one.
pub fn find_class<'a>(pkg: &'a Package, name: &str) -> Option<&'a Export> {
    pkg.exports
        .iter()
        .find(|e| e.class_index == 0 && pkg.name(e.object_name).eq_ignore_ascii_case(name))
}

/// The name of a class's parent class, if it has one.
pub fn parent_name<'a>(pkg: &'a Package, export: &Export) -> Option<&'a str> {
    (export.super_index != 0).then(|| pkg.object_name(export.super_index))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::properties::{find, tests::package_with_names, Value};

    #[test]
    fn finds_the_list_after_unread_bytes() {
        let pkg = package_with_names(&["None", "FireRate", "AmmoAmount", "Junk"]);
        // 7 bytes we can't read, then FireRate = 0.25 (float), AmmoAmount =
        // 60 (int), None.
        let mut data = vec![9, 3, 7, 1, 0x02, 0xAA, 0x01];
        data.extend([1, 0x24]);
        data.extend(0.25f32.to_le_bytes());
        data.extend([2, 0x22]);
        data.extend(60i32.to_le_bytes());
        data.push(0);
        let export = Export {
            class_index: 0,
            super_index: 0,
            package_index: 0,
            rc_unknown: None,
            object_name: 1,
            object_flags: 0,
            serial_size: data.len() as i32,
            serial_offset: 0,
        };
        let is_prop = |n: &str| n == "FireRate" || n == "AmmoAmount";
        let props = class_defaults(&pkg, &data, &export, &is_prop)
            .unwrap()
            .properties;
        assert_eq!(props.len(), 2);
        assert_eq!(
            find(&pkg, &props, "FireRate").unwrap().value,
            Value::Float(0.25)
        );
        assert_eq!(
            find(&pkg, &props, "AmmoAmount").unwrap().value,
            Value::Int(60)
        );
    }
}
