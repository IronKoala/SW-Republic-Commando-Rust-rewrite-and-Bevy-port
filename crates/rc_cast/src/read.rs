//! Finding the cast in a map, without Bevy: which placed actors are
//! characters (their class comes from `Pawn`) or animated props
//! (`AnimProp`), what model each wears, where it stands and which script it
//! follows.
//!
//! A map stores only what a designer changed, so a character's model is
//! usually its class's default `Mesh` (`CloneBoy`, `Kaminoan`), read from
//! the game's script packages with `rc_package::class_library`. The mesh is
//! then found by name in the animation packages (`GameData/Animations/*.ukx`).

use rc_level::scripts::{ScriptActor, Val};
use rc_package::class_library::ClassLibrary;
use rc_package::Package;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// `data/cast/cast.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CastFile {
    pub ambient: f32,
    /// How strongly the level's light where a cast member stands shows on
    /// it (see `rc_game_core::place_light`), as for the squad and droids.
    #[serde(default = "light_scale")]
    pub light_scale: f32,
    pub walk_speed_ratio: f32,
    pub idle: Vec<String>,
    pub walk: Vec<String>,
    pub run: Vec<String>,
    pub wait_for_player_limit: f32,
    pub pose_distance: f32,
    pub leave_out: Vec<String>,
    #[serde(default)]
    pub class: BTreeMap<String, ClassOverride>,
}

fn light_scale() -> f32 {
    1.1
}

/// A class's model, changed by the data file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassOverride {
    /// Another mesh, by name.
    pub mesh: Option<String>,
    /// Replacement materials by slot; "" keeps the mesh's own.
    #[serde(default)]
    pub skins: Vec<String>,
}

impl Default for CastFile {
    fn default() -> Self {
        toml::from_str(include_str!("../../../data/cast/cast.toml"))
            .expect("data/cast/cast.toml is valid")
    }
}

impl CastFile {
    /// Whether a class (given with its parents) belongs to another part.
    pub fn leaves_out(&self, chain: &[String]) -> bool {
        chain
            .iter()
            .any(|c| self.leave_out.iter().any(|p| glob(p, c)))
    }

    /// The data file's changes for `class`, if any (any case).
    pub fn override_for(&self, class: &str) -> Option<&ClassOverride> {
        self.class
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(class))
            .map(|(_, v)| v)
    }
}

/// `pattern` with an optional `*` at the end, any case.
pub fn glob(pattern: &str, name: &str) -> bool {
    let (p, n) = (pattern.to_ascii_lowercase(), name.to_ascii_lowercase());
    match p.strip_suffix('*') {
        Some(start) => n.starts_with(start),
        None => n == p,
    }
}

/// engine.u's `EPhysics`: `PHYS_RootMotion`.
const PHYS_ROOT_MOTION: i32 = 12;

/// What kind of cast member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A character: stands on its feet, its `Location` is the middle of
    /// its body.
    Pawn,
    /// An animated prop (`AnimProp`): drawn at its `Location`, playing one
    /// animation (`AnimHealthy`).
    Prop,
}

/// One placed character or animated prop.
#[derive(Debug, Clone)]
pub struct CastMember {
    /// The map's name for it (`CloneBoy0`).
    pub name: String,
    pub class: String,
    pub kind: Kind,
    /// Unreal units: the middle of the body for a character, the model's
    /// origin for a prop.
    pub location: [f32; 3],
    /// Half the character's height (`CollisionHeight`): its feet are this
    /// far below `location`. 0 for props, and for characters whose class
    /// turns `bAlignBottom` off (engine.u: "Actor aligned to bottom of
    /// cylinder"; ctcharacters.u's `Gunship` sets it false), whose model's
    /// origin is `location` itself, as a prop's.
    pub half_height: f32,
    /// The weapon class it holds (`RequiredEquipment`, with the class's
    /// "...Defaults" class on top: `DC15BlasterClone` from
    /// "Properties.DC15BlasterClone" for a clone trooper); characters only.
    pub weapon: Option<String>,
    /// `bAlignBottom` (engine.u's `Pawn` and `AnimProp` default true):
    /// false draws the model from its own origin at `location`.
    pub align_bottom: bool,
    /// Facing, radians (Unreal's yaw).
    pub yaw: f32,
    pub scale: f32,
    /// The model: animation package (relative to the game folder) and mesh.
    pub package: String,
    pub mesh: String,
    pub skins: Vec<Option<String>>,
    /// `Tag` (the class name when the map gives none, as in Unreal).
    pub tag: String,
    /// The `ScriptedSequence` it follows (`AIScriptTag`).
    pub script: Option<String>,
    pub hidden: bool,
    /// A prop's own animation and whether it loops.
    pub prop_anim: Option<(String, bool)>,
    /// What a prop plays when a level event named by its tag triggers it
    /// (`AnimTriggered`), and whether it loops: the hologram projector's
    /// `ThrowHoloProjector`, which starts the briefings.
    pub prop_trigger: Option<(String, bool)>,
    /// Its physics is root motion (`Physics` 12, `PHYS_RootMotion` in
    /// engine.u), from the map or the class: the original moves such an
    /// actor along with its animation's move bone (geo_01a's troopers in
    /// the gunship door). Every other one stays where it is and its move
    /// bone's travel isn't drawn.
    pub root_motion: bool,
    /// Running speed, Unreal units a second (`GroundSpeed`).
    pub ground_speed: f32,
    /// Casts a shadow (`bActorShadows`, from the map or the class). Only
    /// characters: in the original only a `Pawn` makes a shadow projector.
    pub shadow: bool,
    /// How it can be hurt and die; `None` for props and for characters
    /// this part leaves unhurt (vehicles, classes with no `Health`).
    pub life: Option<Life>,
    /// Its patrol (`PatrolRoute` and `PatrolMode`), when it has one this
    /// part can follow (see [`Patrol`]).
    pub patrol: Option<Patrol>,
    /// It starts switched off (`StartState` `SS_Shutdown`): it does
    /// nothing until an event named by its tag wakes it.
    pub shutdown: bool,
    /// Made by a pawn factory (its `Tag`) rather than placed: it appears
    /// when an event with that name switches the factory on.
    pub made_by: Option<String>,
}

/// engine.u's `Pawn.EPatrolMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatrolMode {
    Loop,
    PingPong,
    Random,
    Once,
}

/// One stop of a patrol (engine.u's `Pawn.PatrolPoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct PatrolPoint {
    /// `PauseTime`, seconds.
    pub pause: f32,
    /// `Animation`, played `NumAnimLoops` times (at least once).
    pub anim: Option<String>,
    pub loops: i32,
    /// `Event`, sent once this stop is done.
    pub event: Option<String>,
    /// `MakeInvulnerable`.
    pub make_invulnerable: bool,
}

/// A character's patrol: the stops in order and how to go through them.
/// Only patrols whose stops name no `Node` are read (no walking between
/// points yet): the flying gunships and fighters, whose stops are each
/// one flight animation.
#[derive(Debug, Clone, PartialEq)]
pub struct Patrol {
    pub points: Vec<PatrolPoint>,
    pub mode: PatrolMode,
}

/// engine.u's `EStartState`: `SS_Shutdown`.
const SS_SHUTDOWN: i32 = 4;

/// A patrol from an actor's `PatrolRoute` (or a factory's
/// `PatrolRouteFromFactory`, under `route`/`mode` names). `Err` says why
/// it isn't followed.
pub fn read_patrol(
    o: &rc_level::scripts::ScriptObject,
    route: &str,
    mode: &str,
) -> Result<Option<Patrol>, String> {
    let Some(Val::Structs(list)) = o.get(route) else {
        return Ok(None);
    };
    if list.is_empty() {
        return Ok(None);
    }
    let text = |s: &rc_level::scripts::Vals, k: &str| match s.get(k) {
        Some(Val::Text(t)) if !t.is_empty() && !t.eq_ignore_ascii_case("None") => Some(t.clone()),
        _ => None,
    };
    let mut points = Vec::new();
    for s in list {
        if let Some(n) = text(s, "Node") {
            return Err(format!(
                "its patrol walks to {n}: walking a patrol isn't done yet"
            ));
        }
        points.push(PatrolPoint {
            pause: match s.get("PauseTime") {
                Some(Val::Float(f)) => *f,
                _ => 0.0,
            },
            anim: text(s, "Animation"),
            loops: match s.get("NumAnimLoops") {
                Some(Val::Int(i)) => *i,
                Some(Val::Byte(b)) => *b as i32,
                _ => 0,
            },
            event: text(s, "Event"),
            make_invulnerable: matches!(s.get("MakeInvulnerable"), Some(Val::Bool(true))),
        });
    }
    let mode = match o.int(mode).unwrap_or(0) {
        1 => PatrolMode::PingPong,
        2 => PatrolMode::Random,
        3 => PatrolMode::Once,
        _ => PatrolMode::Loop,
    };
    Ok(Some(Patrol { points, mode }))
}

/// The characters pawn factories make that this part draws: those whose
/// class comes from `Vehicles` (the flying gunships and fighters; nothing
/// else makes them). Each becomes an actor of the factory's `Prototype`
/// standing where the factory is, with what CTGame.u's
/// `PawnFactory.SetupNewActor` and `Factory.SetupNewActor` give it when it
/// is made at the factory (`SpawnMode` `SPM_FromFactory`): the factory's
/// `PawnTagFromFactory` as its tag, `AIScriptFromFactory`,
/// `PawnEventFromFactory`, `PatrolRouteFromFactory`,
/// `PatrolModeFromFactory`, `StartStateFromFactory` and the physics
/// `InitialPhysicsFromFactory`. Its name is the factory's, with `made`.
/// Returns the actors with the factory's tag for each.
pub fn factory_vehicles(
    actors: &[ScriptActor],
    lib: &ClassLibrary,
    notes: &mut Vec<String>,
) -> Vec<(ScriptActor, String)> {
    let mut out = Vec::new();
    for a in actors {
        let o = &a.obj;
        if !lib
            .chain(&o.class)
            .iter()
            .any(|c| c.eq_ignore_ascii_case("PawnFactory"))
        {
            continue;
        }
        let Some(proto) = o.text("Prototype").and_then(|p| p.rsplit('.').next()) else {
            continue;
        };
        if !lib
            .chain(proto)
            .iter()
            .any(|c| c.eq_ignore_ascii_case("Vehicles"))
        {
            continue;
        }
        let defaults = lib.defaults(&o.class);
        let num = |k: &str, d: i32| {
            o.int(k)
                .or_else(|| defaults.number(k).map(|n| n as i32))
                .unwrap_or(d)
        };
        // ctgame.u's Factory defaults: SpawnMode SPM_FromFactory (2).
        let (mode, capacity) = (num("SpawnMode", 2), num("Capacity", 2));
        if mode != 2 || capacity != 1 || o.flag("bGenerateForever", false) {
            notes.push(format!(
                "{} makes {proto} (spawn mode {mode}, capacity {capacity}): only factories making one at the factory are done; left out",
                o.name
            ));
            continue;
        }
        let mut values = rc_level::scripts::Vals::new();
        let copy = |values: &mut rc_level::scripts::Vals, from: &str, to: &str| {
            if let Some(v) = o.get(from) {
                values.insert(to.to_string(), v.clone());
            }
        };
        copy(&mut values, "Rotation", "Rotation");
        copy(&mut values, "PawnTagFromFactory", "Tag");
        copy(&mut values, "AIScriptFromFactory", "AIScriptTag");
        copy(&mut values, "PawnEventFromFactory", "Event");
        copy(&mut values, "PatrolRouteFromFactory", "PatrolRoute");
        copy(&mut values, "PatrolModeFromFactory", "PatrolMode");
        copy(&mut values, "StartStateFromFactory", "StartState");
        copy(&mut values, "InitialPhysicsFromFactory", "Physics");
        // Values the factory always sets, from its class when the map
        // doesn't.
        for (from, to) in [
            ("StartStateFromFactory", "StartState"),
            ("InitialPhysicsFromFactory", "Physics"),
        ] {
            if !values.contains_key(to) {
                let n = defaults.number(from).unwrap_or(0.0);
                values.insert(to.to_string(), Val::Byte(n as u8));
            }
        }
        // A pawn with no tag is tagged with its class (Unreal).
        values
            .entry("Tag".to_string())
            .or_insert_with(|| Val::Text(proto.to_string()));
        let made = ScriptActor {
            obj: rc_level::scripts::ScriptObject {
                export: o.export,
                class: proto.to_string(),
                name: format!("{} made", o.name),
                values,
            },
            location: a.location,
            actions: Vec::new(),
        };
        out.push((made, o.tag().to_string()));
    }
    out
}

/// A character's health and what its death does, from the map and its
/// class (the class's "...Defaults" class first, as the game lays it on
/// top: see `find_cast`).
#[derive(Debug, Clone, PartialEq)]
pub struct Life {
    /// `Health` (CloneTrooper 100).
    pub health: f32,
    /// The hit cylinder: `CollisionRadius` and `CollisionHeight`.
    pub radius: f32,
    pub half_height: f32,
    /// `Event`: engine.u's `Pawn.Died` triggers it when the character
    /// dies (geo_01a's `GunshipTrooper02Death`).
    pub event: Option<String>,
    /// Seconds the body stays after death: `Pawn.PlayDying` sets
    /// `LifeSpan = RagdollLifeSpan` unless `bSaveMyCorpse`. `None` keeps
    /// it (`bSaveMyCorpse`, or a life span of 0, which Unreal reads as
    /// forever).
    pub corpse_time: Option<f32>,
}

/// Every skeletal mesh in the animation packages, by lower-case name: the
/// package (relative to the game folder) and the mesh's own name.
pub fn mesh_index(animations_dir: &Path) -> (HashMap<String, (String, String)>, Vec<String>) {
    let mut out = HashMap::new();
    let mut notes = Vec::new();
    let mut files: Vec<_> = match std::fs::read_dir(animations_dir) {
        Ok(d) => d.flatten().map(|e| e.path()).collect(),
        Err(e) => {
            notes.push(format!("can't list {}: {e}", animations_dir.display()));
            return (out, notes);
        }
    };
    files.sort();
    for f in files {
        if !f.extension().is_some_and(|x| x.eq_ignore_ascii_case("ukx")) {
            continue;
        }
        let Some(file) = f.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        let parsed = std::fs::read(&f)
            .map_err(|e| e.to_string())
            .and_then(|d| Package::parse(&d).map_err(|e| e.to_string()));
        match parsed {
            Ok(pkg) => {
                for e in &pkg.exports {
                    if pkg.export_class_name(e) == "SkeletalMesh" {
                        let name = pkg.name(e.object_name).to_string();
                        out.entry(name.to_ascii_lowercase())
                            .or_insert((format!("GameData/Animations/{file}"), name));
                    }
                }
            }
            Err(e) => notes.push(format!("{file} not read: {e}")),
        }
    }
    (out, notes)
}

/// The game's script classes: every package in `GameData/System` and
/// `GameData/Properties`.
pub fn class_library(game_data: &Path) -> (ClassLibrary, Vec<String>) {
    let mut lib = ClassLibrary::new();
    let mut notes = Vec::new();
    for dir in ["System", "Properties"] {
        let d = rc_game_core::paths::resolve(game_data, dir);
        let mut files: Vec<_> = match std::fs::read_dir(&d) {
            Ok(r) => r.flatten().map(|e| e.path()).collect(),
            Err(e) => {
                notes.push(format!("can't list {}: {e}", d.display()));
                continue;
            }
        };
        files.sort();
        for f in files {
            if !f.extension().is_some_and(|x| x.eq_ignore_ascii_case("u")) {
                continue;
            }
            let name = f
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            match std::fs::read(&f) {
                Ok(data) => {
                    if let Err(e) = lib.add_package(&name, data) {
                        notes.push(format!("{}: {e}", f.display()));
                    }
                }
                Err(e) => notes.push(format!("can't read {}: {e}", f.display())),
            }
        }
    }
    (lib, notes)
}

/// The `Skins` a map sets on its actors, by export index, each as a full
/// dotted path with its package first. Only actors that set them.
pub fn map_skins(
    pkg: &Package,
    data: &[u8],
    actors: &[ScriptActor],
) -> HashMap<usize, Vec<Option<String>>> {
    use rc_package::properties::Value;
    use rc_package::ObjectRef;
    let mut out = HashMap::new();
    for a in actors {
        if a.obj.get("Skins").is_none() {
            continue;
        }
        let Ok(props) = rc_level::actors::read_actor_properties(pkg, data, a.obj.export) else {
            continue;
        };
        let Some(Value::Raw(b)) = props
            .iter()
            .find(|p| pkg.name(p.name).eq_ignore_ascii_case("Skins"))
            .map(|p| &p.value)
        else {
            continue;
        };
        let mut r = rc_package::Reader::new(b);
        let Ok(n) = r.compact_index() else {
            continue;
        };
        let mut list = Vec::new();
        for _ in 0..n.max(0) {
            let Ok(o) = r.compact_index() else {
                break;
            };
            list.push(match ObjectRef::from_index(o) {
                ObjectRef::None => None,
                ObjectRef::Import(_) => Some(rc_package::class::object_path(pkg, o)),
                // Made inside the map itself: not something we can load.
                ObjectRef::Export(_) => Some(format!(
                    "myLevel.{}",
                    rc_package::class::object_path(pkg, o)
                )),
            });
        }
        out.insert(a.obj.export, list);
    }
    out
}

/// Yaw from an actor's stored `Rotation` (pitch, yaw, roll as Unreal
/// angle units), radians.
fn yaw_of(a: &ScriptActor) -> f32 {
    match a.obj.get("Rotation") {
        Some(Val::Raw(b)) if b.len() >= 8 => {
            let yaw = i32::from_le_bytes([b[4], b[5], b[6], b[7]]);
            yaw as f32 * std::f32::consts::TAU / 65536.0
        }
        _ => 0.0,
    }
}

/// The cast among a map's actors. Lines for the log say what was left out
/// and why.
pub fn find_cast(
    actors: &[ScriptActor],
    lib: &ClassLibrary,
    meshes: &HashMap<String, (String, String)>,
    file: &CastFile,
    map_skins: &HashMap<usize, Vec<Option<String>>>,
) -> (Vec<CastMember>, Vec<String>) {
    let mut out = Vec::new();
    let mut notes = Vec::new();
    // Missions have a `CTSquad` actor; the prologue, briefings and
    // epilogue don't, and every commando there is part of the scene.
    let has_squad = actors
        .iter()
        .any(|a| a.obj.class.eq_ignore_ascii_case("CTSquad"));
    for a in actors {
        let o = &a.obj;
        let chain = lib.chain(&o.class);
        let is_pawn = chain.iter().any(|c| c.eq_ignore_ascii_case("Pawn"));
        let is_prop = chain.iter().any(|c| c.eq_ignore_ascii_case("AnimProp"));
        if !is_pawn && !is_prop {
            continue;
        }
        if file.leaves_out(&chain) {
            notes.push(format!("{} ({}): left to another part", o.name, o.class));
            continue;
        }
        // Delta in a mission belongs to the squad part, apart from
        // commandos the map says can't join the squad.
        if has_squad
            && chain
                .iter()
                .any(|c| c.eq_ignore_ascii_case("CloneCommando"))
            && o.flag("bCanBeSquadMember", true)
        {
            notes.push(format!(
                "{} ({}): a squad mate, left to the squad",
                o.name, o.class
            ));
            continue;
        }
        let defaults = lib.defaults(&o.class);
        let over = file.override_for(&o.class);
        let mesh_name = over
            .and_then(|v| v.mesh.clone())
            .or_else(|| o.text("Mesh").map(str::to_string))
            .or_else(|| defaults.object("Mesh").map(str::to_string))
            .filter(|m| !m.eq_ignore_ascii_case("None"));
        let Some(mesh_name) = mesh_name else {
            notes.push(format!("{} ({}): no mesh; not drawn", o.name, o.class));
            continue;
        };
        let Some((package, mesh)) = meshes.get(&mesh_name.to_ascii_lowercase()).cloned() else {
            notes.push(format!(
                "{} ({}): mesh {mesh_name} not in any animation package; not drawn",
                o.name, o.class
            ));
            continue;
        };
        let number = |k: &str, d: f32| o.float(k).or_else(|| defaults.number(k)).unwrap_or(d);
        // The game lays a class's "...Defaults" class from properties.u
        // over it (core.dll's `Properties.%sDefaults`):
        // `CloneTrooperDefaults` is a child of `CloneTrooper`, so its own
        // defaults with its parents' cover the whole class.
        let laid = Some(format!("{}Defaults", o.class))
            .filter(|d| lib.has_class(d))
            .map(|d| lib.defaults(&d));
        let full = laid.as_ref().unwrap_or(&defaults);
        let full_number = |k: &str| o.float(k).or_else(|| full.number(k));
        let is_vehicle = chain.iter().any(|c| c.eq_ignore_ascii_case("Vehicles"));
        let life = full_number("Health")
            .filter(|h| is_pawn && !is_vehicle && *h > 0.0)
            .map(|health| Life {
                health,
                radius: full_number("CollisionRadius").unwrap_or(40.0),
                half_height: full_number("CollisionHeight").unwrap_or(78.0),
                event: o.text("Event").map(str::to_string),
                corpse_time: if o.flag("bSaveMyCorpse", full.bool("bSaveMyCorpse").unwrap_or(false))
                {
                    None
                } else {
                    full_number("RagdollLifeSpan").filter(|t| *t > 0.0)
                },
            });
        let align_bottom = o.flag(
            "bAlignBottom",
            defaults.bool("bAlignBottom").unwrap_or(true),
        );
        let prop_anim = o
            .structure("AnimHealthy")
            .and_then(|s| match s.get("Anim") {
                Some(Val::Text(t)) if !t.eq_ignore_ascii_case("None") => {
                    Some((t.clone(), !matches!(s.get("bLoop"), Some(Val::Bool(false)))))
                }
                _ => None,
            });
        let prop_trigger = o
            .structure("AnimTriggered")
            .and_then(|s| match s.get("Anim") {
                Some(Val::Text(t)) if !t.is_empty() && !t.eq_ignore_ascii_case("None") => {
                    Some((t.clone(), matches!(s.get("bLoop"), Some(Val::Bool(true)))))
                }
                _ => None,
            })
            .filter(|_| is_prop && !is_pawn);
        out.push(CastMember {
            name: o.name.clone(),
            class: o.class.clone(),
            kind: if is_pawn { Kind::Pawn } else { Kind::Prop },
            location: a.location,
            half_height: if is_pawn && align_bottom {
                number("CollisionHeight", 78.0)
            } else {
                0.0
            },
            align_bottom,
            weapon: if is_pawn {
                // The game lays the class's "...Defaults" class in
                // Properties/properties.u over it (core.dll builds the
                // name "Properties.%sDefaults"): CloneTrooperDefaults swaps
                // the trooper's DC-17m for Properties.DC15BlasterClone.
                Some(format!("{}Defaults", o.class))
                    .filter(|d| lib.has_class(d))
                    .and_then(|d| lib.defaults(&d).string("RequiredEquipment"))
                    .or_else(|| defaults.string("RequiredEquipment"))
                    .and_then(|w| w.rsplit('.').next().map(str::to_string))
                    .filter(|w| !w.is_empty() && !w.eq_ignore_ascii_case("None"))
            } else {
                None
            },
            yaw: yaw_of(a),
            scale: number("DrawScale", 1.0),
            package,
            mesh,
            // Skins: our cast file first, then what the map sets on this
            // actor, then the class's own (the briefing holograms' advisor
            // is a clone trooper mesh in a hologram skin).
            skins: over
                .map(|v| {
                    v.skins
                        .iter()
                        .map(|s| Some(s.clone()).filter(|s| !s.is_empty()))
                        .collect::<Vec<_>>()
                })
                .filter(|s| !s.is_empty())
                .or_else(|| map_skins.get(&o.export).cloned())
                .or_else(|| lib.object_list(&o.class, "Skins"))
                .unwrap_or_default(),
            tag: o.tag().to_string(),
            script: o.text("AIScriptTag").map(str::to_string),
            hidden: o
                .get("bHidden")
                .map(|v| matches!(v, Val::Bool(true)))
                .unwrap_or_else(|| defaults.bool("bHidden").unwrap_or(false)),
            prop_anim,
            prop_trigger,
            root_motion: o
                .int("Physics")
                .or_else(|| defaults.number("Physics").map(|p| p as i32))
                == Some(PHYS_ROOT_MOTION),
            ground_speed: number("GroundSpeed", 440.0),
            shadow: is_pawn
                && o.get("bActorShadows")
                    .map(|v| matches!(v, Val::Bool(true)))
                    .unwrap_or_else(|| defaults.bool("bActorShadows").unwrap_or(false)),
            life,
            patrol: match read_patrol(o, "PatrolRoute", "PatrolMode") {
                Ok(p) => p,
                Err(e) => {
                    notes.push(format!("{} ({}): {e}", o.name, o.class));
                    None
                }
            },
            shutdown: o
                .int("StartState")
                .or_else(|| defaults.number("StartState").map(|n| n as i32))
                == Some(SS_SHUTDOWN),
            made_by: None,
        });
    }
    (out, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_match_starts_and_whole_names() {
        assert!(glob("BattleDroid*", "battledroidTM"));
        assert!(glob("CTPlayer", "ctplayer"));
        assert!(!glob("CTPlayer", "CTPlayerX"));
        assert!(!glob("Geonosian*", "CloneBoy"));
    }

    #[test]
    fn the_data_file_reads_and_leaves_enemies_out() {
        let f = CastFile::default();
        assert!(f.leaves_out(&["BattleDroidTM".into(), "BattleDroid".into(), "Pawn".into()]));
        assert!(!f.leaves_out(&["Kaminoan".into(), "Pawn".into()]));
        assert_eq!(
            f.override_for("commando07delta").map(|o| o.skins.len()),
            Some(1)
        );
    }

    /// The enemies part makes every placed character an enemy file claims
    /// (apart from clone troopers), so the cast must leave each of those
    /// classes out or it would be there twice.
    #[test]
    fn every_enemy_files_classes_are_left_out() {
        let f = CastFile::default();
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/enemies");
        let mut checked = 0;
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let text = std::fs::read_to_string(e.path()).unwrap();
            let v: toml::Value = toml::from_str(&text).unwrap();
            let t = &v["type"];
            if t.get("side").and_then(|s| s.as_str()) == Some("republic") {
                continue;
            }
            for c in t["classes"].as_array().unwrap() {
                let c = c.as_str().unwrap().trim_end_matches('*');
                assert!(
                    f.leaves_out(&[c.to_string()]),
                    "{c} ({}) is an enemy but the cast doesn't leave it out",
                    e.path().display()
                );
                checked += 1;
            }
        }
        assert!(checked > 10);
    }
}
