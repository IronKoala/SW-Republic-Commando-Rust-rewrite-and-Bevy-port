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
//! - each squad mate is an `rc_combat` target (`Health`, `Shield`, `Team`,
//!   `Hittable`) with the original's numbers from `data/squad/squad.toml`;
//!   at 0 health they go down (squad mates can't die) and stop;
//! - they fight: every living `Team::Separatist` character is a threat
//!   (`SquadThreats`), and their shots are `rc_combat::FireShot`, so the
//!   weapons part flies the bolts and does the damage;
//! - downed commandos are revived: squad mates go to each other and to the
//!   player (when the player has `Dead`), and the player revives a mate by
//!   holding `Use` next to them. A revive gives health back and removes
//!   `Dead`;
//! - marked jobs ("context orders", `[job.*]` in `squad.toml`) have
//!   results: bacta gives the squad mate their health and shield back,
//!   charges and grenades send `rc_combat::Damage` to everyone in the
//!   blast, and a finished breach, slice or demolition sends
//!   `SquadJobDone` for whatever owns the anchor (the level's scripts
//!   and doors) to act on;
//! - it writes `SquadReadout`, logs to `GameLog`, and runs only while
//!   playing.
//!
//! The squad is spawned the first frame there is a player and collision.
//! The map's path network and squad markers are read when a map loads
//! (`OnEnter(GameState::Loading)`), or given with `SquadMapData::load` by a
//! program without the loading screen (like `rc_walk`).

pub mod look;
pub mod save;

use bevy::prelude::*;
use rc_combat::{
    CombatSet, Damage, DamageKind, DamageTable, Dead, Died, FireShot, Health, Hittable, Shield,
    ShotLanded, Team, WeaponLibrary,
};
use rc_game_core::save::SaveAppExt;
use rc_game_core::units::{to_bevy, to_unreal};
use rc_game_core::{
    playing, Action, ActionState, CurrentLevel, GameLog, GamePaths, GameSet, GameState,
    LevelCollision, Player, PlayerCamera, SquadMember, SquadReadout, SquadStatus,
};
use rc_movement::{MoveParams, Vec3 as UVec3, World};
use rc_nav::NavGraph;
use rc_squad::{
    aimed_spot, JobResult, Leader, MapSquadInfo, MemberState, Navigator, Order, Patient, Squad,
    SquadEvent, SquadSettings, StraightLine, Threat,
};
use std::collections::HashMap;
use std::f32::consts::FRAC_PI_2;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The settings file, under `data/` (or a mod's folder).
pub const SETTINGS_FILE: &str = "squad/squad.toml";
/// How far the player can order a spot or marker, uu. Our own.
pub const ORDER_REACH: f32 = 4096.0;
/// How close to the aim a marker must be to be picked, radians. Our own.
pub const MARKER_AIM_ANGLE: f32 = 0.12;
/// Log every member's position this often, seconds.
pub const LOG_EVERY: f32 = 1.0;
/// Log the squad's fight numbers this often, seconds.
pub const FIGHT_LOG_EVERY: f32 = 5.0;

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
    /// Post weapons not found, logged once each.
    missing_weapons: std::collections::BTreeSet<String>,
}

/// Player options that change how the squad takes orders. The game sets
/// it from its settings file and options menu; missing means the defaults.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct SquadOptions {
    /// The original's AUTO PULL MANEUVERS (GAME options page, on by
    /// default): a new order that needs more squad mates than are free
    /// takes them off sniper, grenade and anti-armour posts. Off, the
    /// order is refused until the squad is recalled.
    pub auto_pull_maneuvers: bool,
}

impl Default for SquadOptions {
    fn default() -> Self {
        SquadOptions {
            auto_pull_maneuvers: true,
        }
    }
}

/// Enemies the squad knows about this frame, in Unreal units. Rebuilt
/// every frame from the living `Team::Separatist` characters that have a
/// `Hittable` body; `Threat::id` is the entity's bits.
#[derive(Resource, Default, Debug, Clone)]
pub struct SquadThreats(pub Vec<Threat>);

/// The squad's fight so far, per member (in `Squad::members` order), for
/// the log and tests.
#[derive(Resource, Default, Debug, Clone)]
pub struct SquadFightStats {
    pub shots: Vec<u32>,
    /// Shots that landed on an enemy.
    pub hits: Vec<u32>,
    pub kills: Vec<u32>,
    /// Revives done: by each member, and by the player.
    pub revives: Vec<u32>,
    pub player_revives: u32,
    /// Seconds from going down to back up, every revive.
    pub revive_waits: Vec<f32>,
    /// Squad time each commando went down (index = member, last = player).
    down_since: Vec<Option<f32>>,
    since_log: f32,
}

impl SquadFightStats {
    fn sized(n: usize) -> Self {
        SquadFightStats {
            shots: vec![0; n],
            hits: vec![0; n],
            kills: vec![0; n],
            revives: vec![0; n],
            down_since: vec![None; n + 1],
            ..Default::default()
        }
    }
}

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

/// A squad mate finished a job that sets something off: a door breached
/// or sliced open, a demolition charge blown. `anchor` is the map actor
/// the job was for (like `DemolitionCharge2`); the level's scripts and
/// doors listen for it. Positions are in Bevy space.
#[derive(Message, Debug, Clone)]
pub struct SquadJobDone {
    pub job: String,
    pub marker: String,
    pub anchor: String,
    pub anchor_class: String,
    pub at: Vec3,
    /// Who did it.
    pub member: String,
}

/// Job results waiting for `carry_out_jobs` (filled by `run_squad`).
#[derive(Resource, Default, Debug)]
pub struct SquadJobQueue(pub Vec<SquadEvent>);

/// The squad mates' models, read from the game the first time a squad
/// is spawned and kept between maps. `None`: couldn't be read (the mate
/// gets the stand-in body).
#[derive(Resource, Default)]
pub struct SquadLooks {
    tried: bool,
    pub ambient: f32,
    pub blend: look::BlendTimes,
    pub models: HashMap<String, Option<Arc<rc_model::Model>>>,
}

impl SquadLooks {
    fn load(&mut self, paths: Option<&GamePaths>, keys: &[String], log: &Option<Res<GameLog>>) {
        if self.tried {
            return;
        }
        self.tried = true;
        let (looks, file) = match look::LooksFile::read(paths) {
            Ok(x) => x,
            Err(e) => {
                log_line(log, format!("{e}; squad mates get stand-in bodies"));
                return;
            }
        };
        log_line(log, format!("looks from {}", file.display()));
        self.ambient = looks.ambient;
        self.blend = looks.blend;
        log_line(
            log,
            format!(
                "animation blend times: {:.2} s, into firing {:.2} s, into falling down {:.2} s",
                looks.blend.time, looks.blend.fire, looks.blend.down
            ),
        );
        let Some(paths) = paths else {
            log_line(log, "no game folder; squad mates get stand-in bodies");
            return;
        };
        for key in keys {
            let t = std::time::Instant::now();
            let model = match look::load_model(paths, &looks, key) {
                Ok((m, notes)) => {
                    for n in notes {
                        log_line(log, format!("{key}: {n}"));
                    }
                    log_line(
                        log,
                        format!(
                            "{key}: model read in {:.0} ms",
                            t.elapsed().as_secs_f64() * 1000.0
                        ),
                    );
                    Some(Arc::new(m))
                }
                Err(e) => {
                    log_line(
                        log,
                        format!(
                            "{key}: model not loaded, using the stand-in body (placeholder): {e}"
                        ),
                    );
                    None
                }
            };
            self.models.insert(key.clone(), model);
        }
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
            .add_message::<rc_game_core::SquadStanceOrder>()
            .add_message::<SquadJobDone>()
            .init_resource::<SquadJobQueue>()
            .init_resource::<SquadState>()
            .init_resource::<SquadOptions>()
            .init_resource::<SquadThreats>()
            .init_resource::<SquadFightStats>()
            .init_resource::<SquadLooks>()
            .add_save_part::<save::SquadSave>()
            .add_systems(OnEnter(GameState::Loading), load_map_data)
            .add_systems(
                Update,
                save::apply_pending
                    .in_set(GameSet::Simulate)
                    .in_set(SquadSet)
                    .after(spawn_squad)
                    .before(check_down)
                    .run_if(resource_exists::<save::PendingSquadSave>),
            )
            .add_systems(OnExit(GameState::InGame), clear_squad)
            .add_systems(
                Update,
                (
                    spawn_squad,
                    check_down,
                    take_orders,
                    gather_threats,
                    run_squad,
                    carry_out_jobs,
                    write_readout,
                )
                    .chain()
                    .in_set(GameSet::Simulate)
                    .in_set(SquadSet)
                    .before(CombatSet::Fire)
                    .run_if(playing),
            )
            .add_systems(
                Update,
                count_hits
                    .in_set(GameSet::Simulate)
                    .after(CombatSet::ApplyDamage)
                    .run_if(playing),
            )
            .add_systems(
                Update,
                (place_bodies, look::animate)
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

fn clear_squad(
    mut state: ResMut<SquadState>,
    stats: Res<SquadFightStats>,
    log: Option<Res<GameLog>>,
) {
    if let Some(squad) = &state.squad {
        log_line(&log, fight_line(squad, &stats));
    }
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
    images: Option<ResMut<Assets<Image>>>,
    mut looks: ResMut<SquadLooks>,
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
        squad.set_markers(m.info.markers.clone());
        for k in 0..squad.markers.len() {
            let spot = &squad.markers[k].0;
            log_line(
                &log,
                format!(
                    "marker {} -> {}",
                    spot.name,
                    squad
                        .job_of(k)
                        .map_or("no job matches (generic job)".into(), |j| j.summary())
                ),
            );
        }
    }
    for e in &events {
        log_line(&log, e.to_string());
    }
    commands.insert_resource(SquadFightStats::sized(squad.members.len()));
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
    // The original models where they could be read; otherwise a stand-in
    // body (placeholder): a capsule the commando's size in their colour,
    // with a dark visor showing which way they face. Nothing is drawn
    // without a renderer.
    let drawing = meshes.is_some() && materials.is_some() && images.is_some();
    if drawing {
        let keys: Vec<String> = squad
            .members
            .iter()
            .map(|m| m.settings.key.clone())
            .collect();
        looks.load(paths.as_deref(), &keys, &log);
    }
    let mut assets = match (meshes, materials, images) {
        (Some(meshes), Some(materials), Some(images)) => Some((meshes, materials, images)),
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
            Health::new(m.settings.max_health),
            Shield::new(
                m.settings.max_shields,
                m.settings.shield_recharge_rate,
                m.settings.shield_recharge_delay,
            ),
            Team::Republic,
            // Level scripts name squad mates by their class
            // (`MakeInvulnerable Tag=Commando62Delta`). Their own scripted
            // walks aren't carried out yet.
            rc_game_core::ScriptTags {
                tag: Some(m.settings.class.clone()),
                script: None,
                follows_orders: false,
            },
            Hittable::COMMANDO,
            // The original's squad damage table: droid bolts x0.15,
            // explosions x0.4, stuns x0.
            DamageTable("DV_OrganicCTSquadDamage".into()),
        ));
        let Some((meshes, materials, images)) = assets.as_mut() else {
            continue;
        };
        if let Some(Some(model)) = looks.models.get(&m.settings.key) {
            let mut mm = look::spawn_pieces(
                model.clone(),
                looks.ambient,
                &mut e,
                meshes,
                materials,
                images,
            );
            mm.blend_times = looks.blend;
            e.insert(mm);
        } else {
            stand_in_body(&mut e, m.settings.colour, meshes, materials);
        }
    }
    state.squad = Some(squad);
}

/// The placeholder body for a squad mate whose model couldn't be read.
fn stand_in_body(
    e: &mut EntityCommands,
    colour: [f32; 3],
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let radius = 40.0 / rc_game_core::units::UU_PER_UNIT;
    let height = 168.0 / rc_game_core::units::UU_PER_UNIT;
    let body = meshes.add(Capsule3d::new(radius, height - 2.0 * radius));
    let visor = meshes.add(Cuboid::new(0.55, 0.22, 0.3));
    let visor_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.05, 0.05, 0.06),
        perceptual_roughness: 0.3,
        ..default()
    });
    let [r, g, b] = colour;
    let mat = materials.add(StandardMaterial {
        base_color: Color::srgb(r, g, b),
        perceptual_roughness: 0.7,
        ..default()
    });
    e.with_children(|c| {
        c.spawn((
            Mesh3d(body),
            MeshMaterial3d(mat),
            Transform::from_xyz(0.0, height / 2.0, 0.0),
        ));
        c.spawn((
            Mesh3d(visor),
            MeshMaterial3d(visor_mat),
            Transform::from_xyz(0.0, height - 0.45, -0.72),
        ));
    });
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
    mut from_scripts: MessageReader<rc_game_core::SquadStanceOrder>,
    player: Query<&Transform, With<Player>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    collision: Option<Res<LevelCollision>>,
    options: Option<Res<SquadOptions>>,
    log: Option<Res<GameLog>>,
) {
    let (Some(squad), Ok(pt)) = (state.squad.as_mut(), player.single()) else {
        orders.clear();
        from_scripts.clear();
        return;
    };
    let auto_pull = options.map_or(true, |o| o.auto_pull_maneuvers);
    if squad.auto_pull != auto_pull {
        squad.auto_pull = auto_pull;
        log_line(
            &log,
            format!("auto pull maneuvers {}", if auto_pull { "on" } else { "off" }),
        );
    }
    let leader = leader_from(pt);
    let mut wanted: Vec<Order> = orders.read().map(|o| o.0).collect();
    // The level's scripts give squad orders too.
    for o in from_scripts.read() {
        use rc_game_core::SquadStanceOrder as S;
        let order = match *o {
            S::FormUp => Order::FormUp,
            S::SearchAndDestroy => Order::SearchAndDestroy,
            S::SecurePosition(p) => Order::SecurePosition(UVec3::new(p[0], p[1], p[2])),
        };
        log_line(&log, format!("the level's script orders: {order}"));
        wanted.push(order);
    }
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

/// Every living enemy with a body, for the squad to fight.
fn gather_threats(
    mut threats: ResMut<SquadThreats>,
    enemies: Query<(Entity, &Transform, &Team, &Hittable), Without<Dead>>,
) {
    threats.0.clear();
    for (e, t, team, hit) in &enemies {
        if *team != Team::Separatist {
            continue;
        }
        let feet = to_unreal(t.translation);
        let centre = feet + UVec3::new(0.0, 0.0, hit.center_up);
        threats.0.push(Threat {
            id: e.to_bits(),
            feet,
            centre,
            // Eyes near the top of the body (a battle droid's are 60 uu
            // above its middle, a commando's 64).
            eye: centre + UVec3::new(0.0, 0.0, hit.half_height * 0.75),
        });
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run_squad(
    time: Res<Time>,
    mut state: ResMut<SquadState>,
    mut player: Query<(Entity, &Transform, Option<&mut Health>, Has<Dead>), With<Player>>,
    mut mates: Query<(Entity, &SquadMate, &mut Health), Without<Player>>,
    actions: Option<Res<ActionState>>,
    collision: Option<Res<LevelCollision>>,
    map: Option<Res<SquadMapData>>,
    threats: Res<SquadThreats>,
    mut stats: ResMut<SquadFightStats>,
    mut fire: MessageWriter<FireShot>,
    mut jobs: ResMut<SquadJobQueue>,
    library: Option<Res<WeaponLibrary>>,
    mut commands: Commands,
    log: Option<Res<GameLog>>,
) {
    let (Ok((player_e, pt, mut player_health, player_dead)), Some(collision)) =
        (player.single_mut(), collision)
    else {
        return;
    };
    let leader = leader_from(pt);
    let dt = time.delta_secs().min(0.25);
    let state = &mut *state;
    let Some(squad) = state.squad.as_mut() else {
        return;
    };
    squad.threats.clone_from(&threats.0);
    if squad.leader_down != player_dead {
        squad.leader_down = player_dead;
        let n = stats.down_since.len();
        if n > 0 {
            stats.down_since[n - 1] = player_dead.then_some(squad.time);
        }
        log_line(
            &log,
            format!(
                "t={:.2} the player is {}",
                squad.time,
                if player_dead { "DOWN" } else { "back up" }
            ),
        );
    }
    let world: &dyn World = &*collision.0;
    let nav: &dyn Navigator = match map.as_ref().and_then(|m| m.nav.as_ref()) {
        Some(g) => g,
        None => &StraightLine,
    };
    let holding_use = actions.as_ref().is_some_and(|a| a.pressed(Action::Use));
    let mut events = squad.player_use(dt, leader, holding_use);
    events.extend(squad.update(dt, leader, world, nav));
    let usual_weapon = squad.settings.fight.weapon.clone();
    let revive_health = squad.settings.revive.health.clamp(0.01, 1.0);
    for e in &events {
        match e {
            SquadEvent::Fire {
                index,
                origin,
                direction,
                burst_start,
                weapon,
                ..
            } => {
                // A post's own gun, if the weapons part has it.
                let weapon = match &library {
                    Some(lib) if !lib.weapons.contains_key(weapon) => {
                        if state.missing_weapons.insert(weapon.clone()) {
                            log_line(
                                &log,
                                format!("no weapon file `{weapon}`; using {usual_weapon} instead"),
                            );
                        }
                        &usual_weapon
                    }
                    _ => weapon,
                };
                if let Some((entity, _, _)) = mates.iter().find(|(_, m, _)| m.index == *index) {
                    fire.write(FireShot {
                        shooter: entity,
                        origin: to_bevy(*origin),
                        direction: Vec3::new(direction.x, direction.z, direction.y),
                        weapon: weapon.clone(),
                    });
                    if let Some(n) = stats.shots.get_mut(*index) {
                        *n += 1;
                    }
                }
                if *burst_start {
                    log_line(&log, format!("t={:.2} {e}", squad.time));
                }
            }
            SquadEvent::HeldFire { .. } => log_line(&log, format!("t={:.2} {e}", squad.time)),
            SquadEvent::JobResult { .. } | SquadEvent::Blast { .. } => {
                log_line(&log, format!("t={:.2} {e}", squad.time));
                jobs.0.push(e.clone());
            }
            SquadEvent::ReviveDone {
                member, patient, ..
            } => {
                log_line(&log, format!("t={:.2} {e}", squad.time));
                let by = squad
                    .members
                    .iter()
                    .position(|m| &m.settings.name == member);
                match by {
                    Some(i) => stats.revives[i] += 1,
                    None => stats.player_revives += 1,
                }
                let slot = match patient {
                    Patient::Mate(i) => *i,
                    Patient::Leader => stats.down_since.len().saturating_sub(1),
                };
                if let Some(since) = stats.down_since.get_mut(slot).and_then(|s| s.take()) {
                    let wait = squad.time - since;
                    stats.revive_waits.push(wait);
                    log_line(
                        &log,
                        format!(
                            "t={:.2} {} was down for {wait:.1} s",
                            squad.time,
                            e_patient(e)
                        ),
                    );
                }
                // Health back and `Dead` gone: the game sees them up again.
                match patient {
                    Patient::Mate(i) => {
                        for (entity, mate, mut h) in &mut mates {
                            if mate.index == *i {
                                h.current = h.max * revive_health;
                                commands.entity(entity).remove::<Dead>();
                            }
                        }
                    }
                    Patient::Leader => {
                        if let Some(h) = player_health.as_mut() {
                            h.current = h.max * revive_health;
                        }
                        commands.entity(player_e).remove::<Dead>();
                    }
                }
            }
            _ => log_line(&log, format!("t={:.2} {e}", squad.time)),
        }
    }
    state.since_log += dt;
    if state.since_log >= LOG_EVERY {
        state.since_log = 0.0;
        log_line(&log, status_line(squad, leader));
    }
    stats.since_log += dt;
    if stats.since_log >= FIGHT_LOG_EVERY {
        stats.since_log = 0.0;
        log_line(&log, fight_line(squad, &stats));
    }
}

/// Carry out finished jobs: bacta heals the squad mate, blasts hurt
/// everyone near them (friend or foe, as in the original), and jobs that
/// set off their anchor send `SquadJobDone`.
#[allow(clippy::type_complexity)]
fn carry_out_jobs(
    mut jobs: ResMut<SquadJobQueue>,
    mut mates: Query<(Entity, &SquadMate, &mut Health, Option<&mut Shield>)>,
    bodies: Query<(Entity, &Transform, &Hittable), Without<Dead>>,
    mut damage: MessageWriter<Damage>,
    mut done: MessageWriter<SquadJobDone>,
    log: Option<Res<GameLog>>,
) {
    for e in jobs.0.drain(..) {
        match e {
            SquadEvent::JobResult {
                member,
                index,
                marker,
                job,
                result,
                anchor,
                anchor_class,
                at,
            } => match result {
                JobResult::Heal => {
                    for (_, mate, mut h, shield) in &mut mates {
                        if mate.index == index {
                            h.current = h.max;
                            if let Some(mut s) = shield {
                                s.current = s.max;
                            }
                            log_line(&log, format!("{member} healed to {:.0} at {marker}", h.max));
                        }
                    }
                }
                JobResult::Trigger => {
                    log_line(
                        &log,
                        format!("{job} done: sets off {anchor_class} {anchor} (SquadJobDone)"),
                    );
                    done.write(SquadJobDone {
                        job,
                        marker,
                        anchor,
                        anchor_class,
                        at: to_bevy(at),
                        member,
                    });
                }
                JobResult::Nothing => {}
            },
            SquadEvent::Blast {
                index,
                at,
                damage: amount,
                radius,
                what,
                ..
            } => {
                let by = index.and_then(|i| {
                    mates
                        .iter()
                        .find(|(_, m, _, _)| m.index == i)
                        .map(|(e, ..)| e)
                });
                let mut hit = 0;
                for (target, t, body) in &bodies {
                    let centre = to_unreal(t.translation) + UVec3::new(0.0, 0.0, body.center_up);
                    // Distance to the body's edge, not its middle.
                    let d = ((centre - at).length() - body.radius).max(0.0);
                    if radius <= 0.0 || d >= radius {
                        continue;
                    }
                    let falloff = 1.0 - d / radius;
                    let dir = (centre - at).normalize_or_zero();
                    damage.write(Damage {
                        target,
                        amount: amount * falloff,
                        kind: DamageKind::Explosion,
                        instigator: by,
                        point: to_bevy(centre),
                        direction: Vec3::new(dir.x, dir.z, dir.y),
                    });
                    hit += 1;
                }
                log_line(&log, format!("{what} blast hurt {hit} bodies"));
            }
            _ => {}
        }
    }
}

fn e_patient(e: &SquadEvent) -> String {
    match e {
        SquadEvent::ReviveDone { patient_name, .. } => patient_name.clone(),
        _ => String::new(),
    }
}

/// Hits and kills by squad mates, from the weapons part's messages.
fn count_hits(
    state: Res<SquadState>,
    mut stats: ResMut<SquadFightStats>,
    mut landed: MessageReader<ShotLanded>,
    mut died: MessageReader<Died>,
    mates: Query<&SquadMate>,
    players: Query<(), With<Player>>,
    names: Query<&Name>,
    log: Option<Res<GameLog>>,
) {
    let time = state.squad.as_ref().map_or(0.0, |s| s.time);
    for l in landed.read() {
        let (Ok(m), Some(t)) = (mates.get(l.shooter), l.target) else {
            continue;
        };
        if let Ok(other) = mates.get(t) {
            log_line(
                &log,
                format!(
                    "t={time:.2} {} hit a squad mate ({}) by mistake",
                    m.name, other.name
                ),
            );
            continue;
        }
        if players.contains(t) {
            log_line(
                &log,
                format!("t={time:.2} {} hit the player by mistake", m.name),
            );
            continue;
        }
        if let Some(n) = stats.hits.get_mut(m.index) {
            *n += 1;
        }
    }
    for d in died.read() {
        if mates.contains(d.entity) {
            if let Some(slot) = stats.down_since.get_mut(mates.get(d.entity).unwrap().index) {
                *slot = Some(time);
            }
            continue;
        }
        let Some(m) = d.killer.and_then(|k| mates.get(k).ok()) else {
            continue;
        };
        if let Some(n) = stats.kills.get_mut(m.index) {
            *n += 1;
        }
        let who = names
            .get(d.entity)
            .map_or_else(|_| format!("{:?}", d.entity), |n| n.to_string());
        log_line(&log, format!("t={time:.2} {} KILLED {who}", m.name));
    }
}

/// The squad's fight numbers so far, for the log.
pub fn fight_line(squad: &Squad, stats: &SquadFightStats) -> String {
    let mut s = format!("t={:.2} fight:", squad.time);
    for (i, m) in squad.members.iter().enumerate() {
        s.push_str(&format!(
            " {} {} shots, {} hits, {} kills, {} revives, held fire {} times;",
            m.settings.name,
            stats.shots.get(i).copied().unwrap_or(0),
            stats.hits.get(i).copied().unwrap_or(0),
            stats.kills.get(i).copied().unwrap_or(0),
            stats.revives.get(i).copied().unwrap_or(0),
            m.fight.held_fire
        ));
    }
    let waits = &stats.revive_waits;
    s.push_str(&format!(
        " player revives {}; revives done {}{}",
        stats.player_revives,
        waits.len(),
        if waits.is_empty() {
            String::new()
        } else {
            format!(
                ", down for {:.1}-{:.1} s",
                waits.iter().copied().fold(f32::INFINITY, f32::min),
                waits.iter().copied().fold(0.0, f32::max)
            )
        }
    ));
    s
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
            MemberState::Reviving { left } => format!("reviving {left:.1}s"),
            MemberState::Ready => "ready at marker".to_string(),
            MemberState::Posted => "holding post".to_string(),
        };
        let state = if m.down { "DOWN".to_string() } else { state };
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

/// Health at 0 (or `Dead` from combat) puts a squad mate down; health
/// back above 0 with `Dead` gone (a revive) brings them back.
fn check_down(
    mut state: ResMut<SquadState>,
    mates: Query<(&SquadMate, &Health, Has<Dead>)>,
    log: Option<Res<GameLog>>,
) {
    let Some(squad) = state.squad.as_mut() else {
        return;
    };
    for (mate, health, dead) in &mates {
        let down = dead || health.is_dead();
        if let Some(e) = squad.set_down(mate.index, down) {
            log_line(&log, format!("t={:.2} {e}", squad.time));
        }
    }
}

fn write_readout(
    state: Res<SquadState>,
    player: Query<&Transform, With<Player>>,
    mates: Query<(&SquadMate, Option<&Health>, Option<&Shield>)>,
    mut readout: Option<ResMut<SquadReadout>>,
) {
    let (Some(squad), Ok(pt), Some(readout)) =
        (state.squad.as_ref(), player.single(), readout.as_mut())
    else {
        return;
    };
    let mut bars = vec![(1.0, 1.0); squad.members.len()];
    for (mate, health, shield) in &mates {
        if let Some(b) = bars.get_mut(mate.index) {
            let frac = |cur: f32, max: f32| {
                if max > 0.0 {
                    (cur / max).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            };
            *b = (
                health.map_or(1.0, |h| frac(h.current, h.max)),
                shield.map_or(0.0, |s| frac(s.current, s.max)),
            );
        }
    }
    let members: Vec<SquadMember> = squad
        .status(leader_from(pt))
        .into_iter()
        .zip(bars)
        .map(|(m, (health, shield))| SquadMember {
            name: m.name,
            number: m.number,
            health,
            shield,
            status: if m.down {
                SquadStatus::Incapacitated
            } else {
                SquadStatus::Ok
            },
            order: if m.down { "Down".into() } else { m.doing },
        })
        .collect();
    if readout.members != members {
        readout.members = members;
    }
}

fn place_bodies(
    state: Res<SquadState>,
    mut mates: Query<(&SquadMate, &mut Transform), Without<Player>>,
) {
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
