//! `rc_probe --class NAME`: log a script class's default values, and those
//! of each parent class, from the game's script packages (`System/*.u`).
//!
//! This is how the weapon numbers in `data/weapons/` were found. A value
//! set by a class wins over the same value in its parents.

use crate::Log;
use rc_package::class_defaults::{class_defaults, find_class, parent_name, property_names};
use rc_package::properties::{Property, PropertyType, Value};
use rc_package::{Package, Reader};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

struct Loaded {
    file: String,
    data: Vec<u8>,
    pkg: Package,
}

/// Log the defaults of `class` and its parents, looking in the `.u`
/// packages of each folder in `folders` (the game's `GameData/System` and
/// `GameData/Properties`, which holds many per-level variants).
pub fn dump(log: &mut Log, folders: &[PathBuf], class: &str) -> bool {
    let mut packages = Vec::new();
    let mut paths = Vec::new();
    for folder in folders {
        let Ok(entries) = fs::read_dir(folder) else {
            log.line(format!("can't read {}, skipped", folder.display()));
            continue;
        };
        let mut found: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("u")))
            .collect();
        found.sort();
        paths.extend(found);
    }
    for path in paths {
        let Ok(data) = fs::read(&path) else { continue };
        let Ok(pkg) = Package::parse(&data) else {
            log.line(format!("skipped {} (didn't read)", path.display()));
            continue;
        };
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        packages.push(Loaded { file, data, pkg });
    }
    log.line(format!("script packages read: {}", packages.len()));
    let names: HashSet<String> = packages
        .iter()
        .flat_map(|l| property_names(&l.pkg))
        .collect();
    log.line(format!("property names known: {}", names.len()));
    let is_property = |n: &str| names.contains(&n.to_ascii_lowercase());

    let mut next = Some(class.to_string());
    let mut found_any = false;
    while let Some(name) = next.take() {
        let Some((l, export)) = packages
            .iter()
            .find_map(|l| find_class(&l.pkg, &name).map(|e| (l, e)))
        else {
            log.line(format!("class {name}: not in these packages (stops here)"));
            break;
        };
        found_any = true;
        let parent = parent_name(&l.pkg, export).map(str::to_string);
        log.line(format!(
            "=== class {} in {} (parent: {}) ===",
            name,
            l.file,
            parent.as_deref().unwrap_or("none")
        ));
        match class_defaults(&l.pkg, &l.data, export, &is_property) {
            Some(d) => {
                for p in &d.properties {
                    log.line(format!("  {}", describe(&l.pkg, p)));
                }
                log.line(format!(
                    "  ({} values, list starts at byte {} of {})",
                    d.properties.len(),
                    d.start,
                    d.data_len
                ));
                let off = export.serial_offset as usize;
                let before = &l.data[off + d.start.saturating_sub(24)..off + d.start];
                log.file_only(format!("  24 bytes before it: {}", hex(before)));
            }
            None => log.line("  no clean defaults list found"),
        }
        next = parent;
    }
    found_any
}

/// One property as `Name[index] = value`.
fn describe(pkg: &Package, p: &Property) -> String {
    let name = pkg.name(p.name);
    let index = if p.array_index > 0 {
        format!("[{}]", p.array_index)
    } else {
        String::new()
    };
    let value = match &p.value {
        Value::Byte(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Object(o) => pkg.object_name(*o).to_string(),
        Value::Name(n) => format!("'{}'", pkg.name(*n)),
        Value::Raw(raw) => raw_value(pkg, p, raw),
    };
    format!("{name}{index} = {value}")
}

fn raw_value(pkg: &Package, p: &Property, raw: &[u8]) -> String {
    let words = |raw: &[u8], float: bool| {
        raw.chunks_exact(4)
            .map(|c| {
                let b = [c[0], c[1], c[2], c[3]];
                if float {
                    f32::from_le_bytes(b).to_string()
                } else {
                    i32::from_le_bytes(b).to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let struct_name = p.struct_name.map_or("?", |n| pkg.name(n));
    match p.kind {
        PropertyType::Str | PropertyType::String => {
            let mut r = Reader::new(raw);
            r.fstring()
                .map(|s| format!("\"{s}\""))
                .unwrap_or_else(|_| hex(raw))
        }
        PropertyType::Vector => format!("({})", words(raw, true)),
        // Rotators are whole numbers: 65536 is one full turn.
        PropertyType::Rotator => format!("rotator({})", words(raw, false)),
        PropertyType::Struct if struct_name == "Rotator" => {
            format!("rotator({})", words(raw, false))
        }
        PropertyType::Struct if raw.len() % 4 == 0 && raw.len() <= 64 => {
            format!("{struct_name}({}) raw {}", words(raw, true), hex(raw))
        }
        PropertyType::Struct => format!("{struct_name} raw {}", hex(raw)),
        _ => format!("{:?} raw {}", p.kind, hex(raw)),
    }
}

fn hex(raw: &[u8]) -> String {
    raw.iter().map(|b| format!("{b:02x}")).collect()
}
