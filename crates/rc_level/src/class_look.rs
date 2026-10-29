//! What an actor looks like when the map doesn't say: its class's default
//! `StaticMesh`, `DrawScale`, `DrawScale3D` and `bHidden`.
//!
//! A map only stores the values a designer changed. Most props (barrels,
//! generators, lamps, bacta dispensers, ammo boxes) and the double-door
//! movers take their model from their class, which lives in a script
//! package such as `Properties/StaticPropProperties.u` or `CTGame.u`. Those
//! values are the class's "defaults", stored at the end of the class, on top
//! of its parent's (`K_GEO04_WallComponent` -> `KarmaProp` -> ... ->
//! `Actor`).
//!
//! Reading one class's own defaults is `rc_package::class_defaults` (written
//! for the weapon numbers). This file walks the parent chain across
//! packages and picks out the drawing values. Like the rest of this crate
//! it opens no files: the caller hands in a function that opens a package
//! by name.

use crate::bsp::Vec3;
use rc_package::class_defaults::{class_defaults, find_class, property_names};
use rc_package::properties::{Property, Value};
use rc_package::{ObjectRef, Package};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// A package and its bytes.
pub type Loaded = Rc<(Package, Vec<u8>)>;

/// `DrawType` for a static mesh (UE2's `EDrawType::DT_StaticMesh`).
pub const DT_STATIC_MESH: u8 = 8;

#[derive(Clone)]
pub struct ClassLook {
    /// Class first, then its parents.
    pub chain: Vec<String>,
    /// The default static mesh: the package whose reference it is, and the
    /// reference.
    pub static_mesh: Option<(Loaded, i32)>,
    pub draw_type: Option<u8>,
    pub draw_scale: Option<f32>,
    pub draw_scale_3d: Option<Vec3>,
    pub hidden: Option<bool>,
}

impl ClassLook {
    /// The mesh to draw: a default mesh, unless the class draws something
    /// else (a skeletal mesh, a sprite) or is hidden in the game.
    pub fn drawn_mesh(&self) -> Option<&(Loaded, i32)> {
        let draws_mesh = self.draw_type.is_none_or(|t| t == DT_STATIC_MESH);
        (draws_mesh && self.hidden != Some(true))
            .then_some(self.static_mesh.as_ref())
            .flatten()
    }
}

/// Names from the outermost package down to the object.
pub fn object_path(pkg: &Package, index: i32) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = index;
    for _ in 0..16 {
        match ObjectRef::from_index(i) {
            ObjectRef::None => break,
            ObjectRef::Import(k) => {
                let Some(m) = pkg.imports.get(k) else { break };
                out.push(pkg.name(m.object_name).to_string());
                i = m.package_index;
            }
            ObjectRef::Export(k) => {
                let Some(e) = pkg.exports.get(k) else { break };
                out.push(pkg.name(e.object_name).to_string());
                i = e.package_index;
            }
        }
    }
    out.reverse();
    out
}

/// Looks up class defaults, caching each class.
#[derive(Default)]
pub struct ClassLooks {
    cache: HashMap<String, Option<ClassLook>>,
    /// Property names of every script package opened so far.
    names: HashSet<String>,
    names_from: HashSet<String>,
}

impl ClassLooks {
    pub fn new() -> Self {
        Self::default()
    }

    fn learn_names(&mut self, name: &str, pkg: &Package) {
        if self.names_from.insert(name.to_ascii_lowercase()) {
            self.names.extend(property_names(pkg));
        }
    }

    /// The look of the class of export `actor` in `map`. `open` opens a
    /// package by name (`None` if it isn't there).
    pub fn for_actor(
        &mut self,
        map: &Package,
        actor: usize,
        open: &mut dyn FnMut(&str) -> Option<Loaded>,
    ) -> Option<ClassLook> {
        let path = object_path(map, map.exports.get(actor)?.class_index);
        if path.len() != 2 {
            // Classes defined inside the map itself, or odd paths.
            return None;
        }
        self.for_class(&path[0], &path[1], open)
    }

    pub fn for_class(
        &mut self,
        package: &str,
        class: &str,
        open: &mut dyn FnMut(&str) -> Option<Loaded>,
    ) -> Option<ClassLook> {
        let key = format!("{package}.{class}").to_ascii_lowercase();
        if let Some(c) = self.cache.get(&key) {
            return c.clone();
        }
        let look = self.build(package, class, open);
        self.cache.insert(key, look.clone());
        look
    }

    fn build(
        &mut self,
        package: &str,
        class: &str,
        open: &mut dyn FnMut(&str) -> Option<Loaded>,
    ) -> Option<ClassLook> {
        // Every script package can use names from Core and Engine.
        for base in ["Core", "Engine"] {
            if let Some(p) = open(base) {
                self.learn_names(base, &p.0);
            }
        }
        // First the chain of classes, so every package's property names
        // are known before any defaults are read.
        let mut chain: Vec<(Loaded, usize, String)> = Vec::new();
        let (mut pkg_name, mut class_name) = (package.to_string(), class.to_string());
        for _ in 0..32 {
            let Some(loaded) = open(&pkg_name) else { break };
            let Some(k) = find_class(&loaded.0, &class_name)
                .and_then(|e| loaded.0.exports.iter().position(|x| std::ptr::eq(x, e)))
            else {
                break;
            };
            self.learn_names(&pkg_name, &loaded.0);
            let sup = loaded.0.exports[k].super_index;
            chain.push((loaded.clone(), k, class_name.clone()));
            let parent = object_path(&loaded.0, sup);
            match parent.len() {
                0 => break,
                // A parent in the same package.
                1 => class_name = parent[0].clone(),
                _ => {
                    pkg_name = parent[0].clone();
                    class_name = parent[parent.len() - 1].clone();
                }
            }
        }
        if chain.is_empty() {
            return None;
        }
        let names = &self.names;
        let is_property = |n: &str| names.contains(&n.to_ascii_lowercase());
        let mut look = ClassLook {
            chain: chain.iter().map(|c| c.2.clone()).collect(),
            static_mesh: None,
            draw_type: None,
            draw_scale: None,
            draw_scale_3d: None,
            hidden: None,
        };
        // Nearest class wins: walk from the class up and keep the first
        // value found for each.
        for (loaded, k, _) in &chain {
            let (pkg, data) = (&loaded.0, &loaded.1);
            let Some(d) = class_defaults(pkg, data, &pkg.exports[*k], &is_property) else {
                continue;
            };
            let find = |n: &str| -> Option<&Property> {
                d.properties
                    .iter()
                    .find(|p| p.array_index == 0 && pkg.name(p.name).eq_ignore_ascii_case(n))
            };
            if look.static_mesh.is_none() {
                if let Some(Value::Object(o)) = find("StaticMesh").map(|p| &p.value) {
                    look.static_mesh = Some((loaded.clone(), *o));
                }
            }
            if look.draw_type.is_none() {
                if let Some(Value::Byte(b)) = find("DrawType").map(|p| &p.value) {
                    look.draw_type = Some(*b);
                }
            }
            if look.draw_scale.is_none() {
                if let Some(Value::Float(f)) = find("DrawScale").map(|p| &p.value) {
                    look.draw_scale = Some(*f);
                }
            }
            if look.draw_scale_3d.is_none() {
                if let Some(Value::Raw(b)) = find("DrawScale3D").map(|p| &p.value) {
                    if b.len() == 12 {
                        let f = |i: usize| f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
                        look.draw_scale_3d = Some([f(0), f(1), f(2)]);
                    }
                }
            }
            if look.hidden.is_none() {
                if let Some(Value::Bool(b)) = find("bHidden").map(|p| &p.value) {
                    look.hidden = Some(*b);
                }
            }
        }
        // A reference to "no mesh" means the class clears its parent's.
        if look.static_mesh.as_ref().is_some_and(|(_, o)| *o == 0) {
            look.static_mesh = None;
        }
        Some(look)
    }
}

/// Fill in an actor's drawing values from its class when the map leaves
/// them out. Only for actors the map gives no `StaticMesh`. Returns the
/// package `place.static_mesh` now refers into (the class's package), or
/// `None` if the class has no mesh to draw, in which case `place` is
/// unchanged. Values the map does set (`DrawScale`, `DrawScale3D`,
/// `bHidden`) win over the class's.
pub fn apply_class_look(
    looks: &mut ClassLooks,
    map: &Package,
    map_data: &[u8],
    place: &mut crate::actors::Placement,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> Option<Loaded> {
    if place.static_mesh != 0 {
        return None;
    }
    let look = looks.for_actor(map, place.export, open)?;
    let (owner, mesh) = look.static_mesh.clone()?;
    let props = crate::actors::read_actor_properties(map, map_data, place.export).ok()?;
    let set = |n: &str| props.iter().any(|p| map.name(p.name).eq_ignore_ascii_case(n));
    let hidden = if set("bHidden") {
        place.hidden
    } else {
        look.hidden.unwrap_or(false)
    };
    let draw_type = props
        .iter()
        .find(|p| map.name(p.name).eq_ignore_ascii_case("DrawType"))
        .and_then(|p| match p.value {
            Value::Byte(b) => Some(b),
            _ => None,
        })
        .or(look.draw_type);
    if draw_type.is_some_and(|t| t != DT_STATIC_MESH) {
        return None;
    }
    place.static_mesh = mesh;
    place.hidden = hidden;
    if !set("DrawScale") {
        if let Some(s) = look.draw_scale {
            place.draw_scale = s;
        }
    }
    if !set("DrawScale3D") {
        if let Some(s) = look.draw_scale_3d {
            place.draw_scale_3d = s;
        }
    }
    Some(owner)
}
