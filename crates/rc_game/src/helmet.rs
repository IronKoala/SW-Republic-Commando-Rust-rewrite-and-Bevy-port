//! HELMET: the commando's helmet frame round the edge of the view.
//!
//! As in the original, it is a 3D model drawn at the eye with its own view
//! angle (`rc_menus::helmet` reads it from the install and fits it to the
//! screen's shape). Here that is a third camera, a child of the player's
//! camera, that sees only the helmet (render layer [`HELMET_LAYER`]) and
//! draws over the world and the first-person arms, because the visor is
//! nearer your eyes than the gun. Camera orders: sky -1, player 0, arms 1,
//! helmet [`HELMET_CAMERA_ORDER`], menus and display 3.
//!
//! The GAME options page's HELMET row (settings.toml `[game] helmet`) shows
//! or hides it; `data/ui/helmet.toml` holds the rest. It is drawn unlit at
//! the plan's brightness, a guess: the original lights it with lights
//! inside the helmet, which we don't copy yet. If the model can't be read
//! nothing is drawn and `logs/rc_game.log` says why ("helmet: ...").

use crate::settings::Settings;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::camera::Exposure;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::Tonemapping;
use bevy::window::PrimaryWindow;
use rc_game_core::{GameLog, GamePaths, GameSet, GameState, PlayerCamera};
use rc_menus::helmet::{self, HelmetModel, HelmetPlan};
use std::sync::Arc;

/// The render layer only the helmet camera sees (sky 1, arms 2).
pub const HELMET_LAYER: usize = 3;
/// After the arms (1), before menus and the display (3).
pub const HELMET_CAMERA_ORDER: isize = 2;

/// What the helmet shows, for tests and mods.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct HelmetReadout {
    /// The model was read and is ready to draw.
    pub loaded: bool,
    /// It is on screen now (in a level, with the HELMET setting on).
    pub shown: bool,
    /// Triangles drawn.
    pub triangles: usize,
    /// The screen shape it was last fitted to (width / height).
    pub fitted_to: f32,
}

/// The plan and the model (or why there isn't one).
#[derive(Resource, Default)]
pub struct Helmet {
    pub plan: HelmetPlan,
    pub model: Option<Result<Arc<HelmetModel>, String>>,
}

#[derive(Component)]
pub struct HelmetCamera;

/// One drawn part of the model, with its mesh to refit.
#[derive(Component)]
struct HelmetPiece {
    part: usize,
}

pub struct HelmetPlugin;

impl Plugin for HelmetPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Helmet>()
            .init_resource::<HelmetReadout>()
            .add_systems(Startup, read_plan)
            .add_systems(
                Update,
                (load_model, add_helmet_camera, fit_and_show)
                    .chain()
                    .in_set(GameSet::Hud),
            );
    }
}

fn read_plan(mut helmet: ResMut<Helmet>, paths: Option<Res<GamePaths>>, log: Option<Res<GameLog>>) {
    const REL: &str = "ui/helmet.toml";
    let text = paths
        .as_ref()
        .and_then(|p| p.data_file(REL))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| include_str!("../../../data/ui/helmet.toml").to_string());
    match HelmetPlan::parse(&text) {
        Ok(p) => helmet.plan = p,
        Err(e) => {
            if let Some(l) = log {
                l.line(format!("helmet: {REL}: {e}; using the defaults"));
            }
        }
    }
}

/// Read the model once, the first time a level is shown.
fn load_model(
    mut helmet: ResMut<Helmet>,
    paths: Option<Res<GamePaths>>,
    state: Option<Res<State<GameState>>>,
    log: Option<Res<GameLog>>,
    mut readout: ResMut<HelmetReadout>,
) {
    if helmet.model.is_some() || !helmet.plan.enabled {
        return;
    }
    if state.is_none_or(|s| *s.get() != GameState::InGame) {
        return;
    }
    let Some(paths) = paths else {
        return;
    };
    let say = |s: String| match &log {
        Some(l) => l.line(s),
        None => println!("{s}"),
    };
    let mut lib = rc_menus::Library::new(&paths.game_data());
    let made = helmet::load(&mut lib, &helmet.plan);
    match &made {
        Ok(m) => {
            say(format!(
                "helmet: {} read, {} points, DisplayFOV {}",
                helmet.plan.mesh,
                m.points.len(),
                m.display_fov
            ));
            for n in &m.notes {
                say(format!("helmet: {n}"));
            }
        }
        Err(e) => say(format!("helmet: not drawn ({e})")),
    }
    readout.loaded = made.is_ok();
    helmet.model = Some(made.map(Arc::new));
}

/// Give the player's camera its helmet camera and the model's parts.
#[allow(clippy::too_many_arguments)]
fn add_helmet_camera(
    mut commands: Commands,
    helmet: Res<Helmet>,
    players: Query<Entity, With<PlayerCamera>>,
    existing: Query<&ChildOf, With<HelmetCamera>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    images: Option<ResMut<Assets<Image>>>,
) {
    let Some(Ok(model)) = &helmet.model else {
        return;
    };
    // Tests without a renderer have nothing to draw with.
    let (Some(mut meshes), Some(mut materials), Some(mut images)) = (meshes, materials, images)
    else {
        return;
    };
    for cam in &players {
        if existing.iter().any(|c| c.parent() == cam) {
            continue;
        }
        let t = helmet::half_height(model.display_fov);
        let b = helmet.plan.brightness.clamp(0.0, 1.0);
        commands.entity(cam).with_children(|c| {
            c.spawn((
                HelmetCamera,
                Camera3d::default(),
                Camera {
                    order: HELMET_CAMERA_ORDER,
                    // Keep the picture so far; the depth starts afresh.
                    clear_color: ClearColorConfig::None,
                    ..default()
                },
                Tonemapping::None,
                Exposure::default(),
                Projection::Perspective(PerspectiveProjection {
                    fov: 2.0 * t.atan(),
                    near: 0.01,
                    far: 5.0,
                    ..default()
                }),
                RenderLayers::layer(HELMET_LAYER),
                Transform::IDENTITY,
                Visibility::Hidden,
            ))
            .with_children(|h| {
                for (i, part) in model.parts.iter().enumerate() {
                    let pic = &part.picture;
                    let texture = images.add(Image::new(
                        Extent3d {
                            width: pic.width as u32,
                            height: pic.height as u32,
                            depth_or_array_layers: 1,
                        },
                        TextureDimension::D2,
                        pic.pixels.clone(),
                        TextureFormat::Rgba8UnormSrgb,
                        RenderAssetUsages::RENDER_WORLD,
                    ));
                    let material = materials.add(StandardMaterial {
                        base_color: Color::srgb(b, b, b),
                        base_color_texture: Some(texture),
                        unlit: true,
                        // Seen from inside: draw both faces.
                        cull_mode: None,
                        double_sided: true,
                        ..default()
                    });
                    let mesh = meshes.add(part_mesh(
                        model,
                        i,
                        &helmet::fit(model, &helmet.plan, 4.0 / 3.0),
                    ));
                    h.spawn((
                        HelmetPiece { part: i },
                        Mesh3d(mesh),
                        MeshMaterial3d(material),
                        RenderLayers::layer(HELMET_LAYER),
                        // Refitted for each screen shape; bounds don't follow.
                        NoFrustumCulling,
                        // Already in the camera's axes (`part_mesh`).
                        Transform::IDENTITY,
                    ));
                }
            });
        });
    }
}

/// The mesh of part `i`, with the model's points fitted to a screen shape,
/// turned to the camera's axes (forward -Z, right +X, up +Y) and scaled to
/// Bevy units.
fn part_mesh(model: &HelmetModel, i: usize, points: &[[f32; 3]]) -> Mesh {
    let s = 1.0 / rc_game_core::units::UU_PER_UNIT;
    let to_view = |p: &[f32; 3]| [p[1] * s, p[2] * s, -p[0] * s];
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    m.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        points.iter().map(to_view).collect::<Vec<_>>(),
    );
    m.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        model
            .normals
            .iter()
            .map(|n| [n[1], n[2], -n[0]])
            .collect::<Vec<_>>(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, model.uvs.clone());
    // Unreal's axes are left-handed and Bevy's right-handed, so this turn
    // is also a mirror, which keeps the corner order of each front face as
    // it is (as in `rc_level_viewer`).
    m.insert_indices(Indices::U32(model.parts[i].indices.clone()));
    m
}

/// Refit the model when the screen changes shape, and show or hide it.
#[allow(clippy::too_many_arguments)]
fn fit_and_show(
    helmet: Res<Helmet>,
    settings: Res<Settings>,
    state: Option<Res<State<GameState>>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut cameras: Query<(&mut Visibility, &Camera), With<HelmetCamera>>,
    pieces: Query<(&HelmetPiece, &Mesh3d)>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    mut readout: ResMut<HelmetReadout>,
    log: Option<Res<GameLog>>,
) {
    let Some(Ok(model)) = &helmet.model else {
        readout.shown = false;
        return;
    };
    let in_game = state.is_some_and(|s| *s.get() == GameState::InGame);
    let show = in_game && helmet.plan.enabled && settings.game.helmet;
    let mut any = false;
    // The box the helmet is drawn in: the camera's own (smaller than the
    // window when full screen adds black bars), else the window.
    let mut drawn: Option<Vec2> = None;
    for (mut v, cam) in &mut cameras {
        any = true;
        drawn = drawn.or(cam.logical_viewport_size());
        let want = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
    let shown = show && any;
    if shown != readout.shown {
        if let Some(l) = &log {
            l.line(format!(
                "helmet: {}",
                if shown { "shown" } else { "hidden" }
            ));
        }
        readout.shown = shown;
    }
    let Some(size) = drawn.or_else(|| {
        windows
            .single()
            .ok()
            .map(|w| Vec2::new(w.resolution.width(), w.resolution.height()))
    }) else {
        return;
    };
    let (w, h) = (size.x, size.y);
    if w < 1.0 || h < 1.0 {
        return;
    }
    let aspect = w / h;
    let pieces: Vec<_> = pieces.iter().collect();
    if pieces.is_empty() || (aspect - readout.fitted_to).abs() < 1e-4 {
        return;
    }
    let Some(mut meshes) = meshes else {
        return;
    };
    let points = helmet::fit(model, &helmet.plan, aspect);
    let mut triangles = 0;
    for (piece, mesh) in pieces {
        if let Some(mut m) = meshes.get_mut(&mesh.0) {
            *m = part_mesh(model, piece.part, &points);
            triangles += model.parts[piece.part].indices.len() / 3;
        }
    }
    readout.triangles = triangles;
    readout.fitted_to = aspect;
    if let Some(l) = &log {
        l.line(format!(
            "helmet: fitted to a {w}x{h} screen (shape {aspect:.3}), {triangles} triangles"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_plan_reads() {
        let p = HelmetPlan::parse(include_str!("../../../data/ui/helmet.toml")).unwrap();
        assert!(p.enabled);
        assert!(p.brightness > 0.0 && p.brightness <= 1.0);
    }

    #[test]
    fn axes_turn_to_the_camera() {
        // A point straight ahead, a bit right and up, lands in front of the
        // camera (-Z), right (+X) and up (+Y).
        let model = HelmetModel {
            points: vec![[10.0, 2.0, 3.0]; 3],
            normals: vec![[1.0, 0.0, 0.0]; 3],
            uvs: vec![[0.0; 2]; 3],
            parts: vec![helmet::HelmetPart {
                material: "m".into(),
                picture: rc_image::Rgba {
                    width: 1,
                    height: 1,
                    pixels: vec![0; 4],
                },
                indices: vec![0, 1, 2],
            }],
            display_fov: 84.0,
            notes: vec![],
        };
        let m = part_mesh(&model, 0, &model.points);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) =
            m.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("no positions");
        };
        assert!(
            p[0][0] > 0.0 && p[0][1] > 0.0 && p[0][2] < 0.0,
            "{:?}",
            p[0]
        );
    }
}
