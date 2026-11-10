//! Republic Commando stores structs (and lists of structs) inside objects
//! as named values, like the object's own list (see
//! `rc_package::properties`). This turns such a list into a tree of plain
//! values, with object references kept as their full path.

use rc_package::properties::{read_properties, Property, PropertyType, Value};
use rc_package::{ObjectRef, Package, Reader};
use std::collections::BTreeMap;

/// One value.
#[derive(Debug, Clone, PartialEq)]
pub enum Tagged {
    Bool(bool),
    Byte(u8),
    Int(i32),
    Float(f32),
    Name(String),
    Text(String),
    /// An object: its path, package first (`["GEO_Barrels", "Debris", "DebriB"]`);
    /// empty for none.
    Object(Vec<String>),
    Vector([f32; 3]),
    /// A colour, as stored: blue, green, red, alpha.
    Colour([u8; 4]),
    Struct(Values),
    List(Vec<Tagged>),
    Raw(Vec<u8>),
}

/// Named values; an array position other than 0 is written `Name[2]`.
pub type Values = BTreeMap<String, Tagged>;

/// The full path of an object: for an import, its package and groups
/// first; for an object in `pkg` itself, `pkg_name` first.
pub fn object_path(pkg: &Package, pkg_name: &str, index: i32) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = index;
    let mut guard = 0;
    while i != 0 && guard < 16 {
        guard += 1;
        out.push(pkg.object_name(i).to_string());
        i = match ObjectRef::from_index(i) {
            ObjectRef::Import(k) => pkg.imports[k].package_index,
            ObjectRef::Export(k) => {
                let outer = pkg.exports[k].package_index;
                if outer == 0 {
                    out.push(pkg_name.to_string());
                }
                outer
            }
            ObjectRef::None => 0,
        };
    }
    out.reverse();
    out
}

fn float_at(b: &[u8], i: usize) -> f32 {
    f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Turn one property into a value.
pub fn value(pkg: &Package, pkg_name: &str, p: &Property) -> Tagged {
    match &p.value {
        Value::Bool(b) => Tagged::Bool(*b),
        Value::Byte(b) => Tagged::Byte(*b),
        Value::Int(i) => Tagged::Int(*i),
        Value::Float(f) => Tagged::Float(*f),
        Value::Name(n) => Tagged::Name(pkg.name(*n).to_string()),
        Value::Object(o) => Tagged::Object(object_path(pkg, pkg_name, *o)),
        Value::Raw(b) => match p.kind {
            PropertyType::Str => {
                let mut r = Reader::new(b);
                let len = r.compact_index().unwrap_or(0).max(0) as usize;
                let start = b.len().saturating_sub(len);
                let end = b.len().saturating_sub(1).max(start);
                Tagged::Text(String::from_utf8_lossy(&b[start..end]).to_string())
            }
            PropertyType::Struct => {
                let sname = p.struct_name.map(|s| pkg.name(s)).unwrap_or("");
                if (sname.eq_ignore_ascii_case("Vector") || sname.eq_ignore_ascii_case("Rotator"))
                    && b.len() == 12
                {
                    return Tagged::Vector([float_at(b, 0), float_at(b, 4), float_at(b, 8)]);
                }
                if sname.eq_ignore_ascii_case("Color") && b.len() == 4 {
                    return Tagged::Colour([b[0], b[1], b[2], b[3]]);
                }
                let mut r = Reader::new(b);
                match read_properties(pkg, &mut r) {
                    Ok(ps) if r.pos() == b.len() => Tagged::Struct(values(pkg, pkg_name, &ps)),
                    _ => Tagged::Raw(b.clone()),
                }
            }
            PropertyType::Array => array(pkg, pkg_name, b),
            _ => Tagged::Raw(b.clone()),
        },
    }
}

/// A list: a count, then either object references or structs written as
/// named values.
fn array(pkg: &Package, pkg_name: &str, b: &[u8]) -> Tagged {
    let mut r = Reader::new(b);
    let Ok(n) = r.compact_index() else {
        return Tagged::Raw(b.to_vec());
    };
    let n = n.max(0) as usize;
    // Structs first: a list of objects can't be read that way to the end.
    let mut out = Vec::new();
    let mut ok = true;
    for _ in 0..n {
        match read_properties(pkg, &mut r) {
            Ok(ps) => out.push(Tagged::Struct(values(pkg, pkg_name, &ps))),
            Err(_) => {
                ok = false;
                break;
            }
        }
    }
    if ok && n > 0 && r.pos() == b.len() {
        return Tagged::List(out);
    }
    let mut r = Reader::new(b);
    let _ = r.compact_index();
    let mut out = Vec::new();
    for _ in 0..n {
        match r.compact_index() {
            Ok(o) => out.push(Tagged::Object(object_path(pkg, pkg_name, o))),
            Err(_) => return Tagged::Raw(b.to_vec()),
        }
    }
    if r.pos() == b.len() {
        Tagged::List(out)
    } else {
        Tagged::Raw(b.to_vec())
    }
}

/// A list of properties as named values.
pub fn values(pkg: &Package, pkg_name: &str, props: &[Property]) -> Values {
    let mut out = Values::new();
    for p in props {
        let name = pkg.name(p.name);
        let key = if p.array_index > 0 {
            format!("{name}[{}]", p.array_index)
        } else {
            name.to_string()
        };
        out.insert(key, value(pkg, pkg_name, p));
    }
    out
}

/// Read an object's own named values from the start of its data.
pub fn read_object(pkg: &Package, pkg_name: &str, data: &[u8], export: usize) -> Option<Values> {
    let e = pkg.exports.get(export)?;
    let start = usize::try_from(e.serial_offset).ok()?;
    let len = usize::try_from(e.serial_size).ok()?;
    let body = data.get(start..start.checked_add(len)?)?;
    let mut r = Reader::new(body);
    let props = read_properties(pkg, &mut r).ok()?;
    Some(values(pkg, pkg_name, &props))
}

/// Look up a value case-insensitively.
pub fn get<'a>(v: &'a Values, key: &str) -> Option<&'a Tagged> {
    v.get(key).or_else(|| {
        v.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, x)| x)
    })
}

pub fn float(v: &Values, key: &str) -> Option<f32> {
    match get(v, key)? {
        Tagged::Float(f) => Some(*f),
        Tagged::Int(i) => Some(*i as f32),
        Tagged::Byte(b) => Some(*b as f32),
        _ => None,
    }
}

pub fn boolean(v: &Values, key: &str) -> Option<bool> {
    match get(v, key)? {
        Tagged::Bool(b) => Some(*b),
        _ => None,
    }
}
