//! Loose physics props (Karma bodies): rocks, crates, cans and the like
//! that the original simulates with its Karma physics engine, and the
//! `MegaForce` actors that push them when a level script triggers them.
//!
//! What this reads, from the map first and then the actor's class and its
//! parents (`Properties/KarmaPropProperties.u`, `Engine.u`):
//!
//! - which actors are bodies: a `KarmaProp` or `KActor` (but not a joint,
//!   `KConstraint`, or a ragdoll, which uses a skeleton) whose `Physics`
//!   is `PHYS_Karma` (13) and which has `KParams`;
//! - their `KParams` object (a `KarmaParams`): mass, damping, friction,
//!   bounce, whether they move from the start (`KStartEnabled`) and with
//!   what speed, gravity scale, "high detail only";
//! - their shape: the static mesh's `KPhysicsProps` (`rc_package::karma`),
//!   scaled by the actor's `DrawScale` and `DrawScale3D`;
//! - what wakes and hurts them (`bKTakeShot`, `DamageThreshold`) and the
//!   sound they make on landing (`ImpactSounds`, `ImpactInterval`,
//!   `ImpactVolume`).
//!
//! The rules these feed are written down in `docs/DESIGN.md` ("Physics
//! props") with where each was found. Plain Rust, no Bevy, Unreal units.

use crate::actors::{self, LevelInfo, Placement};
use crate::bsp::Vec3;
use crate::class_look::{object_path, ClassLook, ClassLooks, Loaded};
use rc_package::karma::{read_kmesh_props, KElement};
use rc_package::properties::{read_properties, Property, PropertyType, Value};
use rc_package::{Package, Reader};

/// `PHYS_Karma` in the game's `EPhysics`.
pub const PHYS_KARMA: u8 = 13;

/// A `KarmaParams` object's values. Unset values are the `KarmaParams` /
/// `KarmaParamsCollision` class defaults in `Engine.u`.
#[derive(Debug, Clone, PartialEq)]
pub struct KParams {
    /// Which object these came from (`KarmaParams111`), for the log.
    pub name: String,
    pub mass: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub buoyancy: f32,
    pub start_enabled: bool,
    /// Unreal units per second.
    pub start_lin_vel: Vec3,
    pub start_ang_vel: Vec3,
    pub non_spherical_inertia: bool,
    pub gravity_scale: f32,
    pub high_detail_only: bool,
    pub stay_upright: bool,
    pub friction: f32,
    pub restitution: f32,
    /// Karma units per second (the `KImpact` event's threshold).
    pub impact_threshold: f32,
}

impl Default for KParams {
    /// The class defaults (`rc_probe --class KarmaParams`).
    fn default() -> Self {
        KParams {
            name: String::new(),
            mass: 1.0,
            linear_damping: 0.2,
            angular_damping: 0.2,
            buoyancy: 0.0,
            start_enabled: false,
            start_lin_vel: [0.0; 3],
            start_ang_vel: [0.0; 3],
            non_spherical_inertia: false,
            gravity_scale: 1.0,
            high_detail_only: true,
            stay_upright: false,
            friction: 0.0,
            restitution: 0.0,
            impact_threshold: 1_000_000.0,
        }
    }
}

fn vec3(p: &Property) -> Option<Vec3> {
    match &p.value {
        Value::Raw(b) if b.len() == 12 => {
            let f = |i: usize| f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
            Some([f(0), f(1), f(2)])
        }
        _ => None,
    }
}

impl KParams {
    fn apply(&mut self, pkg: &Package, props: &[Property]) {
        for p in props.iter().filter(|p| p.array_index == 0) {
            let f = match p.value {
                Value::Float(f) => Some(f),
                _ => None,
            };
            let b = match p.value {
                Value::Bool(b) => Some(b),
                _ => None,
            };
            match pkg.name(p.name).to_ascii_lowercase().as_str() {
                "kmass" => self.mass = f.unwrap_or(self.mass),
                "klineardamping" => self.linear_damping = f.unwrap_or(self.linear_damping),
                "kangulardamping" => self.angular_damping = f.unwrap_or(self.angular_damping),
                "kbuoyancy" => self.buoyancy = f.unwrap_or(self.buoyancy),
                "kstartenabled" => self.start_enabled = b.unwrap_or(self.start_enabled),
                "kstartlinvel" => self.start_lin_vel = vec3(p).unwrap_or(self.start_lin_vel),
                "kstartangvel" => self.start_ang_vel = vec3(p).unwrap_or(self.start_ang_vel),
                "bknonsphericalinertia" => {
                    self.non_spherical_inertia = b.unwrap_or(self.non_spherical_inertia)
                }
                "kactorgravscale" => self.gravity_scale = f.unwrap_or(self.gravity_scale),
                "bhighdetailonly" => self.high_detail_only = b.unwrap_or(self.high_detail_only),
                "bkstayupright" => self.stay_upright = b.unwrap_or(self.stay_upright),
                "kfriction" => self.friction = f.unwrap_or(self.friction),
                "krestitution" => self.restitution = f.unwrap_or(self.restitution),
                "kimpactthreshold" => self.impact_threshold = f.unwrap_or(self.impact_threshold),
                _ => {}
            }
        }
    }

    /// Read the `KarmaParams` object `index` of `pkg` (an export: the map's
    /// own, or a class default's in its Properties package).
    pub fn read(pkg: &Package, data: &[u8], index: i32) -> Option<KParams> {
        let e = pkg.exports.get(usize::try_from(index - 1).ok()?)?;
        let class = pkg.export_class_name(e);
        // Ragdolls (`KarmaParamsSkel`) need the skeleton physics, not done.
        if !class.eq_ignore_ascii_case("KarmaParams")
            && !class.eq_ignore_ascii_case("KarmaParamsRBFull")
        {
            return None;
        }
        let bytes = actors::object_bytes(data, e).ok()?;
        let mut r = Reader::new(bytes);
        let props = read_properties(pkg, &mut r).ok()?;
        let mut k = KParams {
            name: pkg.name(e.object_name).to_string(),
            ..KParams::default()
        };
        k.apply(pkg, &props);
        Some(k)
    }
}

/// One physics body's collision shape part, in the body's own frame
/// (Unreal units, scaled by the actor's draw scale, not yet rotated).
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// Corner points of a convex hull.
    Convex(Vec<Vec3>),
    Sphere { centre: Vec3, radius: f32 },
    /// A box: centre, its three axes (unit length) and full sizes.
    Box { centre: Vec3, axes: [Vec3; 3], size: Vec3 },
    /// A capsule along `axis`: centre, radius, length of the straight part.
    Capsule { centre: Vec3, axis: Vec3, radius: f32, length: f32 },
    /// A cylinder along `axis`.
    Cylinder { centre: Vec3, axis: Vec3, radius: f32, height: f32 },
}

/// The shape of a static mesh for Karma, from its `KPhysicsProps`.
/// `mesh` is the package holding the mesh and the reference to it there.
/// `scale` is DrawScale x DrawScale3D. Returns the parts and the mesh's
/// stored inertia (for a mass of 1, Karma units) and centre-of-mass nudge.
pub fn mesh_shape(
    mesh: (&Loaded, i32),
    scale: Vec3,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> Result<(Vec<Shape>, [f32; 6], Vec3), String> {
    let (owner, k) = resolve(mesh.0, mesh.1, open)?;
    let (pkg, data) = (&owner.0, &owner.1);
    let e = &pkg.exports[k];
    // The mesh's Karma shape is the `KMeshProps` object stored inside it
    // (its outer is the mesh). The mesh refers to it from its native
    // data, not from a property.
    let kp = pkg
        .exports
        .iter()
        .position(|x| x.package_index == k as i32 + 1 && pkg.export_class_name(x) == "KMeshProps")
        .map(|i| i as i32 + 1)
        .ok_or_else(|| format!("{} has no Karma shape (KMeshProps)", pkg.name(e.object_name)))?;
    let ke = &pkg.exports[(kp - 1) as usize];
    let km = read_kmesh_props(pkg, data, ke).map_err(|e| e.to_string())?;
    let s = |v: [f32; 3]| [v[0] * scale[0], v[1] * scale[1], v[2] * scale[2]];
    let avg = (scale[0].abs() + scale[1].abs() + scale[2].abs()) / 3.0;
    let mut shapes = Vec::new();
    for el in &km.elements {
        let tm = el.tm();
        // Rows: X axis, Y axis, Z axis, origin (`FMatrix`, Unreal's
        // row-vector convention: point' = point * M).
        let row = |i: usize| [tm[i * 4], tm[i * 4 + 1], tm[i * 4 + 2]];
        let (ax, ay, az, o) = (row(0), row(1), row(2), row(3));
        let place = |p: [f32; 3]| {
            s([
                p[0] * ax[0] + p[1] * ay[0] + p[2] * az[0] + o[0],
                p[0] * ax[1] + p[1] * ay[1] + p[2] * az[1] + o[1],
                p[0] * ax[2] + p[1] * ay[2] + p[2] * az[2] + o[2],
            ])
        };
        let unit = |v: [f32; 3]| {
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
            [v[0] / l, v[1] / l, v[2] / l]
        };
        shapes.push(match el {
            KElement::Convex { points, .. } => Shape::Convex(points.iter().map(|p| place(*p)).collect()),
            KElement::Sphere { radius, .. } => Shape::Sphere {
                centre: s(o),
                radius: radius * avg,
            },
            KElement::Box { size, .. } => Shape::Box {
                centre: s(o),
                axes: [unit(ax), unit(ay), unit(az)],
                size: [size[0] * scale[0].abs(), size[1] * scale[1].abs(), size[2] * scale[2].abs()],
            },
            // Karma's capsules and cylinders run along their own Z.
            KElement::Sphyl { radius, length, .. } => Shape::Capsule {
                centre: s(o),
                axis: unit(az),
                radius: radius * avg,
                length: length * avg,
            },
            KElement::Cylinder { radius, height, .. } => Shape::Cylinder {
                centre: s(o),
                axis: unit(az),
                radius: radius * avg,
                height: height * avg,
            },
        });
    }
    if shapes.is_empty() {
        return Err(format!("{}'s Karma shape is empty", pkg.name(e.object_name)));
    }
    Ok((shapes, km.inertia, km.com_nudge))
}

fn resolve(
    owner: &Loaded,
    index: i32,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> Result<(Loaded, usize), String> {
    if index > 0 {
        return Ok((owner.clone(), (index - 1) as usize));
    }
    let path = object_path(&owner.0, index);
    if path.len() < 2 {
        return Err("no mesh".into());
    }
    let pkg = open(&path[0]).ok_or_else(|| format!("package {} not found", path[0]))?;
    let rest = &path[1..];
    let k = (0..pkg.0.exports.len())
        .find(|&i| {
            let p = object_path(&pkg.0, i as i32 + 1);
            p.len() == rest.len() && p.iter().zip(rest).all(|(a, b)| a.eq_ignore_ascii_case(b))
        })
        .ok_or_else(|| format!("{} not found", path.join(".")))?;
    Ok((pkg, k))
}

/// One loose physics prop, placed in the map or made by a level script.
#[derive(Debug, Clone)]
pub struct KarmaBody {
    /// The map's export, or `None` for one a script spawned.
    pub export: Option<usize>,
    pub class: String,
    pub name: String,
    pub tag: String,
    /// `Event`, sent when a shot or blast wakes it (`AffectKarma`).
    pub event: Option<String>,
    pub location: Vec3,
    /// Pitch, yaw, roll (65536 = a full turn).
    pub rotation: [i32; 3],
    /// DrawScale x DrawScale3D.
    pub scale: Vec3,
    /// Full path of the static mesh (package first).
    pub mesh_path: Vec<String>,
    pub params: KParams,
    pub shapes: Vec<Shape>,
    pub inertia: [f32; 6],
    pub com_nudge: Vec3,
    /// `bKTakeShot`: shots and blasts wake and push it.
    pub take_shot: bool,
    pub damage_threshold: i32,
    /// Full paths of its landing sounds; one is picked at random.
    pub impact_sounds: Vec<Vec<String>>,
    pub impact_interval: f32,
    pub impact_volume: f32,
    /// Seconds after it wakes until it is removed (0: stays).
    pub life_time: f32,
    pub blocks_players: bool,
    /// `bBlockKarma`: other bodies bump into it.
    pub blocks_karma: bool,
}

/// Values for an actor: the map's (`own`, may be empty) over its class's.
struct Lookup<'a> {
    pkg: &'a Package,
    data: &'a [u8],
    own: &'a [Property],
    look: &'a ClassLook,
}

impl<'a> Lookup<'a> {
    /// The `KParams` object: the map's own (in the map), else the class's
    /// (in its Properties package).
    fn kparams(&self) -> Result<KParams, String> {
        if let Some(p) = self
            .own
            .iter()
            .find(|p| p.array_index == 0 && self.pkg.name(p.name).eq_ignore_ascii_case("KParams"))
        {
            return match p.value {
                Value::Object(o) if o > 0 => KParams::read(self.pkg, self.data, o)
                    .ok_or_else(|| "KParams isn't a KarmaParams (a ragdoll's?)".to_string()),
                _ => Err("no KParams".into()),
            };
        }
        match self.look.find("KParams", 0) {
            Some((l, p)) => match p.value {
                Value::Object(o) if o > 0 => KParams::read(&l.0, &l.1, o)
                    .ok_or_else(|| "KParams isn't a KarmaParams (a ragdoll's?)".to_string()),
                _ => Err("no KParams".into()),
            },
            None => Err("no KParams".into()),
        }
    }

    fn get(&self, key: &str) -> Option<(&'a Package, &'a Property)> {
        if let Some(p) = self
            .own
            .iter()
            .find(|p| p.array_index == 0 && self.pkg.name(p.name).eq_ignore_ascii_case(key))
        {
            return Some((self.pkg, p));
        }
        self.look.find(key, 0).map(|(l, p)| (&l.0, p))
    }
    fn float(&self, key: &str, d: f32) -> f32 {
        match self.get(key).map(|(_, p)| &p.value) {
            Some(Value::Float(f)) => *f,
            Some(Value::Int(i)) => *i as f32,
            _ => d,
        }
    }
    fn int(&self, key: &str, d: i32) -> i32 {
        match self.get(key).map(|(_, p)| &p.value) {
            Some(Value::Int(i)) => *i,
            _ => d,
        }
    }
    fn flag(&self, key: &str, d: bool) -> bool {
        match self.get(key).map(|(_, p)| &p.value) {
            Some(Value::Bool(b)) => *b,
            _ => d,
        }
    }
    fn byte(&self, key: &str) -> Option<u8> {
        match self.get(key).map(|(_, p)| &p.value) {
            Some(Value::Byte(b)) => Some(*b),
            _ => None,
        }
    }
    fn name(&self, key: &str) -> Option<String> {
        let (pkg, p) = self.get(key)?;
        let n = match p.value {
            Value::Name(n) => pkg.name(n).to_string(),
            _ => return None,
        };
        (!n.is_empty() && !n.eq_ignore_ascii_case("None")).then_some(n)
    }
    /// A list of object references, each as a full path.
    fn object_paths(&self, key: &str) -> Vec<Vec<String>> {
        let Some((pkg, p)) = self.get(key) else {
            return Vec::new();
        };
        let Value::Raw(b) = &p.value else {
            return Vec::new();
        };
        if p.kind != PropertyType::Array {
            return Vec::new();
        }
        let mut r = Reader::new(b);
        let n = r.compact_index().unwrap_or(0).max(0);
        let mut out = Vec::new();
        for _ in 0..n {
            match r.compact_index() {
                Ok(o) if o != 0 => out.push(object_path(pkg, o)),
                _ => break,
            }
        }
        out
    }
}

/// Why an actor that looks like a physics prop isn't one, for the log.
#[derive(Debug, Clone, PartialEq)]
pub struct Skipped {
    pub name: String,
    pub class: String,
    pub why: String,
}

fn build_body(
    l: &Lookup,
    look: &ClassLook,
    export: Option<usize>,
    class: &str,
    name: &str,
    location: Vec3,
    rotation: [i32; 3],
    scale: Vec3,
    mesh: Option<(Loaded, i32)>,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> Result<KarmaBody, String> {
    if look.is_a("KConstraint") {
        return Err("a joint (not done yet)".into());
    }
    if look.is_a("Ragdoll") {
        return Err("a ragdoll (needs the skeleton physics, not done yet)".into());
    }
    let physics = l.byte("Physics").unwrap_or(0);
    if physics != PHYS_KARMA {
        return Err(format!("Physics {physics}, not Karma: it never moves"));
    }
    let params = l.kparams()?;
    let (mesh_owner, mesh_ref) = mesh.ok_or("no static mesh")?;
    let mesh_path = object_path(&mesh_owner.0, mesh_ref);
    let (shapes, inertia, com_nudge) = mesh_shape((&mesh_owner, mesh_ref), scale, open)?;
    Ok(KarmaBody {
        export,
        class: class.to_string(),
        name: name.to_string(),
        tag: l.name("Tag").unwrap_or_else(|| class.to_string()),
        event: l.name("Event"),
        location,
        rotation,
        scale,
        mesh_path,
        params,
        shapes,
        inertia,
        com_nudge,
        take_shot: l.flag("bKTakeShot", false),
        damage_threshold: l.int("DamageThreshold", 0),
        impact_sounds: l.object_paths("ImpactSounds"),
        impact_interval: l.float("ImpactInterval", 0.0),
        impact_volume: l.float("ImpactVolume", 0.0),
        life_time: l.float("LifeTime", 0.0),
        blocks_players: l.flag("bBlockPlayers", false),
        blocks_karma: l.flag("bBlockKarma", false),
    })
}

/// Every loose physics prop placed in the map, and the actors that looked
/// like one but aren't (with why).
pub fn read(
    looks: &mut ClassLooks,
    map: &Loaded,
    level: &LevelInfo,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> (Vec<KarmaBody>, Vec<Skipped>) {
    let (pkg, data) = (&map.0, &map.1);
    let (mut out, mut skipped) = (Vec::new(), Vec::new());
    for &a in level.actors.iter().flatten() {
        let class = pkg.export_class_name(&pkg.exports[a]).to_string();
        if matches!(
            class.as_str(),
            "StaticMeshActor" | "PathNode" | "CoverPoint" | "Light" | "Brush"
        ) {
            continue;
        }
        let Some(look) = looks.for_actor(pkg, a, open) else {
            continue;
        };
        if !look.is_a("KarmaProp") && !look.is_a("KActor") {
            continue;
        }
        let Ok(own) = actors::read_actor_properties(pkg, data, a) else {
            continue;
        };
        let Ok(mut place) = actors::read_placement(pkg, data, a) else {
            continue;
        };
        let mut mesh = (place.static_mesh != 0).then(|| (map.clone(), place.static_mesh));
        if mesh.is_none() {
            if let Some(o) = crate::class_look::apply_class_look(looks, pkg, data, &mut place, open) {
                mesh = Some((o, place.static_mesh));
            }
        }
        let l = Lookup { pkg, data, own: &own, look: &look };
        let result = build_body(
            &l,
            &look,
            Some(a),
            &class,
            &place.name,
            place.location,
            place.rotation,
            scaled(&place),
            mesh,
            open,
        );
        match result {
            Ok(b) => out.push(b),
            Err(why) => skipped.push(Skipped {
                name: place.name.clone(),
                class,
                why,
            }),
        }
    }
    (out, skipped)
}

fn scaled(p: &Placement) -> Vec3 {
    [
        p.draw_scale * p.draw_scale_3d[0],
        p.draw_scale * p.draw_scale_3d[1],
        p.draw_scale * p.draw_scale_3d[2],
    ]
}

/// A body of class `class` (in script package `package`) made by a level
/// script (`ACTION_SpawnActor`) at `location`, turned by `rotation`.
pub fn spawned(
    looks: &mut ClassLooks,
    package: &str,
    class: &str,
    location: Vec3,
    rotation: [i32; 3],
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> Result<KarmaBody, String> {
    let look = looks
        .for_class(package, class, open)
        .ok_or_else(|| format!("class {package}.{class} not found"))?;
    if !look.is_a("KarmaProp") && !look.is_a("KActor") {
        return Err(format!("{class} isn't a physics prop"));
    }
    let first = &look.defaults.first().ok_or("class has no defaults")?.0;
    let l = Lookup {
        // No map values: any package will do for the empty list.
        pkg: &first.0,
        data: &first.1,
        own: &[],
        look: &look,
    };
    let ds = look.draw_scale.unwrap_or(1.0);
    let d3 = look.draw_scale_3d.unwrap_or([1.0; 3]);
    let scale = [ds * d3[0], ds * d3[1], ds * d3[2]];
    build_body(
        &l,
        &look,
        None,
        class,
        class,
        location,
        rotation,
        scale,
        look.static_mesh.clone(),
        open,
    )
}

/// How a `MegaForce` pushes (`EImpulseType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Impulse {
    /// Along a line from it, as far as `distance`, pushing what the line
    /// passes through in its facing direction.
    Line,
    /// Everything within `distance` of it.
    Sphere,
}

/// A `MegaForce` (`CTGame.u`): pushes physics props when its tag is
/// triggered.
#[derive(Debug, Clone, PartialEq)]
pub struct MegaForce {
    pub name: String,
    pub tag: String,
    pub location: Vec3,
    pub rotation: [i32; 3],
    pub kind: Impulse,
    pub distance: f32,
    pub amount: f32,
    /// `bApplyToCenterOfMass` (line only): push at the prop's centre, not
    /// where the line meets it.
    pub at_centre: bool,
    /// `bUseEmitterDirection` (sphere only): push along its facing, not
    /// away from it.
    pub emitter_direction: bool,
    /// `bAutoActivateAffected`: triggers each prop it reaches first.
    pub auto_activate: bool,
}

/// Every `MegaForce` in the map. Class defaults (`CTGame.u`):
/// `ForceDistance` 1000, everything else 0 / off, `IT_Line`.
pub fn megaforces(pkg: &Package, data: &[u8], level: &LevelInfo) -> Vec<MegaForce> {
    let mut out = Vec::new();
    for &a in level.actors.iter().flatten() {
        if !pkg.export_class_name(&pkg.exports[a]).eq_ignore_ascii_case("MegaForce") {
            continue;
        }
        let Ok(props) = actors::read_actor_properties(pkg, data, a) else {
            continue;
        };
        let Ok(place) = actors::read_placement(pkg, data, a) else {
            continue;
        };
        let find = |k: &str| {
            props
                .iter()
                .find(|p| p.array_index == 0 && pkg.name(p.name).eq_ignore_ascii_case(k))
                .map(|p| &p.value)
        };
        let float = |k: &str, d: f32| match find(k) {
            Some(Value::Float(f)) => *f,
            _ => d,
        };
        let flag = |k: &str| matches!(find(k), Some(Value::Bool(true)));
        let tag = match find("Tag") {
            Some(Value::Name(n)) => pkg.name(*n).to_string(),
            _ => "MegaForce".to_string(),
        };
        out.push(MegaForce {
            name: place.name.clone(),
            tag,
            location: place.location,
            rotation: place.rotation,
            kind: if matches!(find("ImpulseType"), Some(Value::Byte(1))) {
                Impulse::Sphere
            } else {
                Impulse::Line
            },
            distance: float("ForceDistance", 1000.0),
            amount: float("ForceAmount", 0.0),
            at_centre: flag("bApplyToCenterOfMass"),
            emitter_direction: flag("bUseEmitterDirection"),
            auto_activate: flag("bAutoActivateAffected"),
        });
    }
    out
}

/// The direction an actor faces (`vector(Rotation)`), Unreal axes.
pub fn facing(rotation: [i32; 3]) -> Vec3 {
    actors::rotation_axes(rotation)[0]
}
