//! Turns a built `Scene` into Bevy meshes, materials and lights. Shared by
//! the level viewer and the first-person walker (`rc_walk`), so a level
//! looks the same in both.

use crate::lighting::{self, LightingSettings, SpawnReport};
use crate::materials;
use crate::scene::Scene;
use bevy::asset::RenderAssetUsages;
use bevy::camera::Exposure;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::camera::visibility::RenderLayers;
use bevy::pbr::Lightmap;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// Sky-blue-grey background, as 0-255 sRGB values.
pub const BACKGROUND: [u8; 3] = [64, 72, 89];

/// What `spawn_level` drew, for the log.
#[derive(Debug, Default, Clone)]
pub struct DrawReport {
    pub triangles: usize,
    pub textured: usize,
    pub lightmapped: usize,
    pub coloured: usize,
    pub full_bright: usize,
    pub lights: SpawnReport,
    /// Batches drawn in the sky (seen only by the sky camera).
    pub sky_batches: usize,
    /// Every entity spawned (meshes and lights), so a game can tag them,
    /// for example to remove them when the level ends.
    pub entities: Vec<Entity>,
}

pub fn to_image(p: &materials::Picture) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: p.image.width as u32,
            height: p.image.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        p.image.pixels.clone(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    // Level textures repeat across big surfaces.
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    image
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

/// sRGB 0-255 to linear light, times `gain` (already linear).
fn colour_to_linear(c: [u8; 4], gain: f32) -> [f32; 4] {
    let lin = |v: u8| Color::srgb_u8(v, 0, 0).to_linear().red * gain;
    [lin(c[0]), lin(c[1]), lin(c[2]), 1.0]
}

/// Spawn every draw batch of `scene` and its real-time lights. The camera
/// that looks at it should use `Exposure::default()` and no tonemapping, as
/// the baked light is matched to that.
pub fn spawn_level(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
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
    let (mut lightmapped, mut coloured, mut full_bright) = (0, 0, 0);
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
        if with_colours {
            let c: Vec<[f32; 4]> = b
                .colours
                .iter()
                .map(|c| colour_to_linear(*c, colour_gain))
                .collect();
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, c);
        }
        mesh.insert_indices(Indices::U32(b.indices.clone()));
        triangles += b.indices.len() / 3;
        let alpha_mode = if b.picture.as_ref().is_some_and(|p| p.additive) {
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
        full_bright,
        lights: report,
        sky_batches,
        entities,
    }
}
