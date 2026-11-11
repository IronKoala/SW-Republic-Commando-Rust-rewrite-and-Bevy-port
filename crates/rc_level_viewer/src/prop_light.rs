//! Lets real-time lights (moving, flickering or switched lights, the
//! helmet lamp, later muzzle flashes) light props and terrain, on top of
//! the light the map baked into them.
//!
//! Before this, props were drawn with Bevy's "unlit" material: their baked
//! light (one colour per corner, from the map) was multiplied straight into
//! the texture, and no real-time light could reach them. Walls already took
//! real-time light, so a lamp lit a wall but not the crate in front of it.
//!
//! Now props use [`PropMaterial`]: Bevy's standard material plus a small
//! shader of ours (`prop_light.wesl`) that shows
//! `texture x baked colour + texture x real-time lights x live_gain`.
//! With no real-time lights near, a prop looks exactly as before.
//!
//! Cost: the same as any lit surface in Bevy (one pass, no extra textures
//! except on the few multiplying-`Combiner` props, which read one more),
//! so it stays within what phones and VR headsets can draw.
//!
//! Every app that draws a level (viewer, walker, game) must add
//! [`PropLightPlugin`]; `spawn_level` needs its `Assets<PropMaterial>`.

use bevy::asset::embedded_asset;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

/// The material props and terrain are drawn with.
pub type PropMaterial = ExtendedMaterial<StandardMaterial, PropLight>;

/// Our part of [`PropMaterial`].
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone, Default)]
pub struct PropLight {
    #[uniform(100)]
    pub settings: PropLightUniform,
    /// A second picture the texture is multiplied by, read with the mesh's
    /// second texture coordinates (`UV_1`): a multiplying `Combiner`
    /// (`materials::Layer`). Only used when `settings.y` is above 0.
    #[texture(101)]
    #[sampler(102)]
    pub layer: Option<Handle<Image>>,
}

#[derive(ShaderType, Debug, Clone, Copy)]
pub struct PropLightUniform {
    /// x: how strongly real-time lights show on props (1 = like on walls,
    /// 0 = props ignore them). y: 0 for no multiplying layer, otherwise
    /// the layer's scale (the `Combiner`'s 1, `Modulate2X` 2 or
    /// `Modulate4X` 4). z, w unused.
    pub settings: Vec4,
}

impl Default for PropLightUniform {
    fn default() -> Self {
        Self {
            settings: Vec4::new(1.0, 0.0, 0.0, 0.0),
        }
    }
}

impl PropLight {
    pub fn new(live_gain: f32) -> Self {
        Self {
            settings: PropLightUniform {
                settings: Vec4::new(live_gain.max(0.0), 0.0, 0.0, 0.0),
            },
            layer: None,
        }
    }

    /// The same, with the texture multiplied by `layer` (on `UV_1`) at
    /// `scale`.
    pub fn with_layer(live_gain: f32, layer: Handle<Image>, scale: f32) -> Self {
        Self {
            settings: PropLightUniform {
                settings: Vec4::new(live_gain.max(0.0), scale.max(0.0), 0.0, 0.0),
            },
            layer: Some(layer),
        }
    }
}

impl MaterialExtension for PropLight {
    fn fragment_shader() -> ShaderRef {
        "embedded://rc_level_viewer/prop_light.wesl".into()
    }
}

/// Adds [`PropMaterial`]. Safe to add more than once (the game's level
/// plugin and an app may both add it).
pub struct PropLightPlugin;

impl Plugin for PropLightPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "prop_light.wesl");
        if !app.is_plugin_added::<MaterialPlugin<PropMaterial>>() {
            app.add_plugins(MaterialPlugin::<PropMaterial>::default());
        }
    }

    fn is_unique(&self) -> bool {
        false
    }
}
