//! Proximity mines in the game (geo_05c's eight, on explosive barrels).
//! The map's mines are read by `rc_level::mines`; how one behaves step by
//! step is `rc_scripts::mines`, which follows the original's script. This
//! part carries it out:
//!
//! - the squad marker's job (`MarkerJob`): `Placed` sets the mine, `Armed`
//!   arms it for the squad's side (both sent partway through Scorch's
//!   placing animation, at the moments its notifies say);
//! - each frame, an armed mine looks for enemies within its radius who
//!   aren't crouched or dead, chirps, and explodes after the alert time;
//! - shots, grenade blasts and other explosions that reach a set mine set
//!   it off; before it is set they go to its barrel instead;
//! - exploding: `ChargeExplode`'s effect and sound, a blast that hurts
//!   characters (`rc_combat::Explode`) and props, the barrel destroyed
//!   (`PropToDestroy`), the mine taken away and its squad marker closed
//!   (`AnchorGone`);
//! - each step's sound, and the mine's material for the step (see `show`).
//!
//! Everything goes to the log with a `mines:` prefix.

use crate::breakables::{bounds, hurt, hurt_around, in_box, to_bevy, to_unreal};
use crate::LevelScript;
use bevy::prelude::*;
use rc_combat::grenade::blast_scale;
use rc_combat::{
    Blast, Crouched, DamageKind, Dead, Explode, Hittable, ShotLanded, Team, WeaponLibrary,
};
use rc_effects_plugin::{PreloadEffect, SpawnEffect};
use rc_game_core::{AnchorGone, GameLog, LevelCollision, MarkerJob, MarkerJobStep};
use rc_level::mines::{MineInfo, Stage};
use rc_level_plugin::LevelParts;
use rc_scripts::mines::{MarkerStep, MineEvent, MineRun, Phase};

/// How far (uu) outside a mine's box a shot may land and still hit it.
const HIT_SLACK: f32 = 4.0;

/// Our number for a side, for `MineRun::team`.
pub(crate) fn team_index(t: Team) -> i32 {
    match t {
        Team::Republic => 0,
        Team::Separatist => 1,
        Team::Neutral => 2,
    }
}

/// One mine of the level.
#[derive(Debug, Clone)]
pub struct Slot {
    pub info: MineInfo,
    pub run: MineRun,
    pub marker: MarkerStep,
    /// Its barrel's export, if the map has it.
    pub barrel: Option<usize>,
    /// The material its look should have now (full path), and whether
    /// `show` must redraw it.
    pub material: Option<Vec<String>>,
    pub redraw: bool,
}

/// The level's mines, and what they did (for the log and self-tests).
#[derive(Debug, Clone, Default)]
pub struct Mines {
    pub list: Vec<Slot>,
    /// "ProximityMine16 Placed", "... Armed", "... Explode", in order.
    pub steps: Vec<String>,
}

impl Mines {
    pub fn new(list: &[MineInfo], barrel_export: &dyn Fn(&str) -> Option<usize>) -> Self {
        Mines {
            list: list
                .iter()
                .filter(|m| !m.hidden)
                .map(|m| Slot {
                    run: MineRun::new(
                        m.team_index,
                        m.trigger_radius,
                        m.alert_time_limit,
                        m.armed.duration,
                    ),
                    marker: MarkerStep::default(),
                    barrel: m.prop_to_destroy.as_deref().and_then(barrel_export),
                    material: m.unplaced.material_path.clone(),
                    redraw: true,
                    info: m.clone(),
                })
                .collect(),
            steps: Vec::new(),
        }
    }

    /// Whether `anchor` is one of the mines.
    pub fn knows(&self, anchor: &str) -> bool {
        self.list
            .iter()
            .any(|s| s.info.name.eq_ignore_ascii_case(anchor))
    }
}

/// Every effect and sound a mine can need, read while the level loads.
pub(crate) fn preload(list: &[MineInfo], out: &mut MessageWriter<PreloadEffect>) {
    let mut seen = std::collections::HashSet::new();
    for m in list {
        for s in [&m.unplaced, &m.placed, &m.armed, &m.explode] {
            let key = (s.effect.clone(), s.sound_path.clone());
            if (key.0.is_some() || key.1.is_some()) && seen.insert(key) {
                out.write(PreloadEffect {
                    effect: s.effect.clone(),
                    sound: s.sound_path.clone(),
                });
            }
        }
        for s in [&m.activate_sound, &m.alert_sound] {
            let key = (None, s.clone());
            if s.is_some() && seen.insert(key) {
                out.write(PreloadEffect {
                    effect: None,
                    sound: s.clone(),
                });
            }
        }
    }
}

/// What a mine's step asks for, gathered while the mines are borrowed.
enum Todo {
    Effect(SpawnEffect),
    Blast(usize),
    Removed(usize),
}

fn stage_effect(m: &MineInfo, s: &Stage) -> Option<SpawnEffect> {
    if s.effect.is_none() && s.sound_path.is_none() {
        return None;
    }
    let at = [0, 1, 2].map(|k| m.location[k] + s.effect_offset[k]);
    Some(SpawnEffect {
        effect: s.effect.clone().unwrap_or_default(),
        at: to_bevy(Vec3::from_array(at)),
        sound: s.sound_path.clone(),
        by: m.name.clone(),
        ..default()
    })
}

fn sound(m: &MineInfo, path: &Option<Vec<String>>) -> Option<SpawnEffect> {
    path.as_ref().map(|p| SpawnEffect {
        effect: String::new(),
        at: to_bevy(Vec3::from_array(m.location)),
        sound: Some(p.clone()),
        by: m.name.clone(),
        ..default()
    })
}

/// Turn a mine's events into sounds, looks and the blast.
fn carry_out(
    slot: &mut Slot,
    k: usize,
    events: Vec<MineEvent>,
    now: f32,
    todo: &mut Vec<Todo>,
    steps: &mut Vec<String>,
    log: &GameLog,
) {
    for e in events {
        let m = &slot.info;
        let fx = match &e {
            MineEvent::Placed => {
                if m.placed.material_path.is_some() {
                    slot.material = m.placed.material_path.clone();
                    slot.redraw = true;
                }
                stage_effect(m, &m.placed)
            }
            MineEvent::Armed => {
                if m.armed.material_path.is_some() {
                    slot.material = m.armed.material_path.clone();
                    slot.redraw = true;
                }
                stage_effect(m, &m.armed)
            }
            MineEvent::Watching => sound(m, &m.activate_sound),
            MineEvent::Chirp => sound(m, &m.alert_sound),
            MineEvent::Explode => {
                todo.push(Todo::Blast(k));
                stage_effect(m, &m.explode)
            }
            MineEvent::Removed => {
                todo.push(Todo::Removed(k));
                None
            }
        };
        log.line(format!(
            "mines: t={now:.2} {} {:?}{}",
            m.name,
            e,
            match &e {
                MineEvent::Armed => format!(
                    " (side {}): watches in {:.1} s for enemies within {} uu",
                    slot.run.team, m.armed.duration, m.trigger_radius
                ),
                MineEvent::Watching => format!(
                    "; sound {}",
                    m.activate_sound
                        .as_ref()
                        .map(|p| p.join("."))
                        .unwrap_or_default()
                ),
                _ => String::new(),
            }
        ));
        if e != MineEvent::Chirp {
            steps.push(format!("{} {:?}", m.name, e));
        }
        if let Some(fx) = fx {
            todo.push(Todo::Effect(fx));
        }
    }
}

/// Run the mines (see the top of this file).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn run(
    time: Res<Time>,
    script: Option<ResMut<LevelScript>>,
    parts: Option<Res<LevelParts>>,
    level: Option<Res<LevelCollision>>,
    lib: Option<Res<WeaponLibrary>>,
    mut jobs: MessageReader<MarkerJob>,
    mut shots: MessageReader<ShotLanded>,
    mut blasts: MessageReader<Blast>,
    mut explosions: MessageReader<Explode>,
    people: Query<(
        &Transform,
        &Team,
        Option<&Hittable>,
        Has<Dead>,
        Has<Crouched>,
    )>,
    mut effects: MessageWriter<SpawnEffect>,
    mut explode: MessageWriter<Explode>,
    mut gone: MessageWriter<AnchorGone>,
    log: Res<GameLog>,
) {
    let Some(mut script) = script else {
        jobs.clear();
        shots.clear();
        blasts.clear();
        explosions.clear();
        return;
    };
    let script = &mut *script;
    if script.mines.list.is_empty() {
        jobs.clear();
        shots.clear();
        blasts.clear();
        explosions.clear();
        return;
    }
    let now = script.director.time;
    let dt = time.delta_secs().min(0.25);
    let mut todo: Vec<Todo> = Vec::new();
    // Barrels to pass damage on to: (export, amount, by).
    let mut barrel_hits: Vec<(usize, f32, String)> = Vec::new();
    let mut steps = std::mem::take(&mut script.mines.steps);

    // The squad marker's job: set, then arm.
    for job in jobs.read() {
        let Some(k) = script
            .mines
            .list
            .iter()
            .position(|s| s.info.name.eq_ignore_ascii_case(&job.anchor))
        else {
            continue;
        };
        let placing = match job.step {
            MarkerJobStep::Placed => true,
            MarkerJobStep::Armed => false,
            _ => continue,
        };
        let slot = &mut script.mines.list[k];
        let Some(placing) = slot.marker.take(placing) else {
            log.line(format!(
                "mines: t={now:.2} {} {:?} by {} ignored (out of order or done)",
                slot.info.name, job.step, job.by
            ));
            continue;
        };
        // Squad mates are on the Republic's side.
        let events = if placing {
            slot.run.place()
        } else {
            slot.run.arm(team_index(Team::Republic))
        };
        log.line(format!(
            "mines: t={now:.2} {} {} by {}",
            slot.info.name,
            if placing { "set" } else { "armed" },
            job.by
        ));
        carry_out(slot, k, events, now, &mut todo, &mut steps, &log);
    }

    // What reaches each mine: shots on it, grenades, explosions.
    let mine_box = |slot: &Slot| -> (Vec3, Vec3) {
        parts
            .as_deref()
            .and_then(|p| bounds(p, slot.info.export))
            .unwrap_or_else(|| {
                let c = Vec3::from_array(slot.info.location);
                (c - Vec3::splat(8.0), c + Vec3::splat(8.0))
            })
    };
    let mut hits: Vec<(usize, f32, String)> = Vec::new();
    for s in shots.read() {
        if !s.on_level || s.damage <= 0.0 {
            continue;
        }
        let p = to_unreal(s.point);
        for (k, slot) in script.mines.list.iter().enumerate() {
            if slot.run.present() && in_box(p, mine_box(slot), HIT_SLACK) {
                hits.push((k, s.damage, format!("a {} shot", s.weapon)));
            }
        }
    }
    let mut area: Vec<(Vec3, f32, f32, String)> = Vec::new();
    for b in blasts.read() {
        if let Some(g) = lib.as_deref().and_then(|l| l.grenades.get(&b.grenade)) {
            if g.damage > 0.0 {
                area.push((
                    to_unreal(b.point),
                    g.damage,
                    g.radius,
                    format!("a {} blast", b.grenade),
                ));
            }
        }
    }
    for x in explosions.read() {
        if x.damage > 0.0 && !script.mines.knows(&x.what) {
            area.push((
                to_unreal(x.point),
                x.damage,
                x.radius,
                format!("{}'s explosion", x.what),
            ));
        }
    }
    for (point, damage, radius, by) in &area {
        for (k, slot) in script.mines.list.iter().enumerate() {
            if !slot.run.present() {
                continue;
            }
            let bx = mine_box(slot);
            let centre = (bx.0 + bx.1) * 0.5;
            let size = (bx.1 - bx.0) * 0.5;
            let scale = blast_scale((centre - *point).length(), size.x.max(size.y), *radius);
            if scale <= 0.0 {
                continue;
            }
            let v = |p: Vec3| rc_movement::Vec3::new(p.x, p.y, p.z);
            let seen = level.as_deref().is_none_or(|l| {
                match l.0.sweep(rc_movement::Vec3::ZERO, v(*point), v(centre)) {
                    None => true,
                    Some(h) => in_box(*point + (centre - *point) * h.time, bx, HIT_SLACK),
                }
            });
            if seen {
                hits.push((k, damage * scale, by.clone()));
            }
        }
    }
    for (k, amount, by) in hits {
        let slot = &mut script.mines.list[k];
        if !slot.run.present() {
            continue;
        }
        let (events, pass_on) = slot.run.hurt();
        if !events.is_empty() {
            log.line(format!(
                "mines: t={now:.2} {by} hits {} ({amount:.0}): it goes off",
                slot.info.name
            ));
        }
        if pass_on {
            if let Some(b) = slot.barrel {
                barrel_hits.push((b, amount, format!("{by} (through {})", slot.info.name)));
            }
        }
        carry_out(slot, k, events, now, &mut todo, &mut steps, &log);
    }

    // Unplaced mines go with their barrel.
    for k in 0..script.mines.list.len() {
        let gone_barrel = script.mines.list[k]
            .barrel
            .is_some_and(|b| script.hidden.contains(&b));
        if gone_barrel {
            let slot = &mut script.mines.list[k];
            let events = slot.run.barrel_gone();
            carry_out(slot, k, events, now, &mut todo, &mut steps, &log);
        }
    }

    // Armed mines watch for enemies.
    let enemy_near = |team: i32, at: [f32; 3], radius: f32| -> bool {
        people.iter().any(|(t, side, hit, dead, crouched)| {
            let side = team_index(*side);
            if dead || crouched || side == team || side == team_index(Team::Neutral) {
                return false;
            }
            // The pawn's Location: the middle of its body.
            let p = to_unreal(t.translation) + Vec3::Z * hit.map_or(0.0, |h| h.center_up);
            (p - Vec3::from_array(at)).length_squared() < radius * radius
        })
    };
    for k in 0..script.mines.list.len() {
        let slot = &mut script.mines.list[k];
        if !slot.run.present() {
            continue;
        }
        let at = slot.info.location;
        let events = slot.run.tick(dt, &mut |team, r| enemy_near(team, at, r));
        carry_out(slot, k, events, now, &mut todo, &mut steps, &log);
    }

    script.mines.steps = steps;
    for (b, amount, by) in barrel_hits {
        hurt(script, b, amount, &by, &log);
    }
    for t in todo {
        match t {
            Todo::Effect(fx) => {
                effects.write(fx);
            }
            Todo::Blast(k) => {
                let m = script.mines.list[k].info.clone();
                let x = &m.explode;
                let point = Vec3::from_array(m.location);
                let ty = x
                    .damage_type
                    .clone()
                    .unwrap_or_else(|| "CTDamageExplosion".into());
                if x.damage > 0.0 && x.damage_radius > 0.0 {
                    explode.write(Explode {
                        point: to_bevy(point),
                        damage: x.damage,
                        radius: x.damage_radius,
                        damage_type: ty.clone(),
                        kind: DamageKind::Explosion,
                        instigator: None,
                        what: m.name.clone(),
                    });
                    if let Some(p) = parts.as_deref() {
                        hurt_around(
                            script,
                            p,
                            level.as_deref(),
                            lib.as_deref(),
                            point,
                            x.damage,
                            x.damage_radius,
                            &ty,
                            None,
                            &format!("{}'s blast", m.name),
                            &log,
                        );
                    }
                }
                // PropToDestroy.GotoState('PropDestroyed').
                if let Some(b) = &m.prop_to_destroy {
                    let done = script.director.destroy_prop_named(b);
                    log.line(format!(
                        "mines: t={now:.2} {} explodes (damage {}, radius {}, {ty}); its barrel {b} {}",
                        m.name,
                        x.damage,
                        x.damage_radius,
                        if done { "is destroyed" } else { "was already gone" }
                    ));
                }
                take_away(script, parts.as_deref(), &m, "it exploded", &mut gone, &log);
            }
            Todo::Removed(k) => {
                let m = script.mines.list[k].info.clone();
                take_away(
                    script,
                    parts.as_deref(),
                    &m,
                    "its barrel blew up first",
                    &mut gone,
                    &log,
                );
            }
        }
    }
}

fn take_away(
    script: &mut LevelScript,
    parts: Option<&LevelParts>,
    m: &MineInfo,
    why: &str,
    gone: &mut MessageWriter<AnchorGone>,
    log: &GameLog,
) {
    if let Some(p) = parts {
        p.set_solid_export(m.export, false);
    }
    script.hidden.insert(m.export);
    if let Some(s) = script
        .mines
        .list
        .iter_mut()
        .find(|s| s.info.export == m.export)
    {
        s.redraw = true;
    }
    log.line(format!("mines: {} taken away: {why}", m.name));
    gone.write(AnchorGone {
        anchor: m.name.clone(),
        why: format!("{} {why}", m.name),
    });
}

/// A mine's own model, drawn with the material of its step (the level's
/// copy of it is hidden while this one stands).
#[derive(Component, Debug, Clone)]
pub struct MineLook {
    pub export: usize,
}

/// Draw each mine with its step's material: the hologram while unplaced
/// (`ChargeUnplaced`), then `ChargePlaced`'s and `ChargeArmed`'s. Gone
/// mines lose their look. Where the model or the material can't be read,
/// the level's own drawing of the mine stays.
#[allow(clippy::too_many_arguments)]
pub(crate) fn show(
    mut commands: Commands,
    script: Option<ResMut<LevelScript>>,
    paths: Option<Res<rc_game_core::GamePaths>>,
    looks: Query<(Entity, &MineLook)>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    images: Option<ResMut<Assets<Image>>>,
    log: Res<GameLog>,
) {
    let Some(mut script) = script else { return };
    if !script.mines.list.iter().any(|s| s.redraw) {
        return;
    }
    let (Some(paths), Some(mut meshes), Some(mut materials), Some(mut images)) =
        (paths, meshes, materials, images)
    else {
        return;
    };
    let mut mats = rc_level_viewer::materials::Materials::new(&paths.game_data());
    let script = &mut *script;
    for k in 0..script.mines.list.len() {
        if !script.mines.list[k].redraw {
            continue;
        }
        script.mines.list[k].redraw = false;
        let slot = script.mines.list[k].clone();
        let export = slot.info.export;
        for (e, l) in &looks {
            if l.export == export {
                commands.entity(e).despawn();
            }
        }
        if !slot.run.present() || slot.run.phase == Phase::Exploded {
            continue;
        }
        let Some(path) = &slot.info.mesh_path else {
            continue;
        };
        let sections = match rc_effects_plugin::static_mesh(&mut mats, path) {
            Ok(s) => s,
            Err(e) => {
                log.line(format!(
                    "mines: {} model {} can't be read: {e}; the level draws it",
                    slot.info.name,
                    path.join(".")
                ));
                continue;
            }
        };
        let swap = slot.material.as_ref().and_then(|p| {
            let (pkg_name, rest) = p.split_first()?;
            let loaded = mats.package(pkg_name)?;
            let i = rc_level_viewer::materials::find_export(&loaded.0, rest)?;
            match mats.resolve(&loaded.0, &loaded.1, i as i32 + 1) {
                Ok(pic) => Some(pic),
                Err(e) => {
                    log.line(format!(
                        "mines: {} material {} can't be drawn ({e:?}); its model's own is used",
                        slot.info.name,
                        p.join(".")
                    ));
                    None
                }
            }
        });
        let index = match slot.run.phase {
            Phase::Unplaced => slot.info.unplaced.material_index,
            Phase::Placed => slot.info.placed.material_index,
            _ => slot.info.armed.material_index,
        };
        for (si, (mut mesh, pic)) in sections.into_iter().enumerate() {
            if let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) =
                mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
            {
                for v in p.iter_mut() {
                    let w = slot.info.to_level([v[0], v[2], v[1]]);
                    *v = to_bevy(Vec3::from_array(w)).to_array();
                }
            }
            let pic = if si == index {
                swap.clone().or(pic)
            } else {
                pic
            };
            let material = StandardMaterial {
                base_color_texture: pic.map(|p| images.add(rc_level_viewer::draw::to_image(&p))),
                unlit: true,
                double_sided: true,
                cull_mode: None,
                alpha_mode: AlphaMode::Blend,
                ..default()
            };
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(materials.add(material)),
                Transform::IDENTITY,
                MineLook { export },
                DespawnOnExit(rc_game_core::GameState::InGame),
            ));
        }
        script.hidden.insert(export);
        log.line(format!(
            "mines: {} drawn as {} with {} ({:?})",
            slot.info.name,
            path.join("."),
            slot.material
                .as_ref()
                .map(|p| p.join("."))
                .unwrap_or_else(|| "its own material".into()),
            slot.run.phase
        ));
    }
}
