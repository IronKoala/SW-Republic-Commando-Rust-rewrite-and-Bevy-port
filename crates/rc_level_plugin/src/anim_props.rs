//! Props drawn with an animated model: bacta dispensers, and anything
//! else whose class draws a skeletal mesh (`DrawType` `DT_Mesh`) rather
//! than a static one. `rc_level::map_collision` lists them
//! (`MapCollision::anim_props`); this file reads each model once with
//! `rc_model` and draws every copy where the map puts it.
//!
//! They stand in the model's own pose until something sends a
//! [`PropAnim`] naming them (the bacta dispenser plays `Load` when the
//! player starts healing and `Holster` when they stop); the animation then
//! plays once and the prop keeps its last frame. Animated props
//! (`AnimProp`, such as geo_04b's anti-air turret) instead start their
//! `AnimHealthy` animation as the level starts, looping when it says so,
//! and an event with their tag starts their `AnimTriggered` one (or stops
//! it, when it is already playing), as the original's `AnimProp` does. A
//! model the level links an animation set to (`CustomMeshSets`) is read
//! with that set. The sounds the animation
//! names (`AnimNotify_Sound`: the dispenser's clunks and hiss) play from
//! the prop. They aren't solid yet. [`AnimProp::bone`] says where one of
//! the model's bones is, for things hung on it (the bacta dispenser's zap
//! effect, on its `Eye` bone).

use bevy::asset::RenderAssetUsages;
use bevy::audio::{PlaybackSettings, Volume};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rc_combat::GameSound;
use rc_game_core::{GameLog, GamePaths, GameState, PlayerCamera, PropAnim, ScriptEvent, SoundAt};
use rc_level::actors::Placement;
use rc_level::class_look::PropAnims;
use rc_model::{to_bevy, Model, ModelSpec};
use std::collections::HashMap;
use std::sync::Arc;

/// Light the model always gets, as for the squad and the droids: the
/// level's light is baked into its surfaces and the scene has no ambient
/// light. A guess (0 to 1).
const AMBIENT: f32 = 0.5;

/// Bevy units to Unreal units (`rc_model::to_bevy` undone).
const UNREAL_PER_BEVY: f32 = 50.0;

/// The animations read for every animated prop, by the model's own
/// sequence names. The bacta dispensers have these two; a model without
/// one just doesn't play it.
pub const ANIMATIONS: [&str; 2] = ["Load", "Holster"];

/// Full loudness out to this distance (Unreal units), then fading as for
/// the enemies' sounds. A guess.
const SOUND_REACH: f32 = 1000.0;

/// One drawn animated prop.
#[derive(Component, Clone)]
pub struct AnimProp {
    /// The map's actor name (`BactaDispenserGEO0`).
    pub name: String,
    pub class: String,
    pub export: usize,
    pub model: Arc<Model>,
    pub place: Placement,
    /// One mesh per model section, in section order.
    pub meshes: Vec<Handle<Mesh>>,
    /// The animation playing and how far in (frames), if any.
    pub playing: Option<(String, f32)>,
    /// The animation playing loops (`bLoop`).
    pub looping: bool,
    /// What it plays by itself, for animated props (`AnimProp`).
    pub anims: Option<PropAnims>,
    /// The last animation played (it holds its last frame).
    pub last: Option<String>,
    /// An animation stopped part way (`AnimTriggered` again): it holds
    /// that frame.
    pub stopped: Option<(String, f32)>,
    /// Every animation started, in order (for the log and self-tests).
    pub played: Vec<String>,
    /// Every sound its animations played, by dotted path ("(not
    /// loaded)" added when it couldn't be read).
    pub heard: Vec<String>,
}

impl AnimProp {
    /// The animation and frame it is posed at now: the one playing, else
    /// the last frame of the last one played, else the model's own pose.
    pub fn pose_now(&self) -> (String, f32) {
        if let Some((a, f)) = self.playing.as_ref().or(self.stopped.as_ref()) {
            return (a.clone(), *f);
        }
        if let Some(a) = &self.last {
            if let Some(c) = self.model.clip(a) {
                return (a.clone(), (c.num_frames - 1.0).max(0.0));
            }
        }
        (String::new(), 0.0)
    }

    /// Where bone `bone` is now, in the level (Unreal units), and its three
    /// axes (unit length, in the level's axes): what the game's
    /// `AttachToBone` hangs an attachment on. `None` if the model has no
    /// such bone.
    pub fn bone(&self, bone: &str) -> Option<([f32; 3], [[f32; 3]; 3])> {
        let (anim, frame) = self.pose_now();
        let (at, axes) = self.model.bone_from_origin(&anim, frame, bone)?;
        let world = self.place.transform_point(at);
        let axes = axes.map(|a| {
            let tip = self
                .place
                .transform_point([at[0] + a[0], at[1] + a[1], at[2] + a[2]]);
            let v = [tip[0] - world[0], tip[1] - world[1], tip[2] - world[2]];
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
            v.map(|c| c / len)
        });
        Some((world, axes))
    }
}

/// The sounds the props' animations play, by [`sound_key`].
#[derive(Resource, Default)]
pub struct AnimPropSounds(pub HashMap<String, GameSound>);

/// A sound's key: its package and name, lower case, without the groups in
/// between (`GEN_Sound.Machines.mach_X` -> `gen_sound.mach_x`), the way
/// the sound packages are read.
pub fn sound_key(path: &str) -> String {
    let package = path.split('.').next().unwrap_or(path);
    let name = path.rsplit('.').next().unwrap_or(path);
    format!("{package}.{name}").to_ascii_lowercase()
}

/// What `spawn` did, for the log.
#[derive(Default, Debug)]
pub struct Report {
    pub drawn: usize,
    /// "Package.Mesh x count" for each model drawn.
    pub models: Vec<String>,
    /// Why some couldn't be drawn, or a sound couldn't be read.
    pub failures: Vec<String>,
    /// Sounds the props' animations play, read.
    pub sounds: usize,
}

/// The game path of an animation package (`BactaDispensers` ->
/// `GameData/Animations/BactaDispensers.ukx`).
pub fn package_path(package: &str) -> String {
    format!("GameData/Animations/{package}.ukx")
}

/// Points of `model` at `frame` of animation `anim` (`""`: posed as
/// stored), placed where `place` stands, in Bevy units.
pub fn placed_points(model: &Model, place: &Placement, anim: &str, frame: f32) -> Vec<[f32; 3]> {
    model
        .posed_from_origin(anim, frame)
        .into_iter()
        .map(|b| {
            let u = [
                b[0] * UNREAL_PER_BEVY,
                b[2] * UNREAL_PER_BEVY,
                b[1] * UNREAL_PER_BEVY,
            ];
            to_bevy(place.transform_point(u))
        })
        .collect()
}

/// A model read once per level, with its materials.
type Loaded = (Arc<Model>, Vec<Handle<StandardMaterial>>);

/// Read each model once and draw every prop. Entities go away when the
/// game leaves `InGame`.
pub fn spawn(
    world: &mut World,
    paths: &GamePaths,
    props: &[(Placement, String, String)],
    prop_anims: &HashMap<usize, PropAnims>,
    custom_sets: &[(String, String)],
) -> Report {
    let mut report = Report::default();
    // Per model: the model and one material per section (shared by copies).
    let mut models: HashMap<String, Option<Loaded>> = HashMap::new();
    let mut counts: Vec<(String, usize)> = Vec::new();
    for (place, package, mesh) in props {
        let anims = prop_anims.get(&place.export).cloned();
        // The level's own animation set for this mesh, if it links one.
        let set = custom_sets
            .iter()
            .find(|(m, _)| m.eq_ignore_ascii_case(mesh))
            .map(|(_, s)| s.clone());
        let mut own: Vec<String> = anims
            .iter()
            .flat_map(|a| [&a.healthy, &a.triggered])
            .flatten()
            .map(|(n, _)| n.clone())
            .collect();
        own.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let key = format!(
            "{package}.{mesh}.{}.{}",
            set.as_deref().unwrap_or(""),
            own.join(",")
        );
        if !models.contains_key(&key) {
            let mut sequences: Vec<(String, String)> = ANIMATIONS
                .iter()
                .map(|a| (a.to_string(), a.to_string()))
                .collect();
            for n in &own {
                if !sequences.iter().any(|(k, _)| k.eq_ignore_ascii_case(n)) {
                    sequences.push((n.clone(), n.clone()));
                }
            }
            let spec = ModelSpec {
                package: package_path(package),
                mesh: mesh.clone(),
                set: set.clone(),
                sequences,
                ..Default::default()
            };
            let loaded = match Model::load(&|rel| paths.game_file(rel), &spec) {
                Ok((m, _notes)) => {
                    let mats = materials_for(world, &m);
                    load_sounds(world, paths, &m, &mut report);
                    Some((Arc::new(m), mats))
                }
                Err(e) => {
                    report.failures.push(format!("{key}: {e}"));
                    None
                }
            };
            models.insert(key.clone(), loaded);
        }
        let Some((model, mats)) = models.get(&key).and_then(|m| m.as_ref()) else {
            continue;
        };
        // An animated prop starts its own animation at once.
        let start = anims
            .as_ref()
            .and_then(|a| a.healthy.clone())
            .filter(|(n, _)| model.clip(n).is_some());
        if let Some((n, _)) = anims.as_ref().and_then(|a| a.healthy.as_ref()) {
            if start.is_none() {
                report
                    .failures
                    .push(format!("{}: has no animation {n} to play", place.name));
            }
        }
        let points = placed_points(model, place, "", 0.0);
        let normals = model.normals(&points);
        let mut pieces = Vec::new();
        for (s, mat) in model.sections.iter().zip(mats) {
            let (pos, nrm) = model.section_attributes(s, &points, &normals);
            let mut m = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            );
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, model.section_uvs(s));
            m.insert_indices(Indices::U32(s.indices.clone()));
            pieces.push((world.resource_mut::<Assets<Mesh>>().add(m), mat.clone()));
        }
        world
            .spawn((
                AnimProp {
                    name: place.name.clone(),
                    class: place.class.clone(),
                    export: place.export,
                    model: model.clone(),
                    place: place.clone(),
                    meshes: pieces.iter().map(|p| p.0.clone()).collect(),
                    playing: start.as_ref().map(|(n, _)| (n.clone(), 0.0)),
                    looping: start.as_ref().is_some_and(|(_, l)| *l),
                    anims,
                    last: None,
                    stopped: None,
                    played: start.iter().map(|(n, _)| n.clone()).collect(),
                    heard: Vec::new(),
                },
                Transform::default(),
                Visibility::default(),
                DespawnOnExit(GameState::InGame),
            ))
            .with_children(|c| {
                for (mesh, mat) in pieces {
                    c.spawn((Mesh3d(mesh), MeshMaterial3d(mat)));
                }
            });
        report.drawn += 1;
        let shown = format!("{package}.{mesh}");
        match counts.iter_mut().find(|(k, _)| *k == shown) {
            Some((_, n)) => *n += 1,
            None => counts.push((shown, 1)),
        }
    }
    report.models = counts
        .into_iter()
        .map(|(k, n)| format!("{k} x{n}"))
        .collect();
    report
}

/// Read the sounds `model`'s animations play, once each.
fn load_sounds(world: &mut World, paths: &GamePaths, model: &Model, report: &mut Report) {
    let mut by_package: HashMap<String, Vec<String>> = HashMap::new();
    {
        let have = world.get_resource_or_init::<AnimPropSounds>();
        for (_, path) in model.sounds.values().flatten() {
            if have.0.contains_key(&sound_key(path)) {
                continue;
            }
            if let Some((package, name)) = path.split_once('.') {
                let name = name.rsplit('.').next().unwrap_or(name).to_string();
                let names = by_package.entry(package.to_string()).or_default();
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }
    for (package, names) in by_package {
        let (found, notes) = world.resource_scope(|_, mut audio: Mut<Assets<AudioSource>>| {
            rc_combat::read_sounds(paths, &package, &names, &mut audio)
        });
        report.failures.extend(notes);
        let mut have = world.resource_mut::<AnimPropSounds>();
        for (name, sound) in found {
            have.0
                .insert(sound_key(&format!("{package}.{name}")), sound);
        }
        report.sounds += names.len();
    }
}

/// Start the animations asked for, step the ones playing, re-pose their
/// meshes and play their sounds.
pub fn animate(
    time: Res<Time>,
    mut asked: MessageReader<PropAnim>,
    mut events: MessageReader<ScriptEvent>,
    mut props: Query<&mut AnimProp>,
    mut meshes: ResMut<Assets<Mesh>>,
    sounds: Option<Res<AnimPropSounds>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    mut commands: Commands,
    log: Res<GameLog>,
) {
    for a in asked.read() {
        let mut found = false;
        for mut p in &mut props {
            if !p.name.eq_ignore_ascii_case(&a.actor) {
                continue;
            }
            found = true;
            match p.model.clip(&a.anim) {
                Some(_) => {
                    log.line(format!("level: {} plays {}", p.name, a.anim));
                    p.playing = Some((a.anim.clone(), 0.0));
                    p.looping = false;
                    p.stopped = None;
                    p.played.push(a.anim.clone());
                    // Sounds at the very start play on the first step.
                    let heard =
                        play_sounds(&p, &a.anim, -1.0, 0.0, &sounds, &camera, &mut commands);
                    for h in heard {
                        log.line(format!("level: {} sound {h}", p.name));
                        p.heard.push(h);
                    }
                }
                None => log.line(format!("level: {} has no animation {}", p.name, a.anim)),
            }
        }
        if !found {
            log.line(format!(
                "level: no animated prop called {} (to play {})",
                a.actor, a.anim
            ));
        }
    }
    // Events with an animated prop's tag (`AnimProp.Trigger`): start its
    // `AnimTriggered` animation, or stop it when it is already playing.
    for e in events.read() {
        for mut p in &mut props {
            let Some((anim, looping)) = p
                .anims
                .as_ref()
                .filter(|a| a.tag.eq_ignore_ascii_case(&e.name))
                .and_then(|a| a.triggered.clone())
            else {
                continue;
            };
            if p.playing
                .as_ref()
                .is_some_and(|(n, _)| n.eq_ignore_ascii_case(&anim))
            {
                log.line(format!("level: {} stops {anim} ({})", p.name, e.name));
                let (n, f) = p.playing.take().unwrap_or_default();
                p.last = None;
                // It keeps the frame it was at.
                p.stopped = Some((n, f));
                continue;
            }
            if p.model.clip(&anim).is_none() {
                log.line(format!("level: {} has no animation {anim}", p.name));
                continue;
            }
            log.line(format!("level: {} plays {anim} ({})", p.name, e.name));
            p.playing = Some((anim.clone(), 0.0));
            p.looping = looping;
            p.stopped = None;
            p.played.push(anim);
        }
    }
    let dt = time.delta_secs().min(0.25);
    for mut p in &mut props {
        let Some((anim, frame)) = p.playing.clone() else {
            continue;
        };
        let Some(clip) = p.model.clip(&anim) else {
            p.playing = None;
            continue;
        };
        let last = (clip.num_frames - 1.0).max(0.0);
        let span = clip.num_frames.max(1.0);
        let looping = p.looping;
        let (next, heard) = if looping {
            // Round and round: the sounds of the part played, across the
            // end and back to the start when it wraps.
            let raw = frame + clip.rate * dt;
            let next = raw.rem_euclid(span);
            let mut heard = play_sounds(
                &p,
                &anim,
                frame / span,
                raw.min(span) / span,
                &sounds,
                &camera,
                &mut commands,
            );
            if raw >= span {
                heard.extend(play_sounds(
                    &p,
                    &anim,
                    -1.0,
                    next / span,
                    &sounds,
                    &camera,
                    &mut commands,
                ));
            }
            (next, heard)
        } else {
            let next = (frame + clip.rate * dt).min(last);
            let heard = play_sounds(
                &p,
                &anim,
                frame / span,
                next / span,
                &sounds,
                &camera,
                &mut commands,
            );
            (next, heard)
        };
        for h in heard {
            log.line(format!("level: {} sound {h}", p.name));
            p.heard.push(h);
        }
        let points = placed_points(&p.model, &p.place, &anim, next);
        let normals = p.model.normals(&points);
        for (s, h) in p.model.sections.iter().zip(&p.meshes) {
            let (pos, nrm) = p.model.section_attributes(s, &points, &normals);
            if let Some(mut m) = meshes.get_mut(h) {
                m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
                m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
            }
        }
        if !looping && next >= last {
            p.playing = None;
            p.last = Some(anim);
        } else {
            p.playing = Some((anim, next));
        }
    }
}

/// Play the sounds of `anim` whose moment falls after `from` and up to
/// `to` (0 to 1 through the animation).
fn play_sounds(
    p: &AnimProp,
    anim: &str,
    from: f32,
    to: f32,
    sounds: &Option<Res<AnimPropSounds>>,
    camera: &Query<&GlobalTransform, With<PlayerCamera>>,
    commands: &mut Commands,
) -> Vec<String> {
    let mut out = Vec::new();
    let Some(list) = p.model.sounds.get(anim) else {
        return out;
    };
    let at = rc_game_core::units::to_bevy(rc_movement::Vec3::new(
        p.place.location[0],
        p.place.location[1],
        p.place.location[2],
    ));
    for (t, path) in list {
        if !(*t > from && *t <= to) {
            continue;
        }
        let Some(h) = sounds
            .as_ref()
            .and_then(|s| s.0.get(&sound_key(path)))
            .and_then(|g| g.pick())
        else {
            out.push(format!("{path} (not loaded)"));
            continue;
        };
        let distance = camera
            .single()
            .map(|c| c.translation().distance(at) * UNREAL_PER_BEVY)
            .unwrap_or(0.0);
        let volume = if distance <= SOUND_REACH {
            1.0
        } else {
            SOUND_REACH / distance
        };
        commands.spawn((
            AudioPlayer(h),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)),
            SoundAt::place(at),
            DespawnOnExit(GameState::InGame),
        ));
        out.push(path.clone());
    }
    out
}

/// One material per section of `model`, with its picture.
fn materials_for(world: &mut World, model: &Model) -> Vec<Handle<StandardMaterial>> {
    let a = AMBIENT;
    model
        .sections
        .iter()
        .map(|s| {
            let texture = s.skin.as_ref().map(|k| {
                world.resource_mut::<Assets<Image>>().add(Image::new(
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
            world
                .resource_mut::<Assets<StandardMaterial>>()
                .add(StandardMaterial {
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
                })
        })
        .collect()
}
