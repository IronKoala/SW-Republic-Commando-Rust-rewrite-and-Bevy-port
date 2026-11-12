//! Pain volumes: pits, crushers and fires that hurt whoever is inside
//! (`rc_level::pain` reads them from the map).
//!
//! The rules are the original's `Engine.PhysicsVolume` and
//! `Engine.VolumeTimer` scripts (read as text from the install's
//! `engine.u`; rewritten here in our own words):
//!
//! - When a character starts touching a volume that is hurting
//!   (`bPainCausing`) and is one it hurts (`ShouldDamageOther`: the
//!   character's class is `ClassProximityType` or a subclass of it, or the
//!   volume names no class), it is hurt at once (`CausePainTo`), and the
//!   volume's timer starts if it isn't running.
//! - The timer goes off every 1.0 s (`VolumeTimer`). Each time, every
//!   character it hurts that is still touching is hurt again. When it goes
//!   off and nobody it hurts is touching, or the volume has stopped
//!   hurting, it stops.
//! - Each hurt is `int(DamagePerSec)` damage of the volume's damage class,
//!   from no one (`TakeDamage(int(DamagePerSec * 1), None, ...)`). A
//!   `DamagePerSec` below 0 heals instead, up to full health.
//! - An event named like the volume's `Tag` switches the hurting on or
//!   off (`PhysicsVolume.Trigger`) and starts the timer when it turns on.
//!
//! What the damage then does is the damage class's (its defaults in the
//! game's script packages): `DamageFallToDeath` goes past shields and
//! kills everyone outright, `Crushed` kills the player outright,
//! `Falling` goes past shields. That is `rc_combat::DamageRules`; the
//! character's damage table (`DamageTable`) multiplies it as for any hit.
//!
//! Not done: the original also tells the AI it is in pain
//! (`Controller.PawnIsInPain`), destroys actors marked
//! `bDestroyInPainVolume` (no class in the game sets it), skips characters
//! not walking in a zone with `bSoftKillZ` (no map sets it), and hurts
//! anything else that can be hurt (props, pickups) when the volume names no
//! class. Only characters are hurt here.
//!
//! Everything goes to the log with a `pain:` prefix.

use crate::PART;
use bevy::prelude::*;
use rc_combat::{Damage, DamageKind, DamageRules, DamageTable, Health, Hittable, WeaponLibrary};
use rc_game_core::units;
use rc_game_core::{GameClass, GameLog, GamePaths, Player, ScriptEvent};
use rc_level::pain::{Defaults, PainVolume};
use rc_package::class_library::ClassLibrary;
use std::collections::{HashMap, HashSet};

/// The `VolumeTimer`'s period.
const TIMER: f32 = 1.0;

/// One volume while the level is played.
#[derive(Debug, Clone)]
pub struct LiveVolume {
    pub volume: PainVolume,
    /// Seconds to the timer going off; `None` while it isn't running.
    pub timer: Option<f32>,
    /// Characters touching it now.
    touching: HashSet<Entity>,
}

/// A damage class's rules, read from the game's script packages.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DamageClass {
    pub kind: DamageKind,
    pub rules: DamageRules,
}

/// The level's pain volumes. Present while a level is played.
#[derive(Resource, Default)]
pub struct LevelPain {
    pub volumes: Vec<LiveVolume>,
    /// The damage classes the volumes use, by name (lower case).
    classes: HashMap<String, DamageClass>,
    /// Each character class's parents (itself first), lower case.
    chains: HashMap<String, Vec<String>>,
    /// Character classes already reported as unknown.
    unknown: HashSet<String>,
    /// Counts for the log and the self-tests.
    pub hurts: usize,
}

impl LevelPain {
    pub fn new(volumes: Vec<PainVolume>, lib: &ClassLibrary) -> LevelPain {
        let mut classes = HashMap::new();
        for v in &volumes {
            let key = v.damage_type.to_ascii_lowercase();
            classes
                .entry(key)
                .or_insert_with(|| damage_class(lib, &v.damage_type));
        }
        LevelPain {
            volumes: volumes
                .into_iter()
                .map(|volume| LiveVolume {
                    volume,
                    timer: None,
                    touching: HashSet::new(),
                })
                .collect(),
            classes,
            chains: HashMap::new(),
            unknown: HashSet::new(),
            hurts: 0,
        }
    }

    /// Built from parts: the volumes, the rules of the damage classes they
    /// use (by name) and the parents of the character classes (each list
    /// starting with the class itself). For tests and tools without the
    /// game's packages.
    pub fn from_parts(
        volumes: Vec<PainVolume>,
        classes: &[(&str, DamageClass)],
        chains: &[&[&str]],
    ) -> LevelPain {
        LevelPain {
            volumes: volumes
                .into_iter()
                .map(|volume| LiveVolume {
                    volume,
                    timer: None,
                    touching: HashSet::new(),
                })
                .collect(),
            classes: classes
                .iter()
                .map(|(n, c)| (n.to_ascii_lowercase(), *c))
                .collect(),
            chains: chains
                .iter()
                .filter_map(|c| {
                    let first = c.first()?.to_ascii_lowercase();
                    Some((first, c.iter().map(|x| x.to_ascii_lowercase()).collect()))
                })
                .collect(),
            unknown: HashSet::new(),
            hurts: 0,
        }
    }

    /// The rules of a damage class the level uses.
    pub fn damage_class(&self, name: &str) -> DamageClass {
        self.classes
            .get(&name.to_ascii_lowercase())
            .copied()
            .unwrap_or(DamageClass {
                kind: DamageKind::Other,
                rules: DamageRules::default(),
            })
    }

    /// Remember a character class's parents (from the game's classes).
    fn learn(&mut self, class: &str, lib: Option<&ClassLibrary>) {
        let key = class.to_ascii_lowercase();
        if self.chains.contains_key(&key) {
            return;
        }
        let chain: Vec<String> = lib
            .map(|l| l.chain(class))
            .unwrap_or_default()
            .into_iter()
            .map(|c| c.to_ascii_lowercase())
            .collect();
        self.chains.insert(key, chain);
    }
}

/// What a damage class does, from its defaults: its kind (by the family
/// it belongs to) and whether it goes past shields and kills outright.
pub fn damage_class(lib: &ClassLibrary, name: &str) -> DamageClass {
    let d = lib.defaults(name);
    let chain: Vec<String> = d.chain.iter().map(|c| c.to_ascii_lowercase()).collect();
    let is = |c: &str| chain.iter().any(|x| x == c);
    let kind = if is("damagefalltodeath") || is("falling") {
        DamageKind::Fall
    } else if is("ctdamagefire") {
        DamageKind::Fire
    } else if is("ctdamageenergy") {
        DamageKind::Blaster
    } else if is("ctdamageexplosion") {
        DamageKind::Explosion
    } else if is("ctdamagesonic") {
        DamageKind::Sonic
    } else if is("ctdamageelectrical") {
        DamageKind::Electrical
    } else if is("ctdamagemelee") {
        DamageKind::Melee
    } else {
        DamageKind::Other
    };
    DamageClass {
        kind,
        rules: DamageRules {
            bypass_shields: d.bool("bBypassesShields").unwrap_or(false),
            kills: d.bool("bKillsNotIncapacitates").unwrap_or(false),
            kills_player: d.bool("bKillsNotIncapacitatesPlayer").unwrap_or(false),
        },
    }
}

/// A volume class's own settings, for what the map leaves out.
pub fn volume_defaults(lib: &ClassLibrary, class: &str) -> Defaults {
    let d = lib.defaults(class);
    Defaults {
        pain_causing: d.bool("bPainCausing").unwrap_or(false),
        damage_per_sec: d.number("DamagePerSec").unwrap_or(0.0),
        damage_type: d.object("DamageType").map(str::to_string),
        proximity: d.object("ClassProximityType").map(str::to_string),
    }
}

/// Read the map's pain volumes while it loads, with the game's classes.
pub fn read(
    pkg: &rc_package::Package,
    data: &[u8],
    level: &rc_level::actors::LevelInfo,
    lib: &ClassLibrary,
    log: &GameLog,
) -> LevelPain {
    let is_volume = |c: &str| {
        lib.chain(c)
            .iter()
            .any(|x| x.eq_ignore_ascii_case("PhysicsVolume"))
    };
    let r = rc_level::pain::read(pkg, data, level, &is_volume, &|c: &str| {
        volume_defaults(lib, c)
    });
    for f in &r.failures {
        log.line(format!("{PART}: pain: volume not read: {f}"));
    }
    let pain = LevelPain::new(r.volumes, lib);
    for lv in &pain.volumes {
        let v = &lv.volume;
        let c = pain.damage_class(&v.damage_type);
        let (lo, hi) = v.bounds;
        log.line(format!(
            "{PART}: pain:   {} (tag {}) {} {} {} a second ({:?}{}{}{}), hurts {}, from ({:.0}, {:.0}, {:.0}) to ({:.0}, {:.0}, {:.0})",
            v.name,
            v.tag,
            if v.pain_causing { "hurts" } else { "is off (an event can switch it on):" },
            v.damage_per_sec,
            v.damage_type,
            c.kind,
            if c.rules.bypass_shields { ", past shields" } else { "" },
            if c.rules.kills { ", kills outright" } else { "" },
            if c.rules.kills_player && !c.rules.kills { ", kills the player outright" } else { "" },
            v.proximity.as_deref().map_or("anyone".to_string(), |p| format!("{p} and its subclasses")),
            lo[0], lo[1], lo[2], hi[0], hi[1], hi[2],
        ));
    }
    log.line(format!(
        "{PART}: pain: {} pain volumes ({} hurting from the start)",
        pain.volumes.len(),
        pain.volumes
            .iter()
            .filter(|v| v.volume.pain_causing)
            .count()
    ));
    pain
}

/// Does the volume hurt a character of this class (`ShouldDamageOther`)?
fn hurts(v: &PainVolume, chain: Option<&Vec<String>>) -> bool {
    match &v.proximity {
        None => true,
        Some(p) => {
            let p = p.to_ascii_lowercase();
            chain.is_some_and(|c| c.iter().any(|x| *x == p))
        }
    }
}

/// One character as the volumes see it.
struct Victim {
    entity: Entity,
    name: String,
    /// The middle of its collision cylinder, Unreal units.
    centre: [f32; 3],
    radius: f32,
    half_height: f32,
    class: Option<String>,
    table: Option<String>,
}

/// The game's classes, read once (they don't change between levels).
#[derive(Resource, Default)]
pub struct PainClasses(pub Option<std::sync::Arc<ClassLibrary>>);

/// Hurt the characters in pain volumes, once a second, as the original
/// does; switch volumes on and off by the level's events.
#[allow(clippy::too_many_arguments)]
pub fn run(
    time: Res<Time>,
    pain: Option<ResMut<LevelPain>>,
    classes: Res<PainClasses>,
    mut events: MessageReader<ScriptEvent>,
    characters: Query<
        (
            Entity,
            &Transform,
            &Hittable,
            Option<&GameClass>,
            Option<&DamageTable>,
            Option<&Name>,
            Has<Player>,
        ),
        With<Health>,
    >,
    mut healths: Query<&mut Health>,
    weapons: Option<Res<WeaponLibrary>>,
    mut damage: MessageWriter<Damage>,
    log: Res<GameLog>,
) {
    let Some(mut pain) = pain else {
        events.clear();
        return;
    };
    let pain = &mut *pain;
    let dt = time.delta_secs().min(0.25);
    // Events switch volumes on and off.
    for e in events.read() {
        for lv in &mut pain.volumes {
            if !lv.volume.tag.eq_ignore_ascii_case(&e.name) {
                continue;
            }
            lv.volume.pain_causing = !lv.volume.pain_causing;
            if lv.volume.pain_causing && lv.timer.is_none() {
                lv.timer = Some(TIMER);
            }
            log.line(format!(
                "{PART}: pain: {} switched {} by {} ({})",
                lv.volume.name,
                if lv.volume.pain_causing { "on" } else { "off" },
                e.name,
                e.by
            ));
        }
    }
    let victims: Vec<Victim> = characters
        .iter()
        .map(|(entity, t, hit, class, table, name, player)| {
            let feet = units::to_unreal(t.translation);
            Victim {
                entity,
                name: name.map_or_else(
                    || {
                        if player {
                            "the player".into()
                        } else {
                            format!("{entity:?}")
                        }
                    },
                    |n| n.to_string(),
                ),
                centre: [feet.x, feet.y, feet.z + hit.center_up],
                radius: hit.radius,
                half_height: hit.half_height,
                class: class.map(|c| c.0.clone()),
                table: table.map(|t| t.0.clone()),
            }
        })
        .collect();
    let lib = classes.0.clone();
    for v in &victims {
        if let Some(c) = &v.class {
            pain.learn(c, lib.as_deref());
        }
    }
    // Who each volume hurts now: (volume, victim index).
    let mut hurt_now: Vec<(usize, usize, &'static str)> = Vec::new();
    for (vi, lv) in pain.volumes.iter_mut().enumerate() {
        let mut now = HashSet::new();
        for (k, v) in victims.iter().enumerate() {
            if !lv.volume.touches(v.centre, v.radius, v.half_height) {
                continue;
            }
            now.insert(v.entity);
            if lv.touching.contains(&v.entity) {
                continue;
            }
            // Just started touching (`Touch`).
            let chain = v
                .class
                .as_ref()
                .and_then(|c| pain.chains.get(&c.to_ascii_lowercase()));
            if lv.volume.pain_causing && hurts(&lv.volume, chain) {
                hurt_now.push((vi, k, "entered"));
                if lv.timer.is_none() {
                    lv.timer = Some(TIMER);
                }
            }
        }
        lv.touching = now;
        // The timer (`VolumeTimer.Timer` -> `TimerPop`).
        if let Some(t) = lv.timer.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                if !lv.volume.pain_causing {
                    lv.timer = None;
                    continue;
                }
                let mut found = false;
                for (k, v) in victims.iter().enumerate() {
                    if !lv.touching.contains(&v.entity) {
                        continue;
                    }
                    let chain = v
                        .class
                        .as_ref()
                        .and_then(|c| pain.chains.get(&c.to_ascii_lowercase()));
                    if hurts(&lv.volume, chain) {
                        hurt_now.push((vi, k, "still inside"));
                        found = true;
                    }
                }
                lv.timer = if found { Some(*t + TIMER) } else { None };
            }
        }
    }
    // Characters with no class the volumes can check.
    for v in &victims {
        if v.class.is_none()
            && pain
                .volumes
                .iter()
                .any(|lv| lv.touching.contains(&v.entity))
        {
            let key = format!("{:?}", v.entity);
            if pain.unknown.insert(key) {
                log.line(format!(
                    "{PART}: pain: {} has no game class: only volumes that hurt anyone hurt it",
                    v.name
                ));
            }
        }
    }
    // `CausePainTo`.
    for (vi, k, why) in hurt_now {
        let v = &victims[k];
        let vol = &pain.volumes[vi].volume;
        let per_sec = vol.damage_per_sec;
        if per_sec > 0.0 {
            let c = pain.damage_class(&vol.damage_type);
            // `int(DamagePerSec * depth)`, depth 1.
            let base = per_sec.trunc();
            let mult = weapons.as_ref().map_or(1.0, |w| {
                w.damage_types
                    .multiplier(v.table.as_deref(), &vol.damage_type)
            });
            let amount = base * mult;
            log.line(format!(
                "{PART}: pain: {} ({why}) hurts {} for {amount} ({} {}{})",
                vol.name,
                v.name,
                vol.damage_type,
                base,
                if mult != 1.0 {
                    format!(" x {mult} by its damage table")
                } else {
                    String::new()
                }
            ));
            damage.write(Damage {
                target: v.entity,
                amount,
                kind: c.kind,
                rules: c.rules,
                instigator: None,
                point: units::to_bevy(rc_movement::Vec3::new(
                    vol.shape.location[0],
                    vol.shape.location[1],
                    vol.shape.location[2],
                )),
                direction: Vec3::ZERO,
            });
            pain.hurts += 1;
        } else if let Ok(mut h) = healths.get_mut(v.entity) {
            // A healing volume: up to full health.
            if h.current < h.max {
                h.current = (h.current - per_sec).min(h.max);
                log.line(format!(
                    "{PART}: pain: {} ({why}) heals {} to {}",
                    vol.name, v.name, h.current
                ));
            }
        }
    }
}

/// Read the game's classes the first time a level needs them.
pub fn classes(
    paths: &GamePaths,
    cache: &mut PainClasses,
    log: &GameLog,
) -> std::sync::Arc<ClassLibrary> {
    if let Some(l) = &cache.0 {
        return l.clone();
    }
    let (lib, missing) = crate::tracker::class_library(paths);
    for m in &missing {
        log.line(format!("{PART}: pain: class package {m}"));
    }
    let lib = std::sync::Arc::new(lib);
    cache.0 = Some(lib.clone());
    lib
}

pub fn forget(mut commands: Commands) {
    commands.remove_resource::<LevelPain>();
}
