//! Droid dispensers in the game: the machine drawn from the game's own
//! model (`GameData/Animations/Dispensers.ukx`), its animations, skins,
//! sounds and effects, the droids riding out of it and unfolding, and
//! hits on it. The rules (when it makes droids, its damage steps) are in
//! `rc_ai::battle::dispense`; this file shows what they decide and reads
//! the animation numbers they need from the game files.
//!
//! - **Model and animations**: read with `rc_model`. The machine plays
//!   its intro (`entranceSky`, ...), then for each droid the slot's
//!   `Dispense<n>`, and `Destroyed` at the end; each plays once and holds
//!   its last frame (`PlayAnim`). It stands where the map puts its
//!   collision cylinder, its model's origin at the cylinder's bottom
//!   (`bAlignBottom`), turned by the map's yaw.
//! - **Skins**: each look step that names a material (`WarmUpInfo`,
//!   `OnlineInfo`, `ShutdownInfo`, `DamagedInfo`) swaps the model's first
//!   material for it (`Skins[0]`), as `MakeTransition` does.
//! - **Effects**: each step's `TransitionEffect` (an emitter class from
//!   `Properties/CTEffects.u`) appears at the dispenser's centre plus the
//!   step's offset, and the step before's effect goes away (the original
//!   keeps one `CurrentEffect`).
//! - **Sounds**: the `AnimNotify_Sound`s in its animations, from the
//!   machine.
//! - **Droids**: while on its slot a droid loops `DispBreathe`; once let go
//!   it plays `DispUnfold` once, the battle moving it by that animation's
//!   root bone; it can't be hit until it has unfolded.
//! - **Hits**: it has `Health` (`MaxHealth`) and its damage table
//!   (`DV_DroidDispenser`: blaster bolts do nothing, explosions half), so
//!   the weapons part hurts it like a character; each change of health
//!   goes to the battle.
//!
//! Not done yet: dispensers that ride a mover (`AttachTag`, ras_04a/b),
//! the effects the intro animations start (`AnimNotify_Effect`), the
//! legs' blockers (droideka dispensers only), and the machine being solid
//! for walking.

use crate::scripted::ScriptedAnim;
use crate::{Droid, Fight, Looks, PART};
use bevy::asset::RenderAssetUsages;
use bevy::audio::{PlaybackSettings, Volume};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rc_ai::{BattleEvent, BoneSample, DispenserMotion, DispenserStep, SlotMotion, Unfold};
use rc_combat::{Body, DamageTable, Dead, GameSound, Health, Hittable, Team};
use rc_effects_plugin::SpawnEffect;
use rc_game_core::units::to_bevy;
use rc_game_core::{GameLog, GamePaths, GameState, PlayerCamera, RaiseEvent, SoundAt};
use rc_model::{Model, ModelSpec};
use rc_movement::Vec3 as UVec3;
use std::collections::HashMap;
use std::sync::Arc;

/// Light the model always gets, as for the animated props (bacta
/// dispensers): the level's light is baked into its surfaces and the
/// scene has no ambient light. A guess (0 to 1), the same as theirs.
const AMBIENT: f32 = 0.5;

/// Full loudness out to this distance (Unreal units), then fading, as for
/// the animated props. A guess.
const SOUND_REACH: f32 = 1000.0;

/// The model's package.
pub const PACKAGE: &str = "GameData/Animations/Dispensers.ukx";

/// One drawn dispenser.
#[derive(Component)]
pub struct DispenserBody {
    /// Its spawner in the battle.
    pub spawner: usize,
    /// The map's actor name (`BDDispenser0`).
    pub name: String,
    pub model: Arc<Model>,
    meshes: Vec<Handle<Mesh>>,
    /// The entities drawing each model section.
    parts: Vec<Entity>,
    /// One material per section for each skin: `""` is the model's own.
    skins: HashMap<String, Vec<Handle<StandardMaterial>>>,
    /// The skin shown.
    pub skin: String,
    /// The animation playing, how far in (frames), and whether it loops.
    pub playing: Option<(String, f32, bool)>,
    /// The last animation played (it holds its last frame).
    pub last: Option<String>,
    /// Every animation started, in order (for the log and self-tests).
    pub played: Vec<String>,
    /// Every sound its animations played.
    pub heard: Vec<String>,
    /// The entity carrying its current effect (`CurrentEffect`), and the
    /// effect's offset from its centre (Unreal units).
    effect: Option<Entity>,
    effect_offset: UVec3,
    /// Its collision cylinder's half height (its model's origin is the
    /// cylinder's bottom).
    half: f32,
    last_health: f32,
}

/// Sounds the dispensers' animations play, by `package.name` lower case.
#[derive(Resource, Default)]
pub struct DispenserSounds(pub HashMap<String, GameSound>);

/// What the dispensers did, for the log and the self-tests.
#[derive(Resource, Default, Debug, Clone)]
pub struct DispenserStats {
    /// Dispensers drawn, and their models.
    pub drawn: usize,
    pub models: Vec<String>,
    /// Why some couldn't be drawn.
    pub failures: Vec<String>,
    /// Droids that came out (were let go), and finished unfolding.
    pub released: u32,
    pub unfolded: u32,
    /// Dispensers destroyed.
    pub destroyed: u32,
    /// Effects started: `dispenser effect`.
    pub effects: Vec<String>,
}

fn v(p: [f32; 3]) -> UVec3 {
    UVec3::new(p[0], p[1], p[2])
}

/// The animations to read for a dispenser: its idle, its slots', its
/// intro, and every animation its look steps name.
pub fn animations(d: &rc_ai::Dispenser) -> Vec<String> {
    let mut out = vec!["ActionBreathe".to_string()];
    for n in 1..=d.num_dispensers {
        out.push(format!("Dispense{n}"));
    }
    out.extend(d.intro_anim.clone());
    for t in [
        &d.warm_up,
        &d.online,
        &d.slightly_damaged,
        &d.heavily_damaged,
        &d.shutdown,
        &d.damaged,
    ] {
        out.extend(t.anim.clone());
    }
    out.dedup();
    out
}

/// A dispenser's animation numbers from its model, for `rc_ai`: its intro
/// length, and for each slot its animation's length, when it lets the
/// droid go (`AnimNotify_ReleaseDroid`) and where the slot's bone goes
/// until then.
pub fn motion(
    model: &Model,
    d: &rc_ai::Dispenser,
    unfold: &Unfold,
) -> (DispenserMotion, Vec<String>) {
    let mut notes = Vec::new();
    let seconds = |name: &str| model.clip(name).map(|c| c.num_frames / c.rate.max(1.0));
    let intro = d.intro_anim.as_deref().unwrap_or("ActionBreathe");
    let intro_seconds = match seconds(intro) {
        Some(s) => s,
        None => {
            // `HasAnim` fails: the original plays `ActionBreathe`.
            notes.push(format!(
                "its intro {intro} isn't in the model: ActionBreathe instead"
            ));
            seconds("ActionBreathe").unwrap_or(0.0)
        }
    };
    // `PHYS_RootMotion`: the intro's root bone (`Move`) carries the whole
    // machine.
    let intro_key = if model.clip(intro).is_some() {
        intro
    } else {
        "ActionBreathe"
    };
    let mut intro_travel = Vec::new();
    if let Some(clip) = model.clip(intro_key) {
        let start = v(model.root_offset(intro_key, 0.0));
        let rate = clip.rate.max(1.0);
        let mut f = 0.0;
        while f <= clip.num_frames - 1.0 {
            intro_travel.push((f / rate, v(model.root_offset(intro_key, f)) - start));
            f += 1.0;
        }
        // Held at its last frame after the end (`PlayAnim`).
        if let Some(&(_, last)) = intro_travel.last() {
            intro_travel.push((clip.num_frames / rate, last));
        }
    }
    if let Some(&(_, end)) = intro_travel.last() {
        if end.length() > 1.0 {
            notes.push(format!(
                "its intro {intro_key} carries it ({:.0}, {:.0}, {:.0}) in its own axes (its Move bone; PHYS_RootMotion)",
                end.x, end.y, end.z
            ));
        }
    }
    let mut slots = Vec::new();
    for n in 1..=d.num_dispensers {
        let anim = format!("Dispense{n}");
        let bone = format!("droidAttach{n}");
        let Some(clip) = model.clip(&anim) else {
            notes.push(format!("no {anim}: slot {n} lets its droid go at once"));
            slots.push(SlotMotion::default());
            continue;
        };
        let release = model
            .notify_classes
            .get(&anim)
            .and_then(|l| {
                l.iter()
                    .find(|(_, c)| c.eq_ignore_ascii_case("AnimNotify_ReleaseDroid"))
            })
            .map(|(t, _)| *t);
        let release = match release {
            Some(r) => r,
            None => {
                notes.push(format!(
                    "{anim} has no AnimNotify_ReleaseDroid: its droid is let go at the end"
                ));
                1.0
            }
        };
        let rate = clip.rate.max(1.0);
        let last = (clip.num_frames * release)
            .ceil()
            .min(clip.num_frames - 1.0)
            .max(0.0);
        let mut path = Vec::new();
        let mut f = 0.0;
        while f <= last {
            // As the game places bones: the move bone's travel moves the
            // machine, not its other bones.
            if let Some((at, axes)) = model.bone_from_origin_in_game(&anim, f, &bone) {
                path.push(BoneSample {
                    seconds: f / rate,
                    at: v(at),
                    x: v(axes[0]),
                    z: v(axes[2]),
                });
            }
            f += 1.0;
        }
        if path.is_empty() {
            notes.push(format!(
                "no bone {bone}: slot {n}'s droid appears at the dispenser"
            ));
        }
        let out = path.last().map(|b| b.at).unwrap_or(UVec3::ZERO);
        notes.push(format!(
            "slot {n}: {anim} {:.2} s, droid let go at {:.2} ({:.2} s) at ({:.0}, {:.0}, {:.0}) from the model's origin",
            clip.num_frames / rate,
            release,
            release * clip.num_frames / rate,
            out.x,
            out.y,
            out.z
        ));
        slots.push(SlotMotion {
            seconds: clip.num_frames / rate,
            release,
            path,
        });
    }
    (
        DispenserMotion {
            intro_seconds,
            intro_travel,
            slots,
            unfold: unfold.clone(),
        },
        notes,
    )
}

/// Materials for each section of `model`.
fn materials(world: &mut World, model: &Model) -> Vec<Handle<StandardMaterial>> {
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

/// The `DispUnfold` of each enemy type the dispensers make, from its look.
fn unfold_of(looks: &Looks, kind: &str) -> Option<(Unfold, String)> {
    unfold_from(&looks.by_type.get(kind)?.look, kind)
}

/// The `DispUnfold` in an enemy type's look: how long it takes and where
/// its root bone takes the droid, with a line for the log.
pub fn unfold_from(look: &crate::look::DroidLook, kind: &str) -> Option<(Unfold, String)> {
    let (seconds, travel) = look.root_travel("DispUnfold")?;
    let end = travel.last().map(|t| t.1).unwrap_or([0.0; 3]);
    Some((
        Unfold {
            seconds,
            travel: travel.into_iter().map(|(t, p)| (t, v(p))).collect(),
        },
        format!(
            "{kind} unfolds in {seconds:.2} s (DispUnfold), its root bone moving it ({:.0}, {:.0}, {:.0}) in its own axes",
            end[0], end[1], end[2]
        ),
    ))
}

/// While the level loads (after the fight is set up): read each
/// dispenser's model, give the battle its animation numbers, and draw it.
pub fn setup(world: &mut World) {
    if !world.contains_resource::<Fight>() {
        return;
    }
    let paths = world.resource::<GamePaths>().clone();
    let log = world.resource::<GameLog>().clone();
    let mut stats = DispenserStats::default();
    let list: Vec<(usize, rc_ai::Factory, String)> = {
        let fight = world.resource::<Fight>();
        fight
            .battle
            .spawners
            .iter()
            .enumerate()
            .filter(|(_, s)| s.factory.dispenser.is_some())
            .map(|(i, s)| (i, s.factory.clone(), s.params.kind.id.clone()))
            .collect()
    };
    if list.is_empty() {
        world.insert_resource(stats);
        return;
    }
    let mut models: HashMap<String, Option<Arc<Model>>> = HashMap::new();
    let mut skin_sets: HashMap<(String, String), Vec<Handle<StandardMaterial>>> = HashMap::new();
    for (i, f, kind) in list {
        let d = f.dispenser.clone().unwrap();
        let anims = animations(&d);
        let key = format!("{}:{}", d.mesh, anims.join(","));
        if !models.contains_key(&key) {
            let spec = ModelSpec {
                package: PACKAGE.into(),
                mesh: d.mesh.clone(),
                sequences: anims.iter().map(|a| (a.clone(), a.clone())).collect(),
                ..Default::default()
            };
            let loaded = match Model::load(&|rel| paths.game_file(rel), &spec) {
                Ok((m, notes)) => {
                    for n in notes {
                        log.line(format!("{PART}: dispenser model: {n}"));
                    }
                    load_sounds(
                        world,
                        &paths,
                        m.sounds.values().flatten().map(|x| &x.1),
                        &log,
                    );
                    Some(Arc::new(m))
                }
                Err(e) => {
                    stats.failures.push(format!("{}: {e}", d.mesh));
                    log.line(format!(
                        "{PART}: dispenser {}: model not read, not drawn: {e}",
                        f.name
                    ));
                    None
                }
            };
            models.insert(key.clone(), loaded);
        }
        let Some(model) = models.get(&key).cloned().flatten() else {
            continue;
        };
        // The sounds its looks (`TransitionSound`) and each droid it makes
        // (`DispenseSound`) play, when the map gives any (no class default
        // does).
        let own: Vec<String> = [
            &d.warm_up,
            &d.online,
            &d.slightly_damaged,
            &d.heavily_damaged,
            &d.shutdown,
            &d.damaged,
        ]
        .iter()
        .filter_map(|t| t.sound.clone())
        .chain(d.dispense_sound.clone())
        .collect();
        if !own.is_empty() {
            load_sounds(world, &paths, own.iter(), &log);
        }
        // The battle's numbers.
        let unfold = {
            let looks = world.resource::<Looks>();
            unfold_of(looks, &kind)
        };
        let unfold = match unfold {
            Some((u, note)) => {
                log.line(format!("{PART}: dispenser {}: {note}", f.name));
                u
            }
            None => {
                log.line(format!(
                    "{PART}: dispenser {}: {kind} has no DispUnfold in its look: it walks at once when let go",
                    f.name
                ));
                Unfold::default()
            }
        };
        let (m, notes) = motion(&model, &d, &unfold);
        for n in notes {
            log.line(format!("{PART}: dispenser {}: {n}", f.name));
        }
        log.line(format!(
            "{PART}: dispenser {} ({}, tag {}): makes {kind}, health {:.0}, {} slots, {} at the start then {} alive at once, every {:.1}{} s, intro {} ({:.2} s), warm-up {:.1} s{}{}{}",
            f.name,
            d.class,
            f.tag,
            d.max_health,
            d.num_dispensers,
            d.num_init_droids,
            f.max_alive,
            f.spawn_interval,
            if f.spawn_interval_max > f.spawn_interval {
                format!("-{:.1}", f.spawn_interval_max)
            } else {
                String::new()
            },
            d.intro_anim.as_deref().unwrap_or("none"),
            m.intro_seconds,
            d.warm_up_time,
            d.online_tag.as_ref().map(|t| format!(", then waits for {t}")).unwrap_or_default(),
            d.damaged.event.as_ref().map(|e| format!(", sends {e} when destroyed")).unwrap_or_default(),
            d.attach_tag.as_ref().map(|t| format!(", rides what is tagged or named {t}")).unwrap_or_default(),
        ));
        world
            .resource_mut::<Fight>()
            .battle
            .set_dispenser_motion(i, m);

        // Its skins: the model's own, and each step's material.
        let mut skins = HashMap::new();
        let own = skin_sets
            .entry((d.mesh.clone(), String::new()))
            .or_default()
            .clone();
        let own = if own.is_empty() {
            let m = materials(world, &model);
            skin_sets.insert((d.mesh.clone(), String::new()), m.clone());
            m
        } else {
            own
        };
        skins.insert(String::new(), own.clone());
        for t in [
            &d.warm_up,
            &d.online,
            &d.slightly_damaged,
            &d.heavily_damaged,
            &d.shutdown,
            &d.damaged,
        ] {
            let Some(mat) = &t.material else { continue };
            if skins.contains_key(mat) {
                continue;
            }
            let k = (d.mesh.clone(), mat.clone());
            if !skin_sets.contains_key(&k) {
                let spec = ModelSpec {
                    package: PACKAGE.into(),
                    mesh: d.mesh.clone(),
                    sequences: vec![("ActionBreathe".into(), "ActionBreathe".into())],
                    skins: vec![Some(mat.clone())],
                    ..Default::default()
                };
                match Model::load(&|rel| paths.game_file(rel), &spec) {
                    Ok((m2, _)) => {
                        let mats = materials(world, &m2);
                        let pic = m2
                            .sections
                            .first()
                            .and_then(|s| s.skin.as_ref())
                            .map(|k| k.texture.clone());
                        log.line(format!(
                            "{PART}: dispenser skin {mat}: {}",
                            pic.map(|p| format!("picture {p}"))
                                .unwrap_or("no picture found".into())
                        ));
                        skin_sets.insert(k.clone(), mats);
                    }
                    Err(e) => {
                        log.line(format!("{PART}: dispenser skin {mat} not read: {e}"));
                        skin_sets.insert(k.clone(), own.clone());
                    }
                }
            }
            skins.insert(mat.clone(), skin_sets[&k].clone());
        }

        // Draw it: posed at its idle's first frame, standing on its
        // cylinder's bottom, turned by its yaw.
        let origin = f.location - UVec3::new(0.0, 0.0, d.collision_height);
        let points = model.posed_rooted("ActionBreathe", 0.0);
        let normals = model.normals(&points);
        let mut pieces = Vec::new();
        for (s, mat) in model.sections.iter().zip(&own) {
            let (pos, nrm) = model.section_attributes(s, &points, &normals);
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, model.section_uvs(s));
            mesh.insert_indices(Indices::U32(s.indices.clone()));
            pieces.push((world.resource_mut::<Assets<Mesh>>().add(mesh), mat.clone()));
        }
        let meshes: Vec<Handle<Mesh>> = pieces.iter().map(|p| p.0.clone()).collect();
        let mut parts = Vec::new();
        let mut e = world.spawn((
            Name::new(format!("dispenser {}", f.name)),
            Transform::from_translation(to_bevy(origin))
                .with_rotation(Quat::from_rotation_y(-f.yaw)),
            if d.hidden {
                Visibility::Hidden
            } else {
                Visibility::default()
            },
            Health::new(d.max_health),
            Hittable {
                radius: d.collision_radius,
                half_height: d.collision_height,
                center_up: d.collision_height,
            },
            DamageTable(d.damage_table.clone()),
            Team::Separatist,
            Body::Droid,
            DespawnOnExit(GameState::InGame),
        ));
        e.with_children(|c| {
            for (mesh, mat) in pieces {
                parts.push(
                    c.spawn((Mesh3d(mesh), MeshMaterial3d(mat), NoFrustumCulling))
                        .id(),
                );
            }
        });
        e.insert(DispenserBody {
            spawner: i,
            name: f.name.clone(),
            model: model.clone(),
            meshes,
            parts,
            skins,
            skin: String::new(),
            playing: None,
            last: None,
            played: Vec::new(),
            heard: Vec::new(),
            effect: None,
            effect_offset: UVec3::ZERO,
            half: d.collision_height,
            last_health: d.max_health,
        });
        stats.drawn += 1;
        stats.models.push(format!("{} {}", f.name, d.mesh));
        log.line(format!(
            "{PART}: dispenser {} drawn at ({:.0}, {:.0}, {:.0}) facing {:.0} deg{}",
            f.name,
            origin.x,
            origin.y,
            origin.z,
            f.yaw.to_degrees(),
            if d.hidden {
                " (hidden, as the map says)"
            } else {
                ""
            }
        ));
    }
    world.insert_resource(stats);
}

/// Read these sounds (dotted paths), once each.
fn load_sounds<'a>(
    world: &mut World,
    paths: &GamePaths,
    wanted: impl Iterator<Item = &'a String>,
    log: &GameLog,
) {
    let mut by_package: HashMap<String, Vec<String>> = HashMap::new();
    {
        let have = world.get_resource_or_init::<DispenserSounds>();
        for path in wanted {
            let key = sound_key(path);
            if have.0.contains_key(&key) {
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
        for n in notes {
            log.line(format!("{PART}: dispenser sounds: {n}"));
        }
        log.line(format!(
            "{PART}: dispenser sounds: {} of {} found in {package}",
            found.len(),
            names.len()
        ));
        let mut have = world.resource_mut::<DispenserSounds>();
        for (name, sound) in found {
            have.0
                .insert(sound_key(&format!("{package}.{name}")), sound);
        }
    }
}

/// A sound's key: package and name, lower case, without the groups.
fn sound_key(path: &str) -> String {
    let package = path.split('.').next().unwrap_or(path);
    let name = path.rsplit('.').next().unwrap_or(path);
    format!("{package}.{name}").to_ascii_lowercase()
}

impl DispenserBody {
    fn play(&mut self, anim: &str, looping: bool) -> bool {
        if self.model.clip(anim).is_none() {
            return false;
        }
        self.playing = Some((anim.to_string(), 0.0, looping));
        self.played.push(anim.to_string());
        true
    }
}

/// Show what the dispensers did this frame (from the battle's events) and
/// what it means for the droids coming out of them.
#[allow(clippy::too_many_arguments)]
pub fn show_steps(
    mut commands: Commands,
    mut fight: ResMut<Fight>,
    mut bodies: Query<&mut DispenserBody>,
    mut droids: Query<(&mut Droid, Option<&mut Health>), Without<DispenserBody>>,
    looks: Res<Looks>,
    mut materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
    // (Optional: tests run the fight without the effects part.)
    mut effects: Option<MessageWriter<SpawnEffect>>,
    sounds: Option<Res<DispenserSounds>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    mut stats: ResMut<DispenserStats>,
    log: Res<GameLog>,
) {
    let fight = &mut *fight;
    let steps = std::mem::take(&mut fight.dispenser_steps);
    let now = fight.battle.time;
    for (spawner, step) in steps {
        let sp = &fight.battle.spawners[spawner];
        let name = sp.factory.name.clone();
        let Some(d) = sp.factory.dispenser.clone() else {
            continue;
        };
        let kind = sp.params.kind.id.clone();
        let body = bodies.iter_mut().find(|b| b.spawner == spawner);
        match step {
            DispenserStep::Intro { anim } => {
                let a = anim.unwrap_or_else(|| "ActionBreathe".into());
                log.line(format!(
                    "{PART}: t={now:.1}s dispenser {name} switched on: plays {a}"
                ));
                if let Some(mut b) = body {
                    if !b.play(&a, false) {
                        b.play("ActionBreathe", false);
                    }
                }
            }
            DispenserStep::Look { which } => {
                let t = match which {
                    "warm_up" => &d.warm_up,
                    "online" => &d.online,
                    "slightly_damaged" => &d.slightly_damaged,
                    "heavily_damaged" => &d.heavily_damaged,
                    "shutdown" => &d.shutdown,
                    _ => &d.damaged,
                };
                log.line(format!(
                    "{PART}: t={now:.1}s dispenser {name}: {which}{}{}{}{}",
                    t.material
                        .as_ref()
                        .map(|m| format!(", skin {m}"))
                        .unwrap_or_default(),
                    t.effect
                        .as_ref()
                        .map(|e| format!(", effect {e}"))
                        .unwrap_or_default(),
                    t.anim
                        .as_ref()
                        .map(|a| format!(", plays {a}"))
                        .unwrap_or_default(),
                    t.sound
                        .as_ref()
                        .map(|s| format!(", sound {s}"))
                        .unwrap_or_default(),
                ));
                // `MakeTransition`: `PlaySound(TransitionSound)`. (The
                // original's offline `Tick` also replays the warm-up's
                // sound every frame while it warms up; no map gives the
                // warm-up a sound, so that isn't copied.)
                if let Some(path) = &t.sound {
                    let at = to_bevy(fight.battle.dispenser_location(spawner));
                    let heard = play_sound(&sounds, &camera, &mut commands, path, at);
                    log.line(format!("{PART}: dispenser {name} sound {heard}"));
                }
                if which == "damaged" {
                    stats.destroyed += 1;
                }
                let Some(mut b) = body else { continue };
                if let Some(mat) = &t.material {
                    if let Some(set) = b.skins.get(mat).cloned() {
                        for (part, m) in b.parts.iter().zip(set) {
                            if let Ok(mut mm) = materials.get_mut(*part) {
                                mm.0 = m;
                            }
                        }
                        b.skin = mat.clone();
                    }
                }
                if let Some(e) = &t.effect {
                    // One effect at a time: the one before goes.
                    if let Some(old) = b.effect.take() {
                        commands.entity(old).despawn();
                    }
                    // `Spawn(..., Location + TransitionEffectOffset)`, then
                    // `SetBase(self)`: it moves with the machine.
                    let centre = fight.battle.dispenser_location(spawner);
                    let floor = to_bevy(centre - UVec3::new(0.0, 0.0, b.half)).y;
                    let at = centre + t.effect_offset;
                    let carrier = commands
                        .spawn((
                            Transform::from_translation(to_bevy(at)),
                            DespawnOnExit(GameState::InGame),
                        ))
                        .id();
                    b.effect = Some(carrier);
                    b.effect_offset = t.effect_offset;
                    if let Some(effects) = effects.as_mut() {
                        effects.write(SpawnEffect {
                            effect: e.clone(),
                            at: to_bevy(at),
                            floor: Some(floor),
                            sound: None,
                            by: format!("dispenser {name} ({which})"),
                            rotation: [0, 0, 0],
                            draw_scale: 1.0,
                            id: None,
                            follow: Some(carrier),
                            vanish_with_carrier: true,
                            axes: None,
                            // `CurrentEffect` stays until the next look replaces it.
                            endless: true,
                        });
                    }
                    stats.effects.push(format!("{name} {e}"));
                }
                if let Some(a) = &t.anim {
                    b.play(a, t.anim_loop);
                }
            }
            DispenserStep::Off => {
                log.line(format!(
                    "{PART}: t={now:.1}s dispenser {name} switched off by its event"
                ));
            }
            DispenserStep::Dispense { id, slot } => {
                let anim = format!("Dispense{}", slot + 1);
                log.line(format!(
                    "{PART}: t={now:.1}s dispenser {name} makes droid {id} on slot {} ({anim})",
                    slot + 1
                ));
                // `GenerateActor`: `PlaySound(DispenseSound)` when it has one.
                if let Some(path) = &d.dispense_sound {
                    let at = to_bevy(fight.battle.dispenser_location(spawner));
                    let heard = play_sound(&sounds, &camera, &mut commands, path, at);
                    log.line(format!("{PART}: dispenser {name} sound {heard}"));
                }
                if let Some(mut b) = body {
                    b.play(&anim, false);
                }
                if let Some(&e) = fight.entities.get(&id) {
                    // Can't be hit until it has unfolded.
                    commands.entity(e).remove::<Hittable>();
                    if let Ok((mut droid, _)) = droids.get_mut(e) {
                        droid.dispense = droid_anim(&looks, &kind, "DispBreathe", true);
                    }
                }
            }
            DispenserStep::Released { id } => {
                stats.released += 1;
                log.line(format!(
                    "{PART}: t={now:.1}s dispenser {name} lets droid {id} go: it unfolds"
                ));
                if let Some(&e) = fight.entities.get(&id) {
                    if let Ok((mut droid, _)) = droids.get_mut(e) {
                        droid.dispense = droid_anim(&looks, &kind, "DispUnfold", false);
                    }
                }
            }
            DispenserStep::Unfolded { id } => {
                stats.unfolded += 1;
                let at = fight
                    .battle
                    .fighters
                    .iter()
                    .find(|f| f.id == id)
                    .map(|f| f.pawn.feet());
                log.line(format!(
                    "{PART}: t={now:.1}s droid {id} from dispenser {name} unfolded{}: it fights",
                    at.map(|p| format!(" at ({:.0}, {:.0}, {:.0})", p.x, p.y, p.z))
                        .unwrap_or_default()
                ));
                if let Some(&e) = fight.entities.get(&id) {
                    let p = &sp.params.movement;
                    commands.entity(e).insert(Hittable {
                        radius: p.collision_radius,
                        half_height: p.collision_height,
                        center_up: p.collision_height,
                    });
                    if let Ok((mut droid, _)) = droids.get_mut(e) {
                        droid.dispense = None;
                    }
                }
            }
            DispenserStep::KilledOnSlot { id } => {
                log.line(format!(
                    "{PART}: t={now:.1}s droid {id} destroyed on its slot with dispenser {name}"
                ));
                fight.stats.droids_destroyed += 1;
                if let Some(&e) = fight.entities.get(&id) {
                    commands.entity(e).insert(Dead);
                    if let Ok((mut droid, health)) = droids.get_mut(e) {
                        droid.dispense = None;
                        if let Some(mut h) = health {
                            h.current = 0.0;
                        }
                    }
                }
            }
        }
    }
}

/// A droid animation by name from type `kind`'s look.
fn droid_anim(looks: &Looks, kind: &str, name: &str, looping: bool) -> Option<ScriptedAnim> {
    let clip = looks.by_type.get(kind)?.look.named_clip(name)?;
    Some(ScriptedAnim {
        name: name.to_string(),
        clip,
        looping,
        frame: 0.0,
    })
}

/// Hits on the dispensers: each change of health goes to the battle (its
/// damage steps and destruction), and what that sends to the level.
pub fn hits(
    mut fight: ResMut<Fight>,
    mut bodies: Query<(&mut DispenserBody, &Health), Changed<Health>>,
    mut raise: MessageWriter<RaiseEvent>,
    log: Res<GameLog>,
) {
    let fight = &mut *fight;
    let now = fight.battle.time;
    for (mut b, h) in &mut bodies {
        if h.current == b.last_health {
            continue;
        }
        log.line(format!(
            "{PART}: t={now:.1}s dispenser {} hit: health {:.0} -> {:.0} of {:.0}",
            b.name, b.last_health, h.current, h.max
        ));
        b.last_health = h.current;
        for e in fight.battle.dispenser_hurt(b.spawner, h.current) {
            match e {
                BattleEvent::Event { name, by } => {
                    log.line(format!("{PART}: t={now:.1}s {by} sends {name}"));
                    raise.write(RaiseEvent { name, by });
                }
                BattleEvent::Dispenser { spawner, step } => {
                    fight.dispenser_steps.push((spawner, step))
                }
                other => fight.queued.push(other),
            }
        }
    }
}

/// Step the dispensers' animations, re-pose their meshes and play their
/// animations' sounds.
#[allow(clippy::too_many_arguments)]
pub fn animate(
    time: Res<Time>,
    fight: Res<Fight>,
    mut bodies: Query<(&mut DispenserBody, &mut Transform)>,
    mut carriers: Query<&mut Transform, Without<DispenserBody>>,
    mut meshes: ResMut<Assets<Mesh>>,
    sounds: Option<Res<DispenserSounds>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    mut commands: Commands,
    // (Optional: tests run the fight without the effects part.)
    mut effects: Option<MessageWriter<SpawnEffect>>,
    mut stats: ResMut<DispenserStats>,
    log: Res<GameLog>,
) {
    let dt = time.delta_secs().min(0.25);
    for (mut b, mut t) in &mut bodies {
        // Where its intro (`PHYS_RootMotion`) and its base (`AttachTag`)
        // have carried it, and its effect with it (`SetBase`).
        let centre = fight.battle.dispenser_location(b.spawner);
        let yaw = fight.battle.dispenser_yaw(b.spawner);
        t.translation = to_bevy(centre - UVec3::new(0.0, 0.0, b.half));
        t.rotation = Quat::from_rotation_y(-yaw);
        if let Some(c) = b.effect {
            if let Ok(mut ct) = carriers.get_mut(c) {
                ct.translation = to_bevy(centre + b.effect_offset);
            }
        }
        let Some((anim, frame, looping)) = b.playing.clone() else {
            continue;
        };
        let Some(clip) = b.model.clip(&anim) else {
            b.playing = None;
            continue;
        };
        let n = clip.num_frames.max(1.0);
        let last = (clip.num_frames - 1.0).max(0.0);
        let raw = frame + clip.rate * dt;
        let next = if looping {
            raw.rem_euclid(n)
        } else {
            raw.min(last)
        };
        // Sounds whose moment passed this step.
        let (from, to) = (frame / n, if looping && raw >= n { 1.0 } else { next / n });
        if let Some(list) = b.model.sounds.get(&anim).cloned() {
            for (at, path) in list {
                let first = frame == 0.0 && at <= 0.0;
                if !(at > from && at <= to) && !(first && b.played.last() == Some(&anim)) {
                    continue;
                }
                let heard = play_sound(&sounds, &camera, &mut commands, &path, t.translation);
                log.line(format!("{PART}: dispenser {} sound {heard}", b.name));
                b.heard.push(heard);
            }
        }
        // Effects whose moment passed this step (`AnimNotify_Effect`).
        if let Some(list) = b.model.effects.get(&anim).cloned() {
            for (at, fx) in list {
                let first = frame == 0.0 && at <= 0.0;
                if !(at > from && at <= to) && !(first && b.played.last() == Some(&anim)) {
                    continue;
                }
                let origin = centre - UVec3::new(0.0, 0.0, b.half);
                let bone = fx
                    .bone
                    .as_ref()
                    .and_then(|bone| b.model.bone_from_origin_in_game(&anim, at * n, bone));
                let (place, axes) = match bone {
                    // At the bone plus the offset along its axes, turned
                    // as the bone.
                    Some((p, ax)) => {
                        let v = |a: [f32; 3]| UVec3::new(a[0], a[1], a[2]);
                        let local = v(p)
                            + v(ax[0]) * fx.offset[0]
                            + v(ax[1]) * fx.offset[1]
                            + v(ax[2]) * fx.offset[2];
                        (
                            origin + turn(local, yaw),
                            [0, 1, 2].map(|k| {
                                let a = turn(v(ax[k]), yaw);
                                [a.x, a.y, a.z]
                            }),
                        )
                    }
                    // At the actor, turned as the actor.
                    None => (
                        centre,
                        [
                            [yaw.cos(), yaw.sin(), 0.0],
                            [-yaw.sin(), yaw.cos(), 0.0],
                            [0.0, 0.0, 1.0],
                        ],
                    ),
                };
                let mut notes = Vec::new();
                if fx.offset_rotation != [0; 3] {
                    notes.push("its OffsetRotation is not applied");
                }
                if fx.attach {
                    notes.push("Attach is not followed (it stays where it started)");
                }
                if fx.draw_scale_3d != [1.0; 3] {
                    notes.push("its DrawScale3D is not applied");
                }
                log.line(format!(
                    "{PART}: dispenser {} {anim} starts effect {} at ({:.0}, {:.0}, {:.0}){}",
                    b.name,
                    fx.path,
                    place.x,
                    place.y,
                    place.z,
                    if notes.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", notes.join("; "))
                    }
                ));
                stats.effects.push(format!("{} {}", b.name, fx.class));
                if let Some(effects) = effects.as_mut() {
                    effects.write(SpawnEffect {
                        effect: fx.class.clone(),
                        at: to_bevy(place),
                        floor: Some(to_bevy(origin).y),
                        sound: None,
                        by: format!("dispenser {} ({anim})", b.name),
                        rotation: [0, 0, 0],
                        draw_scale: fx.draw_scale,
                        id: None,
                        follow: None,
                        vanish_with_carrier: false,
                        axes: Some(axes),
                        endless: false,
                    });
                }
            }
        }
        let points = b.model.posed_rooted(&anim, next);
        let normals = b.model.normals(&points);
        let model = b.model.clone();
        for (s, h) in model.sections.iter().zip(&b.meshes) {
            let (pos, nrm) = model.section_attributes(s, &points, &normals);
            if let Some(mut m) = meshes.get_mut(h) {
                m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
                m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
            }
        }
        if !looping && next >= last {
            b.playing = None;
            b.last = Some(anim);
        } else {
            b.playing = Some((anim, next, looping));
        }
    }
}

fn play_sound(
    sounds: &Option<Res<DispenserSounds>>,
    camera: &Query<&GlobalTransform, With<PlayerCamera>>,
    commands: &mut Commands,
    path: &str,
    at: Vec3,
) -> String {
    let Some(h) = sounds
        .as_ref()
        .and_then(|s| s.0.get(&sound_key(path)))
        .and_then(|g| g.pick())
    else {
        return format!("{path} (not loaded)");
    };
    let distance = camera
        .single()
        .map(|c| c.translation().distance(at) * 50.0)
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
    path.to_string()
}

/// After a save is loaded: show each dispenser as its saved state says
/// (skin, last animation's end, health), with no effects or events.
pub fn after_load(
    mut fight: ResMut<Fight>,
    mut bodies: Query<(Entity, &mut DispenserBody, &mut Health)>,
    mut materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
) {
    if !std::mem::take(&mut fight.dispensers_restored) {
        return;
    }
    for (e, mut b, mut h) in &mut bodies {
        let sp = &fight.battle.spawners[b.spawner];
        let (Some(st), Some(d)) = (&sp.disp, &sp.factory.dispenser) else {
            continue;
        };
        h.current = st.health;
        b.last_health = st.health;
        use rc_ai::Phase;
        let (mat, pose) = match st.phase {
            Phase::WarmUp { .. } | Phase::WaitOnline => {
                (d.warm_up.material.clone(), d.intro_anim.clone())
            }
            Phase::Online | Phase::Waiting { .. } => {
                (d.online.material.clone(), d.intro_anim.clone())
            }
            Phase::Shutdown => (d.shutdown.material.clone(), d.intro_anim.clone()),
            Phase::Destroyed => (d.damaged.material.clone(), d.damaged.anim.clone()),
            _ => (None, None),
        };
        if st.phase == Phase::Destroyed {
            commands.entity(e).insert(Dead);
        }
        if let Some(mat) = mat {
            if let Some(set) = b.skins.get(&mat).cloned() {
                for (part, m) in b.parts.iter().zip(set) {
                    if let Ok(mut mm) = materials.get_mut(*part) {
                        mm.0 = m;
                    }
                }
                b.skin = mat;
            }
        }
        if let Some(a) = pose.filter(|a| b.model.clip(a).is_some()) {
            let last = (b.model.clip(&a).unwrap().num_frames - 1.0).max(0.0);
            let points = b.model.posed_rooted(&a, last);
            let normals = b.model.normals(&points);
            let model = b.model.clone();
            for (s, h) in model.sections.iter().zip(&b.meshes) {
                let (pos, nrm) = model.section_attributes(s, &points, &normals);
                if let Some(mut m) = meshes.get_mut(h) {
                    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
                    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
                }
            }
            b.playing = None;
            b.last = Some(a);
        }
    }
}

/// Turn a point about the vertical axis by `yaw` (radians, Unreal axes).
fn turn(v: UVec3, yaw: f32) -> UVec3 {
    let (s, c) = yaw.sin_cos();
    UVec3::new(v.x * c - v.y * s, v.x * s + v.y * c, v.z)
}
