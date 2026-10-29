//! Default values as a plain tree, independent of the package they came
//! from: names and object references are turned into text, structs into
//! lists of fields. This is what the menu code works on.

use rc_package::class::{self, object_class, object_path};
use rc_package::properties::{Property, PropertyType, Value};
use rc_package::Package;

#[derive(Debug, Clone, PartialEq)]
pub enum V {
    Int(i32),
    Float(f32),
    Byte(u8),
    Bool(bool),
    /// A name; "None" is the empty name.
    Name(String),
    Str(String),
    /// An object reference: its class and full dotted path ("" for none).
    Object {
        class: String,
        path: String,
    },
    /// A colour as red, green, blue, alpha.
    Color([u8; 4]),
    /// A struct with its type name and fields.
    Struct {
        kind: String,
        fields: Vec<Field>,
    },
    /// Anything we don't read yet (dynamic arrays, maps).
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    /// Index in a fixed-size array (`MenuOptions[3]` has index 3).
    pub index: u32,
    pub value: V,
}

impl V {
    /// True for the "not set" value of each type. The original menu code
    /// fills in only fields that are zero ("non-zero properties aren't
    /// overwritten", from the comments in the game's `MenuBase` class).
    pub fn is_zero(&self) -> bool {
        match self {
            V::Int(i) => *i == 0,
            V::Float(f) => *f == 0.0,
            V::Byte(b) => *b == 0,
            V::Bool(b) => !b,
            V::Name(n) => n.is_empty() || n.eq_ignore_ascii_case("None"),
            V::Str(s) => s.is_empty(),
            V::Object { path, .. } => path.is_empty(),
            V::Color(c) => *c == [0, 0, 0, 0],
            V::Struct { fields, .. } => fields.iter().all(|f| f.value.is_zero()),
            V::Other => true,
        }
    }

    pub fn fields(&self) -> &[Field] {
        match self {
            V::Struct { fields, .. } => fields,
            _ => &[],
        }
    }

    pub fn kind(&self) -> &str {
        match self {
            V::Struct { kind, .. } => kind,
            _ => "",
        }
    }

    /// A field of a struct by name (array index 0).
    pub fn get(&self, name: &str) -> Option<&V> {
        self.get_at(name, 0)
    }

    pub fn get_at(&self, name: &str, index: u32) -> Option<&V> {
        self.fields()
            .iter()
            .find(|f| f.index == index && f.name.eq_ignore_ascii_case(name))
            .map(|f| &f.value)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut V> {
        match self {
            V::Struct { fields, .. } => fields
                .iter_mut()
                .find(|f| f.index == 0 && f.name.eq_ignore_ascii_case(name))
                .map(|f| &mut f.value),
            _ => None,
        }
    }

    pub fn float(&self, name: &str) -> f32 {
        match self.get(name) {
            Some(V::Float(f)) => *f,
            _ => 0.0,
        }
    }

    pub fn int(&self, name: &str) -> i32 {
        match self.get(name) {
            Some(V::Int(i)) => *i,
            Some(V::Byte(b)) => *b as i32,
            Some(V::Bool(b)) => *b as i32,
            _ => 0,
        }
    }

    pub fn byte(&self, name: &str) -> u8 {
        match self.get(name) {
            Some(V::Byte(b)) => *b,
            _ => 0,
        }
    }

    pub fn text(&self, name: &str) -> String {
        match self.get(name) {
            Some(V::Str(s)) => s.clone(),
            Some(V::Name(n)) if !n.eq_ignore_ascii_case("None") => n.clone(),
            _ => String::new(),
        }
    }

    pub fn object(&self, name: &str) -> Option<String> {
        match self.get(name) {
            Some(V::Object { path, .. }) if !path.is_empty() => Some(path.clone()),
            _ => None,
        }
    }

    pub fn color(&self, name: &str) -> [u8; 4] {
        match self.get(name) {
            Some(V::Color(c)) => *c,
            _ => [0; 4],
        }
    }

    pub fn set(&mut self, name: &str, value: V) {
        if let V::Struct { fields, .. } = self {
            match fields
                .iter_mut()
                .find(|f| f.index == 0 && f.name.eq_ignore_ascii_case(name))
            {
                Some(f) => f.value = value,
                None => fields.push(Field {
                    name: name.to_string(),
                    index: 0,
                    value,
                }),
            }
        }
    }
}

/// Fill every zero field of `target` from `source`, going into structs.
/// Fields `source` has and `target` lacks are added.
pub fn fill_zero(target: &mut V, source: &V) {
    match (target, source) {
        (V::Struct { fields: t, .. }, V::Struct { fields: s, kind: _ }) => {
            for sf in s {
                match t
                    .iter_mut()
                    .find(|tf| tf.index == sf.index && tf.name.eq_ignore_ascii_case(&sf.name))
                {
                    Some(tf) => fill_zero(&mut tf.value, &sf.value),
                    None => t.push(sf.clone()),
                }
            }
        }
        (t, s) => {
            if t.is_zero() && !matches!(s, V::Struct { .. }) {
                *t = s.clone();
            }
        }
    }
}

/// Turn a property read from `pkg` into a value tree.
pub fn decode(pkg: &Package, p: &Property) -> V {
    if let Some(inner) = class::struct_properties(pkg, p) {
        return V::Struct {
            kind: p.struct_name.map_or("", |s| pkg.name(s)).to_string(),
            fields: decode_all(pkg, &inner),
        };
    }
    match (&p.value, p.kind) {
        (Value::Int(i), _) => V::Int(*i),
        (Value::Float(f), _) => V::Float(*f),
        (Value::Byte(b), _) => V::Byte(*b),
        (Value::Bool(b), _) => V::Bool(*b),
        (Value::Name(n), _) => V::Name(pkg.name(*n).to_string()),
        (Value::Object(o), _) => V::Object {
            class: object_class(pkg, *o),
            path: if *o == 0 {
                String::new()
            } else {
                object_path(pkg, *o)
            },
        },
        (Value::Raw(_), PropertyType::Str | PropertyType::String) => {
            V::Str(class::string_value(p).unwrap_or_default())
        }
        (Value::Raw(_), PropertyType::Struct) => match class::color_value(p) {
            Some(c) => V::Color(c),
            None => V::Other,
        },
        _ => V::Other,
    }
}

pub fn decode_all(pkg: &Package, props: &[Property]) -> Vec<Field> {
    props
        .iter()
        .map(|p| Field {
            name: pkg.name(p.name).to_string(),
            index: p.array_index,
            value: decode(pkg, p),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(fields: Vec<(&str, V)>) -> V {
        V::Struct {
            kind: "T".into(),
            fields: fields
                .into_iter()
                .map(|(n, v)| Field {
                    name: n.into(),
                    index: 0,
                    value: v,
                })
                .collect(),
        }
    }

    #[test]
    fn fill_keeps_non_zero() {
        let mut t = s(vec![("PosX", V::Float(0.5)), ("ScaleX", V::Float(0.0))]);
        let src = s(vec![
            ("PosX", V::Float(0.9)),
            ("ScaleX", V::Float(2.0)),
            ("Style", V::Name("X".into())),
        ]);
        fill_zero(&mut t, &src);
        assert_eq!(t.float("PosX"), 0.5);
        assert_eq!(t.float("ScaleX"), 2.0);
        assert_eq!(t.text("Style"), "X");
    }
}
