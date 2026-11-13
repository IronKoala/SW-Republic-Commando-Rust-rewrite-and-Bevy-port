//! Particle effects in the game: explosions, smoke, sparks and flying
//! debris, read from the game's own effect classes (`rc_effects`) and
//! drawn with Bevy.
//!
//! Anyone can ask for an effect with [`SpawnEffect`] (the breakable props
//! do, when a barrel blows up). The effect plays its own sound
//! (`SpawnSound`) and any extra sound the sender names, and is removed when
//! its last particle is gone.
//!
//! How it is drawn (our choices, kept cheap for phones and VR):
//! - every sprite emitter is one mesh of camera-facing squares, rebuilt each
//!   frame, with the emitter's picture and blending (adding light for
//!   fire and flares, see-through for smoke and dust);
//! - every particle of a mesh emitter (debris) is the game's own model;
//! - the particles that bounce use a flat floor under the effect, not the
//!   level (see `rc_effects::def::EmitterDef::collides`);
//! - a mesh emitter with `UseMeshBlendMode` (the class default) draws its
//!   model solid, as its own materials are; an effect whose `bUnlit` is
//!   off (the flying gunships and fighters) has its models lit by the
//!   level's light where they are, as the droids are
//!   (`rc_game_core::LitByPlace`), starting from `lit_model_light`;
//! - an effect placed turned (the level's effect spawners) has its
//!   relative emitters' models turned the same way.
//!
//! Whoever gives an effect an `id` hears an [`EffectEnded`] with it when
//! the effect is over (the effect spawners count what they made).
//!
//! An effect with an `AmbientSound` (the bacta dispenser's zap hums) loops
//! it for as long as the effect is there. An effect can be carried by an
//! entity (`follow`) and turned by given axes (`axes`), as one hung on a
//! model's bone is; when its carrier goes it fades out, or with
//! `vanish_with_carrier` is taken away at once.
//!
//! Settings are in `data/effects/effects.toml`. Everything goes to the log
//! with an `effects:` prefix, and the numbers to [`EffectStats`].

pub mod held;

use bevy::asset::RenderAssetUsages;
use bevy::audio::{PlaybackSettings, Volume};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use rc_combat::GameSound;
use rc_effects::def::{CoordinateSystem, DrawStyle, EffectDef, EmitterKind};
use rc_effects::read::Library;
use rc_effects::EffectState;
use rc_game_core::{
    units, GameLog, GamePaths, GameSet, GameState, PlayerCamera, SoundAt, SoundBy,
};
use rc_level_viewer::draw::to_image;
use rc_level_viewer::materials::{find_export, Materials, Picture};
use std::collections::HashMap;
use std::sync::Arc;

/// "Show this effect here." Bevy space.
#[derive(Message, Debug, Clone)]
pub struct SpawnEffect {
    /// The effect's class name (`Explosion_Barrel_GEO`).
    pub effect: String,
    pub at: Vec3,
    /// The height (Bevy Y) of the ground under it, for debris that
    /// bounces; `None` lets it fall.
    pub floor: Option<f32>,
    /// Another sound to play with it (a prop's `TransitionSound`): its
    /// path, package first.
    pub sound: Option<Vec<String>>,
    /// Who asked, for the log.
    pub by: String,
    /// Its turn (Unreal pitch, yaw, roll) and draw scale, as placed by an
    /// effect spawner: see `rc_effects::EffectState::placed`.
    pub rotation: [i32; 3],
    pub draw_scale: f32,
    /// A number to report its end with ([`EffectEnded`]); `None` for no
    /// report.
    pub id: Option<u64>,
    /// Carried by this entity (a projectile's trail, `ProjectileEffect`):
    /// it moves with it, and fades out when it's gone.
    pub follow: Option<Entity>,
    /// When the carrier (`follow`) is gone, take the effect away at once
    /// (the game's `Destroy`, as `DestroyAttachment` does to the bacta
    /// dispenser's zap) instead of letting it fade out (`FadeOut`).
    pub vanish_with_carrier: bool,
    /// Its own axes (forward, right, up, Unreal X, Y, Z, in the level's
    /// axes), for an effect hung on a bone (`AttachToBone` with no turn of
    /// its own takes the bone's). Used instead of `rotation` when given.
    pub axes: Option<[[f32; 3]; 3]>,
    /// Runs for as long as its carrier (`follow`) is there, however long
    /// that is: an emitter placed in the map, which in the original
    /// sends particles for the whole level. Without it, an effect that
    /// never ends is stopped after a minute.
    pub endless: bool,
    /// Leave its `AmbientSound` to someone else: an emitter placed in the
    /// map, whose loop the scripts part's map sounds play from its place
    /// with the game's own distance rule (`rc_scripts_plugin::ambient`).
    pub skip_ambient_sound: bool,
}

impl Default for SpawnEffect {
    fn default() -> Self {
        SpawnEffect {
            effect: String::new(),
            at: Vec3::ZERO,
            floor: None,
            sound: None,
            by: String::new(),
            rotation: [0; 3],
            draw_scale: 1.0,
            id: None,
            follow: None,
            vanish_with_carrier: false,
            axes: None,
            endless: false,
            skip_ambient_sound: false,
        }
    }
}

/// A level event names a placed emitter: every effect carried by
/// `carrier` is triggered ([`rc_effects::sim::EffectState::trigger`]).
#[derive(Message, Debug, Clone, PartialEq)]
pub struct TriggerEffect {
    pub carrier: Entity,
}

/// An effect started with an `id` is over (its life ended or its last
/// particle is gone), or couldn't be shown at all.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct EffectEnded {
    pub id: u64,
    pub by: String,
}

/// "This effect (and sound) may be wanted soon": read it now, while the
/// level loads, so the first explosion doesn't stall the game.
#[derive(Message, Debug, Clone)]
pub struct PreloadEffect {
    pub effect: Option<String>,
    pub sound: Option<Vec<String>>,
}

/// Our settings, from `data/effects/effects.toml`.
#[derive(Resource, Debug, Clone)]
pub struct EffectSettings {
    pub enabled: bool,
    /// Multiplies every sprite's size.
    pub size: f32,
    /// Most particles drawn at once over all effects; more are skipped.
    pub max_particles: usize,
    pub volume: f32,
    /// How bright lit effect models (`bUnlit` off) are drawn until the
    /// level's light where they are is known, 0-1 of their own colour.
    pub lit_model_light: f32,
    /// How strongly the level's light shows on lit effect models (the
    /// squad's and droids' value).
    pub lit_model_scale: f32,
}

impl Default for EffectSettings {
    fn default() -> Self {
        EffectSettings {
            enabled: true,
            size: 1.0,
            max_particles: 4000,
            volume: 1.0,
            lit_model_light: 0.5,
            lit_model_scale: 1.1,
        }
    }
}

impl EffectSettings {
    pub fn from_toml(text: &str) -> Result<EffectSettings, String> {
        let v: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
        let mut s = EffectSettings::default();
        let Some(t) = v.get("effects").and_then(|t| t.as_table()) else {
            return Ok(s);
        };
        for (k, val) in t {
            let num = || {
                val.as_float()
                    .or_else(|| val.as_integer().map(|i| i as f64))
                    .ok_or_else(|| format!("effects.{k} should be a number"))
            };
            match k.as_str() {
                "enabled" => {
                    s.enabled = val
                        .as_bool()
                        .ok_or_else(|| "effects.enabled should be true or false".to_string())?
                }
                "size" => s.size = num()? as f32,
                "max_particles" => s.max_particles = num()? as usize,
                "volume" => s.volume = num()? as f32,
                "lit_model_light" => s.lit_model_light = num()? as f32,
                "lit_model_scale" => s.lit_model_scale = num()? as f32,
                other => return Err(format!("unknown setting effects.{other}")),
            }
        }
        Ok(s)
    }
}

/// What effects did, for the log and the self-tests.
#[derive(Resource, Debug, Clone, Default)]
pub struct EffectStats {
    /// Effects started, by class name, in order.
    pub spawned: Vec<String>,
    /// Effects that ran to the end and were removed.
    pub finished: usize,
    /// Most particles alive at once.
    pub peak_particles: usize,
    /// Squares drawn in the last frame, and most in one frame.
    pub sprites_drawn: usize,
    pub peak_sprites: usize,
    /// Debris models drawn, most at once.
    pub peak_meshes: usize,
    /// Models drawn last frame, and the highest one (Unreal Z), for the
    /// self-tests (the gunships fly high over geo_01a).
    pub meshes_drawn: usize,
    pub highest_mesh: Option<f32>,
    /// Sounds started, by name.
    pub sounds: Vec<String>,
    /// Effects, pictures, models or sounds that couldn't be read, with why.
    pub missing: Vec<String>,
}

/// How one emitter is drawn.
#[derive(Clone)]
enum Look {
    Sprites(Handle<StandardMaterial>),
    Meshes(Vec<(Handle<Mesh>, StandardMaterial)>),
    Nothing,
}

struct Loaded {
    def: Arc<EffectDef>,
    looks: Vec<Look>,
}

/// Effects and sounds read so far.
#[derive(Resource, Default)]
pub struct Effects {
    library: Option<Result<Library, String>>,
    effects: HashMap<String, Option<Arc<Loaded>>>,
    sounds: HashMap<String, Option<GameSound>>,
}

/// An effect running. Its particles are in Unreal units.
#[derive(Component)]
pub struct RunningEffect {
    loaded: Arc<Loaded>,
    /// Reported in an [`EffectEnded`] when it's over.
    id: Option<u64>,
    by: String,
    /// The entity carrying it, until that's gone.
    follow: Option<Entity>,
    /// Taken away at once when the carrier goes (see `SpawnEffect`).
    vanish_with_carrier: bool,
    /// Not stopped after a minute while its carrier is there.
    endless: bool,
    /// Its `AmbientSound`, looping while it is there.
    ambient: Option<Entity>,
    pub state: EffectState,
    /// One entity per emitter (sprites), or per particle (meshes).
    parts: Vec<Option<Entity>>,
    debris: Vec<Vec<Entity>>,
}

impl RunningEffect {
    pub fn name(&self) -> &str {
        &self.loaded.def.name
    }
}

/// The plugin. Needs `GamePaths`, `GameLog` and a renderer (it draws
/// nothing without one, but still runs the particles).
pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SpawnEffect>()
            .add_message::<EffectEnded>()
            .add_message::<TriggerEffect>()
            .add_message::<PreloadEffect>()
            .init_resource::<Effects>()
            .init_resource::<EffectStats>()
            .add_systems(Startup, read_settings)
            .add_systems(Update, preload)
            .add_systems(
                Update,
                (spawn, step)
                    .chain()
                    .in_set(GameSet::Simulate)
                    .run_if(rc_game_core::playing),
            )
            .add_systems(
                Update,
                draw.in_set(GameSet::View)
                    .run_if(in_state(GameState::InGame)),
            )
            .add_systems(OnExit(GameState::InGame), forget);
    }
}

fn read_settings(mut commands: Commands, paths: Option<Res<GamePaths>>, log: Option<Res<GameLog>>) {
    let mut s = EffectSettings::default();
    if let Some(path) = paths
        .as_ref()
        .and_then(|p| p.data_file("effects/effects.toml"))
    {
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| EffectSettings::from_toml(&t))
        {
            Ok(v) => s = v,
            Err(e) => {
                if let Some(l) = &log {
                    l.line(format!(
                        "effects: {}: {e}; using the built-in settings",
                        path.display()
                    ));
                }
            }
        }
    }
    commands.insert_resource(s);
}

fn forget(mut stats: ResMut<EffectStats>) {
    *stats = EffectStats::default();
}

fn say(log: &Option<Res<GameLog>>, text: String) {
    if let Some(l) = log {
        l.line(text);
    }
}

/// Unreal units to Bevy.
fn to_bevy(p: [f32; 3]) -> Vec3 {
    units::to_bevy(rc_movement::Vec3::new(p[0], p[1], p[2]))
}

fn to_unreal(p: Vec3) -> [f32; 3] {
    let v = units::to_unreal(p);
    [v.x, v.y, v.z]
}

/// A picture by its path (package first).
fn picture(mats: &mut Materials, path: &[String]) -> Result<Picture, String> {
    let (pkg_name, rest) = path.split_first().ok_or("empty path")?;
    let loaded = mats
        .package(pkg_name)
        .ok_or_else(|| format!("package {pkg_name} not found"))?;
    let k = find_export(&loaded.0, rest).ok_or_else(|| format!("{} not found", path.join(".")))?;
    mats.resolve(&loaded.0, &loaded.1, k as i32 + 1)
        .map_err(|e| format!("{}: {e:?}", path.join(".")))
}

/// A static mesh by its path: one Bevy mesh per section, in the model's own
/// units turned to Bevy's axes (not scaled), with its picture.
pub fn static_mesh(
    mats: &mut Materials,
    path: &[String],
) -> Result<Vec<(Mesh, Option<Picture>)>, String> {
    let (pkg_name, rest) = path.split_first().ok_or("empty path")?;
    let loaded = mats
        .package(pkg_name)
        .ok_or_else(|| format!("package {pkg_name} not found"))?;
    let (pkg, data) = (&loaded.0, &loaded.1);
    let k = find_export(pkg, rest).ok_or_else(|| format!("{} not found", path.join(".")))?;
    let mesh = rc_package::static_mesh::read_static_mesh(pkg, data, &pkg.exports[k])
        .map_err(|e| format!("{}: {e}", path.join(".")))?;
    let uvs = mesh.uv_sets.first().cloned().unwrap_or_default();
    let mut out = Vec::new();
    for (si, sec) in mesh.sections.iter().enumerate() {
        let first = sec.first_index as usize;
        let count = sec.num_faces as usize * 3;
        let Some(tris) = mesh.indices.get(first..first + count) else {
            continue;
        };
        let mut positions = vec![];
        let mut normals = vec![];
        let mut tex = vec![];
        let mut index = vec![];
        let mut remap = HashMap::new();
        for &vi in tris {
            let n = *remap.entry(vi).or_insert_with(|| {
                let v = mesh.vertices[vi as usize];
                // Unreal X, Y, Z to Bevy X, Z, Y (a mirror; drawn two-sided).
                positions.push([v.position[0], v.position[2], v.position[1]]);
                normals.push([v.normal[0], v.normal[2], v.normal[1]]);
                tex.push(uvs.get(vi as usize).copied().unwrap_or([0.0, 0.0]));
                positions.len() as u32 - 1
            });
            index.push(n);
        }
        let m = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, tex)
        .with_inserted_indices(Indices::U32(index));
        let pic = mats
            .resolve(pkg, data, mesh.materials.get(si).copied().unwrap_or(0))
            .ok();
        out.push((m, pic));
    }
    Ok(out)
}

fn alpha_mode(style: DrawStyle) -> AlphaMode {
    match style {
        DrawStyle::Regular => AlphaMode::Mask(0.5),
        DrawStyle::AlphaBlend => AlphaMode::Blend,
        DrawStyle::Modulated | DrawStyle::AlphaModulate | DrawStyle::Darken => AlphaMode::Multiply,
        DrawStyle::Translucent | DrawStyle::Brighten => AlphaMode::Add,
    }
}

#[allow(clippy::too_many_arguments)]
fn load_effect(
    effects: &mut Effects,
    name: &str,
    paths: &GamePaths,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    stats: &mut EffectStats,
    log: &Option<Res<GameLog>>,
    lit_light: f32,
) -> Option<Arc<Loaded>> {
    let key = name.to_ascii_lowercase();
    if let Some(e) = effects.effects.get(&key) {
        return e.clone();
    }
    let t = std::time::Instant::now();
    let lib = effects
        .library
        .get_or_insert_with(|| Library::open(&paths.game_data()));
    let def = match lib {
        Ok(l) => l.effect(name),
        Err(e) => Err(e.clone()),
    };
    let def = match def {
        Ok(d) => d,
        Err(e) => {
            say(log, format!("effects: effect {name} can't be read: {e}"));
            stats.missing.push(format!("{name}: {e}"));
            effects.effects.insert(key, None);
            return None;
        }
    };
    let mut mats = Materials::new(&paths.game_data());
    let mut looks = Vec::new();
    let mut notes = Vec::new();
    for e in &def.emitters {
        let look = match &e.kind {
            EmitterKind::Sprite | EmitterKind::Spark => {
                let tex = e.texture.as_ref().map(|p| picture(&mut mats, p));
                let tex = match tex {
                    Some(Ok(p)) => Some(images.add(to_image(&p))),
                    Some(Err(err)) => {
                        notes.push(format!("{}: picture {err}", e.name));
                        None
                    }
                    None => None,
                };
                Look::Sprites(materials.add(StandardMaterial {
                    base_color: Color::WHITE,
                    base_color_texture: tex,
                    unlit: true,
                    alpha_mode: alpha_mode(e.draw_style),
                    double_sided: true,
                    cull_mode: None,
                    fog_enabled: false,
                    ..default()
                }))
            }
            EmitterKind::Mesh(Some(path)) => match static_mesh(&mut mats, path) {
                Ok(parts) => Look::Meshes(
                    parts
                        .into_iter()
                        .map(|(m, pic)| {
                            let texture = pic.map(|p| images.add(to_image(&p)));
                            let alpha_mode = if e.mesh_blend {
                                // UseMeshBlendMode: as the model's own
                                // materials draw it: solid.
                                AlphaMode::Opaque
                            } else {
                                AlphaMode::Blend
                            };
                            let material = if def.lit {
                                // Lit like the droids: its own colour as a
                                // glow that follows the level's light
                                // (LitByPlace, set when it's drawn).
                                let a = lit_light;
                                StandardMaterial {
                                    base_color: Color::WHITE,
                                    emissive: LinearRgba::rgb(a, a, a),
                                    emissive_texture: texture.clone(),
                                    base_color_texture: texture,
                                    perceptual_roughness: 0.7,
                                    alpha_mode,
                                    double_sided: true,
                                    cull_mode: None,
                                    ..default()
                                }
                            } else {
                                StandardMaterial {
                                    base_color: Color::WHITE,
                                    base_color_texture: texture,
                                    unlit: true,
                                    alpha_mode,
                                    double_sided: true,
                                    cull_mode: None,
                                    ..default()
                                }
                            };
                            (meshes.add(m), material)
                        })
                        .collect(),
                ),
                Err(err) => {
                    notes.push(format!("{}: model {err}", e.name));
                    Look::Nothing
                }
            },
            EmitterKind::Mesh(None) => Look::Nothing,
            EmitterKind::Other(k) => {
                notes.push(format!("{}: a {k}, not drawn yet", e.name));
                Look::Nothing
            }
        };
        looks.push(look);
    }
    let particles: u32 = def.emitters.iter().map(|e| e.max_particles).sum();
    say(
        log,
        format!(
            "effects: read {} ({} emitters, up to {particles} particles, sound {}) in {:.0} ms",
            def.name,
            def.emitters.len(),
            def.sound
                .as_ref()
                .map(|s| s.join("."))
                .unwrap_or_else(|| "none".into()),
            t.elapsed().as_secs_f64() * 1000.0
        ),
    );
    for n in &notes {
        say(log, format!("effects:   {n}"));
        stats.missing.push(format!("{}: {n}", def.name));
    }
    let loaded = Arc::new(Loaded {
        def: Arc::new(def),
        looks,
    });
    effects.effects.insert(key, Some(loaded.clone()));
    Some(loaded)
}

fn load_sound(
    effects: &mut Effects,
    path: &[String],
    paths: &GamePaths,
    audio: &mut Assets<AudioSource>,
    stats: &mut EffectStats,
    log: &Option<Res<GameLog>>,
) -> Option<GameSound> {
    let (Some(package), Some(name)) = (path.first(), path.last()) else {
        return None;
    };
    let key = name.to_ascii_lowercase();
    if let Some(s) = effects.sounds.get(&key) {
        return s.clone();
    }
    let (mut found, notes) = rc_combat::read_sounds(paths, package, &[name.clone()], audio);
    for n in notes {
        say(log, format!("effects: {n}"));
        stats.missing.push(n);
    }
    let s = found
        .drain()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v);
    effects.sounds.insert(key, s.clone());
    s
}

#[allow(clippy::too_many_arguments)]
fn preload(
    mut asks: MessageReader<PreloadEffect>,
    mut effects: ResMut<Effects>,
    paths: Option<Res<GamePaths>>,
    mut stats: ResMut<EffectStats>,
    images: Option<ResMut<Assets<Image>>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    audio: Option<ResMut<Assets<AudioSource>>>,
    settings: Option<Res<EffectSettings>>,
    log: Option<Res<GameLog>>,
) {
    let lit_light = settings.map_or(0.5, |s| s.lit_model_light);
    let (Some(paths), Some(mut images), Some(mut meshes), Some(mut materials)) =
        (paths, images, meshes, materials)
    else {
        asks.clear();
        return;
    };
    let mut audio = audio;
    for a in asks.read() {
        if let Some(e) = &a.effect {
            let loaded = load_effect(
                &mut effects,
                e,
                &paths,
                &mut images,
                &mut meshes,
                &mut materials,
                &mut stats,
                &log,
                lit_light,
            );
            if let (Some(l), Some(au)) = (loaded, audio.as_deref_mut()) {
                for s in l.def.sound.iter().chain(&l.def.ambient_sound) {
                    load_sound(&mut effects, s, &paths, au, &mut stats, &log);
                }
            }
        }
        if let (Some(s), Some(au)) = (&a.sound, audio.as_deref_mut()) {
            load_sound(&mut effects, s, &paths, au, &mut stats, &log);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    mut commands: Commands,
    mut asks: MessageReader<SpawnEffect>,
    mut effects: ResMut<Effects>,
    settings: Option<Res<EffectSettings>>,
    paths: Option<Res<GamePaths>>,
    mut stats: ResMut<EffectStats>,
    images: Option<ResMut<Assets<Image>>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    audio: Option<ResMut<Assets<AudioSource>>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    mut ended: MessageWriter<EffectEnded>,
    log: Option<Res<GameLog>>,
) {
    let settings = settings.map(|s| s.clone()).unwrap_or_default();
    let (Some(paths), Some(mut images), Some(mut meshes), Some(mut materials)) =
        (paths, images, meshes, materials)
    else {
        asks.clear();
        return;
    };
    let mut audio = audio;
    let listener = camera.iter().next().map(|g| g.translation());
    for a in asks.read() {
        let loaded = if settings.enabled {
            load_effect(
                &mut effects,
                &a.effect,
                &paths,
                &mut images,
                &mut meshes,
                &mut materials,
                &mut stats,
                &log,
                settings.lit_model_light,
            )
        } else {
            None
        };
        // Sounds: the effect's own, then the sender's.
        let mut sounds: Vec<Vec<String>> = Vec::new();
        if let Some(s) = loaded.as_ref().and_then(|l| l.def.sound.clone()) {
            sounds.push(s);
        }
        if let Some(s) = &a.sound {
            sounds.push(s.clone());
        }
        let dist = listener.map_or(0.0, |l| (l - a.at).length() * units::UU_PER_UNIT);
        // How loud with distance, and whether it is heard, is the sound
        // engine's: an emitter's sounds come from its place and are played
        // straight, without the hearing check (`AEmitter`, flags 0x400).
        let vol = settings.volume;
        let place = SoundAt::place(a.at);
        if let Some(au) = audio.as_deref_mut() {
            for s in &sounds {
                let Some(h) = load_sound(&mut effects, s, &paths, au, &mut stats, &log)
                    .and_then(|g| g.pick())
                else {
                    continue;
                };
                let name = s.last().cloned().unwrap_or_default();
                commands.spawn((
                    AudioPlayer(h),
                    PlaybackSettings::DESPAWN.with_volume(Volume::Linear(vol)),
                    place,
                    SoundBy {
                        skip_hear_check: true,
                        ..Default::default()
                    },
                    DespawnOnExit(GameState::InGame),
                ));
                stats.sounds.push(name);
            }
        }
        // Its ambient sound loops for as long as it is there (the actor's
        // `AmbientSound`), at the same loudness rule.
        let mut ambient = None;
        if let (false, Some(s), Some(au)) = (
            a.skip_ambient_sound,
            loaded.as_ref().and_then(|l| l.def.ambient_sound.clone()),
            audio.as_deref_mut(),
        ) {
            if let Some(h) =
                load_sound(&mut effects, &s, &paths, au, &mut stats, &log).and_then(|g| g.pick())
            {
                let name = s.last().cloned().unwrap_or_default();
                // The actor's `AmbientSound`: started and stopped with the
                // listener's distance like every actor's loop
                // (`UpdateAmbientList`, `rc_scripts_plugin::ambient`).
                let walls = loaded.as_ref().is_none_or(|l| l.def.sound_occlusion != 1);
                ambient = Some(
                    commands
                        .spawn((
                            rc_game_core::ActorAmbient {
                                sound: h,
                                name: name.clone(),
                                walls,
                                playing: None,
                            },
                            place.1,
                            DespawnOnExit(GameState::InGame),
                        ))
                        .id(),
                );
                say(&log, format!("effects: {} hums {name} (looping)", a.effect));
                stats.sounds.push(name);
            }
        }
        let origin = to_unreal(a.at);
        say(
            &log,
            format!(
                "effects: {} at ({:.0}, {:.0}, {:.0}) for {}; sounds {:?} at loudness {vol:.2} ({dist:.0} uu away){}",
                a.effect,
                origin[0],
                origin[1],
                origin[2],
                a.by,
                sounds.iter().filter_map(|s| s.last()).collect::<Vec<_>>(),
                if loaded.is_none() { "; no particles (see above)" } else { "" }
            ),
        );
        let Some(loaded) = loaded else {
            if let Some(id) = a.id {
                ended.write(EffectEnded {
                    id,
                    by: a.by.clone(),
                });
            }
            if let Some(e) = ambient {
                commands.entity(e).despawn();
            }
            continue;
        };
        stats.spawned.push(loaded.def.name.clone());
        let seed = 0x9E37_79B9 ^ (stats.spawned.len() as u64 * 7919) ^ origin[0].to_bits() as u64;
        let mut state =
            EffectState::new(&loaded.def, origin, seed).placed(a.rotation, a.draw_scale);
        if let Some(axes) = a.axes {
            state.axes = axes;
        }
        state.floor = a.floor.map(|y| to_unreal(Vec3::new(a.at.x, y, a.at.z))[2]);
        let parts = loaded
            .looks
            .iter()
            .map(|l| match l {
                Look::Sprites(m) => Some(
                    commands
                        .spawn((
                            Mesh3d(meshes.add(empty_mesh())),
                            MeshMaterial3d(m.clone()),
                            Transform::IDENTITY,
                            NoFrustumCulling,
                            DespawnOnExit(GameState::InGame),
                        ))
                        .id(),
                ),
                _ => None,
            })
            .collect();
        let n = loaded.looks.len();
        commands.spawn((
            RunningEffect {
                loaded,
                id: a.id,
                by: a.by.clone(),
                follow: a.follow,
                vanish_with_carrier: a.vanish_with_carrier,
                endless: a.endless && a.follow.is_some(),
                ambient,
                state,
                parts,
                debris: vec![Vec::new(); n],
            },
            Transform::IDENTITY,
            Visibility::default(),
            DespawnOnExit(GameState::InGame),
        ));
    }
}

fn empty_mesh() -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new())
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, Vec::<[f32; 2]>::new())
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, Vec::<[f32; 4]>::new())
    .with_inserted_indices(Indices::U32(Vec::new()))
}

/// Move the particles on, and remove effects that are over.
fn step(
    mut commands: Commands,
    time: Res<Time>,
    mut running: Query<(Entity, &mut RunningEffect)>,
    carriers: Query<&GlobalTransform>,
    mut stats: ResMut<EffectStats>,
    mut ended: MessageWriter<EffectEnded>,
    mut triggers: MessageReader<TriggerEffect>,
    log: Option<Res<GameLog>>,
) {
    let dt = time.delta_secs().min(0.1);
    let triggered: Vec<Entity> = triggers.read().map(|t| t.carrier).collect();
    let mut live = 0;
    for (e, mut r) in &mut running {
        let def = r.loaded.def.clone();
        if let Some(c) = r.follow {
            for _ in triggered.iter().filter(|t| **t == c) {
                r.state.trigger(&def);
            }
        }
        let mut vanish = false;
        if let Some(c) = r.follow {
            match carriers.get(c) {
                Ok(g) => r.state.move_to(&def, to_unreal(g.translation())),
                Err(_) if r.vanish_with_carrier => {
                    // `Destroy`: gone at once, with its sound.
                    vanish = true;
                }
                Err(_) => {
                    // `Emitter.FadeOut`, as a projectile's `Destroyed` does.
                    r.follow = None;
                    r.state.fade_out();
                }
            }
        }
        if !vanish {
            r.state.step(&def, dt);
            live += r.state.live();
        }
        // Safety: an effect that loops forever ends after a minute. A
        // placed emitter stays while its carrier is there, as the map's
        // actor does, even with every emitter switched off (a trigger can
        // switch them on again).
        let endless = r.endless && r.follow.is_some();
        if vanish || (!endless && (r.state.done(&def) || r.state.age > 60.0)) {
            say(
                &log,
                format!(
                    "effects: {} {} after {:.2} s",
                    def.name,
                    if vanish {
                        "taken away with its carrier"
                    } else {
                        "over"
                    },
                    r.state.age
                ),
            );
            if let Some(a) = r.ambient.take() {
                commands.entity(a).try_despawn();
            }
            stats.finished += 1;
            for p in r.parts.iter().flatten() {
                commands.entity(*p).try_despawn();
            }
            for d in r.debris.iter().flatten() {
                commands.entity(*d).try_despawn();
            }
            if let Some(id) = r.id {
                ended.write(EffectEnded {
                    id,
                    by: r.by.clone(),
                });
            }
            commands.entity(e).despawn();
        }
    }
    stats.peak_particles = stats.peak_particles.max(live);
}

/// Squares facing the camera, and debris models, where the particles are.
#[allow(clippy::too_many_arguments)]
fn draw(
    mut commands: Commands,
    mut running: Query<&mut RunningEffect>,
    parts: Query<&Mesh3d>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    player_camera: Query<&GlobalTransform, With<PlayerCamera>>,
    cameras: Query<(&GlobalTransform, &Camera), With<Camera3d>>,
    settings: Option<Res<EffectSettings>>,
    mut stats: ResMut<EffectStats>,
) {
    let Some(meshes) = meshes.as_deref_mut() else {
        return;
    };
    let cam = player_camera
        .iter()
        .next()
        .copied()
        .or_else(|| cameras.iter().find(|(_, c)| c.is_active).map(|(g, _)| *g));
    let Some(cam) = cam else { return };
    let (right, up) = (cam.right().as_vec3(), cam.up().as_vec3());
    let size = settings.as_ref().map_or(1.0, |s| s.size) / units::UU_PER_UNIT;
    let budget = settings.as_ref().map_or(4000, |s| s.max_particles);
    let mut drawn = 0;
    let mut debris_drawn = 0;
    let mut highest: Option<f32> = None;
    let (lit_light, lit_scale) = settings
        .as_ref()
        .map_or((0.5, 1.1), |s| (s.lit_model_light, s.lit_model_scale));
    for mut r in &mut running {
        let r = &mut *r;
        let def = r.loaded.def.clone();
        for (i, look) in r.loaded.looks.iter().enumerate() {
            let emitter = &def.emitters[i];
            let sprites = r.state.sprites(&def, i);
            match look {
                Look::Sprites(_) => {
                    let Some(part) = r.parts[i] else { continue };
                    let Ok(m) = parts.get(part) else { continue };
                    let Some(mut mesh) = meshes.get_mut(&m.0) else {
                        continue;
                    };
                    let additive = matches!(
                        emitter.draw_style,
                        DrawStyle::Translucent | DrawStyle::Brighten
                    );
                    let fu = emitter.frames_u.max(1);
                    let fv = emitter.frames_v.max(1);
                    let mut pos = Vec::with_capacity(sprites.len() * 4);
                    let mut uv = Vec::with_capacity(sprites.len() * 4);
                    let mut col = Vec::with_capacity(sprites.len() * 4);
                    let mut idx = Vec::with_capacity(sprites.len() * 6);
                    for s in &sprites {
                        if drawn >= budget {
                            break;
                        }
                        drawn += 1;
                        let c = to_bevy(s.position);
                        let a = s.spin * std::f32::consts::TAU;
                        let (sa, ca) = a.sin_cos();
                        let r2 = (right * ca + up * sa) * s.half_size[0] * size;
                        let u2 = (up * ca - right * sa) * s.half_size[1] * size;
                        let base = pos.len() as u32;
                        for (k, (dx, dy)) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                            .into_iter()
                            .enumerate()
                        {
                            pos.push((c + r2 * dx + u2 * dy).to_array());
                            let fx = (s.frame % fu) as f32;
                            let fy = ((s.frame / fu) % fv) as f32;
                            let cu = [0.0, 1.0, 1.0, 0.0][k];
                            let cv = [1.0, 1.0, 0.0, 0.0][k];
                            uv.push([(fx + cu) / fu as f32, (fy + cv) / fv as f32]);
                        }
                        // Colours are stored as the game shows them; the
                        // GPU wants them linear.
                        let lin =
                            Color::srgba(s.colour[0], s.colour[1], s.colour[2], 1.0).to_linear();
                        let alpha = if additive {
                            // Adding light: the fades dim it; the colour
                            // scale's alpha (often 0) is not used.
                            s.fade
                        } else {
                            s.colour[3] * s.fade
                        };
                        for _ in 0..4 {
                            col.push([lin.red, lin.green, lin.blue, alpha]);
                        }
                        idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
                    }
                    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
                    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
                    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
                    mesh.insert_indices(Indices::U32(idx));
                }
                Look::Meshes(models) => {
                    let Some(materials) = materials.as_deref_mut() else {
                        continue;
                    };
                    let have = &mut r.debris[i];
                    let lit = def.lit;
                    // One entity per live particle, with the model's parts
                    // as children.
                    while have.len() < sprites.len() {
                        let mut e = commands.spawn((
                            Transform::default(),
                            Visibility::default(),
                            DespawnOnExit(GameState::InGame),
                        ));
                        if lit {
                            // Lit by the level's light where it is, as the
                            // droids are.
                            e.insert(rc_game_core::LitByPlace {
                                target: rc_game_core::LightTarget::Emissive,
                                own: 0.0,
                                scale: lit_scale,
                                height: 0.0,
                                made_at: lit_light.max(0.0).powf(1.0 / 2.2),
                            });
                        }
                        let e = e
                            .with_children(|c| {
                                for (m, mat) in models {
                                    c.spawn((
                                        Mesh3d(m.clone()),
                                        MeshMaterial3d(materials.add(mat.clone())),
                                    ));
                                }
                            })
                            .id();
                        have.push(e);
                    }
                    while have.len() > sprites.len() {
                        if let Some(e) = have.pop() {
                            commands.entity(e).despawn();
                        }
                    }
                    // A relative emitter's models are turned with the
                    // effect (Unreal axes to Bevy's: swap Y and Z on both
                    // sides, which keeps it a rotation).
                    let turn = if emitter.coordinate_system == CoordinateSystem::Relative {
                        let ax = r.state.axes;
                        let b = |v: [f32; 3]| Vec3::new(v[0], v[2], v[1]);
                        Quat::from_mat3(&Mat3::from_cols(b(ax[0]), b(ax[2]), b(ax[1])))
                    } else {
                        Quat::IDENTITY
                    };
                    for (s, e) in sprites.iter().zip(have.iter()) {
                        debris_drawn += 1;
                        highest = Some(highest.map_or(s.position[2], |h| h.max(s.position[2])));
                        let a = s.spin * std::f32::consts::TAU;
                        commands.entity(*e).insert(
                            Transform::from_translation(to_bevy(s.position))
                                .with_rotation(
                                    turn * Quat::from_euler(EulerRot::XYZ, a, a * 0.7, a * 0.3),
                                )
                                .with_scale(Vec3::splat(s.half_size[0] / units::UU_PER_UNIT)),
                        );
                    }
                }
                Look::Nothing => {}
            }
        }
    }
    stats.sprites_drawn = drawn;
    stats.peak_sprites = stats.peak_sprites.max(drawn);
    stats.peak_meshes = stats.peak_meshes.max(debris_drawn);
    stats.meshes_drawn = debris_drawn;
    if let Some(h) = highest {
        stats.highest_mesh = Some(stats.highest_mesh.map_or(h, |was| was.max(h)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_read_and_reject_typos() {
        let s = EffectSettings::from_toml("[effects]\nsize = 1.5\nenabled = false\n").unwrap();
        assert_eq!(s.size, 1.5);
        assert!(!s.enabled);
        assert!(EffectSettings::from_toml("[effects]\nsise = 2\n").is_err());
        assert!(EffectSettings::from_toml("").unwrap().enabled);
    }

    #[test]
    fn styles_blend_the_right_way() {
        assert_eq!(alpha_mode(DrawStyle::Brighten), AlphaMode::Add);
        assert_eq!(alpha_mode(DrawStyle::AlphaBlend), AlphaMode::Blend);
        assert_eq!(alpha_mode(DrawStyle::Modulated), AlphaMode::Multiply);
    }
}
