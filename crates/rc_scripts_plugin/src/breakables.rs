//! Props weapons can break: barrels, fuel dispensers and their clamps,
//! holo machines, consoles, rock cover (`rc_level::breakables` reads them,
//! `rc_scripts::Director` keeps their health and sends their events).
//!
//! This part carries out the damage:
//!
//! - a shot that lands on the level (`rc_combat::ShotLanded`) hurts the
//!   breakable prop it landed on, by the shot's damage times the prop's
//!   damage table;
//! - a grenade's blast (`rc_combat::Blast`) hurts every breakable prop in
//!   reach that it can see, falling off with distance as it does for
//!   characters;
//! - a prop with an explosion (barrels, dispensers) goes off a moment
//!   after it breaks (0.1 to 0.2 s, as in the game): it sends
//!   `rc_combat::Explode` (which hurts the characters around it) and hurts
//!   the breakable props around it the same way, so barrels set each other
//!   off.
//!
//! Everything goes to the log with a `props:` prefix.

use crate::LevelScript;
use bevy::prelude::*;
use rc_combat::grenade::blast_scale;
use rc_combat::{Blast, DamageKind, Explode, ShotLanded, WeaponLibrary};
use rc_effects_plugin::{PreloadEffect, SpawnEffect};
use rc_game_core::GamePaths;
use rc_level_viewer::draw::MoverPart;
use rc_game_core::units;
use rc_game_core::{GameLog, LevelCollision};
use rc_level::breakables::Breakable;
use rc_level_plugin::LevelParts;
use rc_scripts::PropHurt;
use std::collections::HashMap;

/// How far (uu) outside a prop's box a shot may land and still count as
/// hitting it: the box is made from its collision triangles, and a shot
/// stops right on their surface.
const HIT_SLACK: f32 = 4.0;

/// The breakable props of the level, and explosions about to go off.
#[derive(Default)]
pub struct Breakables {
    by_export: HashMap<usize, Breakable>,
    /// (director time it goes off, export).
    due: Vec<(f32, usize)>,
    /// Props that just entered a step (export, true for broken), whose
    /// effect, sound and model `show` puts on.
    pub(crate) stepped: Vec<(usize, bool)>,
}

impl Breakables {
    pub fn new(list: &[Breakable]) -> Self {
        Breakables {
            by_export: list.iter().map(|b| (b.export, b.clone())).collect(),
            due: Vec::new(),
            stepped: Vec::new(),
        }
    }

    pub fn get(&self, export: usize) -> Option<&Breakable> {
        self.by_export.get(&export)
    }

    /// Every breakable prop, in no order.
    pub fn iter(&self) -> impl Iterator<Item = &Breakable> {
        self.by_export.values()
    }

    /// A prop just broke: if it explodes, it goes off `delay` seconds
    /// from `now`.
    pub fn broke(&mut self, export: usize, now: f32, delay: f32) -> bool {
        let explodes = self
            .by_export
            .get(&export)
            .and_then(|b| b.explosion.as_ref())
            .is_some_and(|x| x.damage > 0.0 && x.radius > 0.0);
        if explodes {
            self.due.push((now + delay, export));
        }
        explodes
    }
}

/// What weapons did to props, for the log and the self-tests.
#[derive(Debug, Clone, Default)]
pub struct PropStats {
    /// Shots and blasts that reached a breakable prop.
    pub hits: usize,
    /// Props broken by damage (not by a script), by name.
    pub broken: Vec<String>,
    /// Props that exploded, by name.
    pub exploded: Vec<String>,
    /// Props whose broken model was put in their place, by name.
    pub broken_looks: Vec<String>,
}

fn in_box(p: Vec3, (lo, hi): (Vec3, Vec3), slack: f32) -> bool {
    (0..3).all(|i| p[i] >= lo[i] - slack && p[i] <= hi[i] + slack)
}

fn to_vec(v: rc_movement::Vec3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

/// A Bevy position in Unreal units (kept as a Bevy `Vec3` for the maths).
fn to_unreal(p: Vec3) -> Vec3 {
    to_vec(units::to_unreal(p))
}

/// Unreal units back to a Bevy position.
fn to_bevy(p: Vec3) -> Vec3 {
    units::to_bevy(rc_movement::Vec3::new(p.x, p.y, p.z))
}

fn bounds(parts: &LevelParts, export: usize) -> Option<(Vec3, Vec3)> {
    parts
        .bounds_of(export)
        .map(|(lo, hi)| (to_vec(lo), to_vec(hi)))
}

/// Hurt one prop and log what happened.
fn hurt(script: &mut LevelScript, export: usize, amount: f32, by: &str, log: &GameLog) {
    let now = script.director.time;
    let name = script
        .breaking
        .get(export)
        .map(|b| b.name.clone())
        .unwrap_or_else(|| format!("export {export}"));
    let r = script.director.hurt_prop(export, amount);
    if matches!(r, PropHurt::NotBreakable | PropHurt::AlreadyBroken) {
        return;
    }
    script.props.hits += 1;
    let what = match r {
        PropHurt::Hurt(left) => format!("{left:.0} health left"),
        PropHurt::Damaged => "damaged".into(),
        PropHurt::Broken => {
            script.props.broken.push(name.clone());
            "broken".into()
        }
        _ => unreachable!(),
    };
    log.line(format!(
        "props: t={now:.2} {by} hit {name} for {amount:.1}: {what}"
    ));
}

/// Hurt every standing breakable prop a blast at `point` (Unreal units)
/// can see within `radius`, as `HurtRadius` does for characters.
#[allow(clippy::too_many_arguments)]
fn hurt_around(
    script: &mut LevelScript,
    parts: &LevelParts,
    level: Option<&LevelCollision>,
    lib: Option<&WeaponLibrary>,
    point: Vec3,
    damage: f32,
    radius: f32,
    damage_type: &str,
    skip: Option<usize>,
    by: &str,
    log: &GameLog,
) {
    let mut hits = Vec::new();
    for (&export, b) in &script.breaking.by_export {
        if Some(export) == skip || b.hidden {
            continue;
        }
        let Some(bx) = bounds(parts, export) else {
            continue;
        };
        let centre = (bx.0 + bx.1) * 0.5;
        let size = (bx.1 - bx.0) * 0.5;
        let prop_radius = size.x.max(size.y);
        let dist = (centre - point).length();
        let scale = blast_scale(dist, prop_radius, radius);
        if scale <= 0.0 {
            continue;
        }
        // Anything solid in between, other than the prop itself, keeps the
        // blast off.
        let v = |p: Vec3| rc_movement::Vec3::new(p.x, p.y, p.z);
        let seen =
            level.is_none_or(
                |l| match l.0.sweep(rc_movement::Vec3::ZERO, v(point), v(centre)) {
                    None => true,
                    Some(h) => in_box(point + (centre - point) * h.time, bx, HIT_SLACK),
                },
            );
        if !seen {
            continue;
        }
        let m = lib.map_or(1.0, |l| {
            l.damage_types
                .multiplier(b.damage_table.as_deref(), damage_type)
        });
        hits.push((export, damage * scale * m));
    }
    hits.sort_by_key(|h| h.0);
    for (export, amount) in hits {
        if amount > 0.0 && script.director.prop_health(export).is_some() {
            hurt(script, export, amount, by, log);
        }
    }
}

/// Shots that land on a breakable prop hurt it.
pub(crate) fn shots(
    mut landed: MessageReader<ShotLanded>,
    script: Option<ResMut<LevelScript>>,
    parts: Option<Res<LevelParts>>,
    lib: Option<Res<WeaponLibrary>>,
    log: Res<GameLog>,
) {
    let (Some(mut script), Some(parts)) = (script, parts) else {
        landed.clear();
        return;
    };
    for s in landed.read() {
        if !s.on_level || s.damage <= 0.0 {
            continue;
        }
        let point = to_unreal(s.point);
        // The smallest standing breakable prop the shot landed on.
        let mut best: Option<(usize, f32)> = None;
        for (&export, b) in &script.breaking.by_export {
            if b.hidden || script.hidden.contains(&export) {
                continue;
            }
            let Some(bx) = bounds(&parts, export) else {
                continue;
            };
            if in_box(point, bx, HIT_SLACK) {
                let size = bx.1 - bx.0;
                let volume = size.x * size.y * size.z;
                if best.is_none_or(|(_, v)| volume < v) {
                    best = Some((export, volume));
                }
            }
        }
        let Some((export, _)) = best else { continue };
        let table = script
            .breaking
            .get(export)
            .and_then(|b| b.damage_table.clone());
        let m = lib.as_deref().map_or(1.0, |l| {
            l.damage_types.multiplier(table.as_deref(), &s.damage_type)
        });
        let by = format!("a {} shot", s.weapon);
        hurt(&mut script, export, s.damage * m, &by, &log);
    }
}

/// Grenade blasts hurt the breakable props around them.
pub(crate) fn blasts(
    mut blasts: MessageReader<Blast>,
    script: Option<ResMut<LevelScript>>,
    parts: Option<Res<LevelParts>>,
    level: Option<Res<LevelCollision>>,
    lib: Option<Res<WeaponLibrary>>,
    log: Res<GameLog>,
) {
    let (Some(mut script), Some(parts)) = (script, parts) else {
        blasts.clear();
        return;
    };
    for b in blasts.read() {
        // Only thrown grenades: props' own explosions hurt props when they
        // go off (`explode`), and their `Blast` is only for hearing.
        let Some(g) = lib.as_deref().and_then(|l| l.grenades.get(&b.grenade)) else {
            continue;
        };
        let (damage, radius, ty) = (g.damage, g.radius, g.damage_type.clone());
        if damage <= 0.0 {
            continue;
        }
        let by = format!("a {} blast", b.grenade);
        hurt_around(
            &mut script,
            &parts,
            level.as_deref(),
            lib.as_deref(),
            to_unreal(b.point),
            damage,
            radius,
            &ty,
            None,
            &by,
            &log,
        );
    }
}

/// Props whose explosion is due go off.
pub(crate) fn explode(
    script: Option<ResMut<LevelScript>>,
    parts: Option<Res<LevelParts>>,
    level: Option<Res<LevelCollision>>,
    lib: Option<Res<WeaponLibrary>>,
    mut out: MessageWriter<Explode>,
    log: Res<GameLog>,
) {
    let (Some(mut script), Some(parts)) = (script, parts) else {
        return;
    };
    let now = script.director.time;
    if !script.breaking.due.iter().any(|(t, _)| *t <= now) {
        return;
    }
    let (ready, later): (Vec<_>, Vec<_>) = std::mem::take(&mut script.breaking.due)
        .into_iter()
        .partition(|(t, _)| *t <= now);
    script.breaking.due = later;
    for (_, export) in ready {
        let Some(b) = script.breaking.get(export).cloned() else {
            continue;
        };
        let Some(x) = &b.explosion else { continue };
        // From the middle of the prop (its box), else where it stands.
        let point = bounds(&parts, export)
            .map(|(lo, hi)| (lo + hi) * 0.5)
            .unwrap_or(Vec3::from_array(b.location));
        log.line(format!(
            "props: t={now:.2} {} ({}) explodes: damage {}, radius {}, type {}{}",
            b.name,
            b.class,
            x.damage,
            x.radius,
            x.damage_type,
            x.status
                .as_ref()
                .map(|s| format!("; {s} for up to {} s (not done yet)", x.status_secs))
                .unwrap_or_default()
        ));
        script.props.exploded.push(b.name.clone());
        out.write(Explode {
            point: to_bevy(point),
            damage: x.damage,
            radius: x.radius,
            damage_type: x.damage_type.clone(),
            kind: DamageKind::Explosion,
            instigator: None,
            what: b.name.clone(),
        });
        let by = format!("{}'s explosion", b.name);
        hurt_around(
            &mut script,
            &parts,
            level.as_deref(),
            lib.as_deref(),
            point,
            x.damage,
            x.radius,
            &x.damage_type,
            Some(export),
            &by,
            &log,
        );
    }
}

/// A broken prop's model, standing where the prop was until `until`
/// (level script time; `None`: for good).
#[derive(Component, Debug, Clone)]
pub struct BrokenLook {
    pub export: usize,
    pub until: Option<f32>,
}

/// Every effect and sound a breakable prop can need, read while the level
/// loads.
pub(crate) fn preload(list: &[Breakable], out: &mut MessageWriter<PreloadEffect>) {
    let mut seen = std::collections::HashSet::new();
    for b in list {
        for t in [&b.damaged, &b.destroyed] {
            let key = (t.effect.clone(), t.sound_path.clone());
            if (key.0.is_some() || key.1.is_some()) && seen.insert(key) {
                out.write(PreloadEffect {
                    effect: t.effect.clone(),
                    sound: t.sound_path.clone(),
                    def: None,
                });
            }
        }
    }
}

/// Props that entered a step: start its effect and sound, and put the
/// broken model in place.
#[allow(clippy::too_many_arguments)]
pub(crate) fn show(
    mut commands: Commands,
    script: Option<ResMut<LevelScript>>,
    parts: Option<Res<LevelParts>>,
    paths: Option<Res<GamePaths>>,
    mut effects: MessageWriter<SpawnEffect>,
    drawn: Query<(&MoverPart, &Mesh3d)>,
    looks: Query<(Entity, &BrokenLook)>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    images: Option<ResMut<Assets<Image>>>,
    log: Res<GameLog>,
) {
    let Some(mut script) = script else { return };
    let now = script.director.time;
    // Broken models whose time is up go.
    for (e, l) in &looks {
        if l.until.is_some_and(|t| now >= t) {
            commands.entity(e).despawn();
            log.line(format!("props: t={now:.2} broken model of export {} removed", l.export));
        }
    }
    if script.breaking.stepped.is_empty() {
        return;
    }
    let mut meshes = meshes;
    let mut materials = materials;
    let mut images = images;
    let mut mats = paths
        .as_ref()
        .map(|p| rc_level_viewer::materials::Materials::new(&p.game_data()));
    for (export, broken) in std::mem::take(&mut script.breaking.stepped) {
        let Some(b) = script.breaking.get(export).cloned() else {
            continue;
        };
        let step = if broken { &b.destroyed } else { &b.damaged };
        let at = [0, 1, 2].map(|k| b.location[k] + step.effect_offset[k]);
        let floor = parts
            .as_deref()
            .and_then(|p| bounds(p, export))
            .map(|(lo, _)| lo.z)
            .unwrap_or(b.location[2]);
        if step.effect.is_some() || step.sound_path.is_some() {
            effects.write(SpawnEffect {
                effect: step.effect.clone().unwrap_or_default(),
                at: to_bevy(Vec3::from_array(at)),
                floor: Some(to_bevy(Vec3::new(at[0], at[1], floor)).y),
                sound: step.sound_path.clone(),
                by: b.name.clone(),
                ..default()
            });
        }
        if !broken {
            continue;
        }
        let Some(path) = &step.mesh_path else {
            continue;
        };
        let (Some(m), Some(meshes), Some(materials), Some(images)) = (
            mats.as_mut(),
            meshes.as_deref_mut(),
            materials.as_deref_mut(),
            images.as_deref_mut(),
        ) else {
            continue;
        };
        // The intact prop's own stored light, averaged, lights the broken
        // one (it has none of its own).
        let mut sum = Vec4::ZERO;
        let mut n = 0.0;
        for (part, mesh) in &drawn {
            if part.0 != export {
                continue;
            }
            if let Some(bevy::mesh::VertexAttributeValues::Float32x4(c)) = meshes
                .get(&mesh.0)
                .and_then(|m| m.attribute(Mesh::ATTRIBUTE_COLOR))
            {
                for v in c {
                    sum += Vec4::from_array(*v);
                    n += 1.0;
                }
            }
        }
        let light = if n > 0.0 { sum / n } else { Vec4::ONE };
        match rc_effects_plugin::static_mesh(m, path) {
            Ok(sections) => {
                let until = (step.life_span > 0.0).then(|| {
                    let extra = step.life_span_max * ((export as f32 * 0.618).fract());
                    now + step.life_span + extra
                });
                for (mut mesh, pic) in sections {
                    // Model units to the level: placed like the prop.
                    if let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) =
                        mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
                    {
                        for v in p.iter_mut() {
                            // static_mesh gave Bevy axes (x, z, y); back to
                            // the model's own, then into the level.
                            let w = b.to_level([v[0], v[2], v[1]]);
                            *v = to_bevy(Vec3::from_array(w)).to_array();
                        }
                    }
                    let material = StandardMaterial {
                        base_color: Color::linear_rgb(light.x, light.y, light.z),
                        base_color_texture: pic.map(|p| images.add(rc_level_viewer::draw::to_image(&p))),
                        unlit: true,
                        double_sided: true,
                        cull_mode: None,
                        ..default()
                    };
                    commands.spawn((
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(materials.add(material)),
                        Transform::IDENTITY,
                        BrokenLook { export, until },
                        DespawnOnExit(rc_game_core::GameState::InGame),
                    ));
                }
                script.props.broken_looks.push(b.name.clone());
                log.line(format!(
                    "props: t={now:.2} {} shows its broken model {}{}",
                    b.name,
                    path.join("."),
                    until
                        .map(|t| format!(" until t={t:.2}"))
                        .unwrap_or_else(|| " for good".into())
                ));
            }
            Err(e) => log.line(format!(
                "props: t={now:.2} {}: broken model {} can't be read: {e}",
                b.name,
                path.join(".")
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::message::Messages;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn shot_on_a_box_surface_counts() {
        let b = (Vec3::ZERO, Vec3::splat(10.0));
        assert!(in_box(Vec3::new(10.0, 5.0, 5.0), b, HIT_SLACK));
        assert!(in_box(Vec3::new(13.0, 5.0, 5.0), b, HIT_SLACK));
        assert!(!in_box(Vec3::new(15.0, 5.0, 5.0), b, HIT_SLACK));
    }

    #[test]
    fn only_props_with_an_explosion_go_off() {
        use rc_level::breakables::{Explosion, Transition};
        let mut barrel = Breakable {
            export: 3,
            name: "ExplosiveBarrelGEO4".into(),
            class: "ExplosiveBarrelGEO".into(),
            tag: "ExplosiveBarrelGEO".into(),
            location: [0.0; 3],
            rotation: [0; 3],
            draw_scale: 1.0,
            draw_scale_3d: [1.0; 3],
            pre_pivot: [0.0; 3],
            health: 60.0,
            damaged_ratio: 0.0,
            event: None,
            damaged: Transition::default(),
            destroyed: Transition::default(),
            explosion: Some(Explosion {
                damage: 1024.0,
                radius: 640.0,
                damage_type: "CTDamageExplosionGeoBarrel".into(),
                status: None,
                status_secs: 0.0,
                status_damage_per_sec: 0.0,
            }),
            damage_table: None,
            hidden: false,
            anim_prop: false,
        };
        let mut console = barrel.clone();
        console.export = 4;
        console.explosion = None;
        barrel.name = "B".into();
        let mut b = Breakables::new(&[barrel, console]);
        assert!(b.broke(3, 1.0, 0.15));
        assert!(!b.broke(4, 1.0, 0.15));
        assert_eq!(b.due, vec![(1.15, 3)]);
    }

    /// A box of triangles from `lo` to `hi` (Unreal units), as a part's
    /// collision.
    fn cube(lo: [f32; 3], hi: [f32; 3]) -> Vec<[[f32; 3]; 3]> {
        let c = |i: usize| {
            [
                if i & 1 == 0 { lo[0] } else { hi[0] },
                if i & 2 == 0 { lo[1] } else { hi[1] },
                if i & 4 == 0 { lo[2] } else { hi[2] },
            ]
        };
        let faces = [
            [0, 1, 3, 2],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 3, 7, 6],
            [0, 2, 6, 4],
            [1, 3, 7, 5],
        ];
        faces
            .iter()
            .flat_map(|f| [[c(f[0]), c(f[1]), c(f[2])], [c(f[0]), c(f[2]), c(f[3])]])
            .collect()
    }

    fn barrel(export: usize, name: &str, at: [f32; 3]) -> Breakable {
        use rc_level::breakables::{Explosion, Transition};
        Breakable {
            export,
            name: name.into(),
            class: "ExplosiveBarrelGEO".into(),
            tag: "ExplosiveBarrelGEO".into(),
            location: at,
            rotation: [0; 3],
            draw_scale: 1.0,
            draw_scale_3d: [1.0; 3],
            pre_pivot: [0.0; 3],
            health: 60.0,
            damaged_ratio: 0.0,
            event: Some(format!("{name}Gone")),
            damaged: Transition::default(),
            destroyed: Transition::default(),
            explosion: Some(Explosion {
                damage: 1024.0,
                radius: 640.0,
                damage_type: "CTDamageExplosionGeoBarrel".into(),
                status: None,
                status_secs: 0.0,
                status_damage_per_sec: 0.0,
            }),
            damage_table: None,
            hidden: false,
            anim_prop: false,
        }
    }

    /// Two barrels 300 uu apart: three shots on the first break it, it
    /// explodes a moment later, sends `Explode` and sets the second one
    /// off; both send their events to the scripts.
    #[test]
    fn shots_break_a_barrel_and_it_sets_off_the_next() {
        use crate::{Objectives, PlayStats};
        use rc_level::scripts::MapScripts;
        use rc_movement::TriangleWorld;
        use rc_scripts::{Director, Settings};
        use std::sync::Arc;
        let mut w = World::new();
        w.insert_resource(GameLog::terminal());
        let list = vec![
            barrel(1, "BarrelA", [0.0, 0.0, 30.0]),
            barrel(2, "BarrelB", [300.0, 0.0, 30.0]),
        ];
        let tagged: HashMap<usize, (String, String)> = list
            .iter()
            .map(|b| (b.export, (b.class.clone(), b.tag.clone())))
            .collect();
        let tris: HashMap<usize, Vec<[[f32; 3]; 3]>> = [
            (1, cube([-20.0, -20.0, 0.0], [20.0, 20.0, 60.0])),
            (2, cube([280.0, -20.0, 0.0], [320.0, 20.0, 60.0])),
        ]
        .into();
        let parts = LevelParts::new(Arc::new(TriangleWorld::new(vec![])), &tagged, tris);
        w.insert_resource(LevelCollision(parts.world()));
        w.insert_resource(parts);
        let mut director = Director::new(&MapScripts::default(), &[], Settings::default());
        director.add_breakables(&list);
        w.insert_resource(LevelScript {
            director,
            dispensers: vec![],
            hidden: Default::default(),
            held: vec![],
            calls: vec![],
            places: Default::default(),
            sounds: vec![],
            ammo: vec![],
            started: true,
            scene: crate::cutscene::Scene::default(),
            stats: PlayStats::default(),
            breaking: Breakables::new(&list),
            anchors: vec![],
            props: PropStats::default(),
        });
        w.insert_resource(Objectives::default());
        w.init_resource::<Messages<ShotLanded>>();
        w.init_resource::<Messages<Explode>>();
        let shooter = w.spawn_empty().id();
        // Three DC-17m bolts (20 each) landing on the top of barrel A.
        for _ in 0..3 {
            w.write_message(ShotLanded {
                shooter,
                weapon: "dc17m_blaster".into(),
                point: to_bevy(Vec3::new(5.0, 0.0, 60.0)),
                target: None,
                on_level: true,
                distance: 500.0,
                damage: 20.0,
                damage_type: "CTDamageEnergyDC17".into(),
            });
        }
        let step = |w: &mut World, dt: f32| {
            w.run_system_once(shots).unwrap();
            w.run_system_once(explode).unwrap();
            let fx = w.resource_mut::<LevelScript>().director.step(dt, None);
            // What `run` does with a broken prop.
            for e in fx {
                if let rc_scripts::Effect::PropDestroyed { export, .. } = e {
                    let mut s = w.resource_mut::<LevelScript>();
                    let now = s.director.time;
                    s.breaking.broke(export, now, 0.15);
                    s.hidden.insert(export);
                    w.resource::<LevelParts>().set_solid_export(export, false);
                }
            }
        };
        step(&mut w, 0.05);
        {
            let s = w.resource::<LevelScript>();
            assert_eq!(s.props.hits, 3);
            assert_eq!(s.props.broken, vec!["BarrelA".to_string()]);
            assert!(s.props.exploded.is_empty(), "not yet: a moment later");
        }
        for _ in 0..10 {
            step(&mut w, 0.05);
        }
        let s = w.resource::<LevelScript>();
        assert_eq!(s.props.exploded, vec!["BarrelA", "BarrelB"]);
        let sent: Vec<&str> = s.director.history.iter().map(|h| h.1.as_str()).collect();
        assert!(
            sent.contains(&"BarrelAGone") && sent.contains(&"BarrelBGone"),
            "{sent:?}"
        );
        assert_eq!(w.resource::<Messages<Explode>>().len(), 2);
    }
}
