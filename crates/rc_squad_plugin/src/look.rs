//! What the squad mates look like: the original Sev, Fixer and Scorch
//! models, animations and armour pictures, read from the game at run time
//! (`rc_model`). Which mesh and which animations are named in
//! `data/squad/looks.toml`, so a mod can swap them.
//!
//! A mate whose model can't be read gets the old stand-in body (a capsule
//! in their colour with a dark visor), and the log says why. The stand-in
//! is a placeholder only.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rc_game_core::GamePaths;
use rc_model::{Model, ModelSpec};
use rc_squad::MemberState;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// The looks file, under `data/` (or a mod's folder).
pub const LOOKS_FILE: &str = "squad/looks.toml";
/// Moving faster than this (uu/s) plays a walk or run.
const MOVING_SPEED: f32 = 30.0;
/// Seconds the firing animation shows after a shot.
const FIRE_POSE_TIME: f32 = 0.4;

/// What a squad mate is doing, as far as its animation is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pose {
    Idle,
    Walk,
    Run,
    WalkBack,
    StepLeft,
    StepRight,
    Fire,
    /// Knocked out (health at 0), waiting for a revive.
    Down,
    /// Kneeling by a downed commando.
    Revive,
}

impl Pose {
    pub const ALL: [Pose; 9] = [
        Pose::Idle,
        Pose::Walk,
        Pose::Run,
        Pose::WalkBack,
        Pose::StepLeft,
        Pose::StepRight,
        Pose::Fire,
        Pose::Down,
        Pose::Revive,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Pose::Idle => "idle",
            Pose::Walk => "walk",
            Pose::Run => "run",
            Pose::WalkBack => "walk_back",
            Pose::StepLeft => "step_left",
            Pose::StepRight => "step_right",
            Pose::Fire => "fire",
            Pose::Down => "down",
            Pose::Revive => "revive",
        }
    }

    /// Played once and held on the last frame, instead of looping.
    pub fn once(self) -> bool {
        self == Pose::Down
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Animations {
    pub idle: String,
    pub walk: String,
    pub run: String,
    pub walk_back: String,
    pub step_left: String,
    pub step_right: String,
    pub fire: String,
    pub down: String,
    pub revive: String,
}

impl Animations {
    fn get(&self, p: Pose) -> &str {
        match p {
            Pose::Idle => &self.idle,
            Pose::Walk => &self.walk,
            Pose::Run => &self.run,
            Pose::WalkBack => &self.walk_back,
            Pose::StepLeft => &self.step_left,
            Pose::StepRight => &self.step_right,
            Pose::Fire => &self.fire,
            Pose::Down => &self.down,
            Pose::Revive => &self.revive,
        }
    }
}

/// Seconds to blend smoothly from one animation into the next (`[blend]`
/// in the looks file; 0 snaps). Our choice: the original blends inside its
/// compiled engine code.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct BlendTimes {
    /// Into any animation not named below.
    pub time: f32,
    /// Into the firing animation, which is very short.
    pub fire: f32,
    /// Into falling down wounded.
    pub down: f32,
}

impl Default for BlendTimes {
    fn default() -> Self {
        BlendTimes {
            time: 0.2,
            fire: 0.05,
            down: 0.25,
        }
    }
}

impl BlendTimes {
    pub fn for_pose(&self, p: Pose) -> f32 {
        match p {
            Pose::Fire => self.fire,
            Pose::Down => self.down,
            _ => self.time,
        }
        .max(0.0)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberLook {
    pub package: String,
    pub mesh: String,
    #[serde(default)]
    pub set: Option<String>,
    /// Replacement materials by slot; "" keeps the mesh's own.
    #[serde(default)]
    pub skins: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LooksFile {
    /// Light every squad mate always gets, 0 to 1 of its own colour.
    pub ambient: f32,
    pub animations: Animations,
    #[serde(default)]
    pub blend: BlendTimes,
    #[serde(default)]
    pub member: HashMap<String, MemberLook>,
}

impl LooksFile {
    /// The file, or why it couldn't be used.
    pub fn read(paths: Option<&GamePaths>) -> Result<(LooksFile, PathBuf), String> {
        let file = match paths {
            Some(p) => p.data_file(LOOKS_FILE),
            None => Some(PathBuf::from("data").join(LOOKS_FILE)).filter(|p| p.exists()),
        }
        .ok_or_else(|| format!("no {LOOKS_FILE}"))?;
        let text = std::fs::read_to_string(&file)
            .map_err(|e| format!("can't read {}: {e}", file.display()))?;
        let looks =
            toml::from_str(&text).map_err(|e| format!("ERROR in {}: {e}", file.display()))?;
        Ok((looks, file))
    }

    pub fn spec(&self, key: &str) -> Option<ModelSpec> {
        let m = self.member.get(key)?;
        Some(ModelSpec {
            package: m.package.clone(),
            mesh: m.mesh.clone(),
            set: m.set.clone(),
            sequences: Pose::ALL
                .iter()
                .map(|&p| (p.key().to_string(), self.animations.get(p).to_string()))
                .collect(),
            skins: m
                .skins
                .iter()
                .map(|s| Some(s.clone()).filter(|s| !s.is_empty()))
                .collect(),
        })
    }
}

/// Read one squad mate's model. Lines for the log either way.
pub fn load_model(
    paths: &GamePaths,
    looks: &LooksFile,
    key: &str,
) -> Result<(Model, Vec<String>), String> {
    let spec = looks
        .spec(key)
        .ok_or_else(|| format!("no [member.{key}] in {LOOKS_FILE}"))?;
    Model::load(&|rel| paths.game_file(rel), &spec)
}

/// One drawn piece of a squad mate: the mesh of one material section.
pub struct Piece {
    pub section: usize,
    pub mesh: Handle<Mesh>,
}

/// A squad mate drawn with the original model.
#[derive(Component)]
pub struct MateModel {
    pub model: Arc<Model>,
    pub pieces: Vec<Piece>,
    pub pose: Pose,
    pub frame: f32,
    /// Smooths the change from one animation to the next.
    pub blend: rc_anim::Crossfade,
    pub blend_times: BlendTimes,
    /// Shots fired when last looked at, and the time of the last one.
    shots: u32,
    since_shot: f32,
}

/// The turn from the model's axes to the squad mate body's (see
/// `spawn_pieces`).
pub fn model_turn() -> Quat {
    Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
}

/// Make Bevy meshes and materials for `model` posed at `pose`, as
/// children of `parent`.
pub fn spawn_pieces(
    model: Arc<Model>,
    ambient: f32,
    parent: &mut EntityCommands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> MateModel {
    let points = model.posed(Pose::Idle.key(), 0.0);
    let normals = model.normals(&points);
    let mut pieces = Vec::new();
    let a = ambient.max(0.0);
    // The model faces the game's forward (+X, Bevy +X once converted); the
    // squad mate's body faces its local -Z like the player. A quarter turn
    // lines them up.
    parent.with_children(|p| {
        let mut holder = p.spawn((
            Transform::from_rotation(model_turn()),
            Visibility::default(),
        ));
        holder.with_children(|c| {
            for (i, s) in model.sections.iter().enumerate() {
                let (pos, nrm) = model.section_attributes(s, &points, &normals);
                let mut m = Mesh::new(
                    PrimitiveTopology::TriangleList,
                    RenderAssetUsages::default(),
                );
                m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
                m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
                m.insert_attribute(Mesh::ATTRIBUTE_UV_0, model.section_uvs(s));
                m.insert_indices(Indices::U32(s.indices.clone()));
                let mesh = meshes.add(m);
                let texture = s.skin.as_ref().map(|k| {
                    images.add(Image::new(
                        Extent3d {
                            width: k.image.width as u32,
                            height: k.image.height as u32,
                            depth_or_array_layers: 1,
                        },
                        TextureDimension::D2,
                        k.image.pixels.clone(),
                        TextureFormat::Rgba8UnormSrgb,
                        RenderAssetUsages::RENDER_WORLD,
                    ))
                });
                // As for the droids: the level's lights are baked into its
                // surfaces and the scene has no ambient light, so give the
                // character a steady share of its own colour.
                let material = materials.add(StandardMaterial {
                    base_color: if texture.is_some() {
                        Color::WHITE
                    } else {
                        Color::srgb(0.6, 0.6, 0.6)
                    },
                    emissive: LinearRgba::rgb(a, a, a) * if texture.is_some() { 1.0 } else { 0.3 },
                    emissive_texture: texture.clone(),
                    base_color_texture: texture,
                    perceptual_roughness: 0.7,
                    alpha_mode: if s.skin.as_ref().is_some_and(|k| k.cutout) {
                        AlphaMode::Mask(0.5)
                    } else {
                        AlphaMode::Opaque
                    },
                    ..default()
                });
                // Posed on the CPU: the mesh's stored bounds don't follow it.
                c.spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material),
                    NoFrustumCulling,
                ));
                pieces.push(Piece { section: i, mesh });
            }
        });
    });
    MateModel {
        model,
        pieces,
        pose: Pose::Idle,
        frame: 0.0,
        blend: rc_anim::Crossfade::default(),
        blend_times: BlendTimes::default(),
        shots: 0,
        since_shot: f32::INFINITY,
    }
}

/// Which animation fits what the member is doing.
pub fn choose_pose(member: &rc_squad::squad::Member, just_fired: bool, run_speed: f32) -> Pose {
    if member.down {
        return Pose::Down;
    }
    if matches!(member.state, MemberState::Reviving { .. }) {
        return Pose::Revive;
    }
    let v = member.pawn.velocity;
    let speed = (v.x * v.x + v.y * v.y).sqrt();
    if speed > MOVING_SPEED {
        let mut rel = v.y.atan2(v.x) - member.yaw;
        rel = (rel + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        let deg = rel.to_degrees();
        return if deg.abs() <= 45.0 {
            if speed > 0.75 * run_speed {
                Pose::Run
            } else {
                Pose::Walk
            }
        } else if deg.abs() >= 135.0 {
            Pose::WalkBack
        } else if deg > 0.0 {
            // Unreal's yaw grows clockwise seen from above: to the right.
            Pose::StepRight
        } else {
            Pose::StepLeft
        };
    }
    if just_fired {
        Pose::Fire
    } else {
        Pose::Idle
    }
}

/// Advance each squad mate's animation and pose its meshes.
pub fn animate(
    time: Res<Time>,
    state: Res<crate::SquadState>,
    mut mates: Query<(&crate::SquadMate, &mut MateModel)>,
    meshes: Option<ResMut<Assets<Mesh>>>,
) {
    let Some(squad) = state.squad.as_ref() else {
        return;
    };
    let mut meshes = meshes;
    let dt = time.delta_secs();
    for (mate, mut mm) in &mut mates {
        let Some(member) = squad.members.get(mate.index) else {
            continue;
        };
        animate_one(member, &mut mm, dt, meshes.as_deref_mut());
    }
}

fn animate_one(
    member: &rc_squad::squad::Member,
    mm: &mut MateModel,
    dt: f32,
    meshes: Option<&mut Assets<Mesh>>,
) {
    if member.fight.shots != mm.shots {
        mm.shots = member.fight.shots;
        mm.since_shot = 0.0;
    } else {
        mm.since_shot += dt;
    }
    let pose = choose_pose(
        member,
        mm.since_shot < FIRE_POSE_TIME,
        rc_movement::MoveParams::player_commando().ground_speed,
    );
    if pose != mm.pose {
        mm.pose = pose;
        mm.frame = 0.0;
        let t = mm.blend_times.for_pose(pose);
        mm.blend.start(t);
    } else {
        mm.blend.advance(dt);
    }
    let (rate, frames) = match mm.model.clip(pose.key()) {
        Some(c) => (c.rate, c.num_frames),
        None => (0.0, 1.0),
    };
    let last = (frames - 1.0).max(0.0);
    let before = mm.frame;
    mm.frame = if pose.once() {
        (mm.frame + dt * rate).min(last)
    } else {
        (mm.frame + dt * rate).rem_euclid(frames.max(1.0))
    };
    // A body that finished falling doesn't need posing again.
    if pose.once() && before == last && mm.frame == last && !mm.blend.blending() {
        return;
    }
    let Some(meshes) = meshes else {
        return;
    };
    let target = mm.model.local_pose(pose.key(), mm.frame);
    let points = mm.model.posed_local(mm.blend.apply(target));
    let normals = mm.model.normals(&points);
    for p in &mm.pieces {
        let (pos, nrm) =
            mm.model
                .section_attributes(&mm.model.sections[p.section], &points, &normals);
        if let Some(mut m) = meshes.get_mut(&p.mesh) {
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
        }
    }
}
