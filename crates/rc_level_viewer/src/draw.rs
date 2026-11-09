//! Turns a built `Scene` into Bevy meshes, materials and lights. Shared by
//! the level viewer and the first-person walker (`rc_walk`), so a level
//! looks the same in both.

use crate::bump::{self, BumpLitMaterial};
use crate::lighting::{self, LightingSettings, SpawnReport};
use crate::materials;
use crate::scene::Scene;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::Exposure;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::Lightmap;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::HashMap;

/// Sky-blue-grey background, as 0-255 sRGB values.
pub const BACKGROUND: [u8; 3] = [64, 72, 89];

/// Marks the drawn triangles of a mover (door, lift), or of a prop with a
/// tag of its own that a level script may remove, by its export index in
/// the map. Their vertices are where the map placed it; moving the
/// entity's `Transform` moves it, and hiding the entity hides it.
#[derive(Component, Debug, Clone, Copy)]
pub struct MoverPart(pub usize);

/// What `spawn_level` drew, for the log.
#[derive(Debug, Default, Clone)]
pub struct DrawReport {
    pub triangles: usize,
    pub textured: usize,
    pub lightmapped: usize,
    pub coloured: usize,
    /// Of the coloured batches, those drawn with their bump map.
    pub bump_lit: usize,
    pub full_bright: usize,
    pub lights: SpawnReport,
    /// Batches drawn in the sky (seen only by the sky camera).
    pub sky_batches: usize,
    /// Every entity spawned (meshes and lights), so a game can tag them,
    /// for example to remove them when the level ends.
    pub entities: Vec<Entity>,
}

pub fn to_image(p: &materials::Picture) -> Image {
    let mut image = to_image_with(&p.image, TextureFormat::Rgba8UnormSrgb);
    // Level textures repeat across big surfaces.
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

/// Decoded pixels as a Bevy image in the given format (no sampler set).
pub fn to_image_with(p: &rc_image::Rgba, format: TextureFormat) -> Image {
    Image::new(
        Extent3d {
            width: p.width as u32,
            height: p.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        p.pixels.clone(),
        format,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// A lightmap page as a Bevy image. Lightmaps are stored on the same
/// brightness curve as textures, so they are marked sRGB too: Bevy then
/// multiplies the two in linear light, which on screen matches the game
/// multiplying them directly (see `LightingSettings::linear_gain`).
fn lightmap_image(p: &rc_image::Rgba) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: p.width as u32,
            height: p.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        p.pixels.clone(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    image
}

/// sRGB 0-255 to linear light, times `gain` (already linear). Alpha is
/// kept only when `keep_alpha` (terrain layers painted over each other).
fn colour_to_linear(c: [u8; 4], gain: f32, keep_alpha: bool) -> [f32; 4] {
    let lin = |v: u8| Color::srgb_u8(v, 0, 0).to_linear().red * gain;
    let a = if keep_alpha { c[3] as f32 / 255.0 } else { 1.0 };
    [lin(c[0]), lin(c[1]), lin(c[2]), a]
}

/// Spawn every draw batch of `scene` and its real-time lights. The camera
/// that looks at it should use `Exposure::default()` and no tonemapping, as
/// the baked light is matched to that.
///
/// `bump_materials` draws props with the game's bump maps (the app needs
/// `bump::BumpMapPlugin`); `None` draws them flat. `settings.bump_maps`
/// can also turn them off.
pub fn spawn_level(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    mut bump_materials: Option<&mut Assets<BumpLitMaterial>>,
    scene: &Scene,
    settings: &LightingSettings,
) -> DrawReport {
    let baked = settings.baked_light;
    let exposure = Exposure::default();
    // Lit surfaces are scaled by the camera's exposure; undo it so a
    // lightmap of 1.0 at gain 1 shows the texture as it is.
    let lightmap_exposure =
        LightingSettings::linear_gain(settings.lightmap_gain) / exposure.exposure();
    let colour_gain = LightingSettings::linear_gain(settings.vertex_colour_gain);
    let pages: Vec<Handle<Image>> = scene
        .lightmap_pages
        .iter()
        .map(|p| images.add(lightmap_image(p)))
        .collect();
    let mut triangles = 0;
    let mut textured = 0;
    let (mut lightmapped, mut coloured, mut full_bright, mut bump_lit) = (0, 0, 0, 0);
    // One image per bump map, shared by every batch that uses it.
    let mut bump_images: HashMap<*const materials::Bump, Handle<Image>> = HashMap::new();
    let mut sky_batches = 0;
    let mut entities = Vec::new();
    for b in &scene.batches {
        if b.indices.is_empty() {
            continue;
        }
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, b.positions.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, b.normals.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, b.uvs.clone());
        let page = b
            .lightmap_page
            .filter(|_| baked)
            .and_then(|p| pages.get(p).cloned());
        if page.is_some() {
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, b.lightmap_uvs.clone());
        }
        let with_colours = baked && !b.colours.is_empty();
        // Bump-lit props: opaque, one-sided, with a picture, a bump map and
        // a light direction per corner.
        let bump = b.picture.as_ref().and_then(|p| p.bump.clone()).filter(|_| {
            settings.bump_maps
                && with_colours
                && b.bump_lit
                && !b.two_sided
                && !b.translucent
                && b.terrain_layer.is_none()
                && b.light_dirs.len() == b.positions.len()
                && b.picture
                    .as_ref()
                    .is_some_and(|p| !p.has_alpha && !p.additive)
        });
        if let (Some(bump), Some(bump_mats)) = (bump, bump_materials.as_deref_mut()) {
            // The stored light goes in on the game's curve (0..1); the
            // shader turns it into linear light per corner, like
            // `colour_to_linear`, and adds the bump map's shading.
            let c: Vec<[f32; 4]> = b
                .colours
                .iter()
                .map(|c| {
                    [
                        c[0] as f32 / 255.0,
                        c[1] as f32 / 255.0,
                        c[2] as f32 / 255.0,
                        1.0,
                    ]
                })
                .collect();
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, c);
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_TANGENT,
                bump::surface_tangents(&b.positions, &b.normals, &b.uvs, &b.indices),
            );
            mesh.insert_attribute(bump::ATTRIBUTE_LIGHT_DIR, b.light_dirs.clone());
            mesh.insert_attribute(bump::ATTRIBUTE_LIGHT_COLOUR, b.light_colours.clone());
            mesh.insert_indices(Indices::U32(b.indices.clone()));
            triangles += b.indices.len() / 3;
            textured += 1;
            let picture = b.picture.as_ref().expect("bump-lit batches have a picture");
            let bump_image = bump_images
                .entry(std::sync::Arc::as_ptr(&bump))
                .or_insert_with(|| images.add(bump::bump_image(&bump)))
                .clone();
            let material = BumpLitMaterial {
                params: bump::BumpParams {
                    gain: colour_gain,
                    bump_uv_scale: bump.uv_scale,
                    strength: settings.bump_strength,
                    unused: 0.0,
                },
                diffuse: images.add(to_image(picture)),
                bump: bump_image,
            };
            let mut entity = commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(bump_mats.add(material)),
                Transform::default(),
            ));
            if let Some(m) = b.mover {
                entity.insert(MoverPart(m));
            }
            if b.sky {
                entity.insert(RenderLayers::layer(crate::sky::SKY_LAYER));
                sky_batches += 1;
            }
            entities.push(entity.id());
            coloured += 1;
            bump_lit += 1;
            continue;
        }
        if with_colours {
            let c: Vec<[f32; 4]> = b
                .colours
                .iter()
                .map(|c| colour_to_linear(*c, colour_gain, b.terrain_layer.is_some_and(|l| l > 0)))
                .collect();
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, c);
        }
        mesh.insert_indices(Indices::U32(b.indices.clone()));
        triangles += b.indices.len() / 3;
        // Terrain layers above the first are painted on by their alpha.
        let over_layer = b.terrain_layer.filter(|&l| l > 0);
        let alpha_mode = if over_layer.is_some() {
            AlphaMode::Blend
        } else if b.picture.as_ref().is_some_and(|p| p.additive) {
            AlphaMode::Add
        } else if b.translucent {
            AlphaMode::Blend
        } else if b.picture.as_ref().is_some_and(|p| p.has_alpha) {
            AlphaMode::Mask(0.5)
        } else {
            AlphaMode::Opaque
        };
        let material = StandardMaterial {
            // Lightmapped surfaces go through Bevy's lighting (so real-time
            // lights add to them). Props carry their baked light as vertex
            // colours, which an unlit material multiplies in directly.
            // Anything without baked light is drawn at full brightness.
            unlit: page.is_none(),
            lightmap_exposure,
            perceptual_roughness: 1.0,
            reflectance: 0.0,
            base_color: if b.picture.is_some() {
                Color::WHITE
            } else {
                // No texture: a flat grey, so the shape still shows.
                Color::srgb(0.55, 0.55, 0.55)
            },
            base_color_texture: b.picture.as_ref().map(|p| {
                textured += 1;
                images.add(to_image(p))
            }),
            alpha_mode,
            // Terrain layers share the same triangles; pull each later one
            // slightly towards the camera so it wins over the one below.
            depth_bias: over_layer.map_or(0.0, |l| l as f32),
            double_sided: b.two_sided,
            cull_mode: if b.two_sided {
                None
            } else {
                Some(bevy::render::render_resource::Face::Back)
            },
            ..default()
        };
        let mut entity = commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(material)),
            Transform::default(),
        ));
        if let Some(m) = b.mover {
            entity.insert(MoverPart(m));
        }
        if b.sky {
            entity.insert(RenderLayers::layer(crate::sky::SKY_LAYER));
            sky_batches += 1;
        }
        entities.push(entity.id());
        if let Some(image) = page {
            entity.insert(Lightmap { image, ..default() });
            lightmapped += 1;
        } else if with_colours {
            coloured += 1;
        } else {
            full_bright += 1;
        }
    }
    let report = lighting::spawn_lights(commands, settings, &scene.lights);
    entities.extend(report.entities.iter().copied());
    // The baked light already includes the zones' ambient light.
    commands.insert_resource(GlobalAmbientLight::NONE);
    DrawReport {
        triangles,
        textured,
        lightmapped,
        coloured,
        bump_lit,
        full_bright,
        lights: report,
        sky_batches,
        entities,
    }
}
