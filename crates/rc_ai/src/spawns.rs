//! Where enemies come from: the map's spawners ("pawn factories"), the
//! spawn points they use, and the walk-in triggers that switch them on.
//!
//! How the original works, from the property names, the class defaults and
//! the script source in CTGame.u (read only; described in our own words):
//! - A `PawnFactory` makes characters of one class (`Prototype`, for
//!   example `Properties.BattleDroidTM`). It starts switched off. An event
//!   whose name is the factory's `Tag` switches it on, and the same event
//!   again switches it off. It only switches on if it has somewhere to
//!   put characters.
//! - `SpawnMode`: 0 puts each new character at the next `SpawnPoint`
//!   sharing its `Tag` in turn, 1 at a random one, 2 (the class default)
//!   at the factory itself, 3 at the spawn point nearest the player.
//! - While on, it makes one straight away and then tries again every
//!   `SpawnInterval` seconds (a random time up to `SpawnIntervalMax` when
//!   that is larger). It makes one if fewer than `MaxAlive` of its own are
//!   alive (class default 1), it has some of its `Capacity` left (class
//!   default 2) or `bGenerateForever` is set, fewer than `MaxInWorld` of
//!   that class are alive anywhere (when set), and the player is between
//!   `MinPlayerRange` and `MaxPlayerRange` (when set).
//! - It sends `SpawnEvent` each time it makes one, and `ExhaustedEvent`
//!   (then switches off) once its capacity is used up and everything it
//!   made is dead.
//! - What it makes gets the factory's settings when it puts them at the
//!   factory, otherwise the spawn point's: a tag (`PawnTagFromFactory` /
//!   `PawnTag`) that scripts name it by, an event sent when it dies
//!   (`PawnEventFromFactory` / `PawnEvent`), an event sent the first time
//!   it is hurt (`FirstDamagedEventFromFactory` / `FirstDamagedEvent`),
//!   its first behaviour (`StartStateFromFactory` / `StartState`; 6 is
//!   "stationary attack", hold a position and shoot), its team, and the
//!   level script it follows (`AIScriptFromFactory` / `AIScriptTag`, a
//!   `ScriptedSequence`'s tag).
//! - A `Trigger` actor fires its `Event` when the player walks within its
//!   `CollisionRadius`; many spawners are switched on that way, the rest
//!   by level scripts. In the game the level scripts part runs both; the
//!   headless tools (no scripts) use the walk-in triggers directly.
//!
//! Droid dispensers (`BDDispenser`, `SBDDispenser`, `DroidekaDispenser`)
//! are pawn factories too (CTGame.u `DroidDispenser extends PawnFactory`)
//! with their own rules; see [`Dispenser`] and `crate::battle`.

use rc_level::actors;
use rc_movement::Vec3;
use rc_package::properties::{self, Property, Value};
use rc_package::Package;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum StartState {
    #[default]
    Idle,
    Wander,
    Perched,
    PerchedCeiling,
    Shutdown,
    Asleep,
    StationaryAttack,
    Other(u8),
}

impl StartState {
    fn from_byte(b: u8) -> Self {
        // Order of Pawn.EStartState in Engine.u.
        match b {
            0 => StartState::Idle,
            1 => StartState::Wander,
            2 => StartState::Perched,
            3 => StartState::PerchedCeiling,
            4 => StartState::Shutdown,
            5 => StartState::Asleep,
            6 => StartState::StationaryAttack,
            o => StartState::Other(o),
        }
    }
}

/// What a factory gives each character it makes: from the factory itself
/// when it puts them at the factory, otherwise from the spawn point.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Offspring {
    /// The name scripts use for it (`PawnTag...`).
    pub tag: Option<String>,
    /// Sent when it dies (`PawnEvent...`).
    pub event: Option<String>,
    /// Sent the first time it is hurt (`FirstDamagedEvent...`).
    pub first_damaged_event: Option<String>,
    /// Its first behaviour.
    pub start_state: StartState,
    pub team: Option<i32>,
    /// The level script it runs: the tag of a `ScriptedSequence`
    /// (`AIScriptFromFactory` / `AIScriptTag`).
    pub script: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SpawnSpot {
    pub location: Vec3,
    /// Facing, radians (Unreal convention).
    pub yaw: f32,
    /// What characters made here get.
    pub offspring: Offspring,
}

#[derive(Debug, Clone)]
pub struct Factory {
    pub name: String,
    pub tag: String,
    /// Class name of what it makes, e.g. `BattleDroidTM`.
    pub prototype: String,
    pub location: Vec3,
    pub yaw: f32,
    pub spots: Vec<SpawnSpot>,
    pub capacity: u32,
    pub max_alive: u32,
    pub generate_forever: bool,
    /// 0 spawn points in turn, 1 random spawn point, 2 at the factory,
    /// 3 the spawn point nearest the player.
    pub spawn_mode: u8,
    pub spawn_interval: f32,
    /// When larger than `spawn_interval`, the wait is random between them.
    pub spawn_interval_max: f32,
    /// At most this many of its class alive in the whole level (0: no
    /// limit).
    pub max_in_world: f32,
    /// Only makes characters while the player is this far away, Unreal
    /// units (no limit unless `min_player_range < max_player_range`).
    pub min_player_range: f32,
    pub max_player_range: f32,
    /// Only makes characters where the player can't see (not checked yet).
    pub covert: bool,
    /// Sent each time it makes one.
    pub spawn_event: Option<String>,
    /// Sent once its capacity is used and all it made are dead.
    pub exhausted_event: Option<String>,
    /// What it gives characters it puts at the factory itself.
    pub offspring: Offspring,
    pub start_state: StartState,
    pub team: Option<i32>,
    /// Walk-in triggers that switch it on: (centre, radius).
    pub triggers: Vec<(Vec3, f32)>,
    /// `bStartTimerAfterDeath`: once it has `MaxAlive` out, it waits for
    /// one of them to die before it starts its timer again. Only droid
    /// dispensers follow it so far (their defaults turn it on).
    pub start_timer_after_death: bool,
    /// A droid dispenser's own rules and looks; `None` for a plain pawn
    /// factory.
    pub dispenser: Option<Dispenser>,
}

impl Default for Factory {
    /// The class defaults (CTGame.u `Factory`), with no spawn points.
    fn default() -> Self {
        Factory {
            name: String::new(),
            tag: String::new(),
            prototype: String::new(),
            location: Vec3::ZERO,
            yaw: 0.0,
            spots: Vec::new(),
            capacity: 2,
            max_alive: 1,
            generate_forever: false,
            spawn_mode: 2,
            spawn_interval: 1.0,
            spawn_interval_max: 0.0,
            max_in_world: 0.0,
            min_player_range: 0.0,
            max_player_range: 0.0,
            covert: false,
            spawn_event: None,
            exhausted_event: None,
            offspring: Offspring::default(),
            start_state: StartState::Idle,
            team: None,
            triggers: Vec::new(),
            start_timer_after_death: false,
            dispenser: None,
        }
    }
}

impl Factory {
    /// Whether it puts characters at the factory itself rather than at
    /// spawn points.
    pub fn at_factory(&self) -> bool {
        self.spawn_mode == 2
    }

    /// Whether switching it on does anything: the original refuses when
    /// it has no spawn points and doesn't spawn at the factory.
    pub fn can_spawn(&self) -> bool {
        self.at_factory() || !self.spots.is_empty()
    }

    /// Where the n-th character goes in turn (spawn mode 0), or at the
    /// factory.
    pub fn spot(&self, n: usize) -> SpawnSpot {
        if self.spots.is_empty() || self.at_factory() {
            SpawnSpot {
                location: self.location,
                yaw: self.yaw,
                offspring: self.offspring.clone(),
            }
        } else {
            self.spots[n % self.spots.len()].clone()
        }
    }
}

/// One step of a dispenser's looks (CTGame.u `DroidDispenser.TransitionInfo`):
/// what it does when it warms up, comes on, is hurt, shuts down or is
/// destroyed (`MakeTransition`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transition {
    /// The effect (an `Emitter` class, by name: `Dispenser_Explosion`)
    /// put at the dispenser's centre plus `effect_offset`.
    pub effect: Option<String>,
    pub effect_offset: Vec3,
    /// A sound to play, dotted path.
    pub sound: Option<String>,
    /// Its skin becomes this material (dotted path:
    /// `DispensersTextures.BDDispenser.BDDispenser_On`).
    pub material: Option<String>,
    /// An event sent into the level.
    pub event: Option<String>,
    /// An animation it plays (`Destroyed`), once or looping.
    pub anim: Option<String>,
    pub anim_loop: bool,
}

impl Transition {
    fn new(effect: Option<&str>, z: f32, material: Option<&str>, anim: Option<&str>) -> Self {
        Transition {
            effect: effect.map(str::to_string),
            effect_offset: Vec3::new(0.0, 0.0, z),
            material: material.map(str::to_string),
            anim: anim.map(str::to_string),
            ..Default::default()
        }
    }
}

/// A droid dispenser's own settings: the class defaults from the game
/// files, with what the map changed on top.
///
/// Class defaults, as read with `rc_probe --classes`:
/// - CTGame.u `DroidDispenser`: `MaxHealth` 5000, `NumDispensers` 1,
///   `SpawnInterval` 3, `SpawnMode` 0, `bCheckExhausted`;
/// - Properties.u `DroidDispenserDefaults`: `bGenerateForever`,
///   `bStartTimerAfterDeath`, `MyDamageVariance` `DV_DroidDispenser`,
///   slightly hurt effect `Dispenser_LightDamage`, shut down effect
///   `Dispenser_Explosion`;
/// - CTGame.u `BDDispenser`: `NumDispensers` 3, `NumInitDroids` 3,
///   `IntroAnim` `entranceSky`, mesh `Dispensers.BDDispenser`, collision
///   170 x 125; Properties.u `BDDispenserDefaults`: `WarmUpTime` 1,
///   `Prototype` `CTCharacters.BattleDroid`, `Capacity` 3, and its skins
///   and effects per step (below);
/// - CTGame.u `SBDDispenser`: `NumDispensers` 4, mesh
///   `Dispensers.SBDDispenser`, collision 200 x 125; Properties.u
///   `SBDDispenserDefaults`: `WarmUpTime` 1, `Prototype`
///   `CTCharacters.SuperBattleDroid`, `Capacity` 4, collision radius 219;
/// - CTGame.u `DroidekaDispenser`: `WarmUpTime` 2, mesh
///   `Dispensers.DroidekaDispenser`. Neither it nor its "Defaults" class
///   names what it makes, so droideka dispensers make nothing yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Dispenser {
    /// `BDDispenser`, `SBDDispenser` or `DroidekaDispenser`.
    pub class: String,
    /// Its model in `GameData/Animations/Dispensers.ukx`.
    pub mesh: String,
    pub max_health: f32,
    /// Seconds between switching on and making droids (`WarmUpTime`).
    pub warm_up_time: f32,
    /// How many droid slots its model has (`NumDispensers`), each with its
    /// own bone (`droidAttach1`..) and animation (`Dispense1`..).
    pub num_dispensers: u32,
    /// How many it makes at the start before `MaxAlive` counts
    /// (`NumInitDroids`).
    pub num_init_droids: u32,
    /// The animation it plays when switched on (`IntroAnim`): it drops in
    /// from the sky (`entranceSky`), is pushed in (`SlaversPushDispencer`),
    /// or none.
    pub intro_anim: Option<String>,
    /// After its intro it waits for this event before coming on
    /// (`OnlineTag`).
    pub online_tag: Option<String>,
    pub collision_radius: f32,
    pub collision_height: f32,
    /// Not drawn (`bHidden`).
    pub hidden: bool,
    /// It rides this mover (`AttachTag`). Not followed yet.
    pub attach_tag: Option<String>,
    /// Its damage table (`MyDamageVariance`).
    pub damage_table: String,
    /// A sound each time it makes a droid (`DispenseSound`).
    pub dispense_sound: Option<String>,
    pub warm_up: Transition,
    pub online: Transition,
    /// Health at or below 66% (`SlightlyDamagedInfo`).
    pub slightly_damaged: Transition,
    /// Health at or below 33% (`HeavilyDamagedInfo`).
    pub heavily_damaged: Transition,
    /// Nothing left to make (`ShutdownInfo`).
    pub shutdown: Transition,
    /// Destroyed (`DamagedInfo`).
    pub damaged: Transition,
}

/// The kinds of dispenser the maps place.
pub const DISPENSER_CLASSES: [&str; 3] = ["BDDispenser", "SBDDispenser", "DroidekaDispenser"];

impl Dispenser {
    /// The class defaults of `class` (one of [`DISPENSER_CLASSES`]), and
    /// the factory values its classes change: (dispenser, prototype,
    /// capacity, collision radius).
    pub fn class_defaults(class: &str) -> Option<(Dispenser, &'static str, u32)> {
        let tex = |skin: &str| format!("DispensersTextures.{class}.{class}_{skin}");
        let tex = |s: &str| Some(tex(s));
        let (num, init, intro, mesh_radius, warm, proto, capacity) = match class {
            "BDDispenser" => (3, 3, Some("entranceSky"), 170.0, 1.0, "BattleDroid", 3),
            // SBDDispenserDefaults makes the radius 219 (SBDDispenser: 200).
            "SBDDispenser" => (4, 0, None, 219.0, 1.0, "SuperBattleDroid", 4),
            "DroidekaDispenser" => (1, 0, None, 22.0, 2.0, "", 2),
            _ => return None,
        };
        let full = class != "DroidekaDispenser";
        let mat = |s: &str| if full { tex(s) } else { None };
        let d = Dispenser {
            class: class.to_string(),
            mesh: class.to_string(),
            max_health: 5000.0,
            warm_up_time: warm,
            num_dispensers: num,
            num_init_droids: init,
            intro_anim: intro.map(str::to_string),
            online_tag: None,
            collision_radius: mesh_radius,
            collision_height: if full { 125.0 } else { 22.0 },
            hidden: false,
            attach_tag: None,
            damage_table: "DV_DroidDispenser".into(),
            dispense_sound: None,
            warm_up: Transition::new(None, 0.0, mat("Warm").as_deref(), None),
            online: Transition::new(None, 0.0, mat("On").as_deref(), None),
            slightly_damaged: Transition::new(Some("Dispenser_LightDamage"), 0.0, None, None),
            heavily_damaged: if full {
                Transition::new(Some("Dispenser_Damage"), 50.0, None, None)
            } else {
                // DroidDispenserDefaults (the BD and SBD ones change it).
                Transition::new(Some("Fire_Lrg"), 0.0, None, None)
            },
            shutdown: Transition::new(
                Some("Dispenser_Explosion"),
                0.0,
                mat("Off").as_deref(),
                None,
            ),
            damaged: if full {
                Transition::new(
                    Some("Dispenser_Explosion"),
                    -50.0,
                    Some(&tex("Dest").unwrap()),
                    Some("Destroyed"),
                )
            } else {
                Transition::default()
            },
        };
        Some((d, proto, capacity))
    }
}

fn vec3(v: &Value) -> Option<Vec3> {
    match v {
        Value::Raw(b) if b.len() == 12 => {
            let f = |i: usize| f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
            Some(Vec3::new(f(0), f(1), f(2)))
        }
        _ => None,
    }
}

fn yaw(v: Option<&Value>) -> f32 {
    match v {
        Some(Value::Raw(b)) if b.len() == 12 => {
            i32::from_le_bytes(b[4..8].try_into().unwrap()) as f32 / 65536.0 * std::f32::consts::TAU
        }
        _ => 0.0,
    }
}

struct Actor {
    class: String,
    name: String,
    props: Vec<Property>,
}

/// Read every pawn factory in a map, with its spawn points and triggers.
pub fn read_factories(pkg: &Package, data: &[u8]) -> Result<Vec<Factory>, String> {
    let level = actors::read_level(pkg, data).map_err(|e| format!("level: {e}"))?;
    let mut list = Vec::new();
    for &a in level.actors.iter().flatten() {
        let e = &pkg.exports[a];
        let class = pkg.export_class_name(e).to_string();
        if !matches!(class.as_str(), "PawnFactory" | "SpawnPoint" | "Trigger")
            && !DISPENSER_CLASSES.contains(&class.as_str())
        {
            continue;
        }
        if let Ok(props) = actors::read_actor_properties(pkg, data, a) {
            list.push(Actor {
                class,
                name: pkg.name(e.object_name).to_string(),
                props,
            });
        }
    }
    let get = |a: &Actor, n: &str| properties::find(pkg, &a.props, n).map(|p| p.value.clone());
    let name_of = |a: &Actor, n: &str| match get(a, n) {
        Some(Value::Name(i)) => Some(pkg.name(i).to_string()),
        _ => None,
    };
    let int = |a: &Actor, n: &str| match get(a, n) {
        Some(Value::Int(i)) => Some(i),
        _ => None,
    };
    let float = |a: &Actor, n: &str| match get(a, n) {
        Some(Value::Float(x)) => Some(x),
        _ => None,
    };
    // A name property, with `None` (the empty name) as no name.
    let label = |a: &Actor, n: &str| name_of(a, n).filter(|s| !s.eq_ignore_ascii_case("None"));
    let state = |a: &Actor, n: &str| match get(a, n) {
        Some(Value::Byte(b)) => StartState::from_byte(b),
        _ => StartState::Idle,
    };
    // TeamIndex: -1 (the default) leaves the character's own team.
    let team = |a: &Actor, n: &str| int(a, n).filter(|&t| t >= 0);
    let loc = |a: &Actor| {
        get(a, "Location")
            .and_then(|v| vec3(&v))
            .unwrap_or(Vec3::ZERO)
    };

    let object = |a: &Actor, n: &str| match get(a, n) {
        Some(Value::Object(o)) if o != 0 => {
            Some(rc_level::class_look::object_path(pkg, o).join("."))
        }
        _ => None,
    };
    let flag = |a: &Actor, n: &str| match get(a, n) {
        Some(Value::Bool(b)) => Some(b),
        _ => None,
    };
    // A `TransitionInfo` the map stores (all seven values are stored when
    // any differs from the class's).
    let transition = |a: &Actor, n: &str| -> Option<Transition> {
        let Some(Value::Raw(b)) = get(a, n) else {
            return None;
        };
        let ps = properties::read_properties(pkg, &mut rc_package::reader::Reader::new(&b)).ok()?;
        let one = |n: &str| properties::find(pkg, &ps, n).map(|p| p.value.clone());
        let obj = |n: &str| match one(n) {
            Some(Value::Object(o)) if o != 0 => {
                Some(rc_level::class_look::object_path(pkg, o).join("."))
            }
            _ => None,
        };
        let nm = |n: &str| match one(n) {
            Some(Value::Name(i)) => {
                Some(pkg.name(i).to_string()).filter(|s| !s.eq_ignore_ascii_case("None"))
            }
            _ => None,
        };
        Some(Transition {
            // The effect is a class; we know effects by their class name.
            effect: match one("TransitionEffect") {
                Some(Value::Object(o)) if o != 0 => Some(pkg.object_name(o).to_string()),
                _ => None,
            },
            effect_offset: one("TransitionEffectOffset")
                .and_then(|v| vec3(&v))
                .unwrap_or(Vec3::ZERO),
            sound: obj("TransitionSound"),
            material: obj("TransitionMaterial"),
            event: nm("TransitionEvent"),
            anim: nm("TransitionAnim"),
            anim_loop: matches!(one("bAnimLoop"), Some(Value::Bool(true))),
        })
    };

    let mut out = Vec::new();
    for f in list
        .iter()
        .filter(|a| a.class == "PawnFactory" || DISPENSER_CLASSES.contains(&a.class.as_str()))
    {
        // A dispenser starts from its classes' defaults; a pawn factory
        // from `Factory`'s.
        let base = Dispenser::class_defaults(&f.class);
        let tag = name_of(f, "Tag").unwrap_or_else(|| f.class.clone());
        let prototype = match get(f, "Prototype") {
            Some(Value::Object(o)) => pkg.object_name(o).to_string(),
            _ => base.as_ref().map(|b| b.1.to_string()).unwrap_or_default(),
        };
        let mut dispenser = base.as_ref().map(|b| b.0.clone());
        if let Some(d) = dispenser.as_mut() {
            if let Some(h) = float(f, "MaxHealth") {
                d.max_health = h;
            }
            if let Some(t) = float(f, "WarmUpTime") {
                d.warm_up_time = t;
            }
            if let Some(n) = int(f, "NumInitDroids") {
                d.num_init_droids = n.max(0) as u32;
            }
            if let Some(n) = int(f, "NumDispensers") {
                d.num_dispensers = n.max(1) as u32;
            }
            if get(f, "IntroAnim").is_some() {
                d.intro_anim = label(f, "IntroAnim");
            }
            if get(f, "OnlineTag").is_some() {
                d.online_tag = label(f, "OnlineTag");
            }
            if let Some(r) = float(f, "CollisionRadius") {
                d.collision_radius = r;
            }
            if let Some(h) = float(f, "CollisionHeight") {
                d.collision_height = h;
            }
            d.hidden = flag(f, "bHidden").unwrap_or(false);
            d.attach_tag = label(f, "AttachTag");
            if let Some(s) = object(f, "DispenseSound") {
                d.dispense_sound = Some(s);
            }
            for (n, t) in [
                ("WarmUpInfo", &mut d.warm_up),
                ("OnlineInfo", &mut d.online),
                ("SlightlyDamagedInfo", &mut d.slightly_damaged),
                ("HeavilyDamagedInfo", &mut d.heavily_damaged),
                ("ShutdownInfo", &mut d.shutdown),
                ("DamagedInfo", &mut d.damaged),
            ] {
                if let Some(read) = transition(f, n) {
                    *t = read;
                }
            }
        }
        let is_disp = dispenser.is_some();
        // DroidDispenser's class defaults where it changes Factory's.
        let def_capacity = base.as_ref().map_or(2, |b| b.2 as i32);
        let def_interval = if is_disp { 3.0 } else { 1.0 };
        let def_mode = if is_disp { 0 } else { 2 };
        let spots = list
            .iter()
            .filter(|a| {
                a.class == "SpawnPoint" && name_of(a, "Tag").as_deref() == Some(tag.as_str())
            })
            .map(|a| SpawnSpot {
                location: loc(a),
                yaw: yaw(get(a, "Rotation").as_ref()),
                offspring: Offspring {
                    tag: label(a, "PawnTag"),
                    event: label(a, "PawnEvent"),
                    first_damaged_event: label(a, "FirstDamagedEvent"),
                    start_state: state(a, "StartState"),
                    team: team(a, "TeamIndex"),
                    script: label(a, "AIScriptTag"),
                },
            })
            .collect();
        let triggers = list
            .iter()
            .filter(|a| {
                a.class == "Trigger" && name_of(a, "Event").as_deref() == Some(tag.as_str())
            })
            .map(|a| {
                let r = match get(a, "CollisionRadius") {
                    Some(Value::Float(r)) => r,
                    _ => 40.0, // Trigger's UE2 default radius
                };
                (loc(a), r)
            })
            .collect();
        out.push(Factory {
            name: f.name.clone(),
            tag,
            prototype,
            location: loc(f),
            yaw: yaw(get(f, "Rotation").as_ref()),
            spots,
            // Not stored means the class default (see `Factory::default`).
            capacity: int(f, "Capacity").unwrap_or(def_capacity).max(0) as u32,
            max_alive: int(f, "MaxAlive").unwrap_or(1).max(1) as u32,
            // DroidDispenserDefaults turns both on.
            generate_forever: flag(f, "bGenerateForever").unwrap_or(is_disp),
            start_timer_after_death: flag(f, "bStartTimerAfterDeath").unwrap_or(is_disp),
            spawn_mode: match get(f, "SpawnMode") {
                // A dispenser only does spawn points in turn or at random
                // (`DroidDispenser.PostBeginPlay` turns 2 and 3 into 0).
                Some(Value::Byte(2 | 3)) if is_disp => 0,
                Some(Value::Byte(b)) => b,
                _ => def_mode,
            },
            spawn_interval: float(f, "SpawnInterval").unwrap_or(def_interval),
            spawn_interval_max: float(f, "SpawnIntervalMax").unwrap_or(0.0),
            max_in_world: float(f, "MaxInWorld").unwrap_or(0.0),
            min_player_range: float(f, "MinPlayerRange").unwrap_or(0.0),
            max_player_range: float(f, "MaxPlayerRange").unwrap_or(0.0),
            covert: matches!(get(f, "bCovert"), Some(Value::Bool(true))),
            spawn_event: label(f, "SpawnEvent"),
            exhausted_event: label(f, "ExhaustedEvent"),
            offspring: Offspring {
                tag: label(f, "PawnTagFromFactory"),
                event: label(f, "PawnEventFromFactory"),
                first_damaged_event: label(f, "FirstDamagedEventFromFactory"),
                start_state: state(f, "StartStateFromFactory"),
                team: team(f, "TeamIndexFromFactory"),
                script: label(f, "AIScriptFromFactory"),
            },
            start_state: state(f, "StartStateFromFactory"),
            team: int(f, "TeamIndexFromFactory"),
            triggers,
            dispenser,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}
