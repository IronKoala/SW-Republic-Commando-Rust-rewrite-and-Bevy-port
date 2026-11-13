//! Reading effects from the game's script packages.
//!
//! An effect is an `Emitter` class (for example `Explosion_Barrel_GEO` in
//! `Properties/cteffects.u`). Its defaults hold `Emitters`, a list of
//! emitter objects stored in the same package (`SpriteEmitter476`,
//! `MeshEmitter2`, ...), each with its own named values, `SpawnSound` and `AmbientSound`.
//!
//! An emitter only stores what differs from its class's defaults. Those
//! (in `engine.u`, `ParticleEmitter`, `SpriteEmitter`, `MeshEmitter`) were
//! read once with `rc_probe --class` on 2026-11-10 and are written into
//! `EmitterDef::default` and `defaults_for` below, so the game can read an
//! effect from `cteffects.u` alone.

use crate::def::{
    CoordinateSystem, DrawStyle, EffectDef, EmitterDef, EmitterKind, Range, RangeVector,
    StartShape, VelocityFrom,
};
use crate::tagged::{self, boolean, float, get, Tagged, Values};
use rc_package::class_defaults::{class_defaults, find_class, property_names};
use rc_package::{ObjectRef, Package, Reader};
use std::collections::HashSet;

/// A script package that holds effects, read once.
pub struct EffectPackage {
    pub name: String,
    pkg: Package,
    data: Vec<u8>,
    names: HashSet<String>,
}

impl EffectPackage {
    /// `name` is the file name without `.u` (`cteffects`). `extra_names`
    /// are property names from other packages (`engine.u`), which help
    /// find where a class's defaults start; the package's own are always
    /// used.
    pub fn new(name: &str, data: Vec<u8>, extra_names: &[String]) -> Result<Self, String> {
        let pkg = Package::parse(&data).map_err(|e| e.to_string())?;
        let mut names: HashSet<String> = property_names(&pkg).into_iter().collect();
        names.extend(extra_names.iter().map(|n| n.to_ascii_lowercase()));
        Ok(EffectPackage {
            name: name.to_string(),
            pkg,
            data,
            names,
        })
    }

    /// Whether this package has a class by that name.
    pub fn has(&self, effect: &str) -> bool {
        find_class(&self.pkg, effect).is_some()
    }

    /// Every class in the package (all effects, and a few helpers).
    pub fn classes(&self) -> Vec<String> {
        self.pkg
            .exports
            .iter()
            .filter(|e| e.class_index == 0)
            .map(|e| self.pkg.name(e.object_name).to_string())
            .collect()
    }

    /// The name of a class's parent.
    pub fn parent(&self, class: &str) -> Option<String> {
        let e = find_class(&self.pkg, class)?;
        rc_package::class_defaults::parent_name(&self.pkg, e).map(str::to_string)
    }

    /// The class's own defaults as named values.
    pub fn class_values(&self, class: &str) -> Option<Values> {
        let e = find_class(&self.pkg, class)?;
        let is_property = |n: &str| self.names.contains(&n.to_ascii_lowercase());
        let d = class_defaults(&self.pkg, &self.data, e, &is_property)?;
        Some(tagged::values(&self.pkg, &self.name, &d.properties))
    }

    /// Read one effect. `Err` says why it couldn't be read.
    pub fn effect(&self, class: &str) -> Result<EffectDef, String> {
        let e = find_class(&self.pkg, class).ok_or_else(|| format!("no class {class}"))?;
        let is_property = |n: &str| self.names.contains(&n.to_ascii_lowercase());
        let d = class_defaults(&self.pkg, &self.data, e, &is_property)
            .ok_or_else(|| format!("{class}: its defaults couldn't be found"))?;
        let own = tagged::values(&self.pkg, &self.name, &d.properties);
        // The list's raw form: a count, then object references.
        let list = d
            .properties
            .iter()
            .find(|p| self.pkg.name(p.name).eq_ignore_ascii_case("Emitters"));
        let mut emitters = Vec::new();
        if let Some(rc_package::properties::Value::Raw(b)) = list.map(|p| &p.value) {
            let mut r = Reader::new(b);
            let n = r.compact_index().unwrap_or(0).max(0);
            for _ in 0..n {
                let Ok(o) = r.compact_index() else { break };
                let ObjectRef::Export(k) = ObjectRef::from_index(o) else {
                    continue;
                };
                let kind = self.pkg.export_class_name(&self.pkg.exports[k]).to_string();
                let name = self.pkg.name(self.pkg.exports[k].object_name).to_string();
                let Some(v) = tagged::read_object(&self.pkg, &self.name, &self.data, k) else {
                    return Err(format!("{class}: emitter {name} couldn't be read"));
                };
                emitters.push(emitter(&name, &kind, &v));
            }
        }
        let sound = match get(&own, "SpawnSound") {
            Some(Tagged::Object(p)) if !p.is_empty() => Some(p.clone()),
            _ => None,
        };
        let ambient_sound = match get(&own, "AmbientSound") {
            Some(Tagged::Object(p)) if !p.is_empty() => Some(p.clone()),
            _ => None,
        };
        Ok(EffectDef {
            name: self.pkg.name(e.object_name).to_string(),
            emitters,
            sound,
            ambient_sound,
            sound_occlusion: byte(&own, "SoundOcclusion").unwrap_or(0),
            life_span: float(&own, "LifeSpan").unwrap_or(0.0),
            // `Emitter`'s default bUnlit is on (Engine.u).
            lit: !boolean(&own, "bUnlit").unwrap_or(true),
        })
    }
}

fn range(v: Option<&Tagged>, default: Range) -> Range {
    match v {
        Some(Tagged::Struct(s)) => Range {
            min: float(s, "Min").unwrap_or(0.0),
            max: float(s, "Max").unwrap_or(0.0),
        },
        _ => default,
    }
}

fn range_vector(v: Option<&Tagged>, default: RangeVector) -> RangeVector {
    match v {
        Some(Tagged::Struct(s)) => {
            let zero = Range::fixed(0.0);
            RangeVector {
                x: range(get(s, "X"), zero),
                y: range(get(s, "Y"), zero),
                z: range(get(s, "Z"), zero),
            }
        }
        _ => default,
    }
}

fn vector(v: Option<&Tagged>) -> Option<[f32; 3]> {
    match v? {
        Tagged::Vector(x) => Some(*x),
        _ => None,
    }
}

fn byte(v: &Values, key: &str) -> Option<u8> {
    match get(v, key)? {
        Tagged::Byte(b) => Some(*b),
        Tagged::Int(i) => Some(*i as u8),
        _ => None,
    }
}

fn int(v: &Values, key: &str) -> Option<i32> {
    match get(v, key)? {
        Tagged::Int(i) => Some(*i),
        Tagged::Byte(b) => Some(*b as i32),
        _ => None,
    }
}

/// A stored colour (blue, green, red, alpha) as RGBA 0-1.
fn colour(b: [u8; 4]) -> [f32; 4] {
    [b[2], b[1], b[0], b[3]].map(|c| c as f32 / 255.0)
}

/// The engine's defaults for this kind of emitter (from `engine.u`).
fn defaults_for(kind: &str) -> EmitterDef {
    let mut d = EmitterDef::default();
    if kind.eq_ignore_ascii_case("MeshEmitter") {
        // MeshEmitter: StartSizeRange 1 (a scale of the model) and
        // UseMeshBlendMode on.
        d.start_size = RangeVector::fixed(1.0);
        d.mesh_blend = true;
    }
    d
}

/// One emitter from its named values.
pub fn emitter(name: &str, kind: &str, v: &Values) -> EmitterDef {
    let mut d = defaults_for(kind);
    d.name = name.to_string();
    d.kind = match kind.to_ascii_lowercase().as_str() {
        "spriteemitter" => EmitterKind::Sprite,
        "meshemitter" => EmitterKind::Mesh(match get(v, "StaticMesh") {
            Some(Tagged::Object(p)) if !p.is_empty() => Some(p.clone()),
            _ => None,
        }),
        "sparkemitter" => EmitterKind::Spark,
        other => EmitterKind::Other(other.to_string()),
    };
    if let Some(Tagged::Text(t)) = get(v, "Name") {
        if !t.is_empty() {
            d.name = format!("{name} ({t})");
        }
    }
    d.disabled = boolean(v, "Disabled").unwrap_or(false);
    d.trigger_disabled = boolean(v, "TriggerDisabled").unwrap_or(d.trigger_disabled);
    d.reset_on_trigger = boolean(v, "ResetOnTrigger").unwrap_or(d.reset_on_trigger);
    if let Some(n) = int(v, "MaxParticles") {
        d.max_particles = n.max(0) as u32;
    }
    d.respawn = boolean(v, "RespawnDeadParticles").unwrap_or(d.respawn);
    d.auto_rate = boolean(v, "AutomaticInitialSpawning").unwrap_or(d.auto_rate);
    d.initial_rate = float(v, "InitialParticlesPerSecond").unwrap_or(0.0);
    d.rate = float(v, "ParticlesPerSecond").unwrap_or(0.0);
    d.initial_delay = range(get(v, "InitialDelayRange"), d.initial_delay);
    d.lifetime = range(get(v, "LifetimeRange"), d.lifetime);
    d.start_offset = vector(get(v, "StartLocationOffset")).unwrap_or([0.0; 3]);
    d.start_range = range_vector(get(v, "StartLocationRange"), d.start_range);
    d.start_shape = match byte(v, "StartLocationShape").unwrap_or(0) {
        1 => StartShape::Sphere,
        2 => StartShape::Polar,
        _ => StartShape::Box,
    };
    d.sphere_radius = range(get(v, "SphereRadiusRange"), d.sphere_radius);
    d.start_velocity = range_vector(get(v, "StartVelocityRange"), d.start_velocity);
    d.radial_velocity = range(get(v, "StartVelocityRadialRange"), d.radial_velocity);
    d.velocity_from = match byte(v, "GetVelocityDirectionFrom").unwrap_or(0) {
        1 => VelocityFrom::StartPositionAndOwner,
        2 => VelocityFrom::OwnerAndStartPosition,
        3 => VelocityFrom::AddRadial,
        _ => VelocityFrom::None,
    };
    d.velocity_loss = range_vector(get(v, "VelocityLossRange"), d.velocity_loss);
    d.acceleration = vector(get(v, "Acceleration")).unwrap_or([0.0; 3]);
    d.start_size = range_vector(get(v, "StartSizeRange"), d.start_size);
    d.uniform_size = boolean(v, "UniformSize").unwrap_or(d.uniform_size);
    if boolean(v, "UseSizeScale").unwrap_or(false) {
        d.size_scale = list(v, "SizeScale")
            .filter_map(|s| Some((float(s, "RelativeTime")?, float(s, "RelativeSize")?)))
            .collect();
    }
    if boolean(v, "UseColorScale").unwrap_or(false) {
        d.colour_scale = list(v, "ColorScale")
            .filter_map(|s| match get(s, "Color") {
                Some(Tagged::Colour(c)) => {
                    Some((float(s, "RelativeTime").unwrap_or(0.0), colour(*c)))
                }
                _ => None,
            })
            .collect();
    }
    d.colour_multiplier = range_vector(get(v, "ColorMultiplierRange"), d.colour_multiplier);
    d.opacity = float(v, "Opacity").unwrap_or(d.opacity);
    if boolean(v, "FadeOut").unwrap_or(false) {
        d.fade_out = Some(float(v, "FadeOutStartTime").unwrap_or(0.0));
    }
    if boolean(v, "FadeIn").unwrap_or(false) {
        d.fade_in = Some(float(v, "FadeInEndTime").unwrap_or(0.0));
    }
    d.texture = match get(v, "Texture") {
        Some(Tagged::Object(p)) if !p.is_empty() => Some(p.clone()),
        _ => None,
    };
    if let Some(b) = byte(v, "DrawStyle") {
        d.draw_style = DrawStyle::from_byte(b);
    }
    d.frames_u = int(v, "TextureUSubdivisions").unwrap_or(0).max(0) as u32;
    d.frames_v = int(v, "TextureVSubdivisions").unwrap_or(0).max(0) as u32;
    d.random_frame = boolean(v, "UseRandomSubdivision").unwrap_or(false);
    d.frame_start = int(v, "SubdivisionStart").unwrap_or(0).max(0) as u32;
    d.frame_end = int(v, "SubdivisionEnd").unwrap_or(0).max(0) as u32;
    d.spin = boolean(v, "SpinParticles").unwrap_or(false);
    let spin = range_vector(get(v, "StartSpinRange"), RangeVector::fixed(0.0));
    d.start_spin = spin.x;
    let rate = range_vector(get(v, "SpinsPerSecondRange"), RangeVector::fixed(0.0));
    d.spins_per_second = rate.x;
    d.collides = boolean(v, "UseCollision").unwrap_or(false)
        || boolean(v, "UseCollisionPlanes").unwrap_or(false)
        || boolean(v, "UseMaxCollisions").unwrap_or(false);
    d.damping = range_vector(get(v, "DampingFactorRange"), RangeVector::fixed(1.0)).z;
    if let Some(b) = byte(v, "CoordinateSystem") {
        d.coordinate_system = CoordinateSystem::from_byte(b);
    }
    d.mesh_blend = boolean(v, "UseMeshBlendMode").unwrap_or(d.mesh_blend);
    d
}

fn list<'a>(v: &'a Values, key: &str) -> impl Iterator<Item = &'a Values> {
    let items: &[Tagged] = match get(v, key) {
        Some(Tagged::List(l)) => l,
        _ => &[],
    };
    items.iter().filter_map(|t| match t {
        Tagged::Struct(s) => Some(s),
        _ => None,
    })
}

/// The game's effect packages, found from its `GameData` folder: every
/// script package in `Properties` that has emitters (`cteffects.u`), with
/// `System/engine.u`'s property names to help find class defaults.
pub struct Library {
    pub packages: Vec<EffectPackage>,
}

impl Library {
    pub fn open(game_data: &std::path::Path) -> Result<Library, String> {
        let engine = std::fs::read(game_data.join("System").join("engine.u"))
            .ok()
            .and_then(|d| Package::parse(&d).ok())
            .map(|p| property_names(&p))
            .unwrap_or_default();
        let dir = game_data.join("Properties");
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .map_err(|e| format!("can't read {}: {e}", dir.display()))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("u")))
            .collect();
        files.sort();
        let mut packages = Vec::new();
        for f in files {
            let Ok(data) = std::fs::read(&f) else {
                continue;
            };
            // Only packages that hold emitters.
            let Ok(pkg) = Package::parse(&data) else {
                continue;
            };
            if !pkg
                .exports
                .iter()
                .any(|e| pkg.export_class_name(e).ends_with("Emitter"))
            {
                continue;
            }
            let name = f
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            packages.push(EffectPackage::new(&name, data, &engine)?);
        }
        if packages.is_empty() {
            return Err(format!("no effect packages in {}", dir.display()));
        }
        Ok(Library { packages })
    }

    /// Whether `class` is an effect class: one of the effect packages has
    /// it, and its parents lead to engine.u's `Emitter`. For finding the
    /// emitters a map places (geo_01a's `REP_Gunship_GroundDust`).
    pub fn is_emitter(&self, class: &str) -> bool {
        let mut c = class.to_string();
        for _ in 0..16 {
            if c.eq_ignore_ascii_case("Emitter") {
                return true;
            }
            match self
                .packages
                .iter()
                .find(|p| p.has(&c))
                .and_then(|p| p.parent(&c))
            {
                Some(parent) => c = parent,
                None => return false,
            }
        }
        false
    }

    /// Read an effect by class name from whichever package has it.
    pub fn effect(&self, class: &str) -> Result<EffectDef, String> {
        self.packages
            .iter()
            .find(|p| p.has(class))
            .ok_or_else(|| format!("no effect called {class}"))?
            .effect(class)
    }
}
