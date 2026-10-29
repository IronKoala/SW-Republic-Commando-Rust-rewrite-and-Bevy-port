//! Reads the default values ("defaultproperties") of script classes, the
//! numbers a designer set for each kind of character, weapon or AI.
//!
//! Each class in a script package (`.u`) ends with its defaults, stored as
//! the same tagged property list textures and actors use. Only values that
//! differ from the parent class are stored, so the full set for a class is
//! its own list on top of its parent's, its grandparent's, and so on.
//!
//! Finding one class's own list is `class_defaults::class_defaults` (the
//! weapons thread's reader). This file adds what the AI needs on top: many
//! script packages searched together, so a class's parents can live in
//! other packages (Properties -> CTCharacters -> CTGame -> Engine), and the
//! merged result with each value converted to plain names and numbers.

use crate::class_defaults::class_defaults;
use crate::properties::Value;
use crate::{ObjectRef, Package};
use std::collections::{HashMap, HashSet};

/// One default value, with references turned into names so it can be used
/// without the package it came from.
#[derive(Debug, Clone, PartialEq)]
pub enum DefaultValue {
    Byte(u8),
    Int(i32),
    Bool(bool),
    Float(f32),
    /// Name of the object (or class) referred to, `None` for none.
    Object(String),
    Name(String),
    /// Struct, string, array and other values, as stored.
    Raw(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct DefaultEntry {
    pub name: String,
    pub index: u32,
    /// Struct type name, for struct values (`Range`, `Vector`, ...).
    pub struct_name: Option<String>,
    pub value: DefaultValue,
    /// The class whose defaults set this value.
    pub set_by: String,
}

/// A class's full set of defaults, its own on top of its parents'.
#[derive(Debug, Clone, Default)]
pub struct Defaults {
    /// Class first, then its parent, up to the root.
    pub chain: Vec<String>,
    pub entries: Vec<DefaultEntry>,
}

impl Defaults {
    pub fn get(&self, name: &str) -> Option<&DefaultEntry> {
        self.get_index(name, 0)
    }

    pub fn get_index(&self, name: &str, index: u32) -> Option<&DefaultEntry> {
        self.entries
            .iter()
            .find(|e| e.index == index && e.name.eq_ignore_ascii_case(name))
    }

    /// A number, whichever way it is stored (float, int or byte).
    pub fn number(&self, name: &str) -> Option<f32> {
        match &self.get(name)?.value {
            DefaultValue::Float(f) => Some(*f),
            DefaultValue::Int(i) => Some(*i as f32),
            DefaultValue::Byte(b) => Some(*b as f32),
            _ => None,
        }
    }

    pub fn bool(&self, name: &str) -> Option<bool> {
        match &self.get(name)?.value {
            DefaultValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn object(&self, name: &str) -> Option<&str> {
        match &self.get(name)?.value {
            DefaultValue::Object(s) | DefaultValue::Name(s) => Some(s),
            _ => None,
        }
    }

    /// A `Range` struct (Min, Max as two floats).
    pub fn range(&self, name: &str) -> Option<(f32, f32)> {
        match &self.get(name)?.value {
            DefaultValue::Raw(b) if b.len() == 8 => Some((
                f32::from_le_bytes(b[0..4].try_into().ok()?),
                f32::from_le_bytes(b[4..8].try_into().ok()?),
            )),
            _ => None,
        }
    }
}

struct ClassInfo {
    package: usize,
    export: usize,
    parent: Option<String>,
}

/// A set of script packages, searched together so a class's parents can
/// live in other packages (CTCharacters -> CTGame -> Engine).
#[derive(Default)]
pub struct ClassLibrary {
    packages: Vec<(String, Package, Vec<u8>)>,
    classes: HashMap<String, ClassInfo>,
    property_names: HashSet<String>,
}

impl ClassLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one script package. `name` is its file name without extension.
    pub fn add_package(&mut self, name: &str, data: Vec<u8>) -> crate::Result<()> {
        let pkg = Package::parse(&data)?;
        let p = self.packages.len();
        for (i, e) in pkg.exports.iter().enumerate() {
            let class = pkg.export_class_name(e);
            let obj = pkg.name(e.object_name).to_ascii_lowercase();
            if class == "Class" {
                let parent = match pkg.object_name(e.super_index) {
                    "None" => None,
                    s => Some(s.to_string()),
                };
                self.classes.entry(obj).or_insert(ClassInfo {
                    package: p,
                    export: i,
                    parent,
                });
            } else if class.ends_with("Property") {
                self.property_names.insert(obj);
            }
        }
        self.packages.push((name.to_string(), pkg, data));
        Ok(())
    }

    pub fn has_class(&self, class: &str) -> bool {
        self.classes.contains_key(&class.to_ascii_lowercase())
    }

    /// Names of all classes whose parent chain includes `ancestor`.
    pub fn subclasses_of(&self, ancestor: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .classes
            .iter()
            .filter(|(_, c)| self.package_of(c).is_some())
            .map(|(k, c)| {
                let (_, pkg, _) = &self.packages[c.package];
                (k, pkg.name(pkg.exports[c.export].object_name).to_string())
            })
            .filter(|(k, _)| self.chain(k).iter().any(|c| c.eq_ignore_ascii_case(ancestor)))
            .map(|(_, n)| n)
            .collect();
        out.sort();
        out
    }

    fn package_of(&self, c: &ClassInfo) -> Option<&str> {
        self.packages.get(c.package).map(|p| p.0.as_str())
    }

    /// The class and all its parents, class first.
    pub fn chain(&self, class: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = Some(class.to_string());
        while let Some(c) = cur {
            if out.len() > 32 {
                break;
            }
            let info = self.classes.get(&c.to_ascii_lowercase());
            out.push(match info {
                Some(i) => {
                    let (_, pkg, _) = &self.packages[i.package];
                    pkg.name(pkg.exports[i.export].object_name).to_string()
                }
                None => c.clone(),
            });
            cur = info.and_then(|i| i.parent.clone());
        }
        out
    }

    /// Only the values this class itself sets. `None` if the class isn't
    /// known or its list couldn't be found.
    pub fn own_defaults(&self, class: &str) -> Option<Vec<DefaultEntry>> {
        let info = self.classes.get(&class.to_ascii_lowercase())?;
        let (_, pkg, data) = &self.packages[info.package];
        let e = &pkg.exports[info.export];
        let class_name = pkg.name(e.object_name).to_string();
        let is_property = |n: &str| self.property_names.contains(&n.to_ascii_lowercase());
        let found = class_defaults(pkg, data, e, &is_property)?;
        Some(
            found
                .properties
                .iter()
                .map(|p| DefaultEntry {
                    name: pkg.name(p.name).to_string(),
                    index: p.array_index,
                    struct_name: p.struct_name.map(|n| pkg.name(n).to_string()),
                    value: convert(pkg, &p.value),
                    set_by: class_name.clone(),
                })
                .collect(),
        )
    }

    /// Every default value for `class`: its own, then whatever its parents
    /// set that it doesn't.
    pub fn defaults(&self, class: &str) -> Defaults {
        let chain = self.chain(class);
        let mut entries: Vec<DefaultEntry> = Vec::new();
        for c in &chain {
            for d in self.own_defaults(c).unwrap_or_default() {
                let already = entries
                    .iter()
                    .any(|e| e.index == d.index && e.name.eq_ignore_ascii_case(&d.name));
                if !already {
                    entries.push(d);
                }
            }
        }
        Defaults { chain, entries }
    }
}

fn convert(pkg: &Package, v: &Value) -> DefaultValue {
    match v {
        Value::Byte(b) => DefaultValue::Byte(*b),
        Value::Int(i) => DefaultValue::Int(*i),
        Value::Bool(b) => DefaultValue::Bool(*b),
        Value::Float(f) => DefaultValue::Float(*f),
        Value::Object(o) => DefaultValue::Object(match ObjectRef::from_index(*o) {
            ObjectRef::None => "None".to_string(),
            _ => pkg.object_name(*o).to_string(),
        }),
        Value::Name(n) => DefaultValue::Name(pkg.name(*n).to_string()),
        Value::Raw(b) => DefaultValue::Raw(b.clone()),
    }
}
