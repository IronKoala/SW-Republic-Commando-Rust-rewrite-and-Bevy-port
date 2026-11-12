//! Proximity mines (`ProximityMine`, CTMarkers.u): mines a squad mate sets
//! on top of an explosive barrel at a `MarkerSetProximityMine` squad
//! marker (geo_05c has eight). Once set and armed, a mine goes off when an
//! enemy stands near it, and blows up its barrel too.
//!
//! What this file reads, map first, then the class and its parents (the
//! names are the game's):
//!
//! - `PropToDestroy`: the barrel it sits on.
//! - `TriggerRadius` (512): how close an enemy must be.
//! - `AlertTimeLimit` (1.5): seconds an enemy must stay that close.
//! - `ActivateSound` (played when it starts watching) and `AlertSound` (the
//!   chirp each time it sees an enemy).
//! - `TeamIndex`: whose mine it is until someone arms it (the one who arms
//!   it makes it theirs).
//! - The four steps (`ActivationInfo`): `ChargeUnplaced` (waiting to be
//!   set: a see-through hologram), `ChargePlaced` (set, not armed),
//!   `ChargeArmed` (armed; `Duration` is the wait before it starts
//!   watching) and `ChargeExplode` (the effect, sound and blast). Each can
//!   send an event, play a sound, show an effect and change the mine's
//!   material.
//! - `StaticMesh`: the mine's model.
//!
//! How the game runs these values is in the class's own script (state code
//! in CTMarkers.u, readable as text in the package); `rc_scripts::mines`
//! follows it. This file only reads. Plain Rust, no Bevy.

use crate::actors::{self, LevelInfo};
use crate::class_look::{object_path, ClassLooks, Loaded};
use crate::scripts::{to_val, Val};
use rc_package::properties::{read_properties, Property, Value};
use rc_package::{Package, Reader};

/// One step of a mine (the game's `ActivationInfo`). Names are the
/// objects' names; paths are full (package first) for loading.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stage {
    pub event: Option<String>,
    /// An effect class, like `Explosion_Sparks`.
    pub effect: Option<String>,
    pub effect_offset: [f32; 3],
    pub sound_path: Option<Vec<String>>,
    /// The material the mine changes to, and which of its slots.
    pub material_path: Option<Vec<String>>,
    pub material_index: usize,
    /// Seconds (for `ChargeArmed`: until it starts watching).
    pub duration: f32,
    /// A blast on entering this step: damage at the middle, the radius
    /// (uu) and the damage type's class.
    pub damage: f32,
    pub damage_radius: f32,
    pub damage_type: Option<String>,
}

impl Stage {
    fn from_property(pkg: &Package, p: &Property) -> Stage {
        let Val::Struct(v) = to_val(pkg, p) else {
            return Stage::default();
        };
        let text = |k: &str| match v.get(k) {
            Some(Val::Text(t)) if !t.is_empty() && !t.eq_ignore_ascii_case("None") => {
                Some(t.clone())
            }
            _ => None,
        };
        let float = |k: &str| match v.get(k) {
            Some(Val::Float(f)) => *f,
            Some(Val::Int(i)) => *i as f32,
            _ => 0.0,
        };
        let mut s = Stage {
            event: text("Event"),
            effect: text("TransitionEffect"),
            effect_offset: match v.get("TransitionEffectOffset") {
                Some(Val::Vector(o)) => *o,
                _ => [0.0; 3],
            },
            sound_path: None,
            material_path: None,
            material_index: match v.get("MaterialIndex") {
                Some(Val::Int(i)) => (*i).max(0) as usize,
                _ => 0,
            },
            duration: float("Duration"),
            damage: float("Damage"),
            damage_radius: float("DamageRadius"),
            damage_type: text("MyDamageType"),
        };
        if let Value::Raw(b) = &p.value {
            let mut r = Reader::new(b);
            if let Ok(fields) = read_properties(pkg, &mut r) {
                let path = |key: &str| {
                    fields
                        .iter()
                        .find(|f| pkg.name(f.name).eq_ignore_ascii_case(key))
                        .and_then(|f| match f.value {
                            Value::Object(o) if o != 0 => Some(object_path(pkg, o)),
                            _ => None,
                        })
                };
                s.sound_path = path("TransitionSound");
                s.material_path = path("Material");
            }
        }
        s
    }
}

/// One proximity mine in the map.
#[derive(Debug, Clone)]
pub struct MineInfo {
    pub export: usize,
    pub name: String,
    pub class: String,
    /// Unreal units: where the mine is (its middle).
    pub location: [f32; 3],
    /// The barrel it sits on (actor name), if any.
    pub prop_to_destroy: Option<String>,
    /// Its squad marker (actor name), if any.
    pub squad_marker: Option<String>,
    pub team_index: i32,
    pub trigger_radius: f32,
    pub alert_time_limit: f32,
    pub activate_sound: Option<Vec<String>>,
    pub alert_sound: Option<Vec<String>>,
    pub unplaced: Stage,
    pub placed: Stage,
    pub armed: Stage,
    pub explode: Stage,
    /// The model, full path (`MarkerIcons.SetTrap.ProximityMine`).
    pub mesh_path: Option<Vec<String>>,
    /// Hidden in the map (`bHidden`).
    pub hidden: bool,
    /// Where and how it is drawn (for putting its model in place).
    pub placement: Option<actors::Placement>,
}

impl MineInfo {
    /// A point of its model (the model's own units) in the level.
    pub fn to_level(&self, v: [f32; 3]) -> [f32; 3] {
        match &self.placement {
            Some(p) => p.transform_point(v),
            None => [0, 1, 2].map(|k| v[k] + self.location[k]),
        }
    }
}

fn number(v: Option<Val>) -> Option<f32> {
    match v? {
        Val::Float(f) => Some(f),
        Val::Int(i) => Some(i as f32),
        Val::Byte(b) => Some(b as f32),
        _ => None,
    }
}

fn name(v: Option<Val>) -> Option<String> {
    match v? {
        Val::Text(t) if !t.is_empty() && !t.eq_ignore_ascii_case("None") => Some(t),
        _ => None,
    }
}

/// Every proximity mine in the level (any actor whose class is
/// `ProximityMine` or comes from it). `open` opens a script package by
/// name, as for `ClassLooks::for_actor`.
pub fn read(
    looks: &mut ClassLooks,
    pkg: &Package,
    data: &[u8],
    level: &LevelInfo,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> Vec<MineInfo> {
    let mut out = Vec::new();
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
        if !look.is_a("ProximityMine") {
            continue;
        }
        let Ok(props) = actors::read_actor_properties(pkg, data, a) else {
            continue;
        };
        let own = |key: &str| {
            props
                .iter()
                .find(|p| p.array_index == 0 && pkg.name(p.name).eq_ignore_ascii_case(key))
        };
        let value = |key: &str| -> Option<Val> {
            if let Some(p) = own(key) {
                return Some(to_val(pkg, p));
            }
            look.find(key, 0).map(|(l, p)| to_val(&l.0, p))
        };
        let path = |key: &str| -> Option<Vec<String>> {
            let (p, owner): (&Property, &Package) = match own(key) {
                Some(p) => (p, pkg),
                None => {
                    let (l, p) = look.find(key, 0)?;
                    (p, &l.0)
                }
            };
            match p.value {
                Value::Object(o) if o != 0 => Some(object_path(owner, o)),
                _ => None,
            }
        };
        // A step from the map (the whole struct), else from the class.
        let stage = |key: &str| {
            if let Some(p) = own(key) {
                return Stage::from_property(pkg, p);
            }
            look.find(key, 0)
                .map(|(l, p)| Stage::from_property(&l.0, p))
                .unwrap_or_default()
        };
        let mut placement = actors::read_placement(pkg, data, a).ok();
        if let Some(p) = placement.as_mut() {
            crate::class_look::apply_class_look(looks, pkg, data, p, open);
        }
        let location = placement.as_ref().map(|p| p.location).unwrap_or_default();
        out.push(MineInfo {
            export: a,
            name: pkg.name(pkg.exports[a].object_name).to_string(),
            class: class.clone(),
            location,
            prop_to_destroy: name(value("PropToDestroy")),
            squad_marker: name(value("SquadMarker")),
            team_index: number(value("TeamIndex")).unwrap_or(0.0) as i32,
            trigger_radius: number(value("TriggerRadius")).unwrap_or(0.0),
            alert_time_limit: number(value("AlertTimeLimit")).unwrap_or(0.0),
            activate_sound: path("ActivateSound"),
            alert_sound: path("AlertSound"),
            unplaced: stage("ChargeUnplaced"),
            placed: stage("ChargePlaced"),
            armed: stage("ChargeArmed"),
            explode: stage("ChargeExplode"),
            mesh_path: path("StaticMesh"),
            hidden: matches!(value("bHidden"), Some(Val::Bool(true))),
            placement,
        });
    }
    out
}
