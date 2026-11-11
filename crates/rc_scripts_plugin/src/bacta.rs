//! Bacta dispensers: hold Use near one that is switched on to heal.
//!
//! - **Healing.** Starting sends the dispenser's "in use" event
//!   (`BactaActive.Event`) and plays its `Load` animation; health goes up
//!   only once `Load` has finished, as in the game's script; full health
//!   sends its "used" event (`BactaRecharging.Event`); stopping plays
//!   `Holster`.
//! - **The pull and hold** (the game's `bLockPlayerLocation`): while
//!   healing, the player is pulled to a spot `PlayerOffset` (90) units in
//!   front of the dispenser, turned to face it, and held there (no
//!   walking, jumping, crouching, shooting or squad orders) until they let
//!   go of Use or are fully healed. Written to [`UseItem::lock`]; the
//!   player part carries it out. How fast the pull goes is our guess.
//! - **The hologram.** The squad marker's see-through commando
//!   (`MarkerIcons.Revive.reviveSM1BactaDispenser`, `MarkerGetBacta`'s
//!   icon) stands at the dispenser's squad spot (its `NavPts` marker),
//!   facing it, showing where a squad mate heals. As in the game's
//!   `SquadMarker` script, it fades in only while the crosshair is on that
//!   spot within the marker's `TargetingDistance`, and only when the
//!   marker is relevant (`MarkerGetBacta`: a squad mate below full
//!   health). It is hidden while the dispenser is in use or recharging
//!   (the dispenser's script makes its marker obsolete then). Colour
//!   `EnabledColor`. Drawn as a flat see-through tint for now; the
//!   original's flickering hologram shader is another thread's work.
//! - **The prompt**: "PRESS AND HOLD F TO HEAL" (the dispenser's
//!   `ActivatePromptText`, read from the game) while you stand at one you
//!   can use. As in the game, Use does nothing at full health.
//! - **Sounds** (the game's HUD sounds, `CTHUDDefaults`): `TargetMarker`
//!   when the crosshair moves onto a usable dispenser, `HealthRecharging`
//!   over and over while healing, `HealthCharged` at full health. Heard
//!   from the helmet, not from a place. When exactly the original plays
//!   them is native code we don't read; this timing is our reading.
//! - **The use panel** round the crosshair: [`UseItem::panel`], drawn by
//!   the HUD.
//! - **The hands.** While healing, [`UseItem::hands`] is `"bacta"`: the
//!   first-person arms put the weapon away and the commando's hands plug
//!   into the dispenser (the game's `HandSignal`, `SetupBactaHealer`),
//!   played by `rc_combat`'s arms.
//! - **The zap and its hum** ([`zap`]), as the game's `BactaDispenser`
//!   script (ctgame.u) does it: once the dispenser's `Load` animation has
//!   finished, its `DispenserEffectClass` effect (`BactaZap_Dispenser` for
//!   every dispenser class, set in `Properties/MarkerProperties.u`) is
//!   hung on the model's `Eye` bone (`SpawnAttachment('Eye', ...)`), with
//!   the bone's turn. The effect's own `AmbientSound`
//!   (`GEO_Sound.GEO_03.ele_GEO_genEnrgy_lp_01`) is the hum, looping while
//!   it is there. When healing stops the script removes it at once
//!   (`DestroyAttachment('Eye')`), so the hum stops too. The game also
//!   puts `PawnEffectClass` (`BactaZap_Dispenser_Body`) on the player's
//!   body; that isn't shown yet, because its particles start from the
//!   bones of the player's own body model, which the game doesn't have in
//!   first person here.
//!
//! Every number and name is in `[bacta]` of `data/scripts/scripts.toml`.

use bevy::asset::RenderAssetUsages;
use bevy::audio::{AudioSource, PlaybackSettings};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use rc_combat::{GameSound, Health};
use rc_game_core::units::{to_bevy, to_unreal};
use rc_game_core::{
    Action, ActionState, GameLog, GamePaths, GameState, InputPrompts, Player, PlayerCamera,
    SquadReadout, SquadStatus, UseItem, UseLock, UsePanel,
};
use rc_level_viewer::materials::Materials;
use std::collections::HashMap;

use crate::{LevelScript, PART};

/// A dispenser's collision radius (BactaDispenser `CollisionRadius`), for
/// telling when the crosshair is on one.
const AIM_RADIUS: f32 = 40.0;

/// Height of a dispenser's collision cylinder above its middle (the map
/// places it by the middle; the model and the floor are this far below).
/// BactaDispenserGEO's `CollisionHeight`.
const HALF_HEIGHT: f32 = 120.0;

/// The sounds, hologram look and prompt, read while the level loads.
#[derive(Resource, Default)]
pub struct BactaAssets {
    sounds: HashMap<String, GameSound>,
    /// The prompt line, `@` for the Use button.
    prompt: Option<String>,
    /// The hologram's sections, in the model's own space (Unreal units).
    hologram: Vec<HoloPart>,
    /// Each dispenser class's zap effect (`DispenserEffectClass`), by
    /// class name in lower case; `None` when the class names none.
    pub zap_effects: HashMap<String, Option<String>>,
    /// Why something couldn't be read.
    pub problems: Vec<String>,
}

/// One section of the hologram model: points, normals, picture
/// coordinates and triangles.
type HoloPart = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>);

/// One dispenser's hologram.
#[derive(Component)]
pub struct BactaHologram {
    pub dispenser: String,
    /// Where it stands (its feet), Unreal units.
    pub feet: [f32; 3],
    /// How far faded in, 0 (gone) to 1.
    pub alpha: f32,
}

/// The hologram's size for "the crosshair is on it": the model is about
/// 180 units tall and 70 across (our reading of its points).
const HOLO_RADIUS: f32 = 40.0;
const HOLO_HALF_HEIGHT: f32 = 92.0;

/// The healing sound while it plays.
#[derive(Component)]
pub struct HealingSound;

/// What the bacta systems saw, for the log and self-tests.
#[derive(Resource, Debug, Default, Clone)]
pub struct BactaStats {
    /// Holograms placed.
    pub holograms: usize,
    /// Holograms on screen now, and how many have ever been.
    pub holograms_shown: usize,
    pub holograms_ever_shown: bool,
    /// Times a hologram started to show while the player was healing (it
    /// never should).
    pub holograms_while_healing: usize,
    /// The prompt shown last.
    pub prompt: Option<String>,
    /// Sounds played, by setting name ("aim", "healing", "full").
    pub sounds: Vec<String>,
    /// Times the player was pulled to a dispenser, and the furthest the
    /// player stood from the spot while held after the pull arrived.
    pub pulls: usize,
    /// The spot the player was pulled to last (x, y).
    pub last_stand: Option<[f32; 2]>,
    /// Most progress the use panel showed (0 to 1).
    pub panel_best: f32,
    /// Zap effects hung on a dispenser: (effect, dispenser, where the
    /// `Eye` bone was, Unreal units).
    pub zaps: Vec<(String, String, [f32; 3])>,
    /// Zaps taken away again when healing stopped.
    pub zaps_removed: usize,
}

/// The bone the game hangs the zap on (`SpawnAttachment('Eye', ...)` in
/// `BactaDispenser`'s `Active` state).
pub const ZAP_BONE: &str = "Eye";

/// Carries a dispenser's zap effect: it stands on the `Eye` bone, and the
/// effect goes with it when it is removed.
#[derive(Component)]
pub struct BactaZap {
    pub dispenser: String,
}

/// Read the hologram model and the sounds, once the level's dispensers
/// are known. Runs while loading, after the scripts are built.
pub(crate) fn prepare(
    script: Option<Res<LevelScript>>,
    have: Option<Res<BactaAssets>>,
    paths: Res<GamePaths>,
    mut audio: ResMut<Assets<AudioSource>>,
    mut commands: Commands,
    mut preload: MessageWriter<rc_effects_plugin::PreloadEffect>,
    log: Res<GameLog>,
) {
    let Some(script) = script else { return };
    if have.is_some() {
        return;
    }
    let mut assets = BactaAssets::default();
    commands.insert_resource(BactaStats::default());
    if script.dispensers.is_empty() {
        commands.insert_resource(assets);
        return;
    }
    let s = &script.director.settings;
    // Sounds, by package.
    let mut wanted: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (key, path) in [
        ("aim", &s.bacta_sound_aim),
        ("timer", &s.bacta_sound_timer),
        ("healing", &s.bacta_sound_healing),
        ("full", &s.bacta_sound_full),
    ] {
        if path.is_empty() {
            continue;
        }
        let package = path.split('.').next().unwrap_or("").to_string();
        let name = path.rsplit('.').next().unwrap_or("").to_string();
        wanted
            .entry(package)
            .or_default()
            .push((key.to_string(), name));
    }
    for (package, list) in wanted {
        let names: Vec<String> = list.iter().map(|(_, n)| n.clone()).collect();
        let (found, notes) = rc_combat::read_sounds(&paths, &package, &names, &mut audio);
        assets.problems.extend(notes);
        for (key, name) in list {
            match found.iter().find(|(n, _)| n.eq_ignore_ascii_case(&name)) {
                Some((_, sound)) => {
                    assets.sounds.insert(key, sound.clone());
                }
                None => assets
                    .problems
                    .push(format!("bacta sound {package}.{name} not found")),
            }
        }
    }
    assets.prompt = if s.bacta_prompt.is_empty() {
        match read_prompt(&paths) {
            Ok(p) => Some(p),
            Err(e) => {
                assets.problems.push(format!("bacta prompt: {e}"));
                None
            }
        }
    } else {
        Some(s.bacta_prompt.clone())
    };
    // Each dispenser class's zap effect, from the game's class defaults.
    let classes: Vec<String> = script.dispensers.iter().map(|d| d.class.clone()).collect();
    match zap_effects(&paths, &classes) {
        Ok(found) => {
            for (class, effect) in &found {
                log.line(format!(
                    "{PART}: bacta: {class} DispenserEffectClass = {}",
                    effect.as_deref().unwrap_or("none")
                ));
                if let Some(e) = effect {
                    preload.write(rc_effects_plugin::PreloadEffect {
                        effect: Some(e.clone()),
                        sound: None,
                    });
                }
            }
            assets.zap_effects = found;
        }
        Err(e) => assets.problems.push(format!("bacta zap: {e}")),
    }
    if !s.bacta_hologram.is_empty() {
        match read_hologram(&paths, &s.bacta_hologram) {
            Ok(sections) => assets.hologram = sections,
            Err(e) => assets.problems.push(format!("bacta hologram: {e}")),
        }
    }
    log.line(format!(
        "{PART}: bacta: {} sounds and a hologram of {} parts read{}",
        assets.sounds.len(),
        assets.hologram.len(),
        if assets.problems.is_empty() {
            String::new()
        } else {
            format!("; problems: {}", assets.problems.join("; "))
        }
    ));
    commands.insert_resource(assets);
}

/// The game's own prompt for a dispenser: BactaDispenser's
/// `ActivatePromptText` in ctgame.u ("PRESS AND HOLD @ TO HEAL").
fn read_prompt(paths: &GamePaths) -> Result<String, String> {
    use rc_package::class_library::{ClassLibrary, DefaultValue};
    let mut lib = ClassLibrary::new();
    for name in ["Core", "Engine", "CTGame"] {
        let path = paths.game_file(&format!("GameData/System/{name}.u"));
        let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        lib.add_package(name, data)
            .map_err(|e| format!("{name}.u: {e}"))?;
    }
    let d = lib.defaults("BactaDispenser");
    match d.get("ActivatePromptText").map(|e| &e.value) {
        Some(DefaultValue::Raw(b)) => rc_package::Reader::new(b)
            .fstring()
            .map_err(|e| e.to_string()),
        _ => Err("BactaDispenser sets no ActivatePromptText".into()),
    }
}

/// Each class's `DispenserEffectClass`, read from the game's class
/// defaults (`BactaDispenserGEO` and the others are in
/// `Properties/MarkerProperties.u`, below `BactaDispenser` in ctgame.u).
/// Keys are lower case; the effect is its class name alone
/// (`BactaZap_Dispenser`).
fn zap_effects(
    paths: &GamePaths,
    classes: &[String],
) -> Result<HashMap<String, Option<String>>, String> {
    use rc_package::class_library::ClassLibrary;
    let mut lib = ClassLibrary::new();
    for (name, rel) in [
        ("Core", "GameData/System/Core.u"),
        ("Engine", "GameData/System/Engine.u"),
        ("CTGame", "GameData/System/CTGame.u"),
        ("MarkerProperties", "GameData/Properties/MarkerProperties.u"),
    ] {
        let path = paths.game_file(rel);
        let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        lib.add_package(name, data)
            .map_err(|e| format!("{name}: {e}"))?;
    }
    let mut out = HashMap::new();
    for class in classes {
        let key = class.to_ascii_lowercase();
        if out.contains_key(&key) {
            continue;
        }
        let effect = lib
            .defaults(class)
            .object("DispenserEffectClass")
            .map(|o| o.rsplit('.').next().unwrap_or(o).to_string())
            .filter(|o| !o.is_empty() && !o.eq_ignore_ascii_case("None"));
        out.insert(key, effect);
    }
    Ok(out)
}

/// The dispenser's zap and hum (see the top of this file): hung on the
/// `Eye` bone once `Load` has played, while the player heals; taken away
/// at once when they stop.
#[allow(clippy::too_many_arguments)]
pub(crate) fn zap(
    script: Option<Res<LevelScript>>,
    assets: Option<Res<BactaAssets>>,
    props: Query<&rc_level_plugin::anim_props::AnimProp>,
    mut zaps: Query<(Entity, &BactaZap, &mut Transform)>,
    mut effects: MessageWriter<rc_effects_plugin::SpawnEffect>,
    mut stats: Option<ResMut<BactaStats>>,
    mut commands: Commands,
    log: Res<GameLog>,
) {
    let Some(script) = script else { return };
    let now = script.director.time;
    // Remove the zaps of dispensers no longer in use (`EndState`:
    // `DestroyAttachment('Eye')`).
    for (e, z, _) in &zaps {
        let in_use = script
            .dispensers
            .iter()
            .any(|d| d.in_use && d.name.eq_ignore_ascii_case(&z.dispenser));
        if !in_use {
            commands.entity(e).despawn();
            log.line(format!(
                "{PART}: t={now:.2} bacta: {}'s zap taken away",
                z.dispenser
            ));
            if let Some(st) = stats.as_mut() {
                st.zaps_removed += 1;
            }
        }
    }
    for d in script.dispensers.iter().filter(|d| d.in_use) {
        let Some(prop) = props.iter().find(|p| p.name.eq_ignore_ascii_case(&d.name)) else {
            continue;
        };
        let have = zaps
            .iter_mut()
            .find(|(_, z, _)| z.dispenser.eq_ignore_ascii_case(&d.name));
        if let Some((_, _, mut t)) = have {
            // It stays on the bone.
            if let Some((at, _)) = prop.bone(ZAP_BONE) {
                t.translation = to_bevy(rc_movement::Vec3::new(at[0], at[1], at[2]));
            }
            continue;
        }
        // `Begin: FinishAnim(); ... SpawnAttachment('Eye', ...)`: only once
        // the `Load` animation has played to its end.
        if prop.playing.is_some() || prop.last.as_deref() != Some("Load") {
            continue;
        }
        let Some(Some(effect)) = assets
            .as_ref()
            .and_then(|a| a.zap_effects.get(&d.class.to_ascii_lowercase()))
        else {
            continue;
        };
        let Some((at, axes)) = prop.bone(ZAP_BONE) else {
            log.line(format!(
                "{PART}: t={now:.2} bacta: {} has no {ZAP_BONE} bone for its zap",
                d.name
            ));
            continue;
        };
        let pos = to_bevy(rc_movement::Vec3::new(at[0], at[1], at[2]));
        let carrier = commands
            .spawn((
                BactaZap {
                    dispenser: d.name.clone(),
                },
                Transform::from_translation(pos),
                GlobalTransform::from_translation(pos),
                DespawnOnExit(GameState::InGame),
            ))
            .id();
        effects.write(rc_effects_plugin::SpawnEffect {
            effect: effect.clone(),
            at: pos,
            by: d.name.clone(),
            follow: Some(carrier),
            vanish_with_carrier: true,
            axes: Some(axes),
            ..default()
        });
        log.line(format!(
            "{PART}: t={now:.2} bacta: {effect} hung on {}'s {ZAP_BONE} bone at ({:.0}, {:.0}, {:.0}), forward ({:.2}, {:.2}, {:.2}), up ({:.2}, {:.2}, {:.2})",
            d.name, at[0], at[1], at[2], axes[0][0], axes[0][1], axes[0][2], axes[2][0], axes[2][1], axes[2][2]
        ));
        if let Some(st) = stats.as_mut() {
            st.zaps.push((effect.clone(), d.name.clone(), at));
        }
    }
}

/// The hologram model `Package.Group.Name`, one entry per section.
#[allow(clippy::type_complexity)]
fn read_hologram(paths: &GamePaths, path: &str) -> Result<Vec<HoloPart>, String> {
    let package = path.split('.').next().unwrap_or("");
    let name = path.rsplit('.').next().unwrap_or("");
    let mut mats = Materials::new(&paths.game_data());
    let loaded = mats
        .package(package)
        .ok_or_else(|| format!("package {package} not found"))?;
    let (pkg, data) = (&loaded.0, &loaded.1);
    let export = pkg
        .exports
        .iter()
        .find(|e| {
            pkg.export_class_name(e) == "StaticMesh"
                && pkg.name(e.object_name).eq_ignore_ascii_case(name)
        })
        .ok_or_else(|| format!("no static mesh {name} in {package}"))?;
    let mesh =
        rc_package::static_mesh::read_static_mesh(pkg, data, export).map_err(|e| e.to_string())?;
    let uvs = mesh.uv_sets.first().cloned().unwrap_or_default();
    let mut out = Vec::new();
    for sec in &mesh.sections {
        let first = sec.first_index as usize;
        let count = sec.num_faces as usize * 3;
        let Some(tris) = mesh.indices.get(first..first + count) else {
            continue;
        };
        let (mut pos, mut nrm, mut tex, mut idx) = (vec![], vec![], vec![], vec![]);
        let mut remap = HashMap::new();
        for &vi in tris {
            let n = *remap.entry(vi).or_insert_with(|| {
                let v = mesh.vertices[vi as usize];
                pos.push(v.position);
                nrm.push(v.normal);
                tex.push(uvs.get(vi as usize).copied().unwrap_or([0.0, 0.0]));
                pos.len() as u32 - 1
            });
            idx.push(n);
        }
        // No picture: the original's hologram shader doesn't use the
        // model's texture coordinates the usual way (its glow picture is
        // looked up by which way each surface faces the eye), so a plain
        // tint stands in for it.
        out.push((pos, nrm, tex, idx));
    }
    Ok(out)
}

/// Place one hologram per dispenser that has a squad spot, hidden until
/// the player comes near. Runs as the level starts.
pub(crate) fn place_holograms(
    script: Option<Res<LevelScript>>,
    assets: Option<Res<BactaAssets>>,
    mut stats: Option<ResMut<BactaStats>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
    log: Res<GameLog>,
) {
    let (Some(script), Some(assets)) = (script, assets) else {
        return;
    };
    if assets.hologram.is_empty() {
        return;
    }
    let s = &script.director.settings;
    let mut n = 0;
    for d in &script.dispensers {
        let Some(spot) = d.stand else {
            continue;
        };
        // Facing the dispenser, feet on the dispenser's floor.
        let yaw = (d.location[1] - spot[1]).atan2(d.location[0] - spot[0]);
        let base = [spot[0], spot[1], d.location[2] - HALF_HEIGHT];
        let (sin, cos) = yaw.sin_cos();
        let place = |v: [f32; 3]| {
            to_bevy(rc_movement::Vec3::new(
                base[0] + v[0] * cos - v[1] * sin,
                base[1] + v[0] * sin + v[1] * cos,
                base[2] + v[2],
            ))
        };
        let turn = |v: [f32; 3]| {
            let w = rc_movement::Vec3::new(v[0] * cos - v[1] * sin, v[0] * sin + v[1] * cos, v[2]);
            let b = to_bevy(w) * rc_game_core::units::UU_PER_UNIT;
            b.normalize_or_zero()
        };
        log.line(format!(
            "{PART}: bacta: hologram for {} at ({:.0}, {:.0}, {:.0}) facing {:.0} deg",
            d.name,
            base[0],
            base[1],
            base[2],
            yaw.to_degrees()
        ));
        let colour = s.bacta_hologram_colour;
        let entity = commands
            .spawn((
                BactaHologram {
                    dispenser: d.name.clone(),
                    feet: base,
                    alpha: 0.0,
                },
                Transform::default(),
                Visibility::Hidden,
                DespawnOnExit(GameState::InGame),
            ))
            .id();
        for (pos, nrm, tex, idx) in &assets.hologram {
            let m = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            )
            .with_inserted_attribute(
                Mesh::ATTRIBUTE_POSITION,
                pos.iter().map(|p| place(*p).to_array()).collect::<Vec<_>>(),
            )
            .with_inserted_attribute(
                Mesh::ATTRIBUTE_NORMAL,
                nrm.iter().map(|p| turn(*p).to_array()).collect::<Vec<_>>(),
            )
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, tex.clone())
            .with_inserted_indices(Indices::U32(idx.clone()));
            let material = materials.add(hologram_material(colour, 0.0));
            let child = commands
                .spawn((Mesh3d(meshes.add(m)), MeshMaterial3d(material)))
                .id();
            commands.entity(entity).add_child(child);
        }
        n += 1;
    }
    if let Some(stats) = stats.as_mut() {
        stats.holograms = n;
    }
    log.line(format!("{PART}: bacta: {n} squad marker holograms placed"));
}

/// The hologram's colour at `brightness`, fully opaque (it is added onto
/// what is behind it, so the brightness sets how see-through it looks).
fn tint(colour: [u8; 3], brightness: f32) -> LinearRgba {
    let c = Color::srgb_u8(colour[0], colour[1], colour[2]).to_linear();
    LinearRgba::new(
        c.red * brightness,
        c.green * brightness,
        c.blue * brightness,
        1.0,
    )
}

fn hologram_material(colour: [u8; 3], brightness: f32) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::LinearRgba(tint(colour, brightness)),
        unlit: true,
        double_sided: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Add,
        ..default()
    }
}

/// Fade each hologram in while the crosshair is on it and a squad mate
/// could use its dispenser, and out otherwise. See the top of this file.
#[allow(clippy::too_many_arguments)]
pub(crate) fn show_holograms(
    time: Res<Time>,
    script: Option<Res<LevelScript>>,
    squad: Res<SquadReadout>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    mut holos: Query<(&mut BactaHologram, &mut Visibility, &Children)>,
    looks: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut stats: Option<ResMut<BactaStats>>,
    log: Res<GameLog>,
) {
    let Some(script) = script else { return };
    let s = &script.director.settings;
    let aim = camera
        .single()
        .ok()
        .map(|c| (to_unreal(c.translation()), to_unreal(c.forward().as_vec3())));
    // MarkerGetBacta is relevant while a squad mate is below full health.
    let relevant = squad
        .members
        .iter()
        .any(|m| m.health < 0.999 || m.status == SquadStatus::Incapacitated);
    let healing = script.dispensers.iter().any(|d| d.in_use);
    let step = if s.bacta_hologram_fade_seconds > 0.0 {
        time.delta_secs() / s.bacta_hologram_fade_seconds
    } else {
        1.0
    };
    let mut shown = 0;
    for (mut h, mut vis, children) in &mut holos {
        let Some(d) = script.dispensers.iter().find(|d| d.name == h.dispenser) else {
            continue;
        };
        let free = d.enabled && !d.in_use && d.recharge_left <= 0.0;
        let centre = [h.feet[0], h.feet[1], h.feet[2] + HOLO_HALF_HEIGHT];
        let targeted = aim.is_some_and(|(eye, dir)| {
            aims_at_cylinder(
                eye,
                dir,
                centre,
                HOLO_RADIUS,
                HOLO_HALF_HEIGHT,
                s.bacta_hologram_range,
            )
        });
        let want = free && relevant && targeted;
        let before = h.alpha;
        h.alpha = if want {
            (h.alpha + step).min(1.0)
        } else {
            (h.alpha - step).max(0.0)
        };
        if before <= 0.0 && h.alpha > 0.0 {
            log.line(format!("{PART}: bacta: hologram {} fades in", h.dispenser));
            if let Some(stats) = stats.as_mut() {
                stats.holograms_ever_shown = true;
                if healing {
                    stats.holograms_while_healing += 1;
                }
            }
        }
        let target = if h.alpha > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != target {
            *vis = target;
        }
        if h.alpha > 0.0 {
            shown += 1;
        }
        if h.alpha != before {
            for c in children.iter() {
                if let Ok(m) = looks.get(c) {
                    if let Some(mut mat) = materials.get_mut(&m.0) {
                        mat.base_color = Color::LinearRgba(tint(
                            s.bacta_hologram_colour,
                            s.bacta_hologram_brightness * h.alpha,
                        ));
                    }
                }
            }
        }
    }
    if let Some(stats) = stats.as_mut() {
        stats.holograms_shown = shown;
    }
}

/// Hold Use near a switched-on dispenser to heal. See the top of this file.
#[allow(clippy::too_many_arguments)]
pub(crate) fn heal(
    time: Res<Time>,
    script: Option<ResMut<LevelScript>>,
    actions: Res<ActionState>,
    mut player: Query<(&Transform, Option<&mut Health>), With<Player>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    log: Res<GameLog>,
    (mut claimed, mut use_item): (ResMut<rc_game_core::UseClaimed>, ResMut<UseItem>),
    mut anims: MessageWriter<rc_game_core::PropAnim>,
    (assets, mut stats): (Option<Res<BactaAssets>>, Option<ResMut<BactaStats>>),
    (healing_sound, prompts): (Query<Entity, With<HealingSound>>, Option<Res<InputPrompts>>),
    props: Query<&rc_level_plugin::anim_props::AnimProp>,
    mut commands: Commands,
    mut aimed: Local<bool>,
) {
    claimed.0 = false;
    let Some(mut script) = script else {
        use_item.lock = None;
        use_item.panel = UsePanel::Hidden;
        use_item.prompt = None;
        use_item.hands = None;
        return;
    };
    let Ok((t, health)) = player.single_mut() else {
        return;
    };
    let dt = time.delta_secs().min(0.25);
    let feet = to_unreal(t.translation);
    let using = actions.pressed(Action::Use);
    let s = script.director.settings.clone();
    let now = script.director.time;
    let mut health = health;
    // The game's PlayerController: Use at a dispenser does nothing at
    // full health.
    let hurt = health.as_deref().is_some_and(|h| h.current < h.max);
    let health_now = health
        .as_deref()
        .map_or(1.0, |h| (h.current / h.max.max(1.0)).clamp(0.0, 1.0));
    let mut fire = Vec::new();
    let mut healed = 0.0;
    let mut lock = None;
    let mut panel = UsePanel::Hidden;
    let mut play: Vec<&str> = Vec::new();
    let mut healing = false;
    // The crosshair's line, for "aimed at a dispenser".
    let aim = camera
        .single()
        .ok()
        .map(|c| (c.translation(), c.forward().as_vec3()));
    let mut aim_on = false;
    for d in &mut script.dispensers {
        d.recharge_left = (d.recharge_left - dt).max(0.0);
        let dx = feet.x - d.location[0];
        let dy = feet.y - d.location[1];
        let near =
            (dx * dx + dy * dy).sqrt() < s.bacta_range && (feet.z - d.location[2]).abs() < 256.0;
        let ready = d.enabled && d.recharge_left <= 0.0;
        // The crosshair on it, near enough to use (the game's TGT_Panel).
        let targeted = d.enabled
            && aim.is_some_and(|(from, dir)| {
                aims_at(
                    to_unreal(from),
                    to_unreal(dir),
                    d.location,
                    s.bacta_range + AIM_RADIUS,
                )
            });
        claimed.0 |= (near || targeted) && ready;
        aim_on |= targeted;
        let can = using && ((ready && hurt && (near || targeted)) || d.in_use);
        if can && !d.in_use {
            d.in_use = true;
            log.line(format!(
                "{PART}: t={now:.2} the player uses bacta dispenser {}",
                d.name
            ));
            if let Some(e) = &d.active_event {
                fire.push((e.clone(), d.name.clone()));
            }
            // The canister swings in (the model's own animation and sounds).
            anims.write(rc_game_core::PropAnim {
                actor: d.name.clone(),
                anim: "Load".into(),
            });
            play.push("timer");
            if let Some(stats) = stats.as_mut() {
                stats.pulls += 1;
            }
        }
        if (targeted || near) && !d.in_use && panel == UsePanel::Hidden {
            // The ring shows the player's health while it can heal them;
            // otherwise (health full, recharging) only its icon.
            panel = if ready && hurt {
                UsePanel::Ready(health_now)
            } else if targeted {
                UsePanel::Icon(health_now)
            } else {
                UsePanel::Hidden
            };
        }
        if d.in_use {
            // The game's `state Active` heals only once `bActivated` is set,
            // after `Load` has played (`Begin: FinishAnim(); bActivated =
            // true`), the moment the zap appears. A dispenser drawn without
            // its model heals at once.
            let activated = props
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(&d.name))
                .is_none_or(|p| p.playing.is_none() && p.last.as_deref() == Some("Load"));
            let rate = if activated {
                s.bacta_heal_per_second
            } else {
                0.0
            };
            let (full, fraction) = match health.as_deref_mut() {
                Some(h) => {
                    let before = h.current;
                    h.current = (h.current + rate * dt).min(h.max);
                    healed += h.current - before;
                    (
                        h.current >= h.max,
                        (h.current / h.max.max(1.0)).clamp(0.0, 1.0),
                    )
                }
                None => (true, 1.0),
            };
            if full || !can {
                d.in_use = false;
                anims.write(rc_game_core::PropAnim {
                    actor: d.name.clone(),
                    anim: "Holster".into(),
                });
                if full {
                    // The panel shows full for this last frame.
                    panel = UsePanel::Working(1.0);
                    d.recharge_left = s.bacta_recharge_seconds;
                    log.line(format!(
                        "{PART}: t={now:.2} bacta dispenser {} done: health full",
                        d.name
                    ));
                    play.push("full");
                    if let Some(e) = &d.used_event {
                        fire.push((e.clone(), d.name.clone()));
                    }
                } else {
                    log.line(format!(
                        "{PART}: t={now:.2} the player let go of bacta dispenser {} at {:.0}%",
                        d.name,
                        fraction * 100.0
                    ));
                }
            } else {
                healing = true;
                panel = UsePanel::Working(fraction);
                if s.bacta_lock_player {
                    lock = Some(UseLock {
                        speed: s.bacta_pull_speed,
                        turn: s.bacta_turn_speed.to_radians(),
                        ..lock_for(d.location, d.yaw, s.bacta_player_offset)
                    });
                }
            }
        }
    }
    if aim_on && !*aimed {
        play.push("aim");
    }
    *aimed = aim_on;
    script.stats.bacta_healed += healed;
    for (e, by) in fire {
        script.director.fire(&e, &by);
    }
    if lock != use_item.lock {
        match &lock {
            Some(l) => log.line(format!(
                "{PART}: t={now:.2} bacta: player pulled to ({:.0}, {:.0}) facing {:.0} deg and held",
                l.stand[0],
                l.stand[1],
                l.face_yaw.to_degrees()
            )),
            None => log.line(format!("{PART}: t={now:.2} bacta: player let go")),
        }
        use_item.lock = lock;
    }
    if let (Some(stats), Some(l)) = (stats.as_mut(), lock) {
        stats.last_stand = Some(l.stand);
    }
    if let (Some(stats), UsePanel::Working(p)) = (stats.as_mut(), panel) {
        stats.panel_best = stats.panel_best.max(p);
    }
    use_item.panel = panel;
    // The hands plug into the dispenser while it heals (the game's
    // HandSignal: `SetupBactaHealer`), whether or not the body is held.
    let hands = healing.then(|| "bacta".to_string());
    if hands != use_item.hands {
        log.line(format!(
            "{PART}: t={now:.2} bacta: hands {}",
            if hands.is_some() {
                "plug in"
            } else {
                "come away"
            }
        ));
        use_item.hands = hands;
    }
    // The prompt, while standing at one that can be used.
    let prompt = matches!(panel, UsePanel::Ready(_))
        .then(|| assets.as_ref().and_then(|a| a.prompt.as_deref()))
        .flatten()
        .map(|p| {
            let key = prompts
                .as_ref()
                .and_then(|p| p.label(Action::Use))
                .unwrap_or("USE");
            // The original shows the button in quotes: 'F'.
            p.replace('@', &format!("'{}'", key.to_uppercase()))
        });
    if prompt != use_item.prompt {
        if let Some(p) = &prompt {
            log.line(format!("{PART}: t={now:.2} bacta: prompt \"{p}\""));
            if let Some(stats) = stats.as_mut() {
                stats.prompt = Some(p.clone());
            }
        }
        use_item.prompt = prompt;
    }
    // Sounds: the healing one repeats while healing.
    let sound_of = |k: &str| {
        assets
            .as_ref()
            .and_then(|a| a.sounds.get(k))
            .and_then(|g| g.pick())
    };
    if healing && healing_sound.is_empty() {
        if let Some(h) = sound_of("healing") {
            commands.spawn((
                AudioPlayer(h),
                PlaybackSettings::LOOP,
                HealingSound,
                DespawnOnExit(GameState::InGame),
            ));
            play.push("healing");
        }
    } else if !healing {
        for e in &healing_sound {
            commands.entity(e).despawn();
        }
    }
    for k in play {
        if k != "healing" {
            if let Some(h) = sound_of(k) {
                commands.spawn((
                    AudioPlayer(h),
                    PlaybackSettings::DESPAWN,
                    DespawnOnExit(GameState::InGame),
                ));
            }
        }
        log.line(format!("{PART}: t={now:.2} bacta: sound {k}"));
        if let Some(stats) = stats.as_mut() {
            stats.sounds.push(k.to_string());
        }
    }
}

/// Whether a line from `eye` along `dir` (Unreal units, `dir` any
/// length) passes through a dispenser's collision cylinder at `centre`
/// within `range`.
pub fn aims_at(
    eye: rc_movement::Vec3,
    dir: rc_movement::Vec3,
    centre: [f32; 3],
    range: f32,
) -> bool {
    aims_at_cylinder(eye, dir, centre, AIM_RADIUS, HALF_HEIGHT, range)
}

/// Whether a line from `eye` along `dir` passes through an upright
/// cylinder (`radius`, `half_height` above and below `centre`) within
/// `range`.
pub fn aims_at_cylinder(
    eye: rc_movement::Vec3,
    dir: rc_movement::Vec3,
    centre: [f32; 3],
    radius: f32,
    half_height: f32,
    range: f32,
) -> bool {
    let flat = (dir.x * dir.x + dir.y * dir.y).sqrt();
    if flat < 1e-6 {
        return false;
    }
    let len = (flat * flat + dir.z * dir.z).sqrt();
    // Nearest approach across the floor.
    let (fx, fy) = (dir.x / flat, dir.y / flat);
    let (tx, ty) = (centre[0] - eye.x, centre[1] - eye.y);
    let along = tx * fx + ty * fy;
    if along <= 0.0 {
        return false;
    }
    let side = (tx * fy - ty * fx).abs();
    // Height of the line there.
    let z = eye.z + dir.z / flat * along;
    let dist = along * len / flat;
    side < radius && (z - centre[2]).abs() < half_height && dist < range
}

/// Where a dispenser at `location` facing `yaw` holds the player: `offset`
/// units in front of it, facing it.
pub fn lock_for(location: [f32; 3], yaw: f32, offset: f32) -> UseLock {
    let (sin, cos) = yaw.sin_cos();
    UseLock {
        stand: [location[0] + cos * offset, location[1] + sin * offset],
        face_yaw: (yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU),
        speed: 300.0,
        turn: std::f32::consts::TAU,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aiming_at_a_dispenser() {
        let eye = rc_movement::Vec3::new(0.0, 0.0, 60.0);
        let at = [300.0, 0.0, 0.0];
        assert!(aims_at(
            eye,
            rc_movement::Vec3::new(1.0, 0.0, 0.0),
            at,
            3000.0
        ));
        // Off to the side, behind, too far, or looking over the top.
        assert!(!aims_at(
            eye,
            rc_movement::Vec3::new(1.0, 0.3, 0.0),
            at,
            3000.0
        ));
        assert!(!aims_at(
            eye,
            rc_movement::Vec3::new(-1.0, 0.0, 0.0),
            at,
            3000.0
        ));
        assert!(!aims_at(
            eye,
            rc_movement::Vec3::new(1.0, 0.0, 0.0),
            at,
            200.0
        ));
        assert!(!aims_at(
            eye,
            rc_movement::Vec3::new(1.0, 0.0, 1.0),
            at,
            3000.0
        ));
    }

    #[test]
    fn the_player_stands_in_front_facing_the_dispenser() {
        // geo_01a's BactaDispenserGEO4 faces -88 degrees; its squad spot is
        // 135 units that way, so the front is that way.
        let yaw = -16043.0 * std::f32::consts::TAU / 65536.0;
        let l = lock_for([-85.0, -6282.4, -2331.4], yaw, 90.0);
        assert!((l.stand[0] - -82.0).abs() < 2.0, "{:?}", l.stand);
        assert!((l.stand[1] - -6372.4).abs() < 1.0, "{:?}", l.stand);
        // Facing back towards the dispenser: about +92 degrees.
        assert!(
            (l.face_yaw.to_degrees() - 91.9).abs() < 0.5,
            "{}",
            l.face_yaw.to_degrees()
        );
    }
}
