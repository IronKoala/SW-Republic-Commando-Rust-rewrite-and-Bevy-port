//! Drawing the game's bump maps on props.
//!
//! Republic Commando lights most of a level ahead of time, and for props
//! with bump-mapped materials (`HsBump...`, or a `Shader` with a
//! `Bumpmap`) it stores two things per corner: the light's colour on the
//! corner (as for every prop) and the direction the light comes from, with
//! its strength (see `scene::bump_corners`). The original's bump shader
//! uses that direction to light each pixel by how its bump-map tilt faces
//! the light, so flat walls show their rivets, panels and grooves.
//!
//! We do the same with a small shader of our own (`bump_lit.wesl`):
//!
//! ```text
//! light on the pixel = stored corner colour (with the zone's ambient)
//!                    + light colour x (pixel facing - corner facing)
//! ```
//!
//! where "facing" is how squarely the surface faces the light (0 to 1),
//! using the bump map's tilt for the pixel and the plain surface for the
//! corner. A pixel the bump map leaves flat gets exactly the corner colour,
//! so on average the surface is as bright as before, and the brightness
//! measured against the original game (see docs/DESIGN.md) still holds.
//!
//! Real-time lights (moving or switched lights, the helmet lamp) are added
//! on top with Bevy's own lighting, using the bump-tilted surface, so a
//! lamp shows the bumps too (`live_light`, from `live_light_on_props`; see
//! `prop_light.rs` for flat props).
//!
//! **Cost.** One extra texture read and about twenty sums per pixel, plus
//! Bevy's normal per-pixel work for the few real-time lights near, so
//! phones and VR headsets can draw it.
//! `bump_maps = false` in `lighting.cfg` turns it off.

use bevy::asset::embedded_asset;
use bevy::mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef, VertexFormat};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

/// Per corner: the direction towards the light (Bevy axes, unit length,
/// or zero when there is none).
pub const ATTRIBUTE_LIGHT_DIR: MeshVertexAttribute = MeshVertexAttribute::new(
    "RC_BumpLightDir",
    0x5243_4255_4d50_0001,
    VertexFormat::Float32x3,
);

/// Per corner: the light's colour and strength on the game's 0..1 scale,
/// for a surface facing it squarely.
pub const ATTRIBUTE_LIGHT_COLOUR: MeshVertexAttribute = MeshVertexAttribute::new(
    "RC_BumpLightColour",
    0x5243_4255_4d50_0002,
    VertexFormat::Float32x3,
);

/// Adds the bump-lit material. Add it once to any app that draws levels
/// with bump maps (`draw::spawn_level` with `Some` bump materials).
pub struct BumpMapPlugin;

impl Plugin for BumpMapPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "bump_lit.wesl");
        app.add_plugins(MaterialPlugin::<BumpLitMaterial>::default());
    }
}

/// Numbers the shader needs besides the textures.
#[derive(ShaderType, Debug, Clone, Copy)]
pub struct BumpParams {
    /// Linear-light multiplier for the corner colours (the same as the flat
    /// props' `LightingSettings::linear_gain(vertex_colour_gain)`).
    pub gain: f32,
    /// How many times the bump map repeats per repeat of the main picture.
    pub bump_uv_scale: f32,
    /// 0 draws the surface flat (for comparing); 1 draws the bump map.
    pub strength: f32,
    /// How strongly real-time lights show (`live_light_on_props`; 0 = not
    /// at all, as in the sky).
    pub live_light: f32,
}

/// A prop surface drawn with its picture, its stored light and its bump map.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct BumpLitMaterial {
    #[uniform(0)]
    pub params: BumpParams,
    /// The main picture (sRGB).
    #[texture(1)]
    #[sampler(2)]
    pub diffuse: Handle<Image>,
    /// The bump map (`bump_image`, not sRGB).
    #[texture(3)]
    #[sampler(4)]
    pub bump: Handle<Image>,
}

impl Material for BumpLitMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://rc_level_viewer/bump_lit.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://rc_level_viewer/bump_lit.wesl".into()
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
            Mesh::ATTRIBUTE_TANGENT.at_shader_location(4),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(5),
            ATTRIBUTE_LIGHT_DIR.at_shader_location(8),
            ATTRIBUTE_LIGHT_COLOUR.at_shader_location(9),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        Ok(())
    }
}

/// A bump map as a Bevy image: plain (not sRGB) numbers, repeating.
pub fn bump_image(b: &crate::materials::Bump) -> Image {
    let mut image = crate::draw::to_image_with(
        &b.image,
        bevy::render::render_resource::TextureFormat::Rgba8Unorm,
    );
    image.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::Repeat,
        address_mode_v: bevy::image::ImageAddressMode::Repeat,
        ..bevy::image::ImageSamplerDescriptor::linear()
    });
    image
}

/// Per-corner surface directions for a triangle list: the direction the
/// picture's width runs along (U), as Bevy's `ATTRIBUTE_TANGENT`, with
/// `w` = +1 or -1 saying which side the picture's height (V, down the
/// picture) runs: V = w x (normal cross U). Worked out from the corners'
/// positions and picture coordinates, averaged over the triangles that
/// share a corner. Corners with no usable triangle get any direction
/// along the surface.
pub fn surface_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Vec<[f32; 4]> {
    let n = positions.len();
    let mut along_u = vec![Vec3::ZERO; n];
    let mut along_v = vec![Vec3::ZERO; n];
    for tri in indices.as_chunks::<3>().0 {
        let [a, b, c] = tri.map(|i| i as usize);
        if a >= n || b >= n || c >= n {
            continue;
        }
        let (p0, p1, p2) = (
            Vec3::from(positions[a]),
            Vec3::from(positions[b]),
            Vec3::from(positions[c]),
        );
        let (t0, t1, t2) = (Vec2::from(uvs[a]), Vec2::from(uvs[b]), Vec2::from(uvs[c]));
        let (e1, e2) = (p1 - p0, p2 - p0);
        let (d1, d2) = (t1 - t0, t2 - t0);
        let det = d1.x * d2.y - d2.x * d1.y;
        if det.abs() < 1e-12 {
            continue;
        }
        let r = 1.0 / det;
        let u = (e1 * d2.y - e2 * d1.y) * r;
        let v = (e2 * d1.x - e1 * d2.x) * r;
        if !u.is_finite() || !v.is_finite() {
            continue;
        }
        for i in [a, b, c] {
            along_u[i] += u;
            along_v[i] += v;
        }
    }
    (0..n)
        .map(|i| {
            let nrm = Vec3::from(normals[i]).normalize_or_zero();
            let mut t = along_u[i] - nrm * nrm.dot(along_u[i]);
            if t.length_squared() < 1e-12 {
                // No usable triangle: any direction along the surface.
                t = nrm.any_orthonormal_vector();
            }
            let t = t.normalize_or_zero();
            let w = if nrm.cross(t).dot(along_v[i]) < 0.0 {
                -1.0
            } else {
                1.0
            };
            [t.x, t.y, t.z, w]
        })
        .collect()
}

/// What `bump_lit.wesl` works out for one pixel, on the game's 0..1
/// brightness scale (before the gain and the picture). Kept in step with
/// the shader so tests can check the rule.
///
/// `corner` is the stored corner colour with ambient, `light` the light's
/// colour for a surface facing it squarely, `dir` the direction towards the
/// light, `surface` the plain surface direction and `pixel` the surface
/// direction the bump map gives this pixel (all unit length).
pub fn shade(corner: [f32; 3], light: [f32; 3], dir: Vec3, surface: Vec3, pixel: Vec3) -> [f32; 3] {
    let change = dir.dot(pixel).max(0.0) - dir.dot(surface).max(0.0);
    [0, 1, 2].map(|k| (corner[k] + light[k] * change).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_pixel_keeps_the_stored_colour() {
        let n = Vec3::Y;
        let dir = Vec3::new(0.6, 0.8, 0.0);
        let c = shade([0.3, 0.2, 0.1], [0.5, 0.4, 0.3], dir, n, n);
        assert!((c[0] - 0.3).abs() < 1e-6 && (c[2] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn a_pixel_tilted_towards_the_light_is_brighter_and_away_darker() {
        let n = Vec3::Y;
        let dir = Vec3::new(0.6, 0.8, 0.0);
        let towards = (n + Vec3::X * 0.5).normalize();
        let away = (n - Vec3::X * 0.5).normalize();
        let flat = shade([0.3; 3], [0.5; 3], dir, n, n)[0];
        assert!(shade([0.3; 3], [0.5; 3], dir, n, towards)[0] > flat);
        assert!(shade([0.3; 3], [0.5; 3], dir, n, away)[0] < flat);
    }

    #[test]
    fn never_goes_below_black() {
        let n = Vec3::Y;
        let c = shade([0.0; 3], [1.0; 3], Vec3::Y, n, Vec3::X);
        assert_eq!(c, [0.0; 3]);
    }

    #[test]
    fn tangents_follow_the_picture() {
        // A floor square: U runs along +X, V (down the picture) along +Z.
        let positions = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ];
        let normals = [[0.0, 1.0, 0.0]; 4];
        let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let t = surface_tangents(&positions, &normals, &uvs, &[0, 2, 1, 0, 3, 2]);
        for x in &t {
            assert!((x[0] - 1.0).abs() < 1e-5, "{x:?}");
            // normal (Y) cross U (X) is -Z, and V runs along +Z.
            assert_eq!(x[3], -1.0);
        }
    }
}
