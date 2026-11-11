//! The game's falloff shader (`HsFalloff`): light shafts and god rays.
//!
//! geo_01a's sun rays (`Arena_HR.ArenaWalls.SunBeam`, material
//! `Arena_Textures.Unique.rayfalloff`) and the tunnel light cones
//! (`Arena_GM.DecapTunels.VolumetricLight`, `Shader.VolumnLightTex`) are
//! `HsFalloff` materials. Everything below was read from the game's files:
//!
//! - Engine.u: `HsFalloff`'s settings (`DiffuseTexture`, `FalloffGradient`,
//!   `UPanRate`, `VPanRate`, `UOscilationRate`, `VOscilationRate`,
//!   `DiffuseTint`, `GradientTint`, `SrcBlend`, `DestBlend`) and their class
//!   defaults: both tints white, `SrcBlend` 3 and `DestBlend` 2, which the
//!   `ED3DBLEND` enum there names `SRCCOLOR` and `ONE`. Its shader is
//!   `HardwareShaders.Falloff.FalloffImpl`.
//! - hardwareshaders.utx: `FalloffImpl`'s vertex and pixel shader text.
//!   The picture's place is its texture coordinates plus the oscillation
//!   rates x sin(time) plus the pan rates x time. The falloff gradient is
//!   looked up at how squarely the surface faces the eye
//!   (max(0, eye direction . normal)). The colour is picture x
//!   `DiffuseTint` x gradient x `GradientTint` x the vertex colour. Its
//!   `StreamMapping` feeds the vertex colour from the `FVF_Specular`
//!   stream, and the shader writes no fog value.
//! - Engine.dll: `UHsFalloff`'s setup copies the tints (each byte / 255)
//!   into the pixel shader's first two constants, the oscillation rates into
//!   the first vertex constant and the pan rates into the second, and the
//!   blend settings onto the shader.
//!
//! Then the frame buffer gets `SrcBlend` x colour + `DestBlend` x what was
//! there: with the defaults, colour x colour + what was there.
//!
//! Not confirmed in the files (our choices, said in docs/DESIGN.md):
//! - The vertex colour is the mesh's own painted colours (`StaticMesh`
//!   colour stream: the sun beams are painted white at their bright end
//!   and black at the far end); we did not find which stream the game binds
//!   to `FVF_Specular`. The baked light is not used: the shader never reads
//!   it.
//! - No distance fog on these surfaces: the shader writes no fog value, and
//!   in the user's screenshot the rays stand out from the haze.
//! - The colour maths runs on the game's 0-1 colour values and the result
//!   is added in linear light.

use crate::materials::Materials;
use rc_image::Rgba;
use rc_package::class_library::{ClassLibrary, DefaultValue};

/// `ED3DBLEND` values (Engine.u).
pub const D3DBLEND_ONE: u8 = 2;
pub const D3DBLEND_SRCCOLOR: u8 = 3;

/// One `HsFalloff` material, besides its main picture.
#[derive(Clone)]
pub struct Falloff {
    /// `FalloffGradient`: looked up across U by how squarely the surface
    /// faces the eye.
    pub gradient: Rgba,
    /// Where the gradient came from, for the log.
    pub gradient_name: String,
    /// `DiffuseTint` and `GradientTint`, red, green, blue, alpha, 0-1.
    pub diffuse_tint: [f32; 4],
    pub gradient_tint: [f32; 4],
    /// `UPanRate`, `VPanRate`: texture units per second.
    pub pan: [f32; 2],
    /// `UOscilationRate`, `VOscilationRate`: x sin(seconds).
    pub oscillate: [f32; 2],
    /// `SrcBlend` (`ED3DBLEND`): `ONE` or `SRCCOLOR`.
    pub src_blend: u8,
    /// `DestBlend` (`ED3DBLEND`): always `ONE` here.
    pub dest_blend: u8,
}

impl Falloff {
    /// True when the colour is multiplied by itself before it is added
    /// (`SrcBlend` = `SRCCOLOR`).
    pub fn squared(&self) -> bool {
        self.src_blend == D3DBLEND_SRCCOLOR
    }

    /// The blend modes we can draw: source `ONE` or `SRCCOLOR`, destination
    /// `ONE` (every `HsFalloff` the game's levels use, see the log).
    pub fn supported(src: u8, dest: u8) -> bool {
        (src == D3DBLEND_ONE || src == D3DBLEND_SRCCOLOR) && dest == D3DBLEND_ONE
    }

    /// What the game's pixel shader works out for one pixel, in 0-1 colour
    /// values, before blending: picture x tint x gradient x tint x vertex
    /// colour. For tests and the self-check.
    pub fn shade(&self, picture: [f32; 3], gradient: [f32; 3], vertex: [f32; 3]) -> [f32; 3] {
        let mut c = [0.0; 3];
        for k in 0..3 {
            c[k] = picture[k]
                * self.diffuse_tint[k]
                * gradient[k]
                * self.gradient_tint[k]
                * vertex[k];
        }
        if self.squared() {
            c.map(|v| v * v)
        } else {
            c
        }
    }
}

/// `HsFalloff`'s class defaults (Engine.u).
#[derive(Debug, Clone, Copy)]
pub struct FalloffDefaults {
    pub diffuse_tint: [u8; 4],
    pub gradient_tint: [u8; 4],
    pub src_blend: u8,
    pub dest_blend: u8,
}

impl FalloffDefaults {
    /// Read from the game's Engine.u (and Core.u, its parent classes).
    pub fn read(mats: &mut Materials) -> Result<Self, String> {
        let mut lib = ClassLibrary::new();
        for name in ["Core", "Engine"] {
            let p = mats
                .package(name)
                .ok_or_else(|| format!("{name}.u not found"))?;
            lib.add_package(name, p.1.clone())
                .map_err(|e| format!("{name}.u: {e}"))?;
        }
        let d = lib.defaults("HsFalloff");
        let colour = |name: &str| match d.get(name).map(|e| &e.value) {
            Some(DefaultValue::Raw(b)) if b.len() >= 4 => {
                Ok(rc_image::unreal_color([b[0], b[1], b[2], b[3]]))
            }
            _ => Err(format!("HsFalloff has no {name} default")),
        };
        let byte = |name: &str| match d.get(name).map(|e| &e.value) {
            Some(DefaultValue::Byte(b)) => Ok(*b),
            _ => Err(format!("HsFalloff has no {name} default")),
        };
        Ok(Self {
            diffuse_tint: colour("DiffuseTint")?,
            gradient_tint: colour("GradientTint")?,
            src_blend: byte("SrcBlend")?,
            dest_blend: byte("DestBlend")?,
        })
    }
}

/// A colour byte as the game's shader constant (byte / 255).
pub fn constant(c: [u8; 4]) -> [f32; 4] {
    c.map(|v| v as f32 / 255.0)
}

/// Drawing falloff surfaces in Bevy.
pub mod view {
    use super::Falloff;
    use bevy::asset::embedded_asset;
    use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
    use bevy::mesh::MeshVertexBufferLayoutRef;
    use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
    use bevy::prelude::*;
    use bevy::render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
        TextureFormat,
    };
    use bevy::shader::ShaderRef;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// Put on a mesh drawn with a falloff material's plain added-light
    /// `StandardMaterial`, with the mesh's painted colours (0-1) as its
    /// vertex colours; [`FalloffPlugin`] swaps in the game's shader.
    #[derive(Component, Clone)]
    pub struct FalloffSurface(pub Arc<Falloff>);

    /// Set once the surface has its [`FalloffMaterial`].
    #[derive(Component)]
    pub struct FalloffDrawn;

    #[derive(ShaderType, Debug, Clone, Copy)]
    pub struct FalloffParams {
        pub tint: Vec4,
        pub motion: Vec4,
        pub settings: Vec4,
    }

    impl FalloffParams {
        pub fn new(f: &Falloff) -> Self {
            let t = |k: usize| f.diffuse_tint[k] * f.gradient_tint[k];
            Self {
                tint: Vec4::new(t(0), t(1), t(2), 1.0),
                motion: Vec4::new(f.oscillate[0], f.oscillate[1], f.pan[0], f.pan[1]),
                settings: Vec4::new(if f.squared() { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0),
            }
        }
    }

    /// The game's falloff shader (falloff.wesl).
    #[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
    pub struct FalloffMaterial {
        #[uniform(0)]
        pub params: FalloffParams,
        #[texture(1)]
        #[sampler(2)]
        pub picture: Option<Handle<Image>>,
        #[texture(3)]
        #[sampler(4)]
        pub gradient: Handle<Image>,
    }

    impl Material for FalloffMaterial {
        fn vertex_shader() -> ShaderRef {
            "embedded://rc_level_viewer/falloff.wesl".into()
        }

        fn fragment_shader() -> ShaderRef {
            "embedded://rc_level_viewer/falloff.wesl".into()
        }

        fn alpha_mode(&self) -> AlphaMode {
            AlphaMode::Add
        }

        // Added light only: no depth/motion prepass and no shadows. With
        // a prepass on (BLUR EFFECTS adds one for motion blur), Bevy's
        // prepass pipeline would take the vertex layout from `specialize`
        // below, which lacks its colour slot, and the graphics card
        // refuses the pipeline (the game quits on geo_01a's load).
        fn enable_prepass() -> bool {
            false
        }

        fn enable_shadows() -> bool {
            false
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
                Mesh::ATTRIBUTE_COLOR.at_shader_location(3),
            ])?;
            descriptor.vertex.buffers = vec![vertex_layout];
            Ok(())
        }
    }

    /// How many surfaces got the game's falloff shader (for logs and checks).
    #[derive(Resource, Default, Debug)]
    pub struct FalloffReport {
        pub drawn: usize,
    }

    /// Falloff materials made so far, by the plain material they replace.
    #[derive(Resource, Default)]
    struct Swapped {
        made: HashMap<AssetId<StandardMaterial>, Handle<FalloffMaterial>>,
        gradients: HashMap<usize, Handle<Image>>,
    }

    /// Swaps the plain added light on [`FalloffSurface`] meshes for the
    /// game's falloff shader. Added by `prop_light::PropLightPlugin`.
    pub struct FalloffPlugin;

    impl Plugin for FalloffPlugin {
        fn build(&self, app: &mut App) {
            if app.is_plugin_added::<MaterialPlugin<FalloffMaterial>>() {
                return;
            }
            embedded_asset!(app, "falloff.wesl");
            app.add_plugins(MaterialPlugin::<FalloffMaterial>::default())
                .init_resource::<FalloffReport>()
                .init_resource::<Swapped>()
                .add_systems(Update, swap_materials);
        }

        fn is_unique(&self) -> bool {
            false
        }
    }

    /// The gradient as a Bevy image, clamped at its ends (it is looked up
    /// from 0, side-on, to 1, face-on).
    fn gradient_image(f: &Falloff) -> Image {
        let mut image = crate::draw::to_image_with(&f.gradient, TextureFormat::Rgba8UnormSrgb);
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            ..ImageSamplerDescriptor::linear()
        });
        image
    }

    fn swap_materials(
        mut commands: Commands,
        found: Query<(Entity, &FalloffSurface, &MeshMaterial3d<StandardMaterial>), Without<FalloffDrawn>>,
        plain: Res<Assets<StandardMaterial>>,
        mut falloff: ResMut<Assets<FalloffMaterial>>,
        mut images: ResMut<Assets<Image>>,
        mut swapped: ResMut<Swapped>,
        mut report: ResMut<FalloffReport>,
    ) {
        for (e, s, m) in &found {
            let Some(base) = plain.get(&m.0) else {
                continue;
            };
            let Swapped { made, gradients } = &mut *swapped;
            let handle = made
                .entry(m.0.id())
                .or_insert_with(|| {
                    let gradient = gradients
                        .entry(Arc::as_ptr(&s.0) as usize)
                        .or_insert_with(|| images.add(gradient_image(&s.0)))
                        .clone();
                    falloff.add(FalloffMaterial {
                        params: FalloffParams::new(&s.0),
                        picture: base.base_color_texture.clone(),
                        gradient,
                    })
                })
                .clone();
            commands
                .entity(e)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert((MeshMaterial3d(handle), FalloffDrawn));
            report.drawn += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(src: u8) -> Falloff {
        Falloff {
            gradient: Rgba {
                width: 1,
                height: 1,
                pixels: vec![255; 4],
            },
            gradient_name: String::new(),
            diffuse_tint: [1.0, 0.5, 1.0, 1.0],
            gradient_tint: [1.0; 4],
            pan: [0.0; 2],
            oscillate: [0.0; 2],
            src_blend: src,
            dest_blend: D3DBLEND_ONE,
        }
    }

    #[test]
    fn source_colour_blend_squares() {
        let one = sample(D3DBLEND_ONE).shade([0.5; 3], [1.0; 3], [1.0; 3]);
        assert_eq!(one, [0.5, 0.25, 0.5]);
        let sq = sample(D3DBLEND_SRCCOLOR).shade([0.5; 3], [1.0; 3], [1.0; 3]);
        assert_eq!(sq, [0.25, 0.0625, 0.25]);
    }

    #[test]
    fn black_vertex_or_gradient_hides_it() {
        let f = sample(D3DBLEND_ONE);
        assert_eq!(f.shade([1.0; 3], [1.0; 3], [0.0; 3]), [0.0; 3]);
        assert_eq!(f.shade([1.0; 3], [0.0; 3], [1.0; 3]), [0.0; 3]);
    }

    #[test]
    fn supported_blends() {
        assert!(Falloff::supported(D3DBLEND_SRCCOLOR, D3DBLEND_ONE));
        assert!(Falloff::supported(D3DBLEND_ONE, D3DBLEND_ONE));
        assert!(!Falloff::supported(D3DBLEND_ONE, 1));
    }
}
