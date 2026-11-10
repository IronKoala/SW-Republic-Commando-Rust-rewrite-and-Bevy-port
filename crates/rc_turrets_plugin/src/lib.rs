//! Mounted turrets in the game. `TurretsPlugin`:
//!
//! - reads the map's turrets while the level loads (every class a file in
//!   `data/turrets/` claims; `rc_turrets::map`), and draws each with its
//!   own model from the game's `turrets.ukx`, its gun turned where it
//!   points;
//! - lets the player get on with **Use** when close to a usable turret
//!   and facing it ("PRESS USE TO MOUNT TURRET"), and off again with
//!   **Use**. On the turret the player's body is held above it, the look
//!   input turns the gun (at the turret's speed, within its limits), and
//!   the turret's own weapon is in hand (`gunner.weapon`, fired by
//!   `rc_combat` like any other weapon). The turret's model is hidden
//!   meanwhile, as in the original, where the first-person turret shows
//!   instead;
//! - sends the turret's events into the level scripts: `PlayerControlEvent`
//!   and `ControlEvent` on getting on, `ReleaseEvent` on getting off;
//! - carries out the scripts' [`TurretOrder`]s: `ACTION_AttachToTurret`
//!   puts the characters running a script on a turret (its `ControlEvent`
//!   goes off; they shoot at enemies in range with the turret's gun), and
//!   `ACTION_SetTurretUsable` lets the player use a turret or not;
//! - puts squad mates on turrets: one the player sends to a turret marker
//!   ("Man turret") gets on when they reach it (the squad part asks with
//!   [`TurretOrder::Squad`]) and works it like a script's character,
//!   aiming at the nearest enemy in range and firing the turret's gun,
//!   until recalled (F4), sent elsewhere, or down. While on, the squad
//!   mate carries [`OnTurret`], which the squad part reads to hold the
//!   body there and keep their own gun quiet;
//! - saves where each turret points, who is on it and whether the player
//!   may use it (`[turrets]`).
//!
//! The rules themselves (limits, speed, events) are in `rc_turrets`, with
//! no Bevy. The camera and the player's body only read [`Manning`], so a VR
//! mod can aim the turret from a tracked head or hand by writing the look
//! input, without touching this part.
//!
//! **Not done yet:** turrets you can shoot (they have 50 health), turrets
//! blocking the player's way, the turret firing and turning sounds, and
//! the original's automatic turrets (ceiling and core ship turrets).

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rc_anim::math::{Quat as AQuat, Transform as ATransform};
use rc_combat::{Arsenal, Dead, FireShot, Hittable, Team, WeaponLibrary, WeaponState};
use rc_enemies::look::{DroidLook, Pose};
use rc_game_core::save::{SaveAppExt, SavePart};
use rc_game_core::units::{to_bevy, to_unreal};
use rc_game_core::{
    playing, Action, ActionState, CurrentLevel, GameLog, GamePaths, GameSet, GameState, HudMessage,
    LoadingGate, Manning, OnTurret, Player, PlayerCamera, RaiseEvent, ScriptTags, TeleportPlayer,
    TurretOrder,
};
use rc_turrets::{first_tagged, for_marker, Gunner, Mounted, Turret, TurretParams};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// The name this part uses in the loading gate and the log.
pub const PART: &str = "turrets";

/// The player's collision cylinder, uu (the game's commando).
const PLAYER_RADIUS: f32 = 40.0;
const PLAYER_HALF_HEIGHT: f32 = 84.0;

/// The player must face the turret to within this angle to get on,
/// radians (our choice: the original asks for the crosshair on it).
const FACING: f32 = 1.2;

/// A squad turret marker whose anchor isn't the turret itself goes with
/// the nearest turret this close to the spot, uu (our choice).
const MARKER_REACH: f32 = 512.0;

pub struct TurretsPlugin;

/// The turret part's work in a frame. It runs before the player's body
/// moves where it can (the game orders it), so the body reads this
/// frame's [`Manning`].
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct TurretSet;

impl Plugin for TurretsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<TurretOrder>()
            .add_message::<RaiseEvent>()
            .add_message::<TeleportPlayer>()
            .add_save_part::<TurretsSave>()
            .add_systems(OnEnter(GameState::Loading), load)
            .add_systems(
                Update,
                (orders, player_use, player_on, characters, settle, pose)
                    .chain()
                    .in_set(TurretSet)
                    .in_set(GameSet::Simulate)
                    .run_if(playing),
            )
            .add_systems(OnExit(GameState::InGame), forget);
    }
}

/// The turret data files, by id (`data/turrets/<id>.toml`).
#[derive(Debug, Clone, Default)]
pub struct TurretKinds(pub BTreeMap<String, TurretParams>);

/// A kind's model, ready to draw.
struct KindLook {
    look: Arc<DroidLook>,
    /// The resting pose's bones, each relative to its parent.
    rest: Vec<ATransform>,
    yaw_bone: Option<usize>,
    pitch_bone: Option<usize>,
    muzzles: Vec<usize>,
    material: Option<Handle<StandardMaterial>>,
}

/// The level's turrets while it is played.
#[derive(Resource)]
pub struct LevelTurrets {
    pub kinds: TurretKinds,
    pub list: Vec<Turret>,
    /// Each turret's entity (drawn or not), in `list`'s order.
    pub entities: Vec<Entity>,
    looks: HashMap<String, KindLook>,
    /// The player's weapon in hand before getting on (to give it back).
    previous_weapon: Option<usize>,
    /// The turret the player is offered (close and facing it).
    offered: Option<usize>,
    /// Character gunners: when each turret last fired, seconds.
    last_shot: Vec<f32>,
    /// Squad mates on turrets: each one's entity, by turret.
    squad_gunners: HashMap<usize, Entity>,
    time: f32,
    pub stats: TurretStats,
}

/// What happened, for the log and self-tests.
#[derive(Debug, Clone, Default)]
pub struct TurretStats {
    /// Events sent into the level, in order.
    pub events: Vec<String>,
    /// `on NAME`, `off NAME` for the player.
    pub player: Vec<String>,
    /// Shots fired by character gunners.
    pub auto_shots: usize,
}

/// A drawn turret.
#[derive(Component)]
pub struct TurretBody {
    pub index: usize,
    mesh: Option<Handle<Mesh>>,
    /// The yaw and pitch it was last drawn at.
    drawn: (f32, f32),
}

impl LevelTurrets {
    /// The level's turrets with no models (they aren't drawn, and gunners
    /// fire from where they're held). For tests and tools; the game reads
    /// the map instead.
    pub fn without_models(kinds: TurretKinds, list: Vec<Turret>) -> Self {
        let n = list.len();
        LevelTurrets {
            kinds,
            list,
            entities: vec![Entity::PLACEHOLDER; n],
            looks: HashMap::new(),
            previous_weapon: None,
            offered: None,
            last_shot: vec![f32::NEG_INFINITY; n],
            squad_gunners: HashMap::new(),
            time: 0.0,
            stats: TurretStats::default(),
        }
    }

    /// The turret the player is on, if any.
    pub fn player_turret(&self) -> Option<usize> {
        self.list.iter().position(|t| t.gunner == Gunner::Player)
    }
}

fn data_dirs_base_first(paths: &GamePaths) -> Vec<std::path::PathBuf> {
    paths
        .data_dirs
        .iter()
        .rev()
        .map(|d| GamePaths::new(d.clone(), vec![]).game_file("turrets"))
        .collect()
}

/// Read the map's turrets and their models. Blocks for as long as it
/// takes (a few hundred ms); the loading screen stays up meanwhile.
#[allow(clippy::too_many_arguments)]
fn load(
    mut commands: Commands,
    paths: Res<GamePaths>,
    current: Res<CurrentLevel>,
    mut gate: ResMut<LoadingGate>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut images: Option<ResMut<Assets<Image>>>,
    log: Res<GameLog>,
) {
    gate.wait(PART);
    let folders = data_dirs_base_first(&paths);
    let refs: Vec<&std::path::Path> = folders.iter().map(|p| p.as_path()).collect();
    let (kinds, errors) = rc_turrets::params::load_folders(&refs);
    for e in &errors {
        log.line(format!("{PART}: ERROR in a turret file: {e}"));
    }
    let Some(map) = &current.map else {
        gate.done(PART);
        return;
    };
    let path = paths.map_path(map);
    let read = std::fs::read(&path)
        .map_err(|e| e.to_string())
        .and_then(|d| {
            let pkg = rc_package::Package::parse(&d).map_err(|e| e.to_string())?;
            let level = rc_level::actors::read_level(&pkg, &d).map_err(|e| e.to_string())?;
            // Every turret class, to say which are left out.
            let mut all = Vec::new();
            for &a in level.actors.iter().flatten() {
                let c = pkg.export_class_name(&pkg.exports[a]);
                if c.to_ascii_lowercase().contains("turret")
                    || c.to_ascii_lowercase().contains("cannon")
                {
                    all.push((
                        pkg.name(pkg.exports[a].object_name).to_string(),
                        c.to_string(),
                    ));
                }
            }
            let claims = |c: &str| rc_turrets::params::for_class(&kinds, c).is_some();
            let (placed, problems) = rc_turrets::map::read(&pkg, &d, &level, &claims);
            Ok((placed, problems, all))
        });
    let (placed, problems, all) = match read {
        Ok(r) => r,
        Err(e) => {
            log.line(format!(
                "{PART}: ERROR reading {}: {e}; no turrets",
                path.display()
            ));
            gate.done(PART);
            return;
        }
    };
    for p in problems {
        log.line(format!("{PART}: unreadable turret: {p}"));
    }
    for (name, class) in &all {
        if !placed.iter().any(|p| &p.name == name) {
            log.line(format!(
                "{PART}: {name} ({class}) left out: no file in turrets/ claims {class}"
            ));
        }
    }
    let mut list = Vec::new();
    for p in placed {
        let Some((id, params)) = rc_turrets::params::for_class(&kinds, &p.class) else {
            continue;
        };
        list.push(Turret::new(p, id, params));
    }
    // Models, once per kind.
    let mut looks = HashMap::new();
    for t in &list {
        if looks.contains_key(&t.kind) {
            continue;
        }
        let p = &kinds[&t.kind];
        match load_look(&paths, p) {
            Ok((look, notes)) => {
                for n in notes {
                    log.line(format!("{PART}: {}: {n}", t.kind));
                }
                let material = match (images.as_mut(), materials.as_mut()) {
                    (Some(images), Some(materials)) => {
                        let texture = look.look.skin.as_ref().map(|(_, img)| {
                            images.add(Image::new(
                                Extent3d {
                                    width: img.width as u32,
                                    height: img.height as u32,
                                    depth_or_array_layers: 1,
                                },
                                TextureDimension::D2,
                                img.pixels.clone(),
                                TextureFormat::Rgba8UnormSrgb,
                                RenderAssetUsages::RENDER_WORLD,
                            ))
                        });
                        // Lit like the characters: the level's light is
                        // baked into its surfaces, so a share of the
                        // model's own colour stands in for it.
                        let a = 0.35;
                        Some(materials.add(StandardMaterial {
                            base_color: Color::WHITE,
                            emissive: LinearRgba::rgb(a, a, a),
                            emissive_texture: texture.clone(),
                            base_color_texture: texture,
                            perceptual_roughness: 0.7,
                            ..default()
                        }))
                    }
                    _ => None,
                };
                looks.insert(t.kind.clone(), KindLook { material, ..look });
            }
            Err(e) => log.line(format!(
                "{PART}: {}: model {} not read ({e}); its turrets work but aren't drawn",
                t.kind, p.model.mesh
            )),
        }
    }
    let mut entities = Vec::new();
    for (i, t) in list.iter().enumerate() {
        let kl = looks.get(&t.kind);
        let mesh = match (kl, meshes.as_mut()) {
            (Some(kl), Some(meshes)) => {
                let (local, _) = turret_bones(kl, 0.0, 0.0);
                let points = kl.look.posed_local(&local);
                let (pos, normals) = kl.look.wedge_attributes(&points);
                let mut m = Mesh::new(
                    PrimitiveTopology::TriangleList,
                    RenderAssetUsages::default(),
                );
                m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
                m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
                m.insert_attribute(Mesh::ATTRIBUTE_UV_0, kl.look.uvs());
                m.insert_indices(Indices::U32(kl.look.indices.clone()));
                Some(meshes.add(m))
            }
            _ => None,
        };
        let p = &kinds[&t.kind];
        let [x, y, z] = t.place.location;
        let feet = rc_movement::Vec3::new(x, y, z - p.gunner.half_height);
        let mut e = commands.spawn((
            Name::new(format!("turret {}", t.place.name)),
            TurretBody {
                index: i,
                mesh: mesh.clone(),
                drawn: (0.0, 0.0),
            },
            Transform::from_translation(to_bevy(feet))
                .with_rotation(Quat::from_rotation_y(-t.base_yaw())),
            Visibility::Visible,
            DespawnOnExit(GameState::InGame),
        ));
        if let (Some(m), Some(mat)) = (mesh, kl.and_then(|k| k.material.clone())) {
            e.insert((Mesh3d(m), MeshMaterial3d(mat)));
        }
        entities.push(e.id());
        log.line(format!(
            "{PART}: {} ({}, tag {}) at ({x:.0}, {y:.0}, {z:.0}) facing {:.0} deg: {}; yaw {}..{}{}, pitch {}..{}, speed {}; gets on: {:?} (player: {:?}), off: {:?}; gunner gets off at ({:.0}, {:.0}, {:.0}){}",
            t.place.name,
            t.kind,
            t.place.tag,
            t.base_yaw().to_degrees(),
            if t.player_usable { "the player may use it" } else { "not for the player" },
            t.limits.min_yaw,
            t.limits.max_yaw,
            if t.limits.unlimited_yaw { " (all round)" } else { "" },
            t.limits.min_pitch,
            t.limits.max_pitch,
            t.limits.speed,
            t.place.control_event,
            t.place.player_control_event,
            t.place.release_event,
            t.exit_point()[0],
            t.exit_point()[1],
            t.exit_point()[2],
            if kl.is_some() { "" } else { " (not drawn)" }
        ));
    }
    log.line(format!(
        "{PART}: {} turrets ({} the player may use), {} turret files",
        list.len(),
        list.iter().filter(|t| t.player_usable).count(),
        kinds.len()
    ));
    let n = list.len();
    commands.insert_resource(LevelTurrets {
        kinds: TurretKinds(kinds),
        list,
        entities,
        looks,
        previous_weapon: None,
        offered: None,
        last_shot: vec![f32::NEG_INFINITY; n],
        squad_gunners: HashMap::new(),
        time: 0.0,
        stats: TurretStats::default(),
    });
    gate.done(PART);
}

fn forget(mut commands: Commands) {
    commands.remove_resource::<LevelTurrets>();
}

/// Read a kind's model, and find its bones.
fn load_look(paths: &GamePaths, p: &TurretParams) -> Result<(KindLook, Vec<String>), String> {
    let look = rc_ai::params::Look {
        package: p.model.package.clone(),
        mesh: p.model.mesh.clone(),
        skin: String::new(),
        idle: p.model.pose.clone(),
        walk: String::new(),
        run: String::new(),
        walk_back: String::new(),
        step_left: String::new(),
        step_right: String::new(),
        fire: String::new(),
        crouch: String::new(),
        crouch_fire: String::new(),
        death: String::new(),
        throw: String::new(),
        melee: String::new(),
        ambient: 0.5,
        blend_time: 0.0,
        blend_time_fire: 0.0,
        blend_time_death: 0.0,
    };
    let (look, mut notes) = DroidLook::load(paths, &look)?;
    let sk = &look.skeleton;
    let bone = |name: &str, notes: &mut Vec<String>| {
        let b = sk.find(name);
        if b.is_none() {
            notes.push(format!("no bone called {name}; bones are {:?}", sk.names));
        }
        b
    };
    let yaw_bone = bone(&p.model.yaw_bone, &mut notes);
    let pitch_bone = bone(&p.model.pitch_bone, &mut notes);
    let muzzles = p
        .model
        .muzzles
        .iter()
        .filter_map(|m| bone(m, &mut notes))
        .collect();
    let rest = look.local_pose(Pose::Idle, 0.0);
    Ok((
        KindLook {
            look: Arc::new(look),
            rest,
            yaw_bone,
            pitch_bone,
            muzzles,
            material: None,
        },
        notes,
    ))
}

/// A rotation of `angle` radians about `axis` (a unit vector), as
/// `rc_anim` keeps them.
fn axis_angle(axis: [f32; 3], angle: f32) -> AQuat {
    let (s, c) = (angle / 2.0).sin_cos();
    AQuat([axis[0] * s, axis[1] * s, axis[2] * s, c])
}

/// The model's bones with the gun turned `yaw` and `pitch` rotation units
/// from rest: each bone relative to its parent (for drawing) and in the
/// whole model (for where the muzzles are). Turned in the model's own
/// axes, as the original's `SetBoneRotation(..., SPACE_Mesh)`: the yaw
/// bone about the model's up axis, the pitch bone also tipped up about its
/// side axis.
fn turret_bones(kl: &KindLook, yaw: f32, pitch: f32) -> (Vec<ATransform>, Vec<ATransform>) {
    let sk = &kl.look.skeleton;
    let rest_global = rc_anim::globals(&sk.parents, &kl.rest);
    let r_yaw = axis_angle([0.0, 0.0, 1.0], rc_turrets::to_radians(yaw));
    // Pitching up tips the model's forward (+X) towards up (+Z): a turn
    // about +Y the other way round.
    let r_pitch = axis_angle([0.0, 1.0, 0.0], -rc_turrets::to_radians(pitch));
    let mut local = kl.rest.clone();
    let mut global: Vec<ATransform> = Vec::with_capacity(local.len());
    for i in 0..local.len() {
        let parent = (i > 0 && sk.parents[i] < i).then(|| sk.parents[i]);
        let mut g = match parent {
            Some(p) => global[p].then(&local[i]),
            None => local[i],
        };
        let turn = if Some(i) == kl.yaw_bone {
            Some(r_yaw)
        } else if Some(i) == kl.pitch_bone {
            Some(r_yaw.compose(r_pitch))
        } else {
            None
        };
        if let Some(r) = turn {
            g.rot = r.compose(rest_global[i].rot).normalize();
            local[i] = match parent {
                Some(p) => global[p].inverse().then(&g),
                None => g,
            };
        }
        global.push(g);
    }
    (local, global)
}

/// A point of the model (its own units and axes) in the level, uu.
fn model_to_level(look: &DroidLook, t: &Turret, half_height: f32, p: [f32; 3]) -> [f32; 3] {
    let m = &look.mesh;
    let s = m.mesh_scale;
    let o = m.mesh_origin;
    let q = [
        (p[0] - o[0]) * s[0],
        (p[1] - o[1]) * s[1],
        (p[2] - o[2]) * s[2],
    ];
    let yaw = m.rot_origin[1] as f32 * std::f32::consts::TAU / 65536.0 + t.base_yaw();
    let (sin, cos) = yaw.sin_cos();
    let [x, y, z] = t.place.location;
    [
        x + q[0] * cos - q[1] * sin,
        y + q[0] * sin + q[1] * cos,
        z - half_height + q[2] + look.lift,
    ]
}

/// Scripts' orders: characters on and off turrets, usable or not.
fn orders(
    mut commands: Commands,
    turrets: Option<ResMut<LevelTurrets>>,
    mut incoming: MessageReader<TurretOrder>,
    mut raise: MessageWriter<RaiseEvent>,
    chars: Query<(Entity, &ScriptTags)>,
    log: Res<GameLog>,
) {
    let Some(mut lt) = turrets else {
        incoming.clear();
        return;
    };
    for o in incoming.read() {
        match o {
            TurretOrder::Attach {
                turret,
                attach,
                owner,
                by,
            } => {
                let Some(i) = first_tagged(&lt.list, turret) else {
                    log.line(format!("{PART}: {by}: no turret tagged {turret}"));
                    continue;
                };
                let who = owner.clone().unwrap_or_else(|| by.clone());
                let found = chars.iter().any(|(_, st)| {
                    st.script
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(&who))
                });
                let name = lt.list[i].place.name.clone();
                let result = if *attach {
                    lt.list[i].mount(Gunner::Character(who.clone()))
                } else if lt.list[i].gunner == Gunner::Character(who.clone()) {
                    lt.list[i].leave()
                } else {
                    Mounted::NotOn
                };
                match result {
                    Mounted::On(events) | Mounted::Off(events) => {
                        log.line(format!(
                            "{PART}: {by}: the characters running {who} get {} {name}{}",
                            if *attach { "on" } else { "off" },
                            if found {
                                ""
                            } else {
                                " (none in the level yet: the turret counts as taken)"
                            }
                        ));
                        send(&mut lt, &mut raise, events, &name, &log);
                    }
                    Mounted::Taken => log.line(format!(
                        "{PART}: {by}: {name} is taken ({:?}); {who} stays off",
                        lt.list[i].gunner
                    )),
                    Mounted::NotUsable | Mounted::NotOn => log.line(format!(
                        "{PART}: {by}: {who} isn't on {name}; nothing to do"
                    )),
                }
            }
            TurretOrder::Squad {
                anchor,
                near,
                gunner,
                member,
                on,
            } => squad_order(
                &mut commands,
                &mut lt,
                &mut raise,
                (anchor, *near, *gunner, member, *on),
                &log,
            ),
            TurretOrder::SetUsable { turret, usable, by } => {
                let mut n = 0;
                for t in lt
                    .list
                    .iter_mut()
                    .filter(|t| t.place.tag.eq_ignore_ascii_case(turret))
                {
                    t.player_usable = *usable;
                    n += 1;
                }
                log.line(format!(
                    "{PART}: {by}: {n} turrets tagged {turret} {} by the player",
                    if *usable { "usable" } else { "not usable" }
                ));
            }
        }
    }
}

/// A squad mate gets on or off the turret of the marker they were sent to.
fn squad_order(
    commands: &mut Commands,
    lt: &mut LevelTurrets,
    raise: &mut MessageWriter<RaiseEvent>,
    (anchor, near, e, member, on): (&str, [f32; 3], Entity, &str, bool),
    log: &GameLog,
) {
    let who = Gunner::Squad(member.to_string());
    let i = if on {
        for_marker(&lt.list, anchor, near, MARKER_REACH)
    } else {
        lt.list.iter().position(|t| t.gunner == who)
    };
    let Some(i) = i else {
        if on {
            log.line(format!(
                "{PART}: t={:.2} {member}: no turret {anchor} or within {MARKER_REACH} uu of the marker; fights from the spot",
                lt.time
            ));
        }
        commands.entity(e).try_remove::<OnTurret>();
        return;
    };
    let name = lt.list[i].place.name.clone();
    let result = if on {
        lt.list[i].mount(who.clone())
    } else {
        lt.list[i].leave()
    };
    match result {
        Mounted::On(events) => {
            log.line(format!(
                "{PART}: t={:.2} squad mate {member} gets on {name}",
                lt.time
            ));
            lt.squad_gunners.insert(i, e);
            let t = &lt.list[i];
            let (yaw, pitch) = t.view();
            commands.entity(e).insert(OnTurret {
                turret: lt.entities[i],
                name: name.clone(),
                body: t.gunner_body(),
                yaw,
                pitch,
            });
            send(lt, raise, events, &name, log);
        }
        Mounted::Off(events) => {
            log.line(format!(
                "{PART}: t={:.2} squad mate {member} gets off {name}",
                lt.time
            ));
            lt.squad_gunners.remove(&i);
            commands.entity(e).try_remove::<OnTurret>();
            send(lt, raise, events, &name, log);
        }
        Mounted::Taken => log.line(format!(
            "{PART}: t={:.2} {member}: {name} is taken ({}); fights from the spot",
            lt.time,
            lt.list[i].gunner.save_name()
        )),
        Mounted::NotUsable | Mounted::NotOn => {
            commands.entity(e).try_remove::<OnTurret>();
        }
    }
}

fn send(
    lt: &mut LevelTurrets,
    raise: &mut MessageWriter<RaiseEvent>,
    events: Vec<String>,
    name: &str,
    log: &GameLog,
) {
    for e in events {
        log.line(format!("{PART}: t={:.2} {name} sends {e}", lt.time));
        lt.stats.events.push(e.clone());
        raise.write(RaiseEvent {
            name: e,
            by: name.to_string(),
        });
    }
}

/// Getting on: offer the nearest usable turret the player faces, and get
/// on with Use.
#[allow(clippy::too_many_arguments)]
fn player_use(
    mut commands: Commands,
    turrets: Option<ResMut<LevelTurrets>>,
    actions: Res<ActionState>,
    lib: Option<Res<WeaponLibrary>>,
    mut player: Query<
        (Entity, &Transform, Option<&mut Arsenal>),
        (With<Player>, Without<Manning>, Without<Dead>),
    >,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    mut raise: MessageWriter<RaiseEvent>,
    mut hud: MessageWriter<HudMessage>,
    log: Res<GameLog>,
) {
    let Some(mut lt) = turrets else { return };
    let Ok((me, t, arsenal)) = player.single_mut() else {
        lt.offered = None;
        return;
    };
    let feet = to_unreal(t.translation);
    let centre = [feet.x, feet.y, feet.z + PLAYER_HALF_HEIGHT];
    let facing = camera.single().ok().map(|c| {
        let f = to_unreal(*c.forward());
        (f.x, f.y)
    });
    let mut best: Option<(usize, f32)> = None;
    for (i, tur) in lt.list.iter().enumerate() {
        if !tur.usable_by_player() {
            continue;
        }
        let p = &lt.kinds.0[&tur.kind];
        let [x, y, z] = tur.place.location;
        let (dx, dy) = (x - centre[0], y - centre[1]);
        let flat = (dx * dx + dy * dy).sqrt();
        let gap = flat - p.gunner.radius - PLAYER_RADIUS;
        if gap > p.gunner.reach || (z - centre[2]).abs() > p.gunner.half_height + PLAYER_HALF_HEIGHT
        {
            continue;
        }
        if let Some((fx, fy)) = facing {
            let l = (fx * fx + fy * fy).sqrt();
            if l > 1e-3 && flat > 1e-3 {
                let cos = (fx * dx + fy * dy) / (l * flat);
                if cos < FACING.cos() {
                    continue;
                }
            }
        }
        if best.is_none_or(|(_, g)| gap < g) {
            best = Some((i, gap));
        }
    }
    let offer = best.map(|(i, _)| i);
    if offer != lt.offered {
        lt.offered = offer;
        if let Some(i) = offer {
            let p = &lt.kinds.0[&lt.list[i].kind];
            hud.write(HudMessage {
                text: p.gunner.prompt.replace('@', "USE"),
                seconds: 3.0,
            });
            log.line(format!(
                "{PART}: t={:.2} the player can get on {}",
                lt.time, lt.list[i].place.name
            ));
        }
    }
    let Some(i) = offer else { return };
    if !actions.just_pressed(Action::Use) {
        return;
    }
    let Mounted::On(events) = lt.list[i].mount(Gunner::Player) else {
        return;
    };
    let name = lt.list[i].place.name.clone();
    let weapon = lt.kinds.0[&lt.list[i].kind].gunner.weapon.clone();
    // The turret's weapon in hand, the player's own put away.
    if let (Some(mut arsenal), Some(w)) = (arsenal, &weapon) {
        lt.previous_weapon = Some(arsenal.current);
        hand_over(&mut arsenal, w, lib.as_deref(), &log);
    }
    let (yaw, pitch) = lt.list[i].view();
    commands.entity(me).insert(Manning {
        turret: lt.entities[i],
        name: name.clone(),
        body: lt.list[i].gunner_body(),
        yaw,
        pitch,
        weapon,
    });
    lt.offered = None;
    lt.stats.player.push(format!("on {name}"));
    log.line(format!(
        "{PART}: t={:.2} the player gets on {name}, facing {:.0} deg",
        lt.time,
        yaw.to_degrees()
    ));
    send(&mut lt, &mut raise, events, &name, &log);
}

/// Put the turret's weapon in the player's hands (once).
fn hand_over(arsenal: &mut Arsenal, weapon: &str, lib: Option<&WeaponLibrary>, log: &GameLog) {
    if let Some(i) = arsenal.weapons.iter().position(|(id, _)| id == weapon) {
        arsenal.current = i;
        return;
    }
    match lib.and_then(|l| l.weapons.get(weapon)) {
        Some(p) => {
            arsenal
                .weapons
                .push((weapon.to_string(), WeaponState::new(p)));
            arsenal.current = arsenal.weapons.len() - 1;
        }
        None => log.line(format!(
            "{PART}: no weapon file weapons/{weapon}.toml; the turret can't fire"
        )),
    }
}

/// On the turret: the look turns it, Use gets off.
#[allow(clippy::too_many_arguments)]
fn player_on(
    mut commands: Commands,
    time: Res<Time>,
    turrets: Option<ResMut<LevelTurrets>>,
    actions: Res<ActionState>,
    mut player: Query<(Entity, &mut Manning, Option<&mut Arsenal>, Has<Dead>), With<Player>>,
    mut raise: MessageWriter<RaiseEvent>,
    mut teleport: MessageWriter<TeleportPlayer>,
    log: Res<GameLog>,
) {
    let Some(mut lt) = turrets else { return };
    let dt = time.delta_secs().min(0.25);
    lt.time += dt;
    let Ok((me, mut m, arsenal, down)) = player.single_mut() else {
        return;
    };
    let Some(i) = lt.entities.iter().position(|e| *e == m.turret) else {
        commands.entity(me).remove::<Manning>();
        return;
    };
    // Getting off: Use, or going down (the original puts a downed gunner
    // off the turret).
    let leave = actions.just_pressed(Action::Use) || down;
    if leave {
        let name = lt.list[i].place.name.clone();
        let events = match lt.list[i].leave() {
            Mounted::Off(e) => e,
            _ => Vec::new(),
        };
        let exit = lt.list[i].exit_point();
        let (yaw, _) = lt.list[i].view();
        if let (Some(mut arsenal), Some(w)) = (arsenal, &m.weapon) {
            arsenal.weapons.retain(|(id, _)| id != w);
            let back = lt.previous_weapon.take().unwrap_or(0);
            arsenal.current = back.min(arsenal.weapons.len().saturating_sub(1));
        }
        teleport.write(TeleportPlayer {
            location: exit,
            yaw,
            by: format!("getting off {name}"),
        });
        commands.entity(me).remove::<Manning>();
        lt.stats.player.push(format!("off {name}"));
        log.line(format!(
            "{PART}: t={:.2} the player gets off {name}{}, at ({:.0}, {:.0}, {:.0})",
            lt.time,
            if down { " (down)" } else { "" },
            exit[0],
            exit[1],
            exit[2]
        ));
        send(&mut lt, &mut raise, events, &name, &log);
        return;
    }
    let tur = &mut lt.list[i];
    let d_yaw = rc_turrets::to_units(actions.look.x);
    let d_pitch = rc_turrets::to_units(actions.look.y);
    tur.turn(d_yaw, d_pitch, dt);
    let (yaw, pitch) = tur.view();
    m.yaw = yaw;
    m.pitch = pitch;
    m.body = tur.gunner_body();
}

/// Characters and squad mates on turrets: hold the characters there (the
/// squad part holds its own), and shoot at enemies in range.
#[allow(clippy::too_many_arguments)]
fn characters(
    mut commands: Commands,
    turrets: Option<ResMut<LevelTurrets>>,
    time: Res<Time>,
    chars: Query<(Entity, &ScriptTags, Option<&Team>, Has<Dead>)>,
    others: Query<(Option<&Team>, Has<Dead>)>,
    mut on_turret: Query<&mut OnTurret>,
    mut bodies: Query<&mut Transform, Without<TurretBody>>,
    targets: Query<(Entity, &GlobalTransform, &Team, &Hittable), Without<Dead>>,
    mut shots: MessageWriter<FireShot>,
    mut raise: MessageWriter<RaiseEvent>,
    log: Res<GameLog>,
) {
    let Some(mut lt) = turrets else { return };
    let dt = time.delta_secs().min(0.25);
    let now = lt.time;
    for i in 0..lt.list.len() {
        let (who, e, team, dead, squad) = match lt.list[i].gunner.clone() {
            Gunner::Character(who) => {
                let gunner = chars.iter().find(|(_, st, _, _)| {
                    st.script
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(&who))
                });
                let Some((e, _, team, dead)) = gunner else {
                    continue;
                };
                (who, e, team.copied(), dead, false)
            }
            Gunner::Squad(who) => {
                // Not asked for again yet after loading a save: wait.
                let Some(&e) = lt.squad_gunners.get(&i) else {
                    continue;
                };
                match others.get(e) {
                    Ok((team, dead)) => (who, e, team.copied(), dead, true),
                    // Gone from the level: off.
                    Err(_) => (who, e, None, true, true),
                }
            }
            _ => continue,
        };
        if dead {
            let name = lt.list[i].place.name.clone();
            if let Mounted::Off(events) = lt.list[i].leave() {
                log.line(format!(
                    "{PART}: t={now:.2} {who} on {name} is down; off it"
                ));
                send(&mut lt, &mut raise, events, &name, &log);
            }
            if squad {
                lt.squad_gunners.remove(&i);
                commands.entity(e).try_remove::<OnTurret>();
            }
            continue;
        }
        if squad {
            // The squad part holds the body; tell it where the gun points.
            let t = &lt.list[i];
            let (yaw, pitch) = t.view();
            if let Ok(mut o) = on_turret.get_mut(e) {
                o.body = t.gunner_body();
                o.yaw = yaw;
                o.pitch = pitch;
            }
        } else if let Ok(mut tr) = bodies.get_mut(e) {
            // Held on the turret (standing at its gunner spot).
            let [x, y, z] = lt.list[i].gunner_body();
            tr.translation = to_bevy(rc_movement::Vec3::new(x, y, z - PLAYER_HALF_HEIGHT));
        }
        let p = lt.kinds.0[&lt.list[i].kind].clone();
        let Some(weapon) = p.auto.weapon.clone() else {
            continue;
        };
        // The nearest enemy of the gunner's side in range.
        let mine = team.unwrap_or(Team::Republic);
        let pivot = lt.list[i].place.location;
        let mut best: Option<([f32; 3], f32)> = None;
        for (_, g, team, h) in &targets {
            if *team == mine || *team == Team::Neutral {
                continue;
            }
            let at = to_unreal(g.translation());
            let at = [at.x, at.y, at.z + h.center_up];
            let d = ((at[0] - pivot[0]).powi(2)
                + (at[1] - pivot[1]).powi(2)
                + (at[2] - pivot[2]).powi(2))
            .sqrt();
            if d <= p.auto.range && best.is_none_or(|(_, b)| d < b) {
                best = Some((at, d));
            }
        }
        let Some((target, _)) = best else {
            continue;
        };
        lt.list[i].turn_toward(pivot, target, dt);
        if now - lt.last_shot[i] < p.auto.shot_delay {
            continue;
        }
        // From the muzzles; from the gunner with no model.
        let starts: Vec<[f32; 3]> = match lt.looks.get(&lt.list[i].kind) {
            Some(kl) if !kl.muzzles.is_empty() => {
                let (_, global) = turret_bones(kl, lt.list[i].yaw, lt.list[i].pitch);
                let bones: Vec<usize> = if p.auto.muzzles_in_turn {
                    vec![kl.muzzles[lt.stats.auto_shots % kl.muzzles.len()]]
                } else {
                    kl.muzzles.clone()
                };
                bones
                    .iter()
                    .map(|&b| {
                        model_to_level(&kl.look, &lt.list[i], p.gunner.half_height, global[b].pos)
                    })
                    .collect()
            }
            _ => vec![lt.list[i].gunner_body()],
        };
        for s in starts {
            let d =
                Vec3::new(target[0] - s[0], target[1] - s[1], target[2] - s[2]).normalize_or_zero();
            shots.write(FireShot {
                shooter: e,
                origin: to_bevy(rc_movement::Vec3::new(s[0], s[1], s[2])),
                direction: Vec3::new(d.x, d.z, d.y),
                weapon: weapon.clone(),
            });
            lt.stats.auto_shots += 1;
        }
        lt.last_shot[i] = now;
    }
}

/// Turrets nobody is on swing back to rest.
fn settle(turrets: Option<ResMut<LevelTurrets>>, time: Res<Time>) {
    let Some(mut lt) = turrets else { return };
    let dt = time.delta_secs().min(0.25);
    for t in &mut lt.list {
        if t.gunner == Gunner::Nobody {
            t.settle(dt);
        }
    }
}

/// Draw each turret's gun where it points; hide the player's.
fn pose(
    turrets: Option<Res<LevelTurrets>>,
    mut bodies: Query<(&mut TurretBody, &mut Visibility)>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
) {
    let Some(lt) = turrets else { return };
    for (mut b, mut vis) in &mut bodies {
        let Some(t) = lt.list.get(b.index) else {
            continue;
        };
        let hide = t.gunner == Gunner::Player && lt.kinds.0[&t.kind].model.hide_when_player_on;
        let want = if hide {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        if *vis != want {
            *vis = want;
        }
        if (t.yaw, t.pitch) == b.drawn {
            continue;
        }
        b.drawn = (t.yaw, t.pitch);
        let (Some(kl), Some(handle), Some(meshes)) =
            (lt.looks.get(&t.kind), &b.mesh, meshes.as_mut())
        else {
            continue;
        };
        let (local, _) = turret_bones(kl, t.yaw, t.pitch);
        let points = kl.look.posed_local(&local);
        let (pos, normals) = kl.look.wedge_attributes(&points);
        if let Some(mut m) = meshes.get_mut(handle) {
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        }
    }
}

/// The `[turrets]` section of a save file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurretsSave {
    pub turrets: Vec<TurretSave>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurretSave {
    /// The turret's name in the map.
    pub name: String,
    pub yaw: f32,
    pub pitch: f32,
    /// `player`, `nobody`, `squad:<name>` for a squad mate, or the script
    /// whose characters are on it.
    pub gunner: String,
    pub player_usable: bool,
}

impl SavePart for TurretsSave {
    const KEY: &'static str = "turrets";

    fn save(world: &mut World) -> Option<Self> {
        let lt = world.get_resource::<LevelTurrets>()?;
        if lt.list.is_empty() {
            return None;
        }
        Some(TurretsSave {
            turrets: lt
                .list
                .iter()
                .map(|t| TurretSave {
                    name: t.place.name.clone(),
                    yaw: t.yaw,
                    pitch: t.pitch,
                    gunner: t.gunner.save_name(),
                    player_usable: t.player_usable,
                })
                .collect(),
        })
    }

    fn load(world: &mut World, data: Self) -> Result<(), String> {
        let log = world.resource::<GameLog>().clone();
        let mut on = None;
        {
            let Some(mut lt) = world.get_resource_mut::<LevelTurrets>() else {
                return Err("no turrets in the level".into());
            };
            for s in data.turrets {
                let Some(i) = lt.list.iter().position(|t| t.place.name == s.name) else {
                    log.line(format!(
                        "{PART}: the save's turret {} isn't in the map",
                        s.name
                    ));
                    continue;
                };
                let t = &mut lt.list[i];
                t.yaw = s.yaw;
                t.pitch = s.pitch;
                t.player_usable = s.player_usable;
                // Put back as it was: no events (they went off before).
                // A squad mate is asked on again by the squad part, which
                // finds the turret already theirs.
                t.gunner = Gunner::from_save_name(&s.gunner);
                if t.gunner == Gunner::Player {
                    on = Some(i);
                }
            }
        }
        let Some(i) = on else {
            return Ok(());
        };
        // Back on the turret, its weapon in hand.
        let (manning, weapon) = {
            let lt = world.resource::<LevelTurrets>();
            let t = &lt.list[i];
            let (yaw, pitch) = t.view();
            let weapon = lt.kinds.0[&t.kind].gunner.weapon.clone();
            (
                Manning {
                    turret: lt.entities[i],
                    name: t.place.name.clone(),
                    body: t.gunner_body(),
                    yaw,
                    pitch,
                    weapon: weapon.clone(),
                },
                weapon,
            )
        };
        let mut q = world.query_filtered::<Entity, With<Player>>();
        let Some(me) = q.iter(world).next() else {
            return Err("no player in the level".into());
        };
        world.entity_mut(me).insert(manning.clone());
        if let Some(w) = weapon {
            world.resource_scope(|world, lib: Mut<WeaponLibrary>| {
                if let Some(mut a) = world.get_mut::<Arsenal>(me) {
                    hand_over(&mut a, &w, Some(&lib), &log);
                }
            });
        }
        log.line(format!("{PART}: the player is back on {}", manning.name));
        Ok(())
    }
}

#[cfg(test)]
mod tests;
