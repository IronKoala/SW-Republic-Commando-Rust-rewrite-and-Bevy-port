//! Props that can be damaged and broken: explosive barrels, the clamps
//! and fuel dispensers of geo_05b, holo machines, consoles, rock cover,
//! bunker walls.
//!
//! In Republic Commando every `Prop` (and so every `KarmaProp`,
//! `StaticProp`, `ExplosiveBarrel`, ...) can carry these values, in the map
//! or as its class's defaults (`Properties/StaticPropProperties.u`,
//! `Properties/MarkerProperties.u`, `CTMarkers.u`, ...):
//!
//! - `Health`: how much damage it takes to break. A prop that starts with
//!   no health (0 or less) can't be hurt at all; only a level script can
//!   remove it.
//! - `DamagedRatio`: when health falls to this share of the starting
//!   health, the prop goes to its "damaged" look (`DamagedState`) first. 0
//!   skips that step.
//! - `DamagedState` and `DestroyedState`: what happens on entering each
//!   step (`Transition`): an event to send, an effect and a sound, a model
//!   or material to switch to, whether collision goes off, and how long the
//!   prop stays before it disappears.
//! - `ExplosionDamage`, `ExplosionRadius`, `ExplosionDamageType`: a prop
//!   with both a damage and a radius explodes a moment after it breaks,
//!   hurting everything around it (barrels).
//! - `MyDamageVariance`: its damage table (how much each damage type
//!   counts), like a character's.
//! - Its own `Event` is sent when it breaks; if it has none,
//!   `DestroyedState.Event` is.
//!
//! This is our description of how the game uses those values (read from
//! the property names, the maps and how the original plays); the rules
//! themselves are run by `rc_scripts::Director`. This file only reads the
//! values: map first, then the class and its parents. Plain Rust, no Bevy.

use crate::actors::{self, LevelInfo};
use crate::class_look::{ClassLooks, Loaded};
use crate::scripts::{to_val, Val, Vals};
use rc_package::Package;

/// One step a prop goes through (`TransitionInfo` in the game's terms).
/// Names are the objects' names (`Explosion_Barrel_GEO`,
/// `exp_GEN_barrel_01`, `GEO_EplBarrel_Dest`); `None` when not set.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transition {
    pub event: Option<String>,
    pub effect: Option<String>,
    pub sound: Option<String>,
    pub mesh: Option<String>,
    pub material: Option<String>,
    pub disable_collision: bool,
    /// Seconds before it disappears after entering this step (0: stays),
    /// plus up to `life_span_max` more at random.
    pub life_span: f32,
    pub life_span_max: f32,
}

impl Transition {
    /// Whether the prop has something to show after this step (a model or
    /// a material); if not, it disappears at once.
    pub fn keeps_a_look(&self) -> bool {
        self.mesh.is_some() || self.material.is_some()
    }

    fn from_vals(v: &Vals) -> Transition {
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
        Transition {
            event: text("Event"),
            effect: text("TransitionEffect"),
            sound: text("TransitionSound"),
            mesh: text("Mesh"),
            material: text("Material"),
            disable_collision: matches!(v.get("bDisableCollision"), Some(Val::Bool(true))),
            life_span: float("StateLifeSpan"),
            life_span_max: float("StateLifeSpanMax"),
        }
    }
}

/// What a prop does when it explodes.
#[derive(Debug, Clone, PartialEq)]
pub struct Explosion {
    pub damage: f32,
    /// Unreal units.
    pub radius: f32,
    /// The damage type's class name, like `CTDamageExplosionGeoBarrel`.
    pub damage_type: String,
    /// A status effect it leaves (`StatusEffect_Immolate`), for how long at
    /// most, and its damage a second. Not carried out yet.
    pub status: Option<String>,
    pub status_secs: f32,
    pub status_damage_per_sec: f32,
}

/// A prop that can be hurt and broken.
#[derive(Debug, Clone, PartialEq)]
pub struct Breakable {
    pub export: usize,
    pub name: String,
    pub class: String,
    /// Its `Tag` (the class name if the map gives none).
    pub tag: String,
    /// Unreal units.
    pub location: [f32; 3],
    pub health: f32,
    pub damaged_ratio: f32,
    /// The actor's own `Event`, sent when it breaks (before
    /// `destroyed.event`).
    pub event: Option<String>,
    pub damaged: Transition,
    pub destroyed: Transition,
    pub explosion: Option<Explosion>,
    /// `MyDamageVariance`: a `DV_` table name.
    pub damage_table: Option<String>,
    /// Hidden in the map (not drawn, not solid).
    pub hidden: bool,
    /// An `AnimProp` (an animated prop): it keeps its look in the damaged
    /// step even without a damaged model.
    pub anim_prop: bool,
}

impl Breakable {
    /// The event the game sends when it breaks.
    pub fn broken_event(&self) -> Option<&str> {
        self.event.as_deref().or(self.destroyed.event.as_deref())
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

/// Every prop in the level that starts with health (so weapons can break
/// it). `open` opens a script package by name, as for
/// `ClassLooks::for_actor`.
pub fn read(
    looks: &mut ClassLooks,
    pkg: &Package,
    data: &[u8],
    level: &LevelInfo,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> Vec<Breakable> {
    let mut out = Vec::new();
    for &a in level.actors.iter().flatten() {
        let class = pkg.export_class_name(&pkg.exports[a]).to_string();
        // Most of a map is these; none of them are props.
        if matches!(
            class.as_str(),
            "StaticMeshActor" | "PathNode" | "CoverPoint" | "Light" | "Brush"
        ) {
            continue;
        }
        let Some(look) = looks.for_actor(pkg, a, open) else {
            continue;
        };
        if !look.is_a("Prop") {
            continue;
        }
        let Ok(props) = actors::read_actor_properties(pkg, data, a) else {
            continue;
        };
        // A value from the map, else the nearest class's default.
        let value = |key: &str| -> Option<Val> {
            if let Some(p) = props
                .iter()
                .find(|p| p.array_index == 0 && pkg.name(p.name).eq_ignore_ascii_case(key))
            {
                return Some(to_val(pkg, p));
            }
            look.find(key, 0).map(|(l, p)| to_val(&l.0, p))
        };
        let health = number(value("Health")).unwrap_or(0.0);
        if health <= 0.0 {
            continue;
        }
        let transition = |key: &str| match value(key) {
            Some(Val::Struct(v)) => Transition::from_vals(&v),
            _ => Transition::default(),
        };
        let explosion = {
            let damage = number(value("ExplosionDamage")).unwrap_or(0.0);
            let radius = number(value("ExplosionRadius")).unwrap_or(0.0);
            let status = name(value("ExplosionStatusEffect"));
            (damage > 0.0 && radius > 0.0 || status.is_some()).then(|| Explosion {
                damage,
                radius,
                damage_type: name(value("ExplosionDamageType"))
                    .unwrap_or_else(|| "CTDamageExplosion".into()),
                status,
                status_secs: number(value("StatusEffectMaxDuration")).unwrap_or(0.0),
                status_damage_per_sec: number(value("StatusEffectDamagePerSec")).unwrap_or(0.0),
            })
        };
        let placement = actors::read_placement(pkg, data, a).ok();
        out.push(Breakable {
            export: a,
            name: pkg.name(pkg.exports[a].object_name).to_string(),
            tag: name(value("Tag")).unwrap_or_else(|| class.clone()),
            class,
            location: placement.as_ref().map(|p| p.location).unwrap_or_default(),
            health,
            damaged_ratio: number(value("DamagedRatio")).unwrap_or(0.0),
            event: name(value("Event")),
            damaged: transition("DamagedState"),
            destroyed: transition("DestroyedState"),
            explosion,
            damage_table: name(value("MyDamageVariance")),
            hidden: matches!(value("bHidden"), Some(Val::Bool(true))),
            anim_prop: look.is_a("AnimProp"),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_reads_names_and_skips_none() {
        let mut v = Vals::new();
        v.insert("Event".into(), Val::Text("None".into()));
        v.insert(
            "TransitionEffect".into(),
            Val::Text("Explosion_Barrel_GEO".into()),
        );
        v.insert("Mesh".into(), Val::Text("GEO_EplBarrel_Dest".into()));
        v.insert("StateLifeSpan".into(), Val::Float(5.0));
        v.insert("bDisableCollision".into(), Val::Bool(true));
        let t = Transition::from_vals(&v);
        assert_eq!(t.event, None);
        assert_eq!(t.effect.as_deref(), Some("Explosion_Barrel_GEO"));
        assert!(t.keeps_a_look() && t.disable_collision);
        assert_eq!(t.life_span, 5.0);
        assert!(!Transition::default().keeps_a_look());
    }

    #[test]
    fn own_event_comes_first() {
        let mut b = Breakable {
            export: 1,
            name: "Prop1".into(),
            class: "Prop".into(),
            tag: "Prop".into(),
            location: [0.0; 3],
            health: 10.0,
            damaged_ratio: 0.0,
            event: None,
            damaged: Transition::default(),
            destroyed: Transition {
                event: Some("FromState".into()),
                ..Transition::default()
            },
            explosion: None,
            damage_table: None,
            hidden: false,
            anim_prop: false,
        };
        assert_eq!(b.broken_event(), Some("FromState"));
        b.event = Some("Own".into());
        assert_eq!(b.broken_event(), Some("Own"));
    }
}
