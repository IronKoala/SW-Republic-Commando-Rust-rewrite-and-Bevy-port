//! Drawing the first-person arms (see `arms` for which animation plays).
//!
//! As in the game, the arms are drawn after the world with their own field
//! of view (the weapon's `DisplayFOV`) and their own depth, so the gun never
//! pokes into a wall you stand against and a wider view setting doesn't
//! stretch it. Here that is a second camera, a child of the player's camera,
//! that sees only the arms (render layer [`ARMS_LAYER`]) and draws over the
//! player camera's picture. Camera orders: sky -1, player 0, arms
//! [`ARMS_CAMERA_ORDER`], the helmet frame 2 (in `rc_game`), menus and
//! display 3.
//!
//! The model is posed on the CPU each frame like the squad's (`rc_anim`
//! moves its points). It's drawn unlit at the weapon's `arms.brightness`:
//! the game lights it by the zone the player stands in, which we don't read
//! yet.

use crate::arms::{Arms, ArmsAnim, ArmsClips, ArmsInput};
use crate::library::WeaponLibrary;
use crate::melee::{MeleeSwing, Zoom};
use crate::{Arsenal, Dead, ThrowGrenade};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::camera::Exposure;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::Tonemapping;
use rc_game_core::{GameLog, GamePaths, Player, PlayerCamera};
use rc_model::{Model, ModelSpec};
use std::collections::HashMap;
use std::sync::Arc;

/// The render layer only the arms camera sees (the sky has layer 1).
pub const ARMS_LAYER: usize = 2;
/// Drawn after the player's view (0) and before the helmet frame (2).
pub const ARMS_CAMERA_ORDER: isize = 1;

/// What the arms show, for displays, tests and mods: the weapon drawn and
/// the animation playing (the game's sequence name).
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct ArmsReadout {
    pub weapon: Option<String>,
    pub animation: &'static str,
    /// Seconds into it.
    pub time: f32,
    pub hidden: bool,
}

/// The camera that draws the arms.
#[derive(Component)]
pub struct ArmsCamera;

/// The arms model in hand: which weapon, and its pieces to pose.
#[derive(Component)]
pub(crate) struct ArmsPieces {
    weapon: String,
    /// Per material section, its mesh.
    meshes: Vec<(usize, Handle<Mesh>)>,
}

/// Arms models read so far, by weapon id (an error if it couldn't be).
#[derive(Resource, Default)]
pub(crate) struct ArmsModels(HashMap<String, Result<Arc<Model>, String>>);

impl ArmsModels {
    /// Whether `weapon`'s arms are drawn (or still on their way), so the
    /// plain model in `view` should stay away.
    pub(crate) fn covers(&self, weapon: &str) -> bool {
        !matches!(self.0.get(weapon), Some(Err(_)))
    }
}

struct Clips<'a> {
    models: &'a ArmsModels,
    lib: &'a WeaponLibrary,
}

impl ArmsClips for Clips<'_> {
    fn length(&self, weapon: &str, anim: ArmsAnim) -> Option<f32> {
        let model = self.models.0.get(weapon)?.as_ref().ok()?;
        model.clip(anim.sequence()).map(|c| c.duration())
    }

    fn group(&self, weapon: &str) -> Option<&str> {
        self.lib
            .weapons
            .get(weapon)?
            .arms
            .as_ref()?
            .attachment_group
            .as_deref()
    }
}

/// The game's view setting is degrees across a 4:3 screen; Bevy wants the
/// up-and-down angle (radians), which stays the same at any screen shape
/// (Hor+), as in the game shell.
fn vertical_fov(across_4_3_degrees: f32) -> f32 {
    2.0 * ((across_4_3_degrees.to_radians() / 2.0).tan() / (4.0 / 3.0)).atan()
}

/// Give the player's camera its arms camera.
pub(crate) fn add_arms_camera(
    mut commands: Commands,
    players: Query<Entity, With<PlayerCamera>>,
    arms: Query<&ChildOf, With<ArmsCamera>>,
    renderer: Option<Res<Assets<Mesh>>>,
) {
    // Tests without a renderer have nothing to draw with.
    if renderer.is_none() {
        return;
    }
    for cam in &players {
        if arms.iter().any(|c| c.parent() == cam) {
            continue;
        }
        commands.entity(cam).with_children(|c| {
            c.spawn((
                ArmsCamera,
                Camera3d::default(),
                Camera {
                    order: ARMS_CAMERA_ORDER,
                    // Keep the world's picture; the depth starts afresh.
                    clear_color: ClearColorConfig::None,
                    ..default()
                },
                Tonemapping::None,
                Exposure::default(),
                Projection::Perspective(PerspectiveProjection {
                    fov: vertical_fov(85.0),
                    // The gun comes as close as 1 uu to the eye.
                    near: 0.01,
                    far: 20.0,
                    ..default()
                }),
                RenderLayers::layer(ARMS_LAYER),
                Transform::IDENTITY,
            ));
        });
    }
}

/// Read the arms of every weapon the player carries, once each.
pub(crate) fn load_arms(
    lib: Res<WeaponLibrary>,
    paths: Option<Res<GamePaths>>,
    players: Query<&Arsenal, With<Player>>,
    models: Option<ResMut<ArmsModels>>,
    mut commands: Commands,
    log: Option<Res<GameLog>>,
) {
    let Some(mut models) = models else {
        commands.insert_resource(ArmsModels::default());
        return;
    };
    let (Some(paths), Ok(arsenal)) = (paths, players.single()) else {
        return;
    };
    let say = |s: String| match &log {
        Some(l) => l.line(s),
        None => println!("{s}"),
    };
    for (id, _) in &arsenal.weapons {
        if models.0.contains_key(id) {
            continue;
        }
        let Some(a) = lib.weapons.get(id).and_then(|p| p.arms.as_ref()) else {
            continue;
        };
        let spec = ModelSpec {
            package: a.package.clone(),
            mesh: a.mesh.clone(),
            set: None,
            sequences: ArmsAnim::ALL
                .iter()
                .map(|s| (s.sequence().to_string(), s.sequence().to_string()))
                .collect(),
            skins: vec![],
        };
        let made = Model::load(&|rel| paths.game_file(rel), &spec);
        match &made {
            Ok((m, notes)) => {
                let found: Vec<&str> = ArmsAnim::ALL
                    .iter()
                    .map(|a| a.sequence())
                    .filter(|s| m.clip(s).is_some())
                    .collect();
                say(format!(
                    "arms: {id}: {} from {}, {} pieces, animations {}",
                    a.mesh,
                    a.package,
                    m.sections.len(),
                    found.join(" ")
                ));
                // The reader's notes, without the "not in its sets" lines:
                // not every weapon has every move, and the rules expect it.
                for n in notes.iter().filter(|n| !n.contains("no animation called")) {
                    say(format!("arms: {id}: {n}"));
                }
            }
            Err(e) => say(format!(
                "arms: {id}: {} not shown ({e}); the plain model instead",
                a.mesh
            )),
        }
        models.0.insert(id.clone(), made.map(|(m, _)| Arc::new(m)));
    }
}

/// What the arms remember between frames, to notice what changed.
#[derive(Default)]
pub(crate) struct Watch {
    arms: Arms,
    /// The weapon in hand and its shot count last frame.
    shots: Option<(String, u32)>,
    reloading: bool,
    feet: Option<Vec3>,
    logged: Option<(Option<String>, ArmsAnim)>,
}

/// Pick the animation, swap the model when the weapon changes, and pose it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn animate_arms(
    mut commands: Commands,
    time: Res<Time>,
    lib: Res<WeaponLibrary>,
    models: Option<Res<ArmsModels>>,
    zoom: Option<Res<Zoom>>,
    players: Query<(Entity, &Arsenal, &Transform, Has<Dead>), With<Player>>,
    mut cameras: Query<(Entity, &mut Projection), With<ArmsCamera>>,
    mut pieces: Query<(Entity, &ArmsPieces, &mut Visibility)>,
    mut throws: MessageReader<ThrowGrenade>,
    mut swings: MessageReader<MeleeSwing>,
    mut watch: Local<Watch>,
    mut assets: (
        Option<ResMut<Assets<Mesh>>>,
        Option<ResMut<Assets<StandardMaterial>>>,
        Option<ResMut<Assets<Image>>>,
    ),
    log: Option<Res<GameLog>>,
) {
    let (Some(models), Ok((me, arsenal, body, down)), Ok((camera, mut projection))) =
        (models, players.single(), cameras.single_mut())
    else {
        throws.clear();
        swings.clear();
        return;
    };
    let dt = time.delta_secs();
    let current = arsenal.current();
    let weapon = current
        .map(|(id, _)| id.clone())
        .filter(|id| matches!(models.0.get(id), Some(Ok(_))));
    // A shot fired, or a reload begun, with the same weapon as last frame.
    let (fired, reload_started) = match current {
        Some((id, w)) => {
            let same = watch.shots.as_ref().is_some_and(|(i, _)| i == id);
            let fired = same
                && watch
                    .shots
                    .as_ref()
                    .is_some_and(|(_, n)| w.shots_fired > *n);
            let started = same && w.reloading.is_some() && !watch.reloading;
            watch.shots = Some((id.clone(), w.shots_fired));
            watch.reloading = w.reloading.is_some();
            (fired, started)
        }
        None => {
            watch.shots = None;
            (false, false)
        }
    };
    let speed = match watch.feet {
        Some(was) if dt > 0.0 => {
            let d = body.translation - was;
            Vec2::new(d.x, d.z).length() * rc_game_core::units::UU_PER_UNIT / dt
        }
        _ => 0.0,
    };
    watch.feet = Some(body.translation);
    let zoomed = zoom.as_ref().is_some_and(|z| z.zoomed().is_some());
    let input = ArmsInput {
        dt,
        weapon,
        speed,
        zoomed,
        fired,
        reload_started,
        threw: throws.read().any(|t| t.thrower == me),
        melee: swings.read().any(|s| s.attacker == me),
    };
    let clips = Clips {
        models: &models,
        lib: &lib,
    };
    watch.arms.step(&input, &clips);
    let arms = watch.arms.clone();
    let now = (arms.shown.clone(), arms.anim);
    if watch.logged.as_ref() != Some(&now) {
        let line = format!(
            "arms: {} plays {} (speed {speed:.0} uu/s{})",
            arms.shown.as_deref().unwrap_or("nothing"),
            arms.anim.sequence(),
            if zoomed { ", zoomed" } else { "" }
        );
        match &log {
            Some(l) => l.line(line),
            None => println!("{line}"),
        }
        watch.logged = Some(now);
    }

    let params = arms
        .shown
        .as_deref()
        .and_then(|w| lib.weapons.get(w))
        .and_then(|p| p.arms.as_ref());
    if let (Some(a), Projection::Perspective(p)) = (params, &mut *projection) {
        p.fov = vertical_fov(a.fov);
    }
    let hidden = down || (zoomed && params.is_some_and(|a| a.hide_zoomed));
    commands.insert_resource(ArmsReadout {
        weapon: arms.shown.clone(),
        animation: arms.anim.sequence(),
        time: arms.time,
        hidden,
    });

    // The model in hand: keep it, or swap it for the shown weapon's.
    let mut drawn = None;
    for (e, p, mut vis) in &mut pieces {
        if Some(&p.weapon) == arms.shown.as_ref() {
            *vis = if hidden {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
            drawn = Some(e);
        } else {
            commands.entity(e).despawn();
        }
    }
    let (Some(mut meshes), Some(mut materials), Some(mut images)) =
        (assets.0.take(), assets.1.take(), assets.2.take())
    else {
        return;
    };
    let Some(id) = arms.shown.as_ref() else {
        return;
    };
    let Some(Ok(model)) = models.0.get(id) else {
        return;
    };
    let points = pose(model, arms.anim, arms.time);
    let normals = model.normals(&points);
    match drawn.and_then(|e| pieces.get(e).ok()) {
        Some((_, p, _)) => {
            for (section, handle) in &p.meshes {
                let (pos, nrm) =
                    model.section_attributes(&model.sections[*section], &points, &normals);
                if let Some(mut m) = meshes.get_mut(handle) {
                    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
                    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
                }
            }
        }
        None => {
            let brightness = params.map_or(1.0, |a| a.brightness);
            let made = spawn_pieces(
                model,
                &points,
                &normals,
                brightness,
                &mut meshes,
                &mut materials,
                &mut images,
            );
            commands.entity(camera).with_children(|c| {
                c.spawn((
                    ArmsPieces {
                        weapon: id.clone(),
                        meshes: made.iter().map(|(s, m, _)| (*s, m.clone())).collect(),
                    },
                    // The model's axes (to_bevy: forward +X, right +Z) to
                    // the camera's (forward -Z, right +X): a quarter turn.
                    Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
                    if hidden {
                        Visibility::Hidden
                    } else {
                        Visibility::Inherited
                    },
                ))
                .with_children(|h| {
                    for (_, mesh, material) in made {
                        h.spawn((
                            Mesh3d(mesh),
                            MeshMaterial3d(material),
                            RenderLayers::layer(ARMS_LAYER),
                            // Posed on the CPU: the stored bounds don't follow.
                            NoFrustumCulling,
                        ));
                    }
                });
            });
        }
    }
}

/// The points at `time` seconds into `anim`. Moves that play once stop on
/// their last frame rather than running on into the first.
fn pose(model: &Model, anim: ArmsAnim, time: f32) -> Vec<[f32; 3]> {
    let key = anim.sequence();
    let frame = match model.clip(key) {
        Some(c) if anim.loops() => time * c.rate,
        Some(c) => (time * c.rate).min((c.num_frames - 1.0).max(0.0)),
        None => 0.0,
    };
    model.posed_from_origin(key, frame)
}

fn spawn_pieces(
    model: &Model,
    points: &[[f32; 3]],
    normals: &[[f32; 3]],
    brightness: f32,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> Vec<(usize, Handle<Mesh>, Handle<StandardMaterial>)> {
    let b = brightness.clamp(0.0, 1.0);
    let mut out = vec![];
    for (i, s) in model.sections.iter().enumerate() {
        let (pos, nrm) = model.section_attributes(s, points, normals);
        let mut m = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, model.section_uvs(s));
        m.insert_indices(Indices::U32(s.indices.clone()));
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
        let grey = if texture.is_some() { 1.0 } else { 0.6 };
        let material = materials.add(StandardMaterial {
            base_color: Color::srgb(grey * b, grey * b, grey * b),
            base_color_texture: texture,
            unlit: true,
            alpha_mode: if s.skin.as_ref().is_some_and(|k| k.cutout) {
                AlphaMode::Mask(0.5)
            } else {
                AlphaMode::Opaque
            },
            ..default()
        });
        out.push((i, meshes.add(m), material));
    }
    out
}
