//! The sky, drawn the way Unreal 2 does it: a second camera stands inside
//! the map's sky box (at its `SkyZoneInfo`) and is turned the same way as
//! the player's view, but never moves with them. It draws first; the
//! player's camera then draws the level on top without clearing the
//! picture, so wherever the level has a "fake backdrop" surface (left out
//! when the level is built, see `scene.rs`) or nothing at all, the sky
//! shows. Because the sky camera doesn't move, the sky looks infinitely far
//! away.
//!
//! The sky box's polygons and props sit on their own render layer
//! (`SKY_LAYER`), so only the sky camera sees them.
//!
//! Kept separate from the level and from the player camera, so a VR
//! headset can drive it later: anything carrying `SkyViewer` turns the sky.

use crate::scene::Sky;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, Exposure};
use bevy::prelude::*;
use bevy::render::view::Tonemapping;
use bevy::transform::TransformSystems;

/// Render layer of the sky box. The level and the player's camera stay on
/// Bevy's default layer 0.
pub const SKY_LAYER: usize = 1;

/// Put this on the camera the player looks through; the sky camera copies
/// its rotation every frame.
#[derive(Component, Default)]
pub struct SkyViewer;

/// The sky camera itself.
#[derive(Component)]
pub struct SkyCamera;

pub struct SkyPlugin;

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, follow_viewer.before(TransformSystems::Propagate));
    }
}

/// The `Camera` settings for the player's camera when there is a sky: draw
/// after the sky camera and keep what it drew. Without a sky, use
/// `Camera::default()`.
pub fn viewer_camera() -> Camera {
    Camera {
        order: 1,
        clear_color: ClearColorConfig::None,
        ..default()
    }
}

/// Spawn the sky camera for a level's sky. `background` is what shows
/// where the sky box doesn't cover (a map without a sky box, or gaps).
pub fn spawn_sky_camera(commands: &mut Commands, sky: &Sky, background: Color) -> Entity {
    commands
        .spawn((
            Camera3d::default(),
            Camera {
                order: 0,
                clear_color: ClearColorConfig::Custom(background),
                ..default()
            },
            // Same look as the level camera, which the baked light is matched to.
            Tonemapping::None,
            Exposure::default(),
            Projection::Perspective(PerspectiveProjection {
                fov: 75f32.to_radians(),
                near: 0.05,
                far: 5000.0,
                ..default()
            }),
            Transform::from_translation(Vec3::from(sky.position)),
            RenderLayers::layer(SKY_LAYER),
            SkyCamera,
        ))
        .id()
}

/// Turn the sky camera with the viewer, and keep its field of view the same.
fn follow_viewer(
    viewer: Query<(&Transform, &Projection), (With<SkyViewer>, Without<SkyCamera>)>,
    mut sky: Query<(&mut Transform, &mut Projection), With<SkyCamera>>,
) {
    let Ok((view, view_projection)) = viewer.single() else {
        return;
    };
    for (mut t, mut p) in &mut sky {
        t.rotation = view.rotation;
        if let (Projection::Perspective(dst), Projection::Perspective(src)) =
            (p.as_mut(), view_projection)
        {
            if dst.fov != src.fov || dst.aspect_ratio != src.aspect_ratio {
                dst.fov = src.fov;
                dst.aspect_ratio = src.aspect_ratio;
            }
        }
    }
}
