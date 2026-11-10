//! Props drawn with an animated model: bacta dispensers, and anything
//! else whose class draws a skeletal mesh (`DrawType` `DT_Mesh`) rather
//! than a static one. `rc_level::map_collision` lists them
//! (`MapCollision::anim_props`); this file reads each model once with
//! `rc_model` and draws every copy where the map puts it.
//!
//! They stand still in the model's own pose for now: the dispenser's
//! `Load` and `Holster` animations aren't played, and they aren't solid.
//! Each gets an [`AnimProp`] so later parts (a script hiding one, an
//! animation) can find it by the map's actor name.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rc_game_core::{GamePaths, GameState};
use rc_level::actors::Placement;
use rc_model::{to_bevy, Model, ModelSpec};
use std::collections::HashMap;

/// Light the model always gets, as for the squad and the droids: the
/// level's light is baked into its surfaces and the scene has no ambient
/// light. A guess (0 to 1).
const AMBIENT: f32 = 0.5;

/// Bevy units to Unreal units (`rc_model::to_bevy` undone).
const UNREAL_PER_BEVY: f32 = 50.0;

/// One drawn animated prop.
#[derive(Component, Debug, Clone)]
pub struct AnimProp {
    /// The map's actor name (`BactaDispenserGEO0`).
    pub name: String,
    pub class: String,
    pub export: usize,
}

/// What `spawn` did, for the log.
#[derive(Default, Debug)]
pub struct Report {
    pub drawn: usize,
    /// "Package.Mesh x count" for each model drawn.
    pub models: Vec<String>,
    /// Why some couldn't be drawn.
    pub failures: Vec<String>,
}

/// The game path of an animation package (`BactaDispensers` ->
/// `GameData/Animations/BactaDispensers.ukx`).
pub fn package_path(package: &str) -> String {
    format!("GameData/Animations/{package}.ukx")
}

/// Points of `model`, posed as stored, placed where `place` stands, in
/// Bevy units.
pub fn placed_points(model: &Model, place: &Placement) -> Vec<[f32; 3]> {
    model
        .posed_from_origin("", 0.0)
        .into_iter()
        .map(|b| {
            let u = [b[0] * UNREAL_PER_BEVY, b[2] * UNREAL_PER_BEVY, b[1] * UNREAL_PER_BEVY];
            to_bevy(place.transform_point(u))
        })
        .collect()
}

/// Read each model once and draw every prop. Entities go away when the
/// game leaves `InGame`.
pub fn spawn(world: &mut World, paths: &GamePaths, props: &[(Placement, String, String)]) -> Report {
    let mut report = Report::default();
    // Per model: the model and one material per section (shared by copies).
    let mut models: HashMap<String, Option<(Model, Vec<Handle<StandardMaterial>>)>> = HashMap::new();
    let mut counts: Vec<(String, usize)> = Vec::new();
    for (place, package, mesh) in props {
        let key = format!("{package}.{mesh}");
        if !models.contains_key(&key) {
            let spec = ModelSpec {
                package: package_path(package),
                mesh: mesh.clone(),
                ..Default::default()
            };
            let loaded = match Model::load(&|rel| paths.game_file(rel), &spec) {
                Ok((m, _notes)) => {
                    let mats = materials_for(world, &m);
                    Some((m, mats))
                }
                Err(e) => {
                    report.failures.push(format!("{key}: {e}"));
                    None
                }
            };
            models.insert(key.clone(), loaded);
        }
        let Some((model, mats)) = models.get(&key).and_then(|m| m.as_ref()) else {
            continue;
        };
        let points = placed_points(model, place);
        let normals = model.normals(&points);
        let mut pieces = Vec::new();
        for (s, mat) in model.sections.iter().zip(mats) {
            let (pos, nrm) = model.section_attributes(s, &points, &normals);
            let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, model.section_uvs(s));
            m.insert_indices(Indices::U32(s.indices.clone()));
            pieces.push((world.resource_mut::<Assets<Mesh>>().add(m), mat.clone()));
        }
        world
            .spawn((
                AnimProp {
                    name: place.name.clone(),
                    class: place.class.clone(),
                    export: place.export,
                },
                Transform::default(),
                Visibility::default(),
                DespawnOnExit(GameState::InGame),
            ))
            .with_children(|c| {
                for (mesh, mat) in pieces {
                    c.spawn((Mesh3d(mesh), MeshMaterial3d(mat)));
                }
            });
        report.drawn += 1;
        match counts.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) => *n += 1,
            None => counts.push((key, 1)),
        }
    }
    report.models = counts.into_iter().map(|(k, n)| format!("{k} x{n}")).collect();
    report
}

/// One material per section of `model`, with its picture.
fn materials_for(world: &mut World, model: &Model) -> Vec<Handle<StandardMaterial>> {
    let a = AMBIENT;
    model
        .sections
        .iter()
        .map(|s| {
            let texture = s.skin.as_ref().map(|k| {
                world.resource_mut::<Assets<Image>>().add(Image::new(
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
            world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
                base_color: if texture.is_some() {
                    Color::WHITE
                } else {
                    Color::srgb(0.6, 0.6, 0.6)
                },
                emissive: LinearRgba::rgb(a, a, a) * if texture.is_some() { 1.0 } else { 0.3 },
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
        })
        .collect()
}
