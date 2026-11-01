//! The commando's visor modes: normal, tactical and low light (see
//! `rules.rs` for how they work and `data/visor/tactical.toml` for the
//! numbers).
//!
//! How other parts use it:
//!
//! - Put a [`TacticalGlow`] on anything that should glow in tactical mode
//!   ("sev", "fixer", "scorch", "pickup", or any name in the data file).
//!   Every mesh under it, now or later, gets a glowing copy (a "shell").
//! - Set [`VisorSettings`] from the options (the game does this).
//! - Read [`VisorState`] for the mode, and [`VisorReadout`] for counts.
//!
//! The visor never moves the camera or draws on the screen itself: the
//! glow is part of the 3D world (so a VR headset sees it on every body in
//! both eyes), the lamp hangs off whatever `PlayerCamera` is, and the
//! mode's name goes to the on-screen display as a `HudMessage`.

pub mod original;
pub mod rules;

pub use rules::{VisorChoice, VisorMode};

use bevy::asset::embedded_asset;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use original::{Original, VisorFile};
use rc_game_core::units::UU_PER_UNIT;
use rc_game_core::{
    Action, ActionState, GameLog, GamePaths, GameSet, GameState, HudMessage, Play, PlayerCamera,
};
use std::collections::HashMap;

/// The options that change the visor. The game copies them from its
/// settings file.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct VisorSettings {
    pub choice: VisorChoice,
    /// TACTICAL MODE INTENSITY, 0 to 1.
    pub intensity: f32,
}

impl Default for VisorSettings {
    /// The original's defaults: CYCLE, intensity 0.2.
    fn default() -> Self {
        VisorSettings {
            choice: VisorChoice::Cycle,
            intensity: 0.2,
        }
    }
}

/// The mode the visor is in now.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Default)]
pub struct VisorState {
    pub mode: VisorMode,
}

/// Counts for checks and the log.
#[derive(Resource, Debug, Clone, Default)]
pub struct VisorReadout {
    /// Meshes that have a glowing copy.
    pub shells: usize,
    /// Of those, the ones drawn right now.
    pub shown: usize,
    /// Glow names with a colour (from the install or the data file).
    pub colours: usize,
    /// Times the mode changed since start.
    pub changes: u32,
}

/// Glows in tactical mode, in the colour of the named thing.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct TacticalGlow(pub String);

/// A glowing copy of a mesh (child of that mesh).
#[derive(Component, Debug)]
pub struct GlowShell;

/// On a mesh that already has its glowing copy.
#[derive(Component, Debug)]
struct HasShell;

/// The helmet lamp (child of the player's camera) in low light mode.
#[derive(Component, Debug)]
pub struct Headlamp;

/// The glow's material: one per glow name.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct TacticalGlowMaterial {
    #[uniform(0)]
    pub glow: GlowUniform,
}

#[derive(ShaderType, Debug, Clone, Copy)]
pub struct GlowUniform {
    pub color: Vec4,
    pub edge_power: f32,
    pub fill: f32,
    pub strength: f32,
    pub unused: f32,
}

impl Material for TacticalGlowMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://rc_visor/tactical_glow.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    /// Drawn on exactly the same surface as the body: nudge it forward so
    /// it isn't hidden by the body's own depth.
    fn depth_bias(&self) -> f32 {
        1.0
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }
}

/// The data file and what the install gave for it.
#[derive(Resource, Debug, Clone)]
pub struct VisorLook {
    pub file: VisorFile,
    pub original: Original,
}

#[derive(Resource, Default)]
struct GlowMaterials(HashMap<String, Handle<TacticalGlowMaterial>>);

/// Sets the visor's systems run in (inside `Update`).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct VisorSet;

pub struct VisorPlugin;

impl Plugin for VisorPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "tactical_glow.wesl");
        app.add_plugins(MaterialPlugin::<TacticalGlowMaterial>::default())
            .init_resource::<VisorSettings>()
            .init_resource::<VisorState>()
            .init_resource::<VisorReadout>()
            .init_resource::<GlowMaterials>()
            .add_systems(OnEnter(GameState::InGame), (load_look, start_mode).chain())
            .add_systems(
                Update,
                (
                    follow_option_changes,
                    // Not while paused (the key then means nothing).
                    cycle_on_key.run_if(in_state(Play::Running)),
                    attach_shells,
                    update_glow,
                    update_headlamp,
                )
                    .chain()
                    .in_set(VisorSet)
                    // After this frame's input, with the rest of the game.
                    .in_set(GameSet::Simulate)
                    .run_if(in_state(GameState::InGame)),
            );
    }
}

fn say(log: &Option<Res<GameLog>>, s: String) {
    match log {
        Some(l) => l.line(s),
        None => println!("{s}"),
    }
}

/// Read the data file and the install, once.
fn load_look(
    mut commands: Commands,
    look: Option<Res<VisorLook>>,
    paths: Option<Res<GamePaths>>,
    log: Option<Res<GameLog>>,
) {
    if look.is_some() {
        return;
    }
    let path = match &paths {
        Some(p) => p.data_file(original::DATA_FILE),
        None => Some(std::path::PathBuf::from("data").join(original::DATA_FILE)),
    };
    let file = match path.as_deref().map(VisorFile::read) {
        Some(Ok(f)) => f,
        Some(Err(e)) => {
            say(&log, format!("visor: {e}; no tactical glow"));
            return;
        }
        None => {
            say(
                &log,
                format!("visor: no {}; no tactical glow", original::DATA_FILE),
            );
            return;
        }
    };
    let found = match &paths {
        Some(p) => original::read(&file, &|rel| p.game_file(rel)),
        None => Original {
            problems: vec!["no game folder".into()],
            ..default()
        },
    };
    let colours: Vec<String> = found
        .colours
        .iter()
        .map(|(k, [r, g, b])| format!("{k} {r},{g},{b}"))
        .collect();
    say(
        &log,
        format!(
            "visor: glow colours from the game: {}; mode texts {:?} / {:?}",
            colours.join(", "),
            found.tactical_text,
            found.low_light_text
        ),
    );
    for p in &found.problems {
        say(&log, format!("visor: {p}"));
    }
    commands.insert_resource(VisorLook {
        file,
        original: found,
    });
}

/// A new level or a loaded game: the visor starts in tactical mode unless
/// the option is OFF, as the original does when the player gets a body.
fn start_mode(
    settings: Res<VisorSettings>,
    mut state: ResMut<VisorState>,
    log: Option<Res<GameLog>>,
) {
    state.mode = rules::start_mode(settings.choice);
    say(
        &log,
        format!(
            "visor: starts in {} mode (option {}, intensity {:.1})",
            state.mode.name(),
            settings.choice.word(),
            settings.intensity
        ),
    );
}

fn follow_option_changes(
    settings: Res<VisorSettings>,
    mut state: ResMut<VisorState>,
    mut readout: ResMut<VisorReadout>,
    log: Option<Res<GameLog>>,
) {
    if !settings.is_changed() || settings.is_added() {
        return;
    }
    let next = rules::after_option_change(state.mode, settings.choice);
    if next != state.mode {
        say(
            &log,
            format!(
                "visor: option {} moves the visor from {} to {} mode",
                settings.choice.word(),
                state.mode.name(),
                next.name()
            ),
        );
        state.mode = next;
        readout.changes += 1;
    }
}

/// CYCLE VISOR MODE (the original's ToggleHeadlamp, X on PC).
fn cycle_on_key(
    actions: Res<ActionState>,
    settings: Res<VisorSettings>,
    mut state: ResMut<VisorState>,
    mut readout: ResMut<VisorReadout>,
    look: Option<Res<VisorLook>>,
    mut hud: MessageWriter<HudMessage>,
    log: Option<Res<GameLog>>,
) {
    if !actions.just_pressed(Action::Flashlight) {
        return;
    }
    let next = rules::next_mode(state.mode, settings.choice);
    say(
        &log,
        format!(
            "visor: CYCLE VISOR MODE, {} -> {} (option {})",
            state.mode.name(),
            next.name(),
            settings.choice.word()
        ),
    );
    state.mode = next;
    readout.changes += 1;
    // The original names the new mode on the display (nothing for normal).
    if let Some(look) = &look {
        let o = &look.original;
        let text = match next {
            VisorMode::Tactical => o.tactical_text.clone(),
            VisorMode::LowLight => o.low_light_text.clone(),
            VisorMode::Normal => None,
        };
        if let Some(text) = text {
            hud.write(HudMessage {
                text,
                seconds: o.text_seconds.unwrap_or(3.0),
            });
        }
    }
}

fn glow_strength(state: &VisorState, settings: &VisorSettings, look: &VisorLook) -> f32 {
    if state.mode == VisorMode::Tactical {
        settings.intensity.clamp(0.0, 1.0) * look.file.glow.strength_scale
    } else {
        0.0
    }
}

fn uniform(rgb: [u8; 3], look: &VisorLook, strength: f32) -> GlowUniform {
    let c = Color::srgb_u8(rgb[0], rgb[1], rgb[2]).to_linear();
    GlowUniform {
        color: Vec4::new(c.red, c.green, c.blue, 1.0),
        edge_power: look.file.glow.edge_power,
        fill: look.file.glow.fill.clamp(0.0, 1.0),
        strength,
        unused: 0.0,
    }
}

/// Give every mesh under a `TacticalGlow` its glowing copy.
#[allow(clippy::too_many_arguments)]
fn attach_shells(
    mut commands: Commands,
    glowing: Query<(Entity, &TacticalGlow)>,
    children: Query<&Children>,
    meshes: Query<&Mesh3d, (Without<HasShell>, Without<GlowShell>)>,
    look: Option<Res<VisorLook>>,
    state: Res<VisorState>,
    settings: Res<VisorSettings>,
    mut handles: ResMut<GlowMaterials>,
    mut materials: ResMut<Assets<TacticalGlowMaterial>>,
    mut readout: ResMut<VisorReadout>,
    mut missing: Local<Vec<String>>,
    log: Option<Res<GameLog>>,
) {
    let Some(look) = look else {
        return;
    };
    readout.colours = look.original.colours.len();
    let strength = glow_strength(&state, &settings, &look);
    let mut added = 0;
    for (root, glow) in &glowing {
        let Some(&rgb) = look.original.colours.get(&glow.0) else {
            if !missing.contains(&glow.0) {
                say(&log, format!("visor: no glow colour for `{}`", glow.0));
                missing.push(glow.0.clone());
            }
            continue;
        };
        let material = handles
            .0
            .entry(glow.0.clone())
            .or_insert_with(|| {
                materials.add(TacticalGlowMaterial {
                    glow: uniform(rgb, &look, strength),
                })
            })
            .clone();
        for e in std::iter::once(root).chain(children.iter_descendants(root)) {
            let Ok(mesh) = meshes.get(e) else {
                continue;
            };
            commands.entity(e).insert(HasShell).with_child((
                GlowShell,
                Mesh3d(mesh.0.clone()),
                MeshMaterial3d(material.clone()),
                Transform::IDENTITY,
                if strength > 0.0 {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                },
                NotShadowCaster,
            ));
            readout.shells += 1;
            added += 1;
        }
    }
    if added > 0 {
        say(
            &log,
            format!(
                "visor: {added} more glowing copies ({} in all, {} glow colours)",
                readout.shells, readout.colours
            ),
        );
    }
}

/// Mode, intensity or colours changed: update the glow, and hide the
/// copies entirely when they'd add nothing (so they cost nothing).
#[allow(clippy::too_many_arguments)]
fn update_glow(
    state: Res<VisorState>,
    settings: Res<VisorSettings>,
    look: Option<Res<VisorLook>>,
    handles: Res<GlowMaterials>,
    mut materials: ResMut<Assets<TacticalGlowMaterial>>,
    mut shells: Query<&mut Visibility, With<GlowShell>>,
    mut readout: ResMut<VisorReadout>,
    mut last: Local<Option<(VisorMode, u32, usize)>>,
) {
    let Some(look) = look else {
        return;
    };
    let strength = glow_strength(&state, &settings, &look);
    let now = (state.mode, strength.to_bits(), readout.shells);
    if *last == Some(now) {
        return;
    }
    *last = Some(now);
    for (name, handle) in &handles.0 {
        if let (Some(&rgb), Some(mut m)) =
            (look.original.colours.get(name), materials.get_mut(handle))
        {
            m.glow = uniform(rgb, &look, strength);
        }
    }
    let shown = strength > 0.0;
    let mut n = 0;
    for mut v in &mut shells {
        let want = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
        n += 1;
    }
    readout.shown = if shown { n } else { 0 };
}

/// The helmet lamp: on in low light mode. PLACEHOLDER for the original's
/// low light view, which also changes the whole picture.
fn update_headlamp(
    mut commands: Commands,
    state: Res<VisorState>,
    look: Option<Res<VisorLook>>,
    cameras: Query<Entity, With<PlayerCamera>>,
    lamps: Query<(Entity, &ChildOf), With<Headlamp>>,
) {
    let want = state.mode == VisorMode::LowLight;
    let Ok(camera) = cameras.single() else {
        return;
    };
    let has = lamps.iter().any(|(_, p)| p.parent() == camera);
    if want && !has {
        let (lumens, range, angle) = look
            .as_ref()
            .map(|l| {
                (
                    l.file.headlamp.lumens,
                    l.file.headlamp.range,
                    l.file.headlamp.angle,
                )
            })
            .unwrap_or((400_000.0, 1500.0, 50.0));
        let half = (angle.to_radians() / 2.0).clamp(0.05, 1.5);
        commands.entity(camera).with_child((
            Headlamp,
            SpotLight {
                intensity: lumens,
                range: range / UU_PER_UNIT,
                outer_angle: half,
                inner_angle: half * 0.7,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::IDENTITY,
        ));
    } else if !want {
        for (e, _) in &lamps {
            commands.entity(e).despawn();
        }
    }
}
