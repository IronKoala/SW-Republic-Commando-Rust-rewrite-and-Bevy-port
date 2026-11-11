//! Holograms that shimmer and flicker like the original's.
//!
//! Republic Commando's hologram material (`HsHologram`: the prologue's
//! hologram rifles and advisor, the briefing holograms) is drawn by the
//! game's `DynamicHologram` hardware shader. Its assembly text and settings
//! are stored in `GameData/Textures/hardwareshaders.utx`; we read them to
//! learn what it does (nothing is copied) and do the same in our own
//! shader, `hologram.wesl`:
//!
//! - **Edge fade.** A glow picture (`GreyHoloFade`) is looked up by how
//!   squarely the surface faces the eye, so edges seen side-on fade out.
//! - **Shimmer.** A noise picture (`NoiseDigital`) is laid across the model
//!   (one repeat per 58 Unreal units) and jumps to a new place every step.
//! - **Small flicker.** Most steps the brightness dips by up to 40%, and the
//!   model is nudged by about one Unreal unit (2 cm).
//! - **Big flicker.** About one step in a hundred the model is squashed in
//!   height by up to 30%.
//!
//! Both pictures are the game's own, read from the install when the game
//! folder is known ([`HologramGameData`]); until then, plain stand-ins are
//! used (a straight fade and no noise).
//!
//! **How to use.** Add [`HologramPlugin`], and give any mesh drawn with a
//! hologram's `StandardMaterial` (its picture already turned into glow by
//! `rc_image::hologram`) a [`Hologram`] component. The plugin swaps the
//! material for a [`HologramMaterial`]. Without the plugin the hologram
//! stays a steady glow. The numbers are in [`HologramLook`], a resource a
//! mod can change.
//!
//! **Guessed.** The game fills the flicker values (`EVC_Flicker`) in code we
//! can't see. The shader's settings for them come in pairs (0.99 and 0.3
//! for the "big rare flicker", 0.1 and 0.4 for the "small frequent
//! flicker", 0 and 1 for the noise jump); we read them as "chance a step
//! stays at 1" and "most it drops", which fits all three. How often a step
//! comes isn't stored either: the original changed them every frame, at
//! the 30 to 60 frames a second of its day; we use 30 steps a second
//! whatever the frame rate.

use bevy::asset::RenderAssetUsages;
use bevy::asset::{embedded_asset, uuid_handle, AssetId};
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The edge-fade picture: the game's `GreyHoloFade`, or a stand-in.
pub const GLOW_PICTURE: Handle<Image> = uuid_handle!("4b1f6a52-6d0e-4c43-9a3f-1d2c70a1e001");
/// The shimmer picture: the game's `NoiseDigital`, or plain white.
pub const NOISE_PICTURE: Handle<Image> = uuid_handle!("4b1f6a52-6d0e-4c43-9a3f-1d2c70a1e002");

/// Put on a mesh drawn with a hologram's `StandardMaterial`; the plugin
/// swaps in the shimmering [`HologramMaterial`].
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Hologram {
    /// Where the big flicker squashes the height towards, in the mesh's
    /// own space (Bevy units): the model's origin for characters, the
    /// bottom middle for the level's holograms (whose meshes are in world
    /// space).
    pub pivot: Vec3,
}

/// Set once this hologram has its [`HologramMaterial`].
#[derive(Component)]
pub struct HologramDrawn;

/// Where the game's `GameData` folder is, so the plugin can read the
/// original glow and noise pictures. Optional.
#[derive(Resource, Debug, Clone)]
pub struct HologramGameData(pub PathBuf);

/// The numbers the hologram shader uses. Defaults are the game's own
/// (`DynamicHologram`'s constants in `hardwareshaders.utx`), except where
/// marked as our choice.
#[derive(Resource, Debug, Clone, Copy)]
pub struct HologramLook {
    /// How far across the glow picture one unit of facing goes (c15.y,
    /// 0.11: only the first ninth of the picture is used).
    pub glow_per_facing: f32,
    /// Overall brightness (1 = the steady glow drawn before; our choice).
    pub brightness: f32,
    /// Flicker steps per second (our choice, see the crate notes).
    pub steps_per_second: f32,
    /// Noise repeats per Unreal unit (c26.y, 0.0171).
    pub noise_per_unit: f32,
    /// How far the noise jumps across and down per unit of shimmer (c28.xy).
    pub noise_jump: Vec2,
    /// Big flicker: chance a step keeps full height, most it squashes (c21).
    pub big: Vec2,
    /// Small flicker: chance a step keeps full brightness, most it dims
    /// (c22).
    pub small: Vec2,
    /// How far (Unreal units) the small flicker nudges the model per unit
    /// (the shader adds the flicker value itself, so 1).
    pub nudge: f32,
    /// Shimmer: chance the noise stays put, how far it may jump (c25).
    pub shimmer: Vec2,
}

impl Default for HologramLook {
    fn default() -> Self {
        HologramLook {
            glow_per_facing: 0.11,
            brightness: 1.0,
            steps_per_second: 30.0,
            noise_per_unit: 0.0171,
            noise_jump: Vec2::new(0.2, 0.5),
            big: Vec2::new(0.99, 0.3),
            small: Vec2::new(0.1, 0.4),
            nudge: 1.0,
            shimmer: Vec2::new(0.0, 1.0),
        }
    }
}

/// Unreal units per Bevy unit (as everywhere in the port).
const UU_PER_UNIT: f32 = 50.0;

/// The shader's numbers, in the layout `hologram.wesl` reads.
#[derive(ShaderType, Debug, Clone, Copy)]
pub struct HologramParams {
    pub glow: Vec4,
    pub noise: Vec4,
    pub big: Vec4,
    pub small: Vec4,
    pub shimmer: Vec4,
    pub pivot: Vec4,
}

impl HologramParams {
    pub fn new(look: &HologramLook, pivot: Vec3) -> Self {
        HologramParams {
            glow: Vec4::new(
                look.glow_per_facing,
                0.0,
                look.brightness,
                look.steps_per_second,
            ),
            noise: Vec4::new(
                look.noise_per_unit,
                look.noise_jump.x,
                look.noise_jump.y,
                UU_PER_UNIT,
            ),
            big: Vec4::new(look.big.x, look.big.y, 0.0, 0.0),
            small: Vec4::new(look.small.x, look.small.y, look.nudge, 0.0),
            shimmer: Vec4::new(look.shimmer.x, look.shimmer.y, 0.0, 0.0),
            pivot: pivot.extend(1.0),
        }
    }
}

/// A hologram surface: its glow-coloured picture plus the shared glow and
/// noise pictures.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct HologramMaterial {
    #[uniform(0)]
    pub params: HologramParams,
    #[texture(1)]
    #[sampler(2)]
    pub picture: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    pub glow: Handle<Image>,
    #[texture(5)]
    #[sampler(6)]
    pub noise: Handle<Image>,
}

impl Material for HologramMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://rc_hologram/hologram.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://rc_hologram/hologram.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
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
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        // Seen from both sides, like the steady glow before.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// What happened, for logs and self-tests.
#[derive(Resource, Debug, Clone, Default)]
pub struct HologramReport {
    /// Hologram meshes given the shimmering material so far.
    pub drawn: usize,
    /// Whether the game's own glow and noise pictures were read.
    pub game_pictures: bool,
    /// Why they weren't, if reading them failed.
    pub problem: Option<String>,
}

/// Adds [`HologramMaterial`] and swaps it onto [`Hologram`] meshes. Safe to
/// add more than once.
pub struct HologramPlugin;

impl Plugin for HologramPlugin {
    fn build(&self, app: &mut App) {
        if app.is_plugin_added::<MaterialPlugin<HologramMaterial>>() {
            return;
        }
        embedded_asset!(app, "hologram.wesl");
        app.add_plugins(MaterialPlugin::<HologramMaterial>::default())
            .init_resource::<HologramLook>()
            .init_resource::<HologramReport>()
            .init_resource::<Swapped>()
            .add_systems(Startup, stand_in_pictures)
            .add_systems(
                Update,
                (
                    game_pictures.run_if(resource_exists::<HologramGameData>),
                    swap_materials,
                )
                    .chain(),
            );
    }

    fn is_unique(&self) -> bool {
        false
    }
}

/// Hologram materials made so far, by the steady material they replace and
/// the pivot, so meshes sharing one share the other.
#[derive(Resource, Default)]
struct Swapped {
    made: HashMap<(AssetId<StandardMaterial>, [u32; 3]), Handle<HologramMaterial>>,
    tried: Option<PathBuf>,
}

/// Until the game's pictures are read: a straight fade (dark side-on,
/// bright face-on, roughly the game's curve) and plain white noise.
fn stand_in_pictures(mut images: ResMut<Assets<Image>>) {
    let glow: Vec<u8> = (0..64)
        .flat_map(|x| {
            // Over the part the shader uses (the first ninth), from 0.06
            // to full, like GreyHoloFade.
            let f = (0.06 + 1.05 * (x as f32 / 63.0) * 9.0).min(1.0);
            let v = (f * 255.0).round() as u8;
            [v, v, v, v]
        })
        .collect();
    let _ = images.insert(
        GLOW_PICTURE.id(),
        picture(
            &rc_image::Rgba {
                width: 64,
                height: 1,
                pixels: glow,
            },
            false,
        ),
    );
    let _ = images.insert(
        NOISE_PICTURE.id(),
        picture(
            &rc_image::Rgba {
                width: 1,
                height: 1,
                pixels: vec![255; 4],
            },
            true,
        ),
    );
}

/// Read `GreyHoloFade` and `NoiseDigital` from the game, once per folder.
fn game_pictures(
    data: Res<HologramGameData>,
    mut swapped: ResMut<Swapped>,
    mut images: ResMut<Assets<Image>>,
    mut report: ResMut<HologramReport>,
) {
    if swapped.tried.as_ref() == Some(&data.0) {
        return;
    }
    swapped.tried = Some(data.0.clone());
    match read_pictures(&data.0) {
        Ok((glow, noise)) => {
            let _ = images.insert(GLOW_PICTURE.id(), picture(&glow, false));
            let _ = images.insert(NOISE_PICTURE.id(), picture(&noise, true));
            report.game_pictures = true;
            report.problem = None;
            info!(
                "holograms: read GreyHoloFade {}x{} and NoiseDigital {}x{} from the game",
                glow.width, glow.height, noise.width, noise.height
            );
        }
        Err(e) => {
            warn!("holograms: kept the stand-in pictures: {e}");
            report.problem = Some(e);
        }
    }
}

/// Give each new [`Hologram`] mesh its shimmering material.
fn swap_materials(
    mut commands: Commands,
    found: Query<(Entity, &Hologram, &MeshMaterial3d<StandardMaterial>), Without<HologramDrawn>>,
    steady: Res<Assets<StandardMaterial>>,
    mut holo: ResMut<Assets<HologramMaterial>>,
    look: Res<HologramLook>,
    mut swapped: ResMut<Swapped>,
    mut report: ResMut<HologramReport>,
) {
    let before = report.drawn;
    for (e, h, m) in &found {
        let Some(base) = steady.get(&m.0) else {
            continue;
        };
        let key = (m.0.id(), h.pivot.to_array().map(f32::to_bits));
        let handle = swapped
            .made
            .entry(key)
            .or_insert_with(|| {
                holo.add(HologramMaterial {
                    params: HologramParams::new(&look, h.pivot),
                    picture: base.base_color_texture.clone(),
                    glow: GLOW_PICTURE,
                    noise: NOISE_PICTURE,
                })
            })
            .clone();
        commands
            .entity(e)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert((MeshMaterial3d(handle), HologramDrawn));
        report.drawn += 1;
    }
    if report.drawn > before {
        info!(
            "holograms: {} more meshes shimmer ({} in all, game pictures: {})",
            report.drawn - before,
            report.drawn,
            report.game_pictures
        );
    }
}

/// A picture as a Bevy image (on the brightness curve; the shader works on
/// it). The glow picture is clamped at its ends, the noise repeats.
fn picture(p: &rc_image::Rgba, repeat: bool) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: p.width as u32,
            height: p.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        p.pixels.clone(),
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    let mode = if repeat {
        ImageAddressMode::Repeat
    } else {
        ImageAddressMode::ClampToEdge
    };
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: mode,
        address_mode_v: mode,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

/// The game's glow and noise pictures from `hardwareshaders.utx` in the
/// `GameData` folder (read only).
pub fn read_pictures(game_data: &Path) -> Result<(rc_image::Rgba, rc_image::Rgba), String> {
    let path = find_file(&game_data.join("Textures"), "hardwareshaders.utx")
        .ok_or("GameData/Textures/hardwareshaders.utx not found")?;
    let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let pkg = rc_package::Package::parse(&data).map_err(|e| e.to_string())?;
    let glow = read_texture(&pkg, &data, "GreyHoloFade")?;
    let noise = read_texture(&pkg, &data, "NoiseDigital")?;
    Ok((glow, noise))
}

/// A file in `dir`, any upper or lower case (Linux and Android care).
fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.is_file() {
        return Some(exact);
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|f| f.to_str())
                .is_some_and(|f| f.eq_ignore_ascii_case(name))
        })
}

/// One texture of the package, decoded (its largest picture).
fn read_texture(
    pkg: &rc_package::Package,
    data: &[u8],
    name: &str,
) -> Result<rc_image::Rgba, String> {
    use rc_image::PixelFormat;
    use rc_package::texture;
    let export = pkg
        .exports
        .iter()
        .find(|e| {
            pkg.name(e.object_name).eq_ignore_ascii_case(name)
                && pkg.export_class_name(e).eq_ignore_ascii_case("Texture")
        })
        .ok_or_else(|| format!("{name} not found"))?;
    let tex = texture::read_texture(pkg, data, export).map_err(|e| e.to_string())?;
    let format = match tex.format {
        3 => PixelFormat::Dxt1,
        5 => PixelFormat::Bgra8,
        7 => PixelFormat::Dxt3,
        8 => PixelFormat::Dxt5,
        f => {
            return Err(format!(
                "{name}: format {} not handled",
                texture::format_name(f)
            ))
        }
    };
    let mip = tex
        .mips
        .first()
        .ok_or_else(|| format!("{name} has no pictures"))?;
    rc_image::decode(
        format,
        mip.width as usize,
        mip.height as usize,
        &data[mip.data_offset..mip.data_offset + mip.data_len],
        None,
    )
    .map_err(|e| format!("{name}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_games_numbers() {
        let p = HologramParams::new(&HologramLook::default(), Vec3::ZERO);
        assert_eq!(p.glow.x, 0.11);
        assert_eq!(p.noise.x, 0.0171);
        assert_eq!(p.big.truncate().truncate(), Vec2::new(0.99, 0.3));
        assert_eq!(p.small.truncate().truncate(), Vec2::new(0.1, 0.4));
        assert_eq!(p.shimmer.truncate().truncate(), Vec2::new(0.0, 1.0));
    }

    /// Reads the real pictures when RC_GAME_DIR points at the game (skipped
    /// otherwise).
    #[test]
    fn reads_the_games_pictures() {
        let Ok(dir) = std::env::var("RC_GAME_DIR") else {
            return;
        };
        let (glow, noise) = read_pictures(&Path::new(&dir).join("GameData")).unwrap();
        assert_eq!((glow.width, glow.height), (256, 16));
        assert_eq!((noise.width, noise.height), (128, 128));
        // Dark at the side-on end, bright a little way in.
        assert!(glow.pixels[0] < 40);
        assert!(glow.pixels[32 * 4] > 240);
    }
}
