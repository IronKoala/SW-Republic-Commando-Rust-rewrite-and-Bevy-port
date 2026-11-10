//! rc_shadows: character shadows, drawn the way the original game draws
//! them.
//!
//! In the original, a character with `bActorShadows` (set by the map on
//! chosen characters, and by the Delta commandos' class) gets a "shadow
//! projector" while the "Shadows" option (SQUAD SHADOWS) is on: a
//! 128x128 picture of the character's shape, seen from a light high above
//! at a fixed angle (`Normal(vect(1,1,3))`), laid onto the floor around
//! it, fading with distance (350 Unreal units) and not drawn beyond 1200
//! units from the camera. See `data/shadows/shadows.toml`.
//!
//! Ours gives the same picture in a cheaper way, with no extra camera or
//! picture to draw: each of the character's meshes is drawn a second time,
//! flattened onto the floor along the light's direction, in a see-through
//! dark colour. That costs one more draw of each body, with a tiny shader,
//! so it stays within what phones and VR headsets can draw.
//!
//! Differences from the original, on purpose or not yet done:
//! - the floor is taken as flat, at the character's feet. The original's
//!   shadow also falls on steps, slopes and walls nearby;
//! - the edges are sharp, where the original's small picture makes them a
//!   little soft.
//!
//! How other parts use it: put [`CastsShadow`] on a character's top
//! entity (the one at its feet). Every mesh under it, now or later, gets
//! a flattened copy (a "shadow twin"). [`CharacterShadows`] switches them
//! all on or off (the game sets it from SQUAD SHADOWS).
//!
//! Note for later: the visor's tactical glow copies every mesh under a
//! glowing body, so when the squad gets these shadows, the glow must skip
//! [`ShadowTwin`]s.

use bevy::asset::embedded_asset;
use bevy::light::NotShadowCaster;
use bevy::mesh::{MeshVertexBufferLayoutRef, VertexBufferLayout};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, RenderPipelineDescriptor, ShaderType,
    SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use rc_game_core::units::UU_PER_UNIT;
use rc_game_core::{GameLog, GamePaths, PlayerCamera};
use serde::Deserialize;

const PART: &str = "shadows";
const SHADOWS_FILE: &str = "shadows/shadows.toml";

/// `data/shadows/shadows.toml`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ShadowFile {
    /// Towards the light, Unreal axes (Z up).
    pub light_direction: [f32; 3],
    /// Unreal units.
    pub max_trace_distance: f32,
    /// Unreal units.
    pub cull_distance: f32,
    /// 0 to 1.
    pub darkness: f32,
    /// Unreal units.
    pub lift: f32,
}

impl Default for ShadowFile {
    fn default() -> Self {
        toml::from_str(include_str!("../../../data/shadows/shadows.toml"))
            .expect("data/shadows/shadows.toml is valid")
    }
}

impl ShadowFile {
    /// Towards the light in Bevy axes, of length 1. A light below the
    /// horizon (which would throw the shadow off to infinity) is lifted to
    /// at least 10 degrees up.
    pub fn light_bevy(&self) -> Vec3 {
        let [x, y, z] = self.light_direction;
        // Unreal (x, y, z) is Bevy (x, z, y); see rc_game_core::units.
        let d = Vec3::new(x, z, y).normalize_or(Vec3::Y);
        let min_up = 10f32.to_radians().sin();
        if d.y >= min_up {
            d
        } else {
            let flat = Vec3::new(d.x, 0.0, d.z).normalize_or(Vec3::X);
            (flat * min_up.acos().sin() + Vec3::Y * min_up).normalize()
        }
    }

    /// How dark a character's shadow is when the middle of its body
    /// (where the original puts the projector) is `above` Bevy units over
    /// the floor: full `darkness` at the body, fading to nothing at
    /// `max_trace_distance` along the light. (We take the floor to be at
    /// the feet, so this is the same for a character all the time: taller
    /// ones get slightly fainter shadows, as in the original.)
    pub fn strength(&self, above: f32) -> f32 {
        let up = self.light_bevy().y.max(1e-3);
        let along = above.max(0.0) * UU_PER_UNIT / up;
        let reach = self.max_trace_distance.max(1.0);
        (self.darkness * (1.0 - along / reach)).clamp(0.0, 1.0)
    }
}

/// Put on a character's top entity, whose `Transform` is at its feet.
#[derive(Component, Debug, Clone, Copy)]
pub struct CastsShadow {
    /// Feet to the middle of the body, Bevy units (the original's
    /// projector sits at the character's `Location`, the middle).
    pub middle: f32,
}

/// The game's switch for every character shadow (SQUAD SHADOWS).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharacterShadows {
    pub on: bool,
}

impl Default for CharacterShadows {
    fn default() -> Self {
        Self { on: true }
    }
}

/// The settings in use.
#[derive(Resource, Debug, Clone, Default)]
pub struct ShadowLook(pub ShadowFile);

/// Counts for logs and checks.
#[derive(Resource, Debug, Clone, Default)]
pub struct ShadowReadout {
    /// Characters with shadows.
    pub characters: usize,
    /// Flattened copies of their meshes.
    pub twins: usize,
    /// Characters whose shadow is drawn this frame.
    pub drawn: usize,
}

/// A flattened copy of one of a character's meshes.
#[derive(Component, Debug)]
pub struct ShadowTwin;

/// A character's own shadow material (each has its own floor height).
#[derive(Component, Debug)]
struct ShadowOf(Handle<ShadowMaterial>);

/// Marks a mesh that already has its twin.
#[derive(Component, Debug)]
struct HasTwin;

/// The shadow's material: a character's mesh flattened onto its floor.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct ShadowMaterial {
    #[uniform(0)]
    pub shadow: ShadowUniform,
}

/// What the shader needs. Kept in two vec4s for older phones.
#[derive(ShaderType, Debug, Clone, Copy, PartialEq)]
pub struct ShadowUniform {
    /// xyz: towards the light (Bevy axes, length 1). w: lift above the
    /// floor, Bevy units.
    pub light: Vec4,
    /// x: the floor's height (Bevy y). y: darkness 0 to 1. z, w unused.
    pub floor: Vec4,
}

impl Material for ShadowMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://rc_shadows/shadow.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://rc_shadows/shadow.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn enable_oit() -> bool {
        false
    }

    /// Only positions are read. Both sides of every triangle are drawn
    /// (flattening can turn a triangle over). The shadow writes depth and
    /// only draws where nothing nearer was drawn, so the body's triangles
    /// that land on the same spot darken it once, not again and again (the
    /// shader rounds the depth so equal spots compare equal).
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Never drawn in a depth-only pass (prepass and shadows are off
        // above), but if Bevy ever builds one it must keep its own layout.
        if descriptor.label.as_deref() == Some("prepass_pipeline") {
            return Ok(());
        }
        let positions: VertexBufferLayout = layout
            .0
            .get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?;
        descriptor.vertex.buffers = vec![positions];
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(true);
            // Bevy's depth runs from 1 (near) to 0 (far): "greater" is
            // nearer, so an equal depth (the same shadow again) fails.
            depth.depth_compare = Some(CompareFunction::Greater);
        }
        Ok(())
    }
}

pub struct ShadowsPlugin;

impl Plugin for ShadowsPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shadow.wesl");
        app.add_plugins(MaterialPlugin::<ShadowMaterial>::default())
            .init_resource::<CharacterShadows>()
            .init_resource::<ShadowReadout>()
            .add_systems(Startup, load_look)
            .add_systems(
                PostUpdate,
                (attach_twins, update_shadows)
                    .chain()
                    .after(bevy::transform::TransformSystems::Propagate),
            );
    }
}

fn say(log: &Option<Res<GameLog>>, line: impl AsRef<str>) {
    let line = format!("{PART}: {}", line.as_ref());
    match log {
        Some(l) => l.line(line),
        None => info!("{line}"),
    }
}

/// Reads the data file (a mod's copy wins), or uses the built-in values.
fn load_look(mut commands: Commands, paths: Option<Res<GamePaths>>, log: Option<Res<GameLog>>) {
    let file = match paths.as_ref().and_then(|p| p.data_file(SHADOWS_FILE)) {
        None => {
            say(&log, format!("no {SHADOWS_FILE}; built-in values"));
            ShadowFile::default()
        }
        Some(path) => match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| toml::from_str::<ShadowFile>(&t).map_err(|e| e.to_string()))
        {
            Ok(f) => {
                say(&log, format!("settings from {}", path.display()));
                f
            }
            Err(e) => {
                say(
                    &log,
                    format!("ERROR in {}: {e}; built-in values used", path.display()),
                );
                ShadowFile::default()
            }
        },
    };
    let l = file.light_bevy();
    say(
        &log,
        format!(
            "light from ({:.2}, {:.2}, {:.2}) (Bevy axes), fade over {} uu, not drawn beyond {} uu, darkness {}",
            l.x, l.y, l.z, file.max_trace_distance, file.cull_distance, file.darkness
        ),
    );
    commands.insert_resource(ShadowLook(file));
}

fn uniform(look: &ShadowFile, floor: f32, strength: f32) -> ShadowUniform {
    ShadowUniform {
        light: look.light_bevy().extend(look.lift / UU_PER_UNIT),
        floor: Vec4::new(floor, strength, 0.0, 0.0),
    }
}

/// Gives every mesh under a shadow-casting character a flattened twin.
#[allow(clippy::too_many_arguments)]
fn attach_twins(
    mut commands: Commands,
    casters: Query<(Entity, &CastsShadow, &GlobalTransform, Option<&ShadowOf>)>,
    children: Query<&Children>,
    meshes: Query<&Mesh3d, (Without<HasTwin>, Without<ShadowTwin>)>,
    look: Option<Res<ShadowLook>>,
    switch: Res<CharacterShadows>,
    mut materials: ResMut<Assets<ShadowMaterial>>,
    mut readout: ResMut<ShadowReadout>,
    log: Option<Res<GameLog>>,
) {
    let Some(look) = look else {
        return;
    };
    let mut added = 0;
    let mut new_characters = 0;
    for (root, casts, at, own) in &casters {
        let material = match own {
            Some(o) => o.0.clone(),
            None => {
                let floor = at.translation().y;
                let m = materials.add(ShadowMaterial {
                    shadow: uniform(&look.0, floor, look.0.strength(casts.middle)),
                });
                commands.entity(root).insert(ShadowOf(m.clone()));
                new_characters += 1;
                m
            }
        };
        for e in children.iter_descendants(root) {
            let Ok(mesh) = meshes.get(e) else {
                continue;
            };
            commands.entity(e).insert(HasTwin).with_child((
                ShadowTwin,
                Mesh3d(mesh.0.clone()),
                MeshMaterial3d(material.clone()),
                Transform::IDENTITY,
                if switch.on {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                },
                NotShadowCaster,
                // The flattened shape lies outside the body's own bounds.
                bevy::camera::visibility::NoFrustumCulling,
            ));
            added += 1;
        }
    }
    readout.characters += new_characters;
    readout.twins += added;
    if added > 0 {
        say(
            &log,
            format!(
                "{added} more shadow copies ({} characters, {} copies in all)",
                readout.characters, readout.twins
            ),
        );
    }
}

/// Each frame: the floor follows the feet, and shadows far from the camera
/// (or all, when switched off) are hidden.
#[allow(clippy::too_many_arguments)]
fn update_shadows(
    casters: Query<(
        &CastsShadow,
        &GlobalTransform,
        &ShadowOf,
        &InheritedVisibility,
    )>,
    mut twins: Query<(&mut Visibility, &MeshMaterial3d<ShadowMaterial>), With<ShadowTwin>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    look: Option<Res<ShadowLook>>,
    switch: Res<CharacterShadows>,
    mut materials: ResMut<Assets<ShadowMaterial>>,
    mut readout: ResMut<ShadowReadout>,
    mut last_switch: Local<Option<bool>>,
    log: Option<Res<GameLog>>,
) {
    let Some(look) = look else {
        return;
    };
    if *last_switch != Some(switch.on) {
        say(
            &log,
            format!("character shadows {}", if switch.on { "on" } else { "off" }),
        );
        *last_switch = Some(switch.on);
    }
    let eye = camera.iter().next().map(|c| c.translation());
    let cull = look.0.cull_distance / UU_PER_UNIT;
    let mut shown = Vec::new();
    let mut drawn = 0;
    for (casts, at, own, seen) in &casters {
        let feet = at.translation();
        let near = eye.is_none_or(|e| e.distance(feet) <= cull);
        let on = switch.on && near;
        if on {
            shown.push(own.0.id());
            if seen.get() {
                drawn += 1;
            }
            let want = uniform(&look.0, feet.y, look.0.strength(casts.middle));
            // Only touch the material when it changed (a change re-sends it
            // to the graphics card).
            if materials.get(&own.0).is_some_and(|m| m.shadow != want) {
                if let Some(mut m) = materials.get_mut(&own.0) {
                    m.shadow = want;
                }
            }
        }
    }
    readout.drawn = drawn;
    for (mut vis, material) in &mut twins {
        let want = if shown.contains(&material.0.id()) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_file_reads_with_the_originals_numbers() {
        let f = ShadowFile::default();
        assert_eq!(f.light_direction, [1.0, 1.0, 3.0]);
        assert_eq!(f.max_trace_distance, 350.0);
        assert_eq!(f.cull_distance, 1200.0);
    }

    #[test]
    fn the_light_is_high_above_and_unreal_z_becomes_bevy_y() {
        let l = ShadowFile::default().light_bevy();
        assert!((l.length() - 1.0).abs() < 1e-5);
        // (1, 1, 3) normalised: 3 / sqrt(11) up.
        assert!((l.y - 3.0 / 11f32.sqrt()).abs() < 1e-5);
        assert!((l.x - l.z).abs() < 1e-6 && l.x > 0.0);
    }

    #[test]
    fn a_light_below_the_horizon_is_lifted() {
        let f = ShadowFile {
            light_direction: [1.0, 0.0, -1.0],
            ..ShadowFile::default()
        };
        let l = f.light_bevy();
        assert!(l.y > 0.17 && l.y < 0.18, "{l}");
    }

    #[test]
    fn a_standing_trooper_is_about_three_quarters_dark_and_it_fades_with_height() {
        let f = ShadowFile::default();
        // A trooper's middle is 78 uu above its feet.
        let s = f.strength(78.0 / UU_PER_UNIT);
        assert!((0.70..0.80).contains(&s), "{s}");
        assert!(f.strength(200.0 / UU_PER_UNIT) < s);
        assert_eq!(f.strength(1000.0 / UU_PER_UNIT), 0.0);
    }
}
