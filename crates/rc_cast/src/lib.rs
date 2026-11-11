//! rc_cast: the characters placed in a map that no other part owns. These
//! are the Kaminoans, young clones, clone troopers and Delta commandos of
//! the prologue, briefings and epilogue, the clones sitting in the
//! nursery (animated props), and troopers standing about in the missions.
//!
//! [`CastPlugin`] follows `docs/PLUGINS.md`:
//! - when a map loads (`OnEnter(GameState::Loading)`) it finds the cast
//!   (`read::find_cast`): every placed actor whose class comes from `Pawn`
//!   or `AnimProp`, apart from the classes `data/cast/cast.toml` leaves to
//!   other parts (enemies) and squad mates. Each wears its class's model
//!   from the game's files (`rc_model`, the class's default `Mesh` read
//!   from the game's script packages);
//! - each is spawned at the start of play with `ScriptTags` (its `Tag`
//!   and `AIScriptTag`, `follows_orders`), so the level scripts part sends
//!   it `CharacterOrder`s: walk or run to a point, go to the player, turn
//!   to the player, wait for the player, play one of its own animations,
//!   wait for it to end, teleport, hide, go away. It answers with
//!   `CharacterOrderDone` when the script waits;
//! - it moves in straight lines, without collision or falling (cutscene
//!   walks are short and on clear floors), and is posed on the CPU each
//!   frame like the squad.
//!
//! The cast takes no damage, doesn't fight and isn't kept in save files
//! (the cutscene maps have no checkpoints). Everything it does is logged
//! with the prefix `cast:`.

pub mod read;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rc_anim::Clip;
use rc_game_core::units::{to_bevy, to_unreal, UU_PER_UNIT};
use rc_game_core::{
    playing, CharacterAct, CharacterOrder, CharacterOrderDone, CurrentLevel, GameLog, GamePaths,
    GameSet, GameState, Player, PlayerCamera, RaiseEvent, ScriptEvent, ScriptTags,
};
use rc_model::NotifyTrigger;
use rc_model::{Model, ModelSpec};
use read::{CastFile, CastMember, Kind};
use std::collections::HashMap;
use std::sync::Arc;

const PART: &str = "cast";
const CAST_FILE: &str = "cast/cast.toml";
/// Close enough to a point to have arrived, Unreal units.
const ARRIVED: f32 = 8.0;
/// Turning speed, radians a second (Pawn RotationRate yaw 20000 a second,
/// as the droids).
const TURN_RATE: f32 = 20000.0 * std::f32::consts::TAU / 65536.0;

pub struct CastPlugin;

impl Plugin for CastPlugin {
    fn build(&self, app: &mut App) {
        // Holograms in the cast (the prologue's rifles and advisor) shimmer.
        app.add_plugins(rc_hologram::HologramPlugin);
        app.init_resource::<CastCache>()
            .init_resource::<CastPlan>()
            .add_message::<CharacterOrder>()
            .add_message::<CharacterOrderDone>()
            .add_message::<ScriptEvent>()
            .add_message::<RaiseEvent>()
            .add_systems(OnEnter(GameState::Loading), load_cast)
            .add_systems(
                Update,
                (spawn_cast, follow_orders, trigger_props, move_cast)
                    .chain()
                    .in_set(GameSet::Simulate)
                    .run_if(playing),
            )
            .add_systems(
                Update,
                (place_cast, animate_cast)
                    .chain()
                    .in_set(GameSet::View)
                    .run_if(in_state(GameState::InGame)),
            );
    }
}

fn log_line(log: &Option<Res<GameLog>>, line: impl AsRef<str>) {
    let line = format!("{PART}: {}", line.as_ref());
    match log {
        Some(l) => l.line(line),
        None => println!("{line}"),
    }
}

/// What lasts from one map to the next: the game's classes, where each
/// mesh is, and the models read so far.
#[derive(Resource, Default)]
pub struct CastCache {
    classes: Option<Arc<rc_package::class_library::ClassLibrary>>,
    meshes: Option<Arc<HashMap<String, (String, String)>>>,
    models: HashMap<String, Result<Arc<Model>, String>>,
}

/// The current map's cast, waiting to be spawned.
#[derive(Resource, Default)]
pub struct CastPlan {
    pub members: Vec<(CastMember, Option<Arc<Model>>)>,
    pub settings: Option<Arc<CastFile>>,
    pub spawned: bool,
}

/// The settings file, or the built-in one (logged why).
fn settings(paths: Option<&GamePaths>, log: &Option<Res<GameLog>>) -> CastFile {
    let Some(file) = paths.and_then(|p| p.data_file(CAST_FILE)) else {
        log_line(log, format!("no {CAST_FILE}; built-in values"));
        return CastFile::default();
    };
    match std::fs::read_to_string(&file)
        .map_err(|e| e.to_string())
        .and_then(|t| toml::from_str::<CastFile>(&t).map_err(|e| e.to_string()))
    {
        Ok(f) => {
            log_line(log, format!("settings from {}", file.display()));
            f
        }
        Err(e) => {
            log_line(
                log,
                format!("ERROR in {}: {e}; built-in values used", file.display()),
            );
            CastFile::default()
        }
    }
}

fn load_cast(
    mut plan: ResMut<CastPlan>,
    mut cache: ResMut<CastCache>,
    level: Option<Res<CurrentLevel>>,
    paths: Option<Res<GamePaths>>,
    log: Option<Res<GameLog>>,
) {
    *plan = CastPlan::default();
    let (Some(level), Some(paths)) = (level, paths) else {
        return;
    };
    let Some(map) = level.map.as_deref() else {
        return;
    };
    let file = settings(Some(&paths), &log);
    let game_data = paths.game_data();
    if cache.classes.is_none() {
        let (lib, notes) = read::class_library(&game_data);
        for n in notes {
            log_line(&log, n);
        }
        cache.classes = Some(Arc::new(lib));
    }
    if cache.meshes.is_none() {
        let dir = rc_game_core::paths::resolve(&game_data, "Animations");
        let (index, notes) = read::mesh_index(&dir);
        for n in notes {
            log_line(&log, n);
        }
        log_line(
            &log,
            format!("{} character meshes in the animation packages", index.len()),
        );
        cache.meshes = Some(Arc::new(index));
    }
    let path = paths.map_path(map);
    let read = std::fs::read(&path)
        .map_err(|e| e.to_string())
        .and_then(|d| {
            let pkg = rc_package::Package::parse(&d).map_err(|e| e.to_string())?;
            let level = rc_level::actors::read_level(&pkg, &d).map_err(|e| e.to_string())?;
            let scripts = rc_level::scripts::read(&pkg, &d, &level);
            let skins = read::map_skins(&pkg, &d, &scripts.actors);
            Ok((scripts, skins))
        });
    let (scripts, map_skins) = match read {
        Ok(s) => s,
        Err(e) => {
            log_line(&log, format!("can't read {}: {e}; no cast", path.display()));
            return;
        }
    };
    let (Some(lib), Some(meshes)) = (cache.classes.clone(), cache.meshes.clone()) else {
        return;
    };
    let (members, notes) = read::find_cast(&scripts.actors, &lib, &meshes, &file, &map_skins);
    for n in notes {
        log_line(&log, n);
    }
    let mut list = Vec::new();
    for m in members {
        let key = format!("{}|{}|{:?}", m.package, m.mesh, m.skins);
        let model = cache
            .models
            .entry(key)
            .or_insert_with(|| {
                let spec = ModelSpec {
                    package: m.package.clone(),
                    mesh: m.mesh.clone(),
                    skins: m.skins.clone(),
                    keep_sets: true,
                    ..Default::default()
                };
                match Model::load(&|rel| paths.game_file(rel), &spec) {
                    Ok((model, notes)) => {
                        for n in notes
                            .iter()
                            .filter(|n| n.starts_with("mesh ") || n.contains("no picture"))
                        {
                            log_line(&log, n);
                        }
                        Ok(Arc::new(model))
                    }
                    Err(e) => {
                        log_line(&log, format!("{} not read: {e}", m.mesh));
                        Err(e)
                    }
                }
            })
            .as_ref()
            .ok()
            .cloned();
        log_line(
            &log,
            format!(
                "{} ({}) at ({:.0}, {:.0}, {:.0}) facing {:.0} deg: {} from {}{}{}{}{}",
                m.name,
                m.class,
                m.location[0],
                m.location[1],
                m.location[2],
                m.yaw.to_degrees(),
                m.mesh,
                m.package,
                m.script
                    .as_ref()
                    .map(|s| format!(", follows {s}"))
                    .unwrap_or_default(),
                if m.tag != m.class {
                    format!(", tag {}", m.tag)
                } else {
                    String::new()
                },
                if m.hidden { ", hidden" } else { "" },
                if m.skins.is_empty() {
                    String::new()
                } else {
                    format!(
                        ", skins {}",
                        m.skins
                            .iter()
                            .map(|s| s.as_deref().unwrap_or("-"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                }
            ),
        );
        list.push((m, model));
    }
    log_line(&log, format!("{} in the cast of {map}", list.len()));
    plan.members = list;
    plan.settings = Some(Arc::new(file));
}

/// What a cast member is doing for its script.
#[derive(Debug, Clone, PartialEq)]
enum Goal {
    Stand,
    MoveTo {
        to: [f32; 3],
        walk: bool,
        face: Option<f32>,
    },
    MoveToPlayer {
        tolerance: f32,
    },
    TurnToPlayer,
    WaitForPlayer {
        distance: f32,
        since: f32,
    },
    AnimEnd,
}

/// An animation a script plays.
#[derive(Debug, Clone)]
struct Scripted {
    clip: Clip,
    looping: bool,
}

/// One cast member in play.
#[derive(Component)]
pub struct Cast {
    pub name: String,
    pub class: String,
    /// Unreal units (see [`CastMember::location`]).
    pub location: [f32; 3],
    half_height: f32,
    pub yaw: f32,
    run_speed: f32,
    walk_speed: f32,
    goal: Goal,
    /// The script action waiting for this one to finish.
    waiting: Option<(String, usize)>,
    scripted: Option<Scripted>,
    moving: Option<bool>,
    look: Option<Look>,
    time: f32,
    /// Props: the name level events must have to trigger it, and what it
    /// plays then (`AnimTriggered`, looping or not).
    tag: String,
    trigger: Option<(String, bool)>,
}

impl Cast {
    /// The original's `AnimProp.Trigger`: stop `AnimTriggered` if it is
    /// playing, else start it. What happened, for the log.
    fn trigger(&mut self) -> Option<String> {
        let (anim, looping) = self.trigger.clone()?;
        let look = self.look.as_ref()?;
        let on = self
            .scripted
            .as_ref()
            .is_some_and(|s| s.clip.name.eq_ignore_ascii_case(&anim));
        if on {
            self.scripted = None;
            return Some(format!("stops {anim}"));
        }
        let Some(clip) = look.model.named_clip(&anim) else {
            return Some(format!("has no animation {anim}"));
        };
        self.scripted = Some(Scripted { clip, looping });
        Some(format!("plays {anim}"))
    }
}

/// The model and the animations it plays on its own.
struct Look {
    model: Arc<Model>,
    pieces: Vec<(usize, Handle<Mesh>)>,
    idle: Option<Clip>,
    walk: Option<Clip>,
    run: Option<Clip>,
    /// The clip playing (`None`: the model's own pose) and the frame.
    playing: Option<String>,
    frame: f32,
    posed_once: bool,
}

fn first_clip(model: &Model, names: &[String]) -> Option<Clip> {
    names.iter().find_map(|n| model.named_clip(n))
}

#[allow(clippy::too_many_arguments)]
fn spawn_cast(
    mut commands: Commands,
    mut plan: ResMut<CastPlan>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    images: Option<ResMut<Assets<Image>>>,
    log: Option<Res<GameLog>>,
) {
    if plan.spawned {
        return;
    }
    plan.spawned = true;
    let Some(file) = plan.settings.clone() else {
        return;
    };
    let mut assets = match (meshes, materials, images) {
        (Some(a), Some(b), Some(c)) => Some((a, b, c)),
        _ => None,
    };
    let mut drawn = 0;
    for (m, model) in &plan.members {
        let feet = [m.location[0], m.location[1], m.location[2] - m.half_height];
        let mut e = commands.spawn((
            Transform::from_translation(to_bevy(rc_movement::Vec3::new(feet[0], feet[1], feet[2])))
                .with_rotation(Quat::from_rotation_y(-m.yaw)),
            if m.hidden {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            },
            ScriptTags {
                tag: Some(m.tag.clone()),
                script: m.script.clone(),
                follows_orders: true,
            },
            DespawnOnExit(GameState::InGame),
            Name::new(format!("cast {}", m.name)),
        ));
        if m.shadow {
            e.insert(rc_shadows::CastsShadow {
                middle: m.half_height / UU_PER_UNIT,
            });
        }
        let look = match (model, assets.as_mut()) {
            (Some(model), Some((meshes, materials, images))) => {
                drawn += 1;
                let mut look = spawn_pieces(
                    model.clone(),
                    m,
                    file.ambient,
                    file.light_scale,
                    &mut e,
                    meshes,
                    materials,
                    images,
                );
                look.idle = m
                    .prop_anim
                    .as_ref()
                    .and_then(|(a, _)| model.named_clip(a))
                    .or_else(|| first_clip(model, &file.idle));
                look.walk = first_clip(model, &file.walk);
                look.run = first_clip(model, &file.run);
                Some(look)
            }
            _ => None,
        };
        e.insert(Cast {
            name: m.name.clone(),
            class: m.class.clone(),
            location: m.location,
            half_height: m.half_height,
            yaw: m.yaw,
            run_speed: m.ground_speed,
            walk_speed: m.ground_speed * file.walk_speed_ratio,
            goal: Goal::Stand,
            waiting: None,
            scripted: None,
            moving: None,
            look,
            time: 0.0,
            tag: m.tag.clone(),
            trigger: m.prop_trigger.clone(),
        });
    }
    if !plan.members.is_empty() {
        log_line(
            &log,
            format!(
                "{} cast members in the level, {drawn} drawn with their models",
                plan.members.len()
            ),
        );
    }
}

/// The model's meshes as children of the cast member.
fn spawn_pieces(
    model: Arc<Model>,
    m: &CastMember,
    ambient: f32,
    light_scale: f32,
    parent: &mut EntityCommands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> Look {
    let points = model.posed_local(&model.skeleton.local);
    let normals = model.normals(&points);
    let a = ambient.max(0.0);
    let mut pieces = Vec::new();
    // A prop's origin is where the map put it, and so is a character's
    // whose class turns `bAlignBottom` off (the gunship); any other
    // character stands on its feet (the model is lifted to the floor, see
    // rc_model).
    let drop = match m.kind {
        Kind::Pawn if m.align_bottom => 0.0,
        _ => -model.lift / UU_PER_UNIT,
    } * m.scale;
    parent.with_children(|p| {
        let mut holder = p.spawn((
            Transform {
                translation: Vec3::new(0.0, drop, 0.0),
                // The model faces the game's forward (+X, Bevy +X once
                // converted), as the cast member's yaw expects.
                rotation: Quat::IDENTITY,
                scale: Vec3::splat(m.scale),
            },
            Visibility::default(),
        ));
        holder.with_children(|c| {
            for (i, s) in model.sections.iter().enumerate() {
                let (pos, nrm) = model.section_attributes(s, &points, &normals);
                let mut mesh = Mesh::new(
                    PrimitiveTopology::TriangleList,
                    RenderAssetUsages::default(),
                );
                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
                mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, model.section_uvs(s));
                mesh.insert_indices(Indices::U32(s.indices.clone()));
                let handle = meshes.add(mesh);
                let texture = s.skin.as_ref().map(|k| {
                    images.add(Image::new(
                        Extent3d {
                            width: k.image.width as u32,
                            height: k.image.height as u32,
                            depth_or_array_layers: 1,
                        },
                        TextureDimension::D2,
                        k.image.pixels.clone(),
                        TextureFormat::Rgba8UnormSrgb,
                        RenderAssetUsages::RENDER_WORLD,
                    ))
                });
                // Holograms (the prologue's hologram rifles and advisor)
                // glow by themselves: unlit, added to what's behind them,
                // seen from both sides.
                let material = if s.skin.as_ref().is_some_and(|k| k.hologram) {
                    materials.add(StandardMaterial {
                        base_color_texture: texture,
                        unlit: true,
                        alpha_mode: AlphaMode::Add,
                        double_sided: true,
                        cull_mode: None,
                        ..default()
                    })
                } else {
                    // As for the squad: the level's light is baked into its
                    // surfaces and the scene has no ambient light, so give the
                    // character a steady share of its own colour.
                    materials.add(StandardMaterial {
                        base_color: if texture.is_some() {
                            Color::WHITE
                        } else {
                            Color::srgb(0.6, 0.6, 0.6)
                        },
                        emissive: LinearRgba::rgb(a, a, a)
                            * if texture.is_some() { 1.0 } else { 0.3 },
                        emissive_texture: texture.clone(),
                        base_color_texture: texture,
                        perceptual_roughness: 0.7,
                        alpha_mode: if s.skin.as_ref().is_some_and(|k| k.cutout) {
                            AlphaMode::Mask(0.5)
                        } else {
                            AlphaMode::Opaque
                        },
                        ..default()
                    })
                };
                let mut piece = c.spawn((
                    Mesh3d(handle.clone()),
                    MeshMaterial3d(material),
                    NoFrustumCulling,
                ));
                // rc_hologram swaps in the shimmering, flickering look.
                if s.skin.as_ref().is_some_and(|k| k.hologram) {
                    piece.insert(rc_hologram::Hologram { pivot: Vec3::ZERO });
                } else {
                    // Drawn by the level's light where it stands, as the
                    // squad and the droids are (the original lights
                    // characters by where they are, not at one level).
                    piece.insert(rc_game_core::LitByPlace {
                        target: rc_game_core::LightTarget::Emissive,
                        own: 0.0,
                        scale: light_scale,
                        height: 1.0,
                        made_at: a.powf(1.0 / 2.2),
                    });
                }
                pieces.push((i, handle));
            }
        });
    });
    Look {
        model,
        pieces,
        idle: None,
        walk: None,
        run: None,
        playing: None,
        frame: 0.0,
        posed_once: false,
    }
}

fn done_msg(w: &mut MessageWriter<CharacterOrderDone>, waiting: Option<(String, usize)>) {
    if let Some((script, index)) = waiting {
        w.write(CharacterOrderDone { script, index });
    }
}

/// Take the scripts' orders for the cast.
fn follow_orders(
    mut orders: MessageReader<CharacterOrder>,
    mut done: MessageWriter<CharacterOrderDone>,
    mut cast: Query<(&mut Cast, &mut Visibility)>,
    mut commands: Commands,
    log: Option<Res<GameLog>>,
) {
    for o in orders.read() {
        let Ok((mut c, mut vis)) = cast.get_mut(o.target) else {
            continue; // not one of ours
        };
        let at = format!("t={:.2} {} ({} #{})", c.time, c.name, o.script, o.index);
        // A new order replaces one the script stopped waiting for.
        c.waiting = None;
        let mut finished = false;
        match &o.act {
            CharacterAct::MoveTo { to, walk, face } => {
                let d = dist2d(c.location, *to);
                log_line(
                    &log,
                    format!(
                        "{at} {} to ({:.0}, {:.0}, {:.0}), {d:.0} uu away",
                        if *walk { "walks" } else { "runs" },
                        to[0],
                        to[1],
                        to[2]
                    ),
                );
                c.scripted = None;
                c.goal = Goal::MoveTo {
                    to: *to,
                    walk: *walk,
                    face: *face,
                };
            }
            CharacterAct::MoveToPlayer { tolerance } => {
                log_line(
                    &log,
                    format!("{at} goes to the player (within {tolerance:.0} uu)"),
                );
                c.scripted = None;
                c.goal = Goal::MoveToPlayer {
                    tolerance: *tolerance,
                };
            }
            CharacterAct::TurnToPlayer => {
                log_line(&log, format!("{at} turns to the player"));
                c.goal = Goal::TurnToPlayer;
            }
            CharacterAct::WaitForPlayer { distance } => {
                log_line(
                    &log,
                    format!("{at} waits for the player within {distance:.0} uu"),
                );
                c.goal = Goal::WaitForPlayer {
                    distance: *distance,
                    since: c.time,
                };
            }
            CharacterAct::PlayAnim { name, looping } => {
                let clip = c.look.as_ref().and_then(|l| l.model.named_clip(name));
                match clip {
                    Some(clip) => {
                        let secs = clip.num_frames / clip.rate.max(1.0);
                        log_line(
                            &log,
                            format!(
                                "{at} plays {name} ({secs:.2} s{})",
                                if *looping { ", looping" } else { "" }
                            ),
                        );
                        c.scripted = Some(Scripted {
                            clip,
                            looping: *looping,
                        });
                        if let Some(l) = c.look.as_mut() {
                            l.playing = None;
                        }
                        if *looping {
                            finished = true;
                        } else {
                            c.goal = Goal::AnimEnd;
                        }
                    }
                    None => {
                        log_line(
                            &log,
                            format!(
                                "{at} has no animation {name} in its sets{}; skipped",
                                if c.look.is_none() { " (no model)" } else { "" }
                            ),
                        );
                        finished = true;
                    }
                }
            }
            CharacterAct::WaitForAnimEnd => {
                if c.scripted.as_ref().is_some_and(|s| !s.looping) {
                    c.goal = Goal::AnimEnd;
                } else {
                    finished = true;
                }
            }
            CharacterAct::Teleport { to, yaw } => {
                log_line(
                    &log,
                    format!(
                        "{at} teleported to ({:.0}, {:.0}, {:.0})",
                        to[0], to[1], to[2]
                    ),
                );
                c.location = *to;
                c.yaw = *yaw;
                c.goal = Goal::Stand;
                finished = true;
            }
            CharacterAct::Hidden(h) => {
                *vis = if *h {
                    Visibility::Hidden
                } else {
                    Visibility::Inherited
                };
                log_line(
                    &log,
                    format!("{at} {}", if *h { "hidden" } else { "shown" }),
                );
                finished = true;
            }
            CharacterAct::Remove => {
                log_line(&log, format!("{at} removed"));
                commands.entity(o.target).despawn();
                finished = true;
            }
        }
        if o.waits {
            if finished {
                done.write(CharacterOrderDone {
                    script: o.script.clone(),
                    index: o.index,
                });
            } else {
                c.waiting = Some((o.script.clone(), o.index));
            }
        }
    }
}

fn dist2d(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Turn `yaw` toward `want` by at most `step`.
fn turn_toward(yaw: f32, want: f32, step: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let d = (want - yaw + PI).rem_euclid(TAU) - PI;
    yaw + d.clamp(-step, step)
}

/// Walk, run, turn and wait as the scripts asked.
fn move_cast(
    time: Res<Time>,
    plan: Res<CastPlan>,
    mut cast: Query<&mut Cast>,
    player: Query<&Transform, With<Player>>,
    mut done: MessageWriter<CharacterOrderDone>,
    log: Option<Res<GameLog>>,
) {
    let dt = time.delta_secs();
    let limit = plan
        .settings
        .as_ref()
        .map_or(20.0, |s| s.wait_for_player_limit);
    let player = player.single().ok().map(|t| {
        let p = to_unreal(t.translation);
        [p.x, p.y, p.z]
    });
    for mut c in &mut cast {
        c.time += dt;
        let c = &mut *c;
        c.moving = None;
        let goal = c.goal.clone();
        let mut finished = false;
        // Toward a point: (point, walk, stop within, face at the end).
        let target = match &goal {
            Goal::MoveTo { to, walk, face } => Some((*to, *walk, ARRIVED, *face)),
            Goal::MoveToPlayer { tolerance } => {
                player.map(|p| (p, false, tolerance.max(ARRIVED), None))
            }
            _ => None,
        };
        if let Some((to, walk, within, face)) = target {
            let d = dist2d(c.location, to);
            let speed = if walk { c.walk_speed } else { c.run_speed };
            if d <= within || speed <= 0.0 {
                if let Some(f) = face {
                    c.yaw = f;
                }
                // A character walking to a point ends on it, height and all
                // (the original's pawn would have walked the floor there).
                if matches!(goal, Goal::MoveTo { .. }) {
                    c.location = to;
                }
                finished = true;
            } else {
                let step = (speed * dt).min(d);
                let k = step / d;
                let dz = to[2] - c.location[2];
                c.location[0] += (to[0] - c.location[0]) * k;
                c.location[1] += (to[1] - c.location[1]) * k;
                c.location[2] += dz * k;
                let want = (to[1] - c.location[1]).atan2(to[0] - c.location[0]);
                c.yaw = turn_toward(c.yaw, want, TURN_RATE * dt * 2.0);
                c.moving = Some(walk);
            }
        }
        match &goal {
            Goal::TurnToPlayer => match player {
                Some(p) => {
                    let want = (p[1] - c.location[1]).atan2(p[0] - c.location[0]);
                    c.yaw = turn_toward(c.yaw, want, TURN_RATE * dt);
                    if (turn_toward(c.yaw, want, 0.01) - c.yaw).abs() < 0.01 {
                        finished = true;
                    }
                }
                None => finished = true,
            },
            Goal::WaitForPlayer { distance, since } => {
                let near = player.is_some_and(|p| {
                    let d = c
                        .location
                        .iter()
                        .zip(p)
                        .map(|(a, b)| (a - b).powi(2))
                        .sum::<f32>();
                    d.sqrt() <= *distance
                });
                if near {
                    finished = true;
                } else if limit > 0.0 && c.time - since > limit {
                    log_line(
                        &log,
                        format!(
                            "t={:.2} {}: the player didn't come within {distance:.0} uu in {limit:.0} s; carrying on",
                            c.time, c.name
                        ),
                    );
                    finished = true;
                }
            }
            Goal::AnimEnd => {
                let ended = match (&c.scripted, &c.look) {
                    (Some(s), Some(l)) => {
                        !s.looping
                            && l.playing.as_deref() == Some(&*s.clip.name)
                            && l.frame >= (s.clip.num_frames - 1.0).max(0.0)
                    }
                    _ => true,
                };
                if ended {
                    finished = true;
                }
            }
            _ => {}
        }
        if finished && goal != Goal::Stand {
            c.goal = Goal::Stand;
            done_msg(&mut done, c.waiting.take());
        }
    }
}

/// Put each cast member's body where it is.
fn place_cast(mut cast: Query<(&Cast, &mut Transform)>) {
    for (c, mut t) in &mut cast {
        let feet =
            rc_movement::Vec3::new(c.location[0], c.location[1], c.location[2] - c.half_height);
        let want =
            Transform::from_translation(to_bevy(feet)).with_rotation(Quat::from_rotation_y(-c.yaw));
        if t.translation != want.translation || t.rotation != want.rotation {
            t.translation = want.translation;
            t.rotation = want.rotation;
        }
    }
}

/// Level events trigger the props tagged with their name (`AnimProp`'s
/// `Trigger`): the scripts send `HoloProjector` and the projector plays
/// its throw.
fn trigger_props(
    mut events: MessageReader<ScriptEvent>,
    mut cast: Query<&mut Cast>,
    log: Option<Res<GameLog>>,
) {
    for e in events.read() {
        for mut c in &mut cast {
            if c.trigger.is_none() || !c.tag.eq_ignore_ascii_case(&e.name) {
                continue;
            }
            if let Some(what) = c.trigger() {
                log_line(
                    &log,
                    format!(
                        "t={:.2} {} triggered by {} ({}): {what}",
                        c.time, c.name, e.name, e.by
                    ),
                );
            }
        }
    }
}

/// Play each cast member's animation and pose its meshes. Only those
/// near the player are posed (`pose_distance`): the cutscene maps hold
/// many rooms, and the player sees one at a time.
fn animate_cast(
    time: Res<Time>,
    plan: Res<CastPlan>,
    mut cast: Query<(Entity, &mut Cast, &InheritedVisibility, &Transform)>,
    player: Query<&GlobalTransform, With<PlayerCamera>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    mut raise: MessageWriter<RaiseEvent>,
    log: Option<Res<GameLog>>,
) {
    let Some(mut meshes) = meshes else {
        return;
    };
    let dt = time.delta_secs();
    let reach = plan.settings.as_ref().map_or(4000.0, |s| s.pose_distance) / UU_PER_UNIT;
    let eye = player.single().ok().map(|t| t.translation());
    // Props whose animation triggers them again (`TriggerSelf`).
    let mut again = Vec::new();
    for (entity, mut c, shown, at) in &mut cast {
        let name_of = c.name.clone();
        let Cast {
            scripted,
            moving,
            look,
            ..
        } = &mut *c;
        let Some(look) = look.as_mut() else {
            continue;
        };
        let (clip, looping) = match (scripted.as_ref(), *moving) {
            (_, Some(true)) => (look.walk.as_ref(), true),
            (_, Some(false)) => (look.run.as_ref(), true),
            (Some(s), None) => (Some(&s.clip), s.looping),
            (None, None) => (look.idle.as_ref(), true),
        };
        let same = match (clip, &look.playing) {
            (Some(k), Some(p)) => k.name == *p,
            (None, None) => true,
            _ => false,
        };
        // The stretches of the clip passed this frame (0 to 1 through
        // it), for the events its notifies send.
        let mut passed: Vec<(f32, f32)> = Vec::new();
        if !same {
            look.playing = clip.map(|k| k.name.clone());
            look.frame = 0.0;
            passed.push((-1.0, 0.0));
        } else if let Some(k) = clip {
            let n = k.num_frames.max(1.0);
            let before = look.frame;
            let ahead = before + dt * k.rate;
            look.frame = if looping {
                ahead.rem_euclid(n)
            } else {
                ahead.min(n - 1.0)
            };
            if looping && ahead >= n {
                passed.push((before / n, 1.0));
                passed.push((-1.0, look.frame / n));
            } else if !looping && look.frame >= n - 1.0 {
                // The last frame shows the end: notifies at the very end
                // go off too, once.
                if before < n - 1.0 {
                    passed.push((before / n, 1.0));
                }
            } else {
                passed.push((before / n, look.frame / n));
            }
        }
        let mut self_trigger = false;
        if let Some(k) = clip {
            for (from, to) in passed {
                for t in look.model.triggers_between(&k.name, from, to) {
                    match t {
                        NotifyTrigger::Event(name) => {
                            log_line(
                                &log,
                                format!("{} animation {} sends event {name}", name_of, k.name),
                            );
                            raise.write(RaiseEvent {
                                name,
                                by: format!("{name_of} animation"),
                            });
                        }
                        NotifyTrigger::SelfTrigger => self_trigger = true,
                    }
                }
            }
        }
        if self_trigger {
            again.push(entity);
        }
        // Hidden or far away ones aren't posed; a still pose only once.
        let far = reach > 0.0 && eye.is_some_and(|e| e.distance(at.translation) > reach);
        if !shown.get() || (far && look.posed_once) || (clip.is_none() && look.posed_once) {
            continue;
        }
        look.posed_once = true;
        let local = match clip {
            Some(k) => look.model.local_clip(k, look.frame),
            None => look.model.skeleton.local.clone(),
        };
        let points = look.model.posed_local(&local);
        let normals = look.model.normals(&points);
        for (section, handle) in &look.pieces {
            let (pos, nrm) =
                look.model
                    .section_attributes(&look.model.sections[*section], &points, &normals);
            if let Some(mut m) = meshes.get_mut(handle) {
                m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
                m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
            }
        }
    }
    for e in again {
        if let Ok((_, mut c, _, _)) = cast.get_mut(e) {
            if let Some(what) = c.trigger() {
                log_line(
                    &log,
                    format!("{} animation triggers itself: {what}", c.name),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_takes_the_short_way_round() {
        let y = turn_toward(3.0, -3.0, 0.1);
        assert!(y > 3.0, "turned the long way: {y}");
        assert!((turn_toward(0.0, 0.05, 0.1) - 0.05).abs() < 1e-4);
    }
}
