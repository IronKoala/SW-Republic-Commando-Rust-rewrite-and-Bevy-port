//! The low light mode's night-vision picture.
//!
//! In low light mode a [`NightVisionView`] goes on the player's camera. It
//! is one full-screen pass (`night_vision.wesl`) over the finished 3D view,
//! done the way the original's own low light shader is set up (see
//! `night_original.rs` for what is read from the install):
//!
//! 1. a slightly blurred copy of the picture is turned into one brightness
//!    per pixel, the sum of its red, green and blue squared, so the picture
//!    goes black and white, and dark parts get darker while lit parts
//!    stand out;
//! 2. it fades in from black when the mode is switched on;
//! 3. it is seen through the visor-shaped window of the original's overlay
//!    picture (`HUD_GlassNight`: outside the window is black), whose faint
//!    colour is added on top;
//! 4. the original's film grain picture is laid over it, shifted to a new
//!    random place every frame.
//!
//! The helmet lamp stays on as well (the original's vision modes switch the
//! flashlight on), and the original's distance fog for the mode is put on
//! the camera, so far things fade into the dark.
//!
//! The HUD and the helmet frame are drawn by their own cameras afterwards,
//! so they stay sharp. The pass costs a handful of texture reads per pixel
//! and no extra passes, which phones manage. Nothing in it assumes one flat
//! screen: for VR it would go on each eye's camera (and the window can be
//! switched off in the data file).
//!
//! The render plumbing below follows Bevy's own `FullscreenMaterial`
//! (bevy_core_pipeline, MIT OR Apache-2.0), with two pictures added; that
//! one can only pass numbers to its shader.

use bevy::asset::{embedded_asset, RenderAssetUsages};
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
    UniformComponentPlugin,
};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::{
    AddressMode, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
    CachedRenderPipelineId, ColorTargetState, ColorWrites, Extent3d, FilterMode, FragmentState,
    Operations, PipelineCache, RenderPassColorAttachment, RenderPassDescriptor,
    RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages,
    ShaderType, TextureDimension, TextureFormat, TextureSampleType,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, ViewTarget};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use serde::Deserialize;
use std::collections::HashMap;

/// The night-vision look, as written in `[low_light]` of the data file.
/// The original's own numbers and pictures are read from the install; the
/// ones here only change or stand in for them.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LowLightLook {
    /// Off: low light mode is only the lamp.
    pub picture: bool,
    /// The original's vision mode object read for the look.
    pub mode: String,
    /// Brightness boost on top of the original's (1 is the original).
    pub boost: f32,
    /// Colour the black-and-white picture is tinted to: red, green, blue
    /// from 0 to 255. White is the original.
    pub tint: [u8; 3],
    /// See through the original's visor-shaped window (outside is black).
    pub window: bool,
    /// On screens wider than 4:3 the window is widened rather than
    /// stretched: this much of its middle (in screen heights from the
    /// centre) and the outer parts keep their shape, and the plain part in
    /// between is drawn longer.
    pub window_middle: f32,
    /// Times the original's grain strength.
    pub grain_scale: f32,
    /// Use the mode's own distance fog (far things fade into the dark).
    pub fog: bool,
    /// Seconds the picture takes to fade in from black.
    pub fade_seconds: f32,
    /// Used when the install can't be read: grain strength 0 to 1, grain
    /// repeats across and down a 4:3 screen, fog start and end (Unreal
    /// units).
    pub fallback_grain: f32,
    pub fallback_grain_tiles: [f32; 2],
    pub fallback_fog: [f32; 2],
}

impl Default for LowLightLook {
    fn default() -> Self {
        LowLightLook {
            picture: true,
            mode: "FrameFX.VisionModes.VisionNight".into(),
            boost: 1.0,
            tint: [255, 255, 255],
            window: true,
            window_middle: 0.25,
            grain_scale: 1.0,
            fog: true,
            fade_seconds: 0.2,
            fallback_grain: 0.12,
            fallback_grain_tiles: [9.0, 6.5],
            fallback_fog: [555.0, 4500.0],
        }
    }
}

/// Put on a 3D camera to show it through the night-vision visor. The visor
/// does this itself in low light mode.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default, Debug, PartialEq)]
#[extract_app(RenderApp)]
pub struct NightVisionView {
    /// Linear red, green, blue the brightness is multiplied by; alpha
    /// unused.
    pub tint: Vec4,
    /// Brightness boost (1 is the original).
    pub boost: f32,
    /// Film grain strength, 0 to 1.
    pub grain: f32,
    /// 0 is black, 1 fully faded in.
    pub fade: f32,
    /// 1 to see through the visor window, 0 for the whole screen.
    pub window: f32,
    /// See [`LowLightLook::window_middle`].
    pub window_middle: f32,
    /// Blur step, pixels.
    pub blur: f32,
    /// Grain repeats across and down a 4:3 screen.
    pub grain_tiles: Vec2,
    /// Where the grain is shifted to this frame, 0 to 1.
    pub grain_offset: Vec2,
    pub unused: Vec2,
}

/// The pictures the pass uses: the visor window and the film grain. Made
/// from the install's pictures, or plain stand-ins (an open window, flat
/// grain) when they can't be read.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub struct NightVisionImages {
    pub window: Handle<Image>,
    pub grain: Handle<Image>,
}

/// A picture for the pass. Kept as it is stored (not as screen colours):
/// the shader works on the same numbers the original's did.
pub fn image_from(width: usize, height: usize, rgba: Vec<u8>) -> Image {
    Image::new(
        Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    )
}

pub(crate) struct NightVisionPlugin;

impl Plugin for NightVisionPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "night_vision.wesl");
        app.add_plugins((
            ExtractComponentPlugin::<NightVisionView>::default(),
            UniformComponentPlugin::<NightVisionView>::default(),
            ExtractResourcePlugin::<NightVisionImages>::default(),
        ))
        .add_systems(Startup, stand_in_images);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Render, prepare_pipelines.in_set(RenderSystems::Prepare))
            // After tone mapping: the original worked on the finished
            // picture's colours, and this way the brightness option is
            // already in.
            .add_systems(
                Core3d,
                night_vision_pass
                    .in_set(Core3dSystems::PostProcess)
                    .after(tonemapping),
            );
    }
}

/// Plain pictures until the install's are read: a fully open window and
/// flat grey grain (no grain).
fn stand_in_images(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    have: Option<Res<NightVisionImages>>,
) {
    if have.is_some() {
        return;
    }
    commands.insert_resource(NightVisionImages {
        window: images.add(image_from(1, 1, vec![0, 0, 0, 255])),
        grain: images.add(image_from(1, 1, vec![128, 128, 128, 255])),
    });
}

#[derive(Resource)]
struct NightPipeline {
    layout: BindGroupLayoutDescriptor,
    clamp: Sampler,
    repeat: Sampler,
    descriptor: RenderPipelineDescriptor,
    by_format: HashMap<TextureFormat, CachedRenderPipelineId>,
}

fn init_pipeline(
    mut commands: Commands,
    device: Res<RenderDevice>,
    assets: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "night_vision_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<NightVisionView>(true),
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let clamp = device.create_sampler(&SamplerDescriptor {
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    let repeat = device.create_sampler(&SamplerDescriptor {
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    let descriptor = RenderPipelineDescriptor {
        label: Some("night_vision_pipeline".into()),
        layout: vec![layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: assets.load("embedded://rc_visor/night_vision.wesl"),
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::Rgba8UnormSrgb,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    };
    commands.insert_resource(NightPipeline {
        layout,
        clamp,
        repeat,
        descriptor,
        by_format: HashMap::new(),
    });
}

#[derive(Component)]
struct NightPipelineId(CachedRenderPipelineId);

fn prepare_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    pipeline: Option<ResMut<NightPipeline>>,
    views: Query<(Entity, &ExtractedView, Has<NightVisionView>)>,
) {
    let Some(mut pipeline) = pipeline else {
        return;
    };
    for (e, view, on) in &views {
        if !on {
            commands.entity(e).remove::<NightPipelineId>();
            continue;
        }
        let format = view.target_format;
        let id = match pipeline.by_format.get(&format) {
            Some(id) => *id,
            None => {
                let mut d = pipeline.descriptor.clone();
                if let Some(f) = d.fragment.as_mut() {
                    f.targets = vec![Some(ColorTargetState {
                        format,
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })];
                }
                let id = cache.queue_render_pipeline(d);
                pipeline.by_format.insert(format, id);
                id
            }
        };
        commands.entity(e).insert(NightPipelineId(id));
    }
}

#[allow(clippy::too_many_arguments)]
fn night_vision_pass(
    view: ViewQuery<(
        &ViewTarget,
        &DynamicUniformIndex<NightVisionView>,
        &NightPipelineId,
    )>,
    pipeline: Option<Res<NightPipeline>>,
    cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<NightVisionView>>,
    images: Option<Res<NightVisionImages>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (target, index, id) = view.into_inner();
    let (Some(pipeline), Some(images)) = (pipeline, images) else {
        return;
    };
    let Some(render_pipeline) = cache.get_render_pipeline(id.0) else {
        return;
    };
    let Some(binding) = uniforms.uniforms().binding() else {
        return;
    };
    let (Some(window), Some(grain)) = (
        gpu_images.get(&images.window),
        gpu_images.get(&images.grain),
    ) else {
        return;
    };
    let post = target.post_process_write();
    let bind_group = device.create_bind_group(
        "night_vision_bind_group",
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((
            post.source,
            &pipeline.clamp,
            binding,
            &window.texture_view,
            &grain.texture_view,
            &pipeline.repeat,
        )),
    );
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("night_vision_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: post.destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    pass.set_pipeline(render_pipeline);
    pass.set_bind_group(0, &bind_group, &[index.index()]);
    pass.draw(0..3, 0..1);
}
