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
//! Characters can also be placed straight in the map, with no factory
//! (Engine.u `Pawn.PostBeginPlay`, read only): every placed pawn that is
//! alive when the level starts (its health is set to its full health
//! just before) gets its controller (its AI) straight away,
//! through its `AIScriptTag`'s script when it names one, unless
//! `bDontPossess` is set, and is handed its default weapons. Its own
//! `Tag`, `Event` (sent when it dies), `FirstDamagedEvent`, `StartState`,
//! `TeamIndex` and `AIScriptTag` are the same values a factory copies onto
//! the characters it makes (CTGame.u `PawnFactory.SetupNewActor`).
//! [`read_placed`] turns each one into a [`Factory`] that makes exactly
//! that one character, at its own place, as the level starts
//! ([`Factory::placed`]).

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
    /// The route it walks (`PatrolRouteFromFactory` / `PatrolRoute`) and
    /// how (`PatrolModeFromFactory` / `PatrolMode`). Empty: none.
    pub patrol: Vec<PatrolPoint>,
    pub patrol_mode: PatrolMode,
}

/// How a character goes round its patrol route (Engine.u
/// `Pawn.EPatrolMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PatrolMode {
    /// From the first point to the last, then the first again.
    #[default]
    Loop,
    /// To the last and back, and so on.
    PingPong,
    /// A random point each time.
    Random,
    /// Once to the last, then it stops patrolling.
    Once,
}

impl PatrolMode {
    pub fn from_byte(b: u8) -> Self {
        match b {
            1 => PatrolMode::PingPong,
            2 => PatrolMode::Random,
            3 => PatrolMode::Once,
            _ => PatrolMode::Loop,
        }
    }
}

/// One point of a patrol route (Engine.u `Pawn.PatrolPoint`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PatrolPoint {
    /// The actor it goes to (`Node`), by name; `None`: it stays where it
    /// is for this point.
    pub node: Option<String>,
    /// Where it walks for the node: the node's "anchor" (see
    /// [`resolve_patrol_anchors`]); `None` when the node has none.
    pub anchor: Option<Vec3>,
    /// The node's facing, radians (for `orient`).
    pub node_yaw: f32,
    /// Seconds it waits there (`PauseTime`).
    pub pause: f32,
    /// An animation it plays there (`Animation`), `NumAnimLoops` times
    /// (at least once).
    pub animation: Option<String>,
    pub loops: i32,
    /// Sent once it has done all of the above (`Event`).
    pub event: Option<String>,
    /// Runs rather than walks (`RunToNode`).
    pub run: bool,
    /// May fight on the way (`ShootWhileMoving`).
    pub shoot_while_moving: bool,
    /// Turns to the node's facing there (`OrientToNode`).
    pub orient: bool,
    /// Can't be hurt from this point on, until a point without it
    /// (`MakeInvulnerable`).
    pub invulnerable: bool,
}

/// Read a patrol route (`key`, a list of `PatrolPoint` structs) and its
/// mode (`mode_key`) from the actor `export`.
fn read_patrol(
    pkg: &Package,
    data: &[u8],
    export: usize,
    key: &str,
    mode_key: &str,
) -> (Vec<PatrolPoint>, PatrolMode) {
    use rc_level::scripts::Val;
    let Ok(obj) = rc_level::scripts::read_object(pkg, data, export) else {
        return (Vec::new(), PatrolMode::Loop);
    };
    let mode = match obj.get(mode_key) {
        Some(Val::Byte(b)) => PatrolMode::from_byte(*b),
        _ => PatrolMode::Loop,
    };
    let list = match obj.get(key) {
        Some(Val::Structs(l)) => l.clone(),
        Some(Val::Struct(v)) => vec![v.clone()],
        _ => Vec::new(),
    };
    let text = |v: &rc_level::scripts::Vals, k: &str| match v.get(k) {
        Some(Val::Text(t)) if !t.is_empty() && !t.eq_ignore_ascii_case("None") => Some(t.clone()),
        _ => None,
    };
    let flag = |v: &rc_level::scripts::Vals, k: &str| matches!(v.get(k), Some(Val::Bool(true)));
    let num = |v: &rc_level::scripts::Vals, k: &str| match v.get(k) {
        Some(Val::Float(f)) => *f,
        Some(Val::Int(i)) => *i as f32,
        _ => 0.0,
    };
    let points = list
        .iter()
        .map(|v| PatrolPoint {
            node: text(v, "Node"),
            anchor: None,
            node_yaw: 0.0,
            pause: num(v, "PauseTime"),
            animation: text(v, "Animation"),
            loops: num(v, "NumAnimLoops") as i32,
            event: text(v, "Event"),
            run: flag(v, "RunToNode"),
            shoot_while_moving: flag(v, "ShootWhileMoving"),
            orient: flag(v, "OrientToNode"),
            invulnerable: flag(v, "MakeInvulnerable"),
        })
        .collect();
    (points, mode)
}

/// Where a character walks for each patrol point: the node's anchor, as
/// the original's patrol (CTGame.dll `UStatePatrol::UpdateSelf`, which
/// asks the node for `GetAnchor()`, read in Engine.dll) does: a
/// navigation point is its own anchor, a turret's is the first of its
/// `NavPts` (where its gunner stands), a plain actor has none (the
/// character doesn't move for that point). `is_a(class, ancestor)` says
/// whether a map class comes from another. Not done (listed in the
/// returned notes): a character as the node (the original takes its
/// anchor where it is then), a turret with no `NavPts` (the same), and a
/// bacta dispenser (its own `GetAnchor` in CTGame.dll, not read).
pub fn resolve_patrol_anchors(
    pkg: &Package,
    data: &[u8],
    factories: &mut [Factory],
    is_a: &dyn Fn(&str, &str) -> bool,
) -> Vec<String> {
    use rc_level::scripts::Val;
    let mut notes = Vec::new();
    let Ok(level) = actors::read_level(pkg, data) else {
        return notes;
    };
    let find = |name: &str| {
        level.actors.iter().flatten().copied().find(|&a| {
            pkg.name(pkg.exports[a].object_name)
                .eq_ignore_ascii_case(name)
        })
    };
    let place = |a: usize| -> Option<(Vec3, f32)> {
        let props = actors::read_actor_properties(pkg, data, a).ok()?;
        let get = |n: &str| properties::find(pkg, &props, n).map(|p| p.value.clone());
        let at = get("Location").and_then(|v| vec3(&v))?;
        Some((at, yaw(get("Rotation").as_ref())))
    };
    for f in factories.iter_mut() {
        let spots = f.spots.iter_mut().map(|s| &mut s.offspring);
        for o in std::iter::once(&mut f.offspring).chain(spots) {
            for p in o.patrol.iter_mut() {
                let Some(node) = p.node.clone() else { continue };
                let Some(a) = find(&node) else {
                    notes.push(format!("{}: patrol node {node} is not in the map", f.name));
                    continue;
                };
                let class = pkg.export_class_name(&pkg.exports[a]).to_string();
                if let Some((_, y)) = place(a) {
                    p.node_yaw = y;
                }
                let nav_pts =
                    rc_level::scripts::read_object(pkg, data, a)
                        .ok()
                        .and_then(|o| match o.get("NavPts") {
                            Some(Val::List(l)) => l.first().cloned(),
                            _ => None,
                        });
                let mut why = "";
                if is_a(&class, "NavigationPoint") {
                    p.anchor = place(a).map(|(at, _)| at);
                } else if is_a(&class, "Turret") {
                    p.anchor = nav_pts
                        .and_then(|spot| find(&spot))
                        .and_then(place)
                        .map(|(at, _)| at);
                    why = " (a turret with no NavPts: not done)";
                } else if is_a(&class, "Pawn") || is_a(&class, "BactaDispenser") {
                    why = " (not done)";
                } else {
                    // A plain actor (`AActor::GetAnchor` gives none): the
                    // character stays where it is for this point.
                    continue;
                }
                if p.anchor.is_none() {
                    notes.push(format!(
                        "{}: patrol node {node} ({class}) has no place to walk to{why}",
                        f.name
                    ));
                }
            }
        }
    }
    notes
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
    /// Not a real factory: a character placed straight in the map (see
    /// [`read_placed`]). It makes that one character when the level
    /// starts and nothing else; events, scripts' factory orders and
    /// walk-in triggers leave it alone.
    pub placed: bool,
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
            placed: false,
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
    export: usize,
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
        if !matches!(class.as_str(), "PawnFactory" | "SpawnPoint" | "Trigger") {
            continue;
        }
        if let Ok(props) = actors::read_actor_properties(pkg, data, a) {
            list.push(Actor {
                export: a,
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

    let mut out = Vec::new();
    for f in list.iter().filter(|a| a.class == "PawnFactory") {
        let tag = name_of(f, "Tag").unwrap_or_else(|| "PawnFactory".into());
        let prototype = match get(f, "Prototype") {
            Some(Value::Object(o)) => pkg.object_name(o).to_string(),
            _ => String::new(),
        };
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
                    patrol: read_patrol(pkg, data, a.export, "PatrolRoute", "PatrolMode").0,
                    patrol_mode: read_patrol(pkg, data, a.export, "PatrolRoute", "PatrolMode").1,
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
            capacity: int(f, "Capacity").unwrap_or(2).max(0) as u32,
            max_alive: int(f, "MaxAlive").unwrap_or(1).max(1) as u32,
            generate_forever: matches!(get(f, "bGenerateForever"), Some(Value::Bool(true))),
            spawn_mode: match get(f, "SpawnMode") {
                Some(Value::Byte(b)) => b,
                _ => 2,
            },
            spawn_interval: float(f, "SpawnInterval").unwrap_or(1.0),
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
                patrol: read_patrol(
                    pkg,
                    data,
                    f.export,
                    "PatrolRouteFromFactory",
                    "PatrolModeFromFactory",
                )
                .0,
                patrol_mode: read_patrol(
                    pkg,
                    data,
                    f.export,
                    "PatrolRouteFromFactory",
                    "PatrolModeFromFactory",
                )
                .1,
            },
            start_state: state(f, "StartStateFromFactory"),
            team: int(f, "TeamIndexFromFactory"),
            triggers,
            placed: false,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// One character placed straight in a map, as [`read_placed`] found it.
#[derive(Debug, Clone)]
pub struct Placed {
    /// The actor's name in the map, e.g. `GeonosianWarriorNoWeap0`.
    pub name: String,
    pub class: String,
    /// The one-character factory that puts it in the level.
    pub factory: Factory,
    /// Why the original gives it no AI at the start, if it doesn't
    /// (`bDontPossess` set). The game leaves such characters out for now:
    /// it only makes characters that fight.
    pub left_out: Option<&'static str>,
}

/// Every character placed straight in the map whose class `wanted` says
/// yes to (the caller checks it is a pawn of a kind it can make), each as a
/// one-character [`Factory`] (see the module notes). Sorted by name.
pub fn read_placed(
    pkg: &Package,
    data: &[u8],
    wanted: &dyn Fn(&str) -> bool,
) -> Result<Vec<Placed>, String> {
    let level = actors::read_level(pkg, data).map_err(|e| format!("level: {e}"))?;
    let mut out = Vec::new();
    for &a in level.actors.iter().flatten() {
        let e = &pkg.exports[a];
        let class = pkg.export_class_name(e).to_string();
        if !wanted(&class) {
            continue;
        }
        let Ok(props) = actors::read_actor_properties(pkg, data, a) else {
            continue;
        };
        let get = |n: &str| properties::find(pkg, &props, n).map(|p| p.value.clone());
        let label = |n: &str| match get(n) {
            Some(Value::Name(i)) => {
                Some(pkg.name(i).to_string()).filter(|s| !s.eq_ignore_ascii_case("None"))
            }
            _ => None,
        };
        let name = pkg.name(e.object_name).to_string();
        let location = get("Location").and_then(|v| vec3(&v)).unwrap_or(Vec3::ZERO);
        let left_out = if matches!(get("bDontPossess"), Some(Value::Bool(true))) {
            Some("bDontPossess is set: the original gives it no AI")
        } else {
            None
        };
        let offspring = Offspring {
            // Unreal gives a placed actor its class name as its tag when
            // the map stores none.
            tag: Some(label("Tag").unwrap_or_else(|| class.clone())),
            event: label("Event"),
            first_damaged_event: label("FirstDamagedEvent"),
            start_state: match get("StartState") {
                Some(Value::Byte(b)) => StartState::from_byte(b),
                _ => StartState::Idle,
            },
            team: match get("TeamIndex") {
                Some(Value::Int(t)) if t >= 0 => Some(t),
                _ => None,
            },
            script: label("AIScriptTag"),
            patrol: read_patrol(pkg, data, a, "PatrolRoute", "PatrolMode").0,
            patrol_mode: read_patrol(pkg, data, a, "PatrolRoute", "PatrolMode").1,
        };
        out.push(Placed {
            factory: Factory {
                name: name.clone(),
                // No event switches it on or off.
                tag: String::new(),
                prototype: class.clone(),
                location,
                yaw: yaw(get("Rotation").as_ref()),
                capacity: 1,
                max_alive: 1,
                spawn_mode: 2,
                start_state: offspring.start_state.clone(),
                team: offspring.team,
                offspring,
                placed: true,
                ..Factory::default()
            },
            name,
            class,
            left_out,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}
