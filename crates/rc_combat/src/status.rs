//! Status effects: electrocution, burning and stuns. A character under one
//! takes damage each second for a while, and a computer-controlled one
//! stops what it is doing. Grenades leave them (the EMP grenade, the
//! thermal detonator, the flash grenade) and so do level scripts
//! (`ACTION_SpawnStatusEffect`, geo_05a's sparking droid racks).
//!
//! The rules are the original's script, read from the game files and
//! written here in our own words:
//!
//! - `Engine.StatusEffect` (engine.u): `CreateStatusEffects` reaches every
//!   character it can see within a radius of where it starts, except the
//!   one it starts from. Each gets `max time x (1 - distance / (radius -
//!   their collision radius))`, kept between 0 and the max, and the
//!   damage a second plus a random amount between -0.5 and +0.5.
//!   `AddStatusEffectTo` gives one character one: never to a character
//!   whose damage table ignores the effect's damage type, and a character
//!   has only one at a time (the same kind again adds its time to the one
//!   it has; another kind is ignored). It hurts `damage a second x time`
//!   while it lasts (only when the damage a second is above 0) and ends
//!   when its time is up.
//! - Who is in reach is the engine's native code (engine.dll,
//!   `AActor::execVisibleCollidingActors` and
//!   `FOctreeNode::ActorRadiusCheck`, decompiled for reference only): a
//!   character counts when the middle of its collision box is closer than
//!   the radius (strictly), and a line from the start to its middle
//!   meets no level geometry or mover. A radius of 0 reaches nobody.
//! - The kinds (ctgame.u): `StatusEffect_Electrocute` lasts half as long on
//!   clone commandos (the player and the squad); `StatusEffect_Immolate`
//!   lasts at most 5 s on a computer-controlled clone commando, which then
//!   puts itself out; `StatusEffect_Stun` adds no random damage and only
//!   reaches the player when they look towards it. Each stops a
//!   computer-controlled character (it stops firing and moving, and plays
//!   its animation for the time).
//! - `ACTION_SpawnStatusEffect` (gameplay.u): with an `ActorTag`, it starts
//!   from each actor with that tag (and gives that actor the full time too
//!   when `AffectTaggedActor` is set); without one, from the script's
//!   character, or from the script itself (a `ScriptedTrigger`'s spot).
//!
//! Each kind's numbers are a data file, `data/status_effects/*.toml`.

use crate::data::Table;
use crate::health::DamageKind;
use bevy::math::Vec3;
use bevy::prelude::{Component, Entity, Message};
use std::collections::BTreeMap;

/// One kind of status effect (a `StatusEffect` class in the original).
#[derive(Debug, Clone, PartialEq)]
pub struct StatusClass {
    /// The original's class name (`StatusEffect_Electrocute`).
    pub class: String,
    /// The class it is a kind of, if it is one of ours too: an EMP
    /// grenade's electrocution is a kind of electrocution.
    pub parent: Option<String>,
    /// The damage type it hurts with (`DamageType`). A character whose
    /// damage table gives this type x0 never gets the effect.
    pub damage_type: String,
    /// The kind of the damage, for sounds and the log.
    pub damage_kind: DamageKind,
    /// The particle effect on the character while it lasts
    /// (`EffectClass`). Not drawn yet (it needs the character's bones).
    pub effect: Option<String>,
    /// Adds -0.5 to +0.5 to the damage a second when it spreads from a
    /// point (the base class's `CreateStatusEffects`; the stun has its
    /// own without it).
    pub random_damage: bool,
    /// Reaches the player only when they look towards where it starts
    /// (the stun).
    pub player_must_face: bool,
    /// Its time on a clone commando is multiplied by this (the
    /// electrocution: 0.5).
    pub commando_time_scale: f32,
    /// Its longest time on a computer-controlled clone commando (the
    /// burning: 5 s, then it puts itself out).
    pub commando_ai_max_time: Option<f32>,
    /// What a computer-controlled character plays while it lasts.
    pub ai_anim: String,
}

impl StatusClass {
    pub fn from_text(text: &str) -> Result<StatusClass, String> {
        let mut t = Table::parse(text)?;
        let class = t.text("class")?.ok_or("`class` is missing")?;
        let damage_kind = match t.text("damage_kind")? {
            None => DamageKind::Other,
            Some(k) => DamageKind::from_name(&k)
                .ok_or_else(|| format!("damage_kind `{k}` is not a known kind"))?,
        };
        let s = StatusClass {
            class,
            parent: t.text("parent")?.filter(|p| !p.is_empty()),
            damage_type: t.text("damage_type")?.ok_or("`damage_type` is missing")?,
            damage_kind,
            effect: t.text("effect")?.filter(|p| !p.is_empty()),
            random_damage: t.boolean("random_damage")?.unwrap_or(true),
            player_must_face: t.boolean("player_must_face")?.unwrap_or(false),
            commando_time_scale: t.number("commando.time_scale")?.unwrap_or(1.0),
            commando_ai_max_time: t.number("commando.ai_max_time")?,
            ai_anim: t.text("ai.anim")?.unwrap_or_default(),
        };
        t.finish()?;
        Ok(s)
    }
}

/// Every status effect kind, by lower-case class name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatusClasses(pub BTreeMap<String, StatusClass>);

impl StatusClasses {
    pub fn get(&self, class: &str) -> Option<&StatusClass> {
        self.0.get(&class.to_ascii_lowercase())
    }

    /// Whether `class` is `ancestor` or a kind of it (UnrealScript's
    /// `IsA`).
    pub fn is_a(&self, class: &str, ancestor: &str) -> bool {
        let mut cur = Some(class.to_string());
        for _ in 0..16 {
            let Some(c) = cur else { return false };
            if c.eq_ignore_ascii_case(ancestor) {
                return true;
            }
            cur = self.get(&c).and_then(|s| s.parent.clone());
        }
        false
    }
}

/// A character under a status effect for `left` more seconds. While it
/// lasts it hurts `damage_per_sec` (already multiplied by the character's
/// damage table) each second; when `stops` is set, a computer-controlled
/// character can't act.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct StatusEffect {
    /// The kind (`StatusEffect_Electrocute`).
    pub class: String,
    pub left: f32,
    pub damage_per_sec: f32,
    pub kind: DamageKind,
    pub stops: bool,
    pub instigator: Option<Entity>,
    /// What a computer-controlled character plays while it lasts.
    pub anim: String,
    /// Damage built up since the last quarter-second hit (sent in steps so
    /// the log isn't a line a frame).
    pub pending: f32,
}

/// A clone commando (the original's `CloneCommando` class): the player and
/// the squad mates. Clone troopers aren't. Some status effects treat them
/// differently.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Commando;

/// What we know about a character a status effect could reach.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target<'a> {
    /// Controlled by the player (not stopped by it, and the stun needs
    /// them to look at it).
    pub player: bool,
    /// A clone commando: the player or a squad mate.
    pub commando: bool,
    /// Its damage table's multiplier for the effect's damage type.
    pub multiplier: f32,
    /// Takes no damage at all (`Invulnerable`, the original's god mode).
    pub invulnerable: bool,
    /// The status effect it has now, if any.
    pub existing: Option<&'a StatusEffect>,
}

/// What [`add_status`] decided.
#[derive(Debug, Clone, PartialEq)]
pub enum Added {
    /// Its damage table ignores the effect's damage type (or it can't be
    /// hurt at all).
    Immune,
    /// It already has another kind; nothing happens.
    HasAnother(String),
    /// It already has this kind: this much more time.
    MoreTime(f32),
    /// A new one.
    New(StatusEffect),
    /// The time came to nothing.
    NoTime,
}

/// The original's `AddStatusEffectTo` with the kind's `Activate` and
/// `IncreaseDuration`: what giving `target` this effect for `secs` at
/// `damage_per_sec` does.
pub fn add_status(
    classes: &StatusClasses,
    class: &StatusClass,
    target: Target,
    secs: f32,
    damage_per_sec: f32,
    instigator: Option<Entity>,
) -> Added {
    if target.invulnerable || target.multiplier == 0.0 {
        return Added::Immune;
    }
    let ai_commando_max = |t: f32| match class.commando_ai_max_time {
        Some(m) if target.commando && !target.player => t.min(m),
        _ => t,
    };
    if let Some(e) = target.existing {
        if !classes.is_a(&e.class, &class.class) {
            return Added::HasAnother(e.class.clone());
        }
        // `IncreaseDuration` adds the time as given (no commando half).
        // The burning's own clamps a computer-controlled commando's total.
        let total = ai_commando_max(e.left + secs);
        return Added::MoreTime(total - e.left);
    }
    let mut left = secs;
    if target.commando {
        left *= class.commando_time_scale;
    }
    let left = ai_commando_max(left);
    if left <= 0.0 {
        return Added::NoTime;
    }
    Added::New(StatusEffect {
        class: class.class.clone(),
        left,
        damage_per_sec: damage_per_sec * target.multiplier,
        kind: class.damage_kind,
        stops: !target.player,
        instigator,
        anim: class.ai_anim.clone(),
        pending: 0.0,
    })
}

/// How long a character `dist` from where an effect starts gets it: the
/// original's `1 - dist / (radius - character radius)`, kept between 0 and
/// 1, times the max. `dist` is to the character's middle.
pub fn status_time(max_time: f32, dist: f32, character_radius: f32, radius: f32) -> f32 {
    let f = 1.0 - dist / (radius - character_radius);
    if f.is_nan() {
        return 0.0;
    }
    max_time * f.clamp(0.0, 1.0)
}

/// Whether a character whose middle is `dist` away is in reach of an
/// effect with this radius (engine.dll's radius check: closer than the
/// radius, so a radius of 0 reaches nobody).
pub fn in_reach(dist: f32, radius: f32) -> bool {
    dist * dist < radius * radius
}

/// Where a status effect starts from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StatusFrom {
    /// A point (Bevy space): a script's spot, a grenade's blast.
    Point(Vec3),
    /// A character: it starts from their middle, and they themselves are
    /// left out unless `also_them` is set (then they get the full time).
    Character { entity: Entity, also_them: bool },
}

/// "Start this status effect here" (the original's `CreateStatusEffects`,
/// or `ACTION_SpawnStatusEffect`). Anyone can send it.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct SpawnStatusEffect {
    /// The kind's class name (`StatusEffect_Electrocute`).
    pub class: String,
    pub from: StatusFrom,
    /// Unreal units.
    pub radius: f32,
    /// Seconds for a character right at the start.
    pub max_time: f32,
    pub damage_per_sec: f32,
    pub instigator: Option<Entity>,
    /// Who sent it, for the log.
    pub by: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes() -> StatusClasses {
        let mut m = BTreeMap::new();
        for text in [
            "class = \"StatusEffect_Electrocute\"\ndamage_type = \"CTDamageElectricity\"\ndamage_kind = \"electrical\"\ncommando.time_scale = 0.5\nai.anim = \"ElectrocutionBreathe\"\n",
            "class = \"StatusEffect_ElectrocuteEMPGrenade\"\nparent = \"StatusEffect_Electrocute\"\ndamage_type = \"CTDamageElectricityEMPGrenade\"\ndamage_kind = \"electrical\"\ncommando.time_scale = 0.5\n",
            "class = \"StatusEffect_Immolate\"\ndamage_type = \"CTDamageFire\"\ndamage_kind = \"fire\"\ncommando.ai_max_time = 5\n",
        ] {
            let c = StatusClass::from_text(text).unwrap();
            m.insert(c.class.to_ascii_lowercase(), c);
        }
        StatusClasses(m)
    }

    fn target(existing: Option<&StatusEffect>) -> Target<'_> {
        Target {
            player: false,
            commando: false,
            multiplier: 1.0,
            invulnerable: false,
            existing,
        }
    }

    #[test]
    fn time_falls_off_with_distance_like_the_original() {
        // 256 uu radius, a 22 uu wide character 117 uu away: half.
        assert!((status_time(15.0, 117.0, 22.0, 256.0) - 7.5).abs() < 1e-4);
        assert_eq!(status_time(15.0, 0.0, 22.0, 256.0), 15.0);
        assert_eq!(status_time(15.0, 300.0, 22.0, 256.0), 0.0);
        // A radius smaller than the character: the sum is negative, so
        // the time is the full max (when it is in reach at all).
        assert_eq!(status_time(10.0, 5.0, 22.0, 10.0), 10.0);
        assert!(!in_reach(0.0, 0.0), "a radius of 0 reaches nobody");
        assert!(in_reach(255.9, 256.0) && !in_reach(256.0, 256.0));
    }

    #[test]
    fn one_effect_at_a_time() {
        let c = classes();
        let elec = c.get("StatusEffect_Electrocute").unwrap();
        let emp = c.get("StatusEffect_ElectrocuteEMPGrenade").unwrap();
        let fire = c.get("StatusEffect_Immolate").unwrap();
        let Added::New(e) = add_status(&c, elec, target(None), 10.0, 2.0, None) else {
            panic!()
        };
        assert!(e.stops && e.left == 10.0 && e.anim == "ElectrocutionBreathe");
        // Burning on an electrocuted character: ignored.
        assert_eq!(
            add_status(&c, fire, target(Some(&e)), 5.0, 1.0, None),
            Added::HasAnother("StatusEffect_Electrocute".into())
        );
        // The same kind again: more time.
        assert_eq!(
            add_status(&c, elec, target(Some(&e)), 4.0, 1.0, None),
            Added::MoreTime(4.0)
        );
        // An EMP electrocution is a kind of electrocution: an EMP on a
        // character already electrocuted by a script adds no time (the
        // one it has isn't an EMP one) ...
        assert!(matches!(
            add_status(&c, emp, target(Some(&e)), 4.0, 1.0, None),
            Added::HasAnother(_)
        ));
        // ... but a plain electrocution on an EMP-electrocuted one does.
        let Added::New(em) = add_status(&c, emp, target(None), 6.0, 50.0, None) else {
            panic!()
        };
        assert_eq!(
            add_status(&c, elec, target(Some(&em)), 1.0, 1.0, None),
            Added::MoreTime(1.0)
        );
    }

    #[test]
    fn commandos_and_immunity() {
        let c = classes();
        let elec = c.get("StatusEffect_Electrocute").unwrap();
        let fire = c.get("StatusEffect_Immolate").unwrap();
        let mut t = target(None);
        t.commando = true;
        t.player = true;
        let Added::New(e) = add_status(&c, elec, t, 10.0, 0.0, None) else {
            panic!()
        };
        assert_eq!(e.left, 5.0, "half as long on a commando");
        assert!(!e.stops, "the player keeps control");
        // A squad mate burns 5 s at most; the player as long as given.
        t.player = false;
        let Added::New(f) = add_status(&c, fire, t, 12.0, 15.0, None) else {
            panic!()
        };
        assert_eq!(f.left, 5.0);
        t.player = true;
        let Added::New(f) = add_status(&c, fire, t, 12.0, 15.0, None) else {
            panic!()
        };
        assert_eq!(f.left, 12.0);
        let mut d = target(None);
        d.multiplier = 0.0;
        assert_eq!(add_status(&c, fire, d, 12.0, 15.0, None), Added::Immune);
    }
}
