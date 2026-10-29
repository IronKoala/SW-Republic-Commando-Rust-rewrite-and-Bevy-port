//! Delta Squad in Bevy: `SquadPlugin` puts Sev, Fixer and Scorch in the
//! game, keeps them moving with `rc_squad`, takes the squad orders from the
//! player's actions, and fills the squad panel of the on-screen display.
//!
//! It follows the rules in docs/PLUGINS.md:
//! - orders come only from `ActionState` (`SquadFormUp`, `SquadSecurePosition`,
//!   `SquadSearchAndDestroy`, `SquadCancel`, `SquadCommand`, and `Use` while
//!   aiming at a squad marker), or from a `SquadOrder` message, so keys,
//!   gamepads, touch and VR all work the same way;
//! - the leader is the entity with `Player`, the aim is the `PlayerCamera`;
//! - bodies collide with `LevelCollision`;
//! - it writes `SquadReadout`, logs to `GameLog`, and runs only while
//!   playing.
//!
//! The squad is spawned the first frame there is a player and collision.
//! The map's path network and squad markers are read when a map loads
//! (`OnEnter(GameState::Loading)`), or given with `SquadMapData::load` by a
//! program without the loading screen (like `rc_walk`).

use bevy::prelude::*;
use rc_game_core::units::{to_bevy, to_unreal};
use rc_game_core::{
    playing, Action, ActionState, CurrentLevel, GameLog, GamePaths, GameSet, GameState,
    LevelCollision, Player, PlayerCamera, SquadMember, SquadReadout, SquadStatus,
};
use rc_movement::{MoveParams, Vec3 as UVec3, World};
use rc_nav::NavGraph;
use rc_squad::{
    aimed_spot, Leader, MapSquadInfo, MemberState, Navigator, Order, Squad, SquadSettings,
    StraightLine,
};
use std::f32::consts::FRAC_PI_2;
use std::path::{Path, PathBuf};

/// The settings file, under `data/` (or a mod's folder).
pub const SETTINGS_FILE: &str = "squad/squad.toml";
/// How far the player can order a spot or marker, uu. Our own.
pub const ORDER_REACH: f32 = 4096.0;
/// How close to the aim a marker must be to be picked, radians. Our own.
pub const MARKER_AIM_ANGLE: f32 = 0.12;
/// Log every member's position this often, seconds.
pub const LOG_EVERY: f32 = 1.0;

/// An order from something other than the player's buttons: a script, a
/// touch menu, a VR gesture, or a test. Aimed orders need their spot
/// worked out already (see `rc_squad::aimed_spot`).
#[derive(Message, Debug, Clone, Copy)]
pub struct SquadOrder(pub Order);

/// The squad's state while in a level. `None` until spawned.
#[derive(Resource, Default)]
pub struct SquadState {
    pub squad: Option<Squad>,
    since_log: f32,
}

/// Enemies the squad should know about, in Unreal units. Filled by
/// whatever tracks enemies (until combat ties in, nothing does and the
/// squad only follows and holds).
#[derive(Resource, Default, Debug, Clone)]
pub struct SquadThreats(pub Vec<UVec3>);

/// What the current map tells the squad: marked jobs, where the map would
/// bring squad mates in, and its path network.
#[derive(Resource, Default)]
pub struct SquadMapData {
    pub info: MapSquadInfo,
    pub nav: Option<NavGraph>,
    /// One line per thing found, for the log.
    pub notes: Vec<String>,
}

impl SquadMapData {
    /// Read a map file. Never fails: what can't be read is noted and
    /// left out (the squad then walks straight, with no markers).
    pub fn load(path: &Path) -> SquadMapData {
        let mut out = SquadMapData::default();
        let parsed = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|d| {
                rc_package::Package::parse(&d)
                    .map(|p| (p, d))
                    .map_err(|e| e.to_string())
            });
        let (pkg, data) = match parsed {
            Ok(x) => x,
            Err(e) => {
                out.notes.push(format!(
                    "can't read {}: {e}; no markers, walking straight",
                    path.display()
                ));
                return out;
            }
        };
        match rc_squad::map::read(&pkg, &data) {
            Ok(info) => {
                out.notes.push(format!(
                    "{} squad markers, {} squad-mate spawners in the map",
                    info.markers.len(),
                    info.factories.len()
                ));
                for m in &info.markers {
                    out.notes.push(format!(
                        "  marker {} ({}, anchor {} {}) stand at ({:.0}, {:.0}, {:.0})",
                        m.name, m.job, m.anchor_class, m.anchor, m.stand.x, m.stand.y, m.stand.z
                    ));
                }
                for f in &info.factories {
                    out.notes.push(format!(
                        "  the map would bring in {} at ({:.0}, {:.0}, {:.0}) on event '{}' ({})",
                        f.class, f.location.x, f.location.y, f.location.z, f.tag, f.actor
                    ));
                }
                out.info = info;
            }
            Err(e) => out.notes.push(format!("squad markers not read: {e}")),
        }
        match NavGraph::from_map(&pkg, &data) {
            Ok(g) => {
                out.notes
                    .push(format!("path network: {} nodes", g.nodes.len()));
                out.nav = Some(g);
            }
            Err(e) => out
                .notes
                .push(format!("no path network ({e}); walking straight")),
        }
        out
    }
}

/// One squad mate's body in the world. Its `Transform` is at the feet,
/// turned to face the way they face (like the player's).
#[derive(Component, Debug)]
pub struct SquadMate {
    /// Index in `Squad::members`.
    pub index: usize,
    pub name: String,
}

/// The squad's gameplay systems (inside `GameSet::Simulate`). Order the
/// player's movement `.before(SquadSet)` so the squad sees this frame's
/// leader.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct SquadSet;

pub struct SquadPlugin;

impl Plugin for SquadPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SquadOrder>()
            .init_resource::<SquadState>()
            .init_resource::<SquadThreats>()
            .add_systems(OnEnter(GameState::Loading), load_map_data)
            .add_systems(OnExit(GameState::InGame), clear_squad)
            .add_systems(
                Update,
                (spawn_squad, take_orders, run_squad, write_readout)
                    .chain()
                    .in_set(GameSet::Simulate)
                    .in_set(SquadSet)
                    .run_if(playing),
            )
            .add_systems(
                Update,
                place_bodies
                    .in_set(GameSet::View)
                    .run_if(in_state(GameState::InGame)),
            );
    }
}

fn log_line(log: &Option<Res<GameLog>>, s: impl AsRef<str>) {
    match log {
        Some(l) => l.line(format!("squad: {}", s.as_ref())),
        None => println!("squad: {}", s.as_ref()),
    }
}

fn load_map_data(
    mut commands: Commands,
    level: Option<Res<CurrentLevel>>,
    paths: Option<Res<GamePaths>>,
    log: Option<Res<GameLog>>,
) {
    let (Some(level), Some(paths)) = (level, paths) else {
        return;
    };
    let Some(map) = level.map.as_deref() else {
        return;
    };
    let path = if map.to_ascii_lowercase().ends_with(".ctm") {
        PathBuf::from(map)
    } else {
        paths.map_path(map)
    };
    let data = SquadMapData::load(&path);
    for n in &data.notes {
        log_line(&log, n);
    }
    commands.insert_resource(data);
}

fn clear_squad(mut state: ResMut<SquadState>) {
    state.squad = None;
}

/// Read the settings file (a mod's copy first), or use the built-in values.
pub fn load_settings(paths: Option<&GamePaths>, log: &Option<Res<GameLog>>) -> SquadSettings {
    let file = match paths {
        Some(p) => p.data_file(SETTINGS_FILE),
        None => Some(PathBuf::from("data").join(SETTINGS_FILE)).filter(|p| p.exists()),
    };
    let Some(file) = file else {
        log_line(log, format!("no {SETTINGS_FILE}; using built-in values"));
        return SquadSettings::default();
    };
    match std::fs::read_to_string(&file)
        .map_err(|e| e.to_string())
        .and_then(|t| SquadSettings::default().with_overrides(&t))
    {
        Ok(s) => {
            log_line(log, format!("settings from {}", file.display()));
            s
        }
        Err(e) => {
            log_line(
                log,
                format!("ERROR in {}: {e}; using built-in values", file.display()),
            );
            SquadSettings::default()
        }
    }
}

/// The leader as the squad sees them, from the `Player` entity. The
/// player's `Transform` faces along its local -Z, like a Bevy camera.
pub fn leader_from(t: &Transform) -> Leader {
    let fwd = t.rotation * Vec3::NEG_Z;
    Leader {
        feet: to_unreal(t.translation),
        yaw: fwd.z.atan2(fwd.x),
    }
}

/// A Bevy rotation facing an Unreal yaw (the inverse of `leader_from`).
pub fn yaw_rotation(yaw: f32) -> Quat {
    Quat::from_rotation_y(-(yaw + FRAC_PI_2))
}

#[allow(clippy::too_many_arguments)]
fn spawn_squad(
    mut commands: Commands,
    mut state: ResMut<SquadState>,
    player: Query<&Transform, With<Player>>,
    collision: Option<Res<LevelCollision>>,
    map: Option<Res<SquadMapData>>,
    paths: Option<Res<GamePaths>>,
    log: Option<Res<GameLog>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    if state.squad.is_some() {
        return;
    }
    let (Ok(pt), Some(collision)) = (player.single(), collision) else {
        return;
    };
    let settings = load_settings(paths.as_deref(), &log);
    for line in settings.summary() {
        log_line(&log, line);
    }
    if settings.spawn == rc_squad::Spawn::None {
        log_line(&log, "spawn = none: no squad this time");
        state.squad = Some(
            Squad::new(
                SquadSettings {
                    members: vec![],
                    ..settings
                },
                MoveParams::player_commando(),
                leader_from(pt),
                &*collision.0,
            )
            .0,
        );
        return;
    }
    let leader = leader_from(pt);
    let (mut squad, events) = Squad::new(
        settings,
        MoveParams::player_commando(),
        leader,
        &*collision.0,
    );
    if let Some(m) = &map {
        squad.markers = m
            .info
            .markers
            .iter()
            .cloned()
            .map(|s| (s, rc_squad::MarkerState::Open))
            .collect();
    }
    for e in &events {
        log_line(&log, e.to_string());
    }
    log_line(
        &log,
        format!(
            "{} squad mates with the player at ({:.0}, {:.0}, {:.0}); {} markers; path network {}",
            squad.members.len(),
            leader.feet.x,
            leader.feet.y,
            leader.feet.z,
            squad.markers.len(),
            if map.as_ref().is_some_and(|m| m.nav.is_some()) {
                "yes"
            } else {
                "no (walking straight)"
            }
        ),
    );
    // Stand-in bodies: a capsule the commando's size in their colour, with
    // a dark visor showing which way they face. Skipped without a renderer.
    let look = match (meshes, materials) {
        (Some(mut meshes), Some(mut materials)) => {
            let radius = 40.0 / rc_game_core::units::UU_PER_UNIT;
            let height = 168.0 / rc_game_core::units::UU_PER_UNIT;
            let body = meshes.add(Capsule3d::new(radius, height - 2.0 * radius));
            let visor = meshes.add(Cuboid::new(0.55, 0.22, 0.3));
            let visor_mat = materials.add(StandardMaterial {
                base_color: Color::srgb(0.05, 0.05, 0.06),
                perceptual_roughness: 0.3,
                ..default()
            });
            let mats: Vec<_> = squad
                .members
                .iter()
                .map(|m| {
                    let [r, g, b] = m.settings.colour;
                    materials.add(StandardMaterial {
                        base_color: Color::srgb(r, g, b),
                        perceptual_roughness: 0.7,
                        ..default()
                    })
                })
                .collect();
            Some((body, visor, visor_mat, mats, height))
        }
        _ => None,
    };
    for (i, m) in squad.members.iter().enumerate() {
        let mut e = commands.spawn((
            SquadMate {
                index: i,
                name: m.settings.name.clone(),
            },
            Transform::from_translation(to_bevy(m.pawn.feet())).with_rotation(yaw_rotation(m.yaw)),
            Visibility::default(),
            DespawnOnExit(GameState::InGame),
            Name::new(m.settings.name.clone()),
        ));
        if let Some((body, visor, visor_mat, mats, height)) = &look {
            e.with_children(|c| {
                c.spawn((
                    Mesh3d(body.clone()),
                    MeshMaterial3d(mats[i].clone()),
                    Transform::from_xyz(0.0, height / 2.0, 0.0),
                ));
                c.spawn((
                    Mesh3d(visor.clone()),
                    MeshMaterial3d(visor_mat.clone()),
                    Transform::from_xyz(0.0, height - 0.45, -0.72),
                ));
            });
        }
    }
    state.squad = Some(squad);
}

/// The player's eye and aim, in Unreal units.
fn aim(camera: &Query<&GlobalTransform, With<PlayerCamera>>) -> Option<(UVec3, UVec3)> {
    let g = camera.single().ok()?;
    let d = g.forward();
    Some((to_unreal(g.translation()), UVec3::new(d.x, d.z, d.y)))
}

#[allow(clippy::too_many_arguments)]
fn take_orders(
    mut state: ResMut<SquadState>,
    actions: Option<Res<ActionState>>,
    mut orders: MessageReader<SquadOrder>,
    player: Query<&Transform, With<Player>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    collision: Option<Res<LevelCollision>>,
    log: Option<Res<GameLog>>,
) {
    let (Some(squad), Ok(pt)) = (state.squad.as_mut(), player.single()) else {
        orders.clear();
        return;
    };
    let leader = leader_from(pt);
    let mut wanted: Vec<Order> = orders.read().map(|o| o.0).collect();
    if let Some(a) = actions {
        let aimed = || -> Option<(UVec3, UVec3)> { aim(&camera) };
        let spot = |eye: UVec3, dir: UVec3| {
            collision.as_ref().and_then(|c| {
                aimed_spot(&*c.0, eye, dir, ORDER_REACH, &MoveParams::player_commando())
            })
        };
        if a.just_pressed(Action::SquadFormUp) {
            wanted.push(Order::FormUp);
        }
        if a.just_pressed(Action::SquadSearchAndDestroy) {
            wanted.push(Order::SearchAndDestroy);
        }
        if a.just_pressed(Action::SquadCancel) {
            wanted.push(Order::Cancel);
        }
        // Aimed orders: a marker under the aim takes priority for the
        // context order (and for Use); otherwise the spot aimed at.
        let context = a.just_pressed(Action::SquadCommand);
        let secure = a.just_pressed(Action::SquadSecurePosition);
        let use_key = a.just_pressed(Action::Use);
        if context || secure || use_key {
            match aimed() {
                None => log_line(&log, "aimed order ignored: no player camera"),
                Some((eye, dir)) => {
                    let marker = squad.marker_in_aim(eye, dir, ORDER_REACH, MARKER_AIM_ANGLE);
                    if let (Some(k), true) = (marker, context || use_key) {
                        wanted.push(Order::Marker(k));
                    } else if context || secure {
                        match spot(eye, dir) {
                            Some(p) => wanted.push(Order::SecurePosition(p)),
                            None => log_line(
                                &log,
                                "secure position ignored: nothing in reach under the aim",
                            ),
                        }
                    }
                }
            }
        }
    }
    let Some(collision) = collision else {
        return;
    };
    for o in wanted {
        for e in squad.command(o, leader, &*collision.0) {
            log_line(&log, format!("t={:.2} {e}", squad.time));
        }
    }
}

fn run_squad(
    time: Res<Time>,
    mut state: ResMut<SquadState>,
    player: Query<&Transform, With<Player>>,
    collision: Option<Res<LevelCollision>>,
    map: Option<Res<SquadMapData>>,
    threats: Res<SquadThreats>,
    log: Option<Res<GameLog>>,
) {
    let (Ok(pt), Some(collision)) = (player.single(), collision) else {
        return;
    };
    let leader = leader_from(pt);
    let dt = time.delta_secs().min(0.25);
    let state = &mut *state;
    let Some(squad) = state.squad.as_mut() else {
        return;
    };
    squad.threats.clone_from(&threats.0);
    let world: &dyn World = &*collision.0;
    let nav: &dyn Navigator = match map.as_ref().and_then(|m| m.nav.as_ref()) {
        Some(g) => g,
        None => &StraightLine,
    };
    let events = squad.update(dt, leader, world, nav);
    for e in &events {
        log_line(&log, format!("t={:.2} {e}", squad.time));
    }
    state.since_log += dt;
    if state.since_log >= LOG_EVERY {
        state.since_log = 0.0;
        log_line(&log, status_line(squad, leader));
    }
}

/// One line with everyone's position and state, for the log.
pub fn status_line(squad: &Squad, leader: Leader) -> String {
    let mut s = format!(
        "t={:.2} stance {} | leader ({:.0}, {:.0}, {:.0})",
        squad.time,
        squad.stance_settings().name,
        leader.feet.x,
        leader.feet.y,
        leader.feet.z
    );
    for m in squad.status(leader) {
        let state = match m.state {
            MemberState::Idle => "idle".to_string(),
            MemberState::Moving { urgent: true, .. } => "running (urgent)".to_string(),
            MemberState::Moving { .. } => "moving".to_string(),
            MemberState::Working { left } => format!("working {left:.1}s"),
        };
        s.push_str(&format!(
            " | {} ({:.0}, {:.0}, {:.0}) {:.0} uu from leader, {:.0} to spot, speed {:.0}, {}{}",
            m.name,
            m.feet.x,
            m.feet.y,
            m.feet.z,
            m.leader_distance,
            m.goal_distance,
            m.speed,
            state,
            if m.on_ground { "" } else { ", in the air" }
        ));
    }
    s
}

fn write_readout(
    state: Res<SquadState>,
    player: Query<&Transform, With<Player>>,
    mut readout: Option<ResMut<SquadReadout>>,
) {
    let (Some(squad), Ok(pt), Some(readout)) =
        (state.squad.as_ref(), player.single(), readout.as_mut())
    else {
        return;
    };
    let members: Vec<SquadMember> = squad
        .status(leader_from(pt))
        .into_iter()
        .map(|m| SquadMember {
            name: m.name,
            number: m.number,
            // Full until combat ties in.
            health: 1.0,
            shield: 1.0,
            status: SquadStatus::Ok,
            order: m.doing,
        })
        .collect();
    if readout.members != members {
        readout.members = members;
    }
}

fn place_bodies(state: Res<SquadState>, mut mates: Query<(&SquadMate, &mut Transform)>) {
    let Some(squad) = state.squad.as_ref() else {
        return;
    };
    for (mate, mut t) in &mut mates {
        if let Some(m) = squad.members.get(mate.index) {
            t.translation = to_bevy(m.pawn.feet());
            t.rotation = yaw_rotation(m.yaw);
        }
    }
}
