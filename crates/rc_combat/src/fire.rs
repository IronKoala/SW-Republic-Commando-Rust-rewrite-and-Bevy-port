//! Pulling the trigger, launching shots, flying bolts and landing hits.
//!
//! Positions are worked out in Unreal space (see `shot.rs`) and converted
//! to and from Bevy space at the edges, with the same rule as
//! `rc_game_core::units`.

use crate::damage_types::DamageTable;
use crate::health::{Damage, DamageKind, Dead};
use crate::library::WeaponLibrary;
use crate::shot::{self, Bolt, Rng};
use crate::weapon::{ShotKind, Trigger, WeaponParams};
use crate::{Arsenal, FireShot, Hittable, ShotLanded, ViewKick};
use bevy::audio::{PlaybackSettings, Volume};
use bevy::prelude::*;
use rc_game_core::{
    units::UU_PER_UNIT, Action, ActionState, GameLog, GameState, LevelCollision, Player,
    PlayerCamera, WeaponReadout,
};

/// How far the crosshair looks for something to aim at, uu (the game's
/// `AimSpot`, 10000 uu ahead).
const CROSSHAIR_RANGE: f32 = 10000.0;
/// The game's rotation units in a full turn.
const ROTATION_UNITS: f32 = 65536.0;

#[derive(Resource)]
pub(crate) struct CombatRng(pub Rng);

/// What's under the crosshair this frame, for the on-screen display.
#[derive(Resource, Default)]
pub(crate) struct Crosshair {
    pub on_target: bool,
}

/// A bolt in the air.
#[derive(Component, Debug, Clone)]
pub struct BoltFlight {
    pub bolt: Bolt,
    pub shooter: Entity,
    pub weapon: String,
    pub damage: f32,
    pub kind: DamageKind,
    /// The damage type class, looked up in the target's damage table.
    pub damage_type: String,
    /// Where it was fired from, Unreal space.
    pub start: Vec3,
}

pub(crate) fn to_unreal(p: Vec3) -> Vec3 {
    Vec3::new(p.x, p.z, p.y) * UU_PER_UNIT
}

pub(crate) fn to_bevy(p: Vec3) -> Vec3 {
    Vec3::new(p.x, p.z, p.y) / UU_PER_UNIT
}

/// A direction, Bevy to Unreal (no scaling).
fn dir_to_unreal(d: Vec3) -> Vec3 {
    Vec3::new(d.x, d.z, d.y)
}

fn dir_to_bevy(d: Vec3) -> Vec3 {
    Vec3::new(d.x, d.z, d.y)
}

/// Something a line can hit.
struct Target {
    entity: Entity,
    center: Vec3,
    hit: Hittable,
}

/// The first thing on the line `a`..`b` (Unreal space).
struct LineHit {
    /// 0 to 1 along the line.
    t: f32,
    entity: Option<Entity>,
}

fn trace(
    level: Option<&LevelCollision>,
    targets: &[Target],
    a: Vec3,
    b: Vec3,
    ignore: Option<Entity>,
) -> Option<LineHit> {
    let mut best: Option<LineHit> = None;
    if let Some(level) = level {
        let v = |p: Vec3| rc_movement::Vec3::new(p.x, p.y, p.z);
        if let Some(h) = level.0.sweep(rc_movement::Vec3::ZERO, v(a), v(b)) {
            best = Some(LineHit {
                t: h.time,
                entity: None,
            });
        }
    }
    for tg in targets {
        if Some(tg.entity) == ignore {
            continue;
        }
        if let Some(t) =
            shot::line_hits_cylinder(a, b, tg.center, tg.hit.radius, tg.hit.half_height)
        {
            if best.as_ref().is_none_or(|h| t < h.t) {
                best = Some(LineHit {
                    t,
                    entity: Some(tg.entity),
                });
            }
        }
    }
    best
}

fn gather_targets(q: &Query<(Entity, &GlobalTransform, &Hittable), Without<Dead>>) -> Vec<Target> {
    q.iter()
        .map(|(entity, gt, hit)| Target {
            entity,
            center: to_unreal(gt.translation()) + Vec3::Z * hit.center_up,
            hit: *hit,
        })
        .collect()
}

/// `amount` of damage type `ty` against `target`, after the target's
/// damage table (`DamageTable`): the original's `DamageVariance` lists.
pub(crate) fn against(
    lib: &WeaponLibrary,
    tables: &Query<&DamageTable>,
    target: Entity,
    ty: &str,
    amount: f32,
) -> f32 {
    let table = tables.get(target).ok().map(|t| t.0.as_str());
    amount * lib.damage_types.multiplier(table, ty)
}

fn play(commands: &mut Commands, lib: &WeaponLibrary, name: &str, volume: f32) {
    if let Some(h) = lib.sounds.get(name).and_then(|s| s.pick()) {
        commands.spawn((
            AudioPlayer(h),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)),
        ));
    }
}

/// The keys for one weapon each, and a piece of that weapon's file name
/// (`data/weapons/*.toml`).
const WEAPON_KEYS: [(Action, &str); 4] = [
    (Action::WeaponPistol, "dc15s"),
    (Action::WeaponBlaster, "dc17m_blaster"),
    (Action::WeaponSniper, "dc17m_sniper"),
    (Action::WeaponAntiArmor, "dc17m_antiarmor"),
];

/// The first carried weapon whose name holds `part`.
fn weapon_with<T>(weapons: &[(String, T)], part: &str) -> Option<usize> {
    weapons.iter().position(|(id, _)| id.contains(part))
}

/// The player's trigger, reload and weapon switching, from the actions.
#[allow(clippy::too_many_arguments)]
pub(crate) fn player_trigger(
    mut commands: Commands,
    actions: Res<ActionState>,
    time: Res<Time>,
    lib: Res<WeaponLibrary>,
    mut players: Query<(Entity, &mut Arsenal), (With<Player>, Without<Dead>)>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    targets_q: Query<(Entity, &GlobalTransform, &Hittable), Without<Dead>>,
    teams: Query<&crate::Team>,
    level: Option<Res<LevelCollision>>,
    mut rng: ResMut<CombatRng>,
    mut shots: MessageWriter<FireShot>,
    mut kicks: MessageWriter<ViewKick>,
    log: Option<Res<GameLog>>,
) {
    let dt = time.delta_secs();
    let Ok((me, mut arsenal)) = players.single_mut() else {
        return;
    };
    let n = arsenal.weapons.len();
    if n == 0 {
        return;
    }
    let say = |s: String| match &log {
        Some(l) => l.line(s),
        None => println!("{s}"),
    };
    // Switching. **Not done:** the game's holster and draw animations take
    // time (0.37 s and 0.67 s for the DC-17m); here the switch is instant.
    let before = arsenal.current;
    if actions.just_pressed(Action::NextWeapon) {
        arsenal.current = (arsenal.current + 1) % n;
    }
    if actions.just_pressed(Action::PrevWeapon) {
        arsenal.current = (arsenal.current + n - 1) % n;
    }
    // The number keys go straight to one weapon, found by its file name.
    for (action, part) in WEAPON_KEYS {
        if actions.just_pressed(action) {
            if let Some(i) = weapon_with(&arsenal.weapons, part) {
                arsenal.current = i;
            }
        }
    }
    if arsenal.current != before {
        if let Some((_, w)) = arsenal.weapons.get_mut(before) {
            w.reloading = None;
        }
        say(format!(
            "weapons: switched to {}",
            arsenal.weapons[arsenal.current].0
        ));
    }
    // Every carried weapon recharges and counts down.
    let current = arsenal.current;
    for (i, (id, state)) in arsenal.weapons.iter_mut().enumerate() {
        if let Some(p) = lib.weapons.get(id) {
            if state.tick(p, dt) && i == current {
                say(format!(
                    "weapons: {id} reloaded: magazine {}, spare {}",
                    state.shots_in_clip(),
                    state.reserve
                ));
            }
        }
    }
    let (id, state) = &mut arsenal.weapons[current];
    let Some(p) = lib.weapons.get(id.as_str()) else {
        return;
    };
    let Ok(cam) = camera.single() else {
        return;
    };
    let eye = to_unreal(cam.translation());
    let view = dir_to_unreal(*cam.forward()).normalize_or_zero();
    let yaw = view.y.atan2(view.x);
    let pitch = view.z.clamp(-1.0, 1.0).asin();
    let axes = shot::view_axes(yaw, pitch);
    let targets = gather_targets(&targets_q);
    let level = level.as_deref();

    // What the crosshair is on.
    let far = eye + view * CROSSHAIR_RANGE;
    let under = trace(level, &targets, eye, far, Some(me));
    let crosshair = under.as_ref().map_or(far, |h| eye + (far - eye) * h.t);
    let on_target = under
        .as_ref()
        .and_then(|h| h.entity)
        .and_then(|e| teams.get(e).ok())
        .is_some_and(|t| *t == crate::Team::Separatist);
    commands.insert_resource(crate::fire::Crosshair { on_target });

    if actions.just_pressed(Action::Reload) && state.start_reload(p) {
        say(format!("weapons: {id} reloading ({} s)", p.reload_time));
        if let Some(s) = &p.reload_sound {
            play(&mut commands, &lib, s, 1.0);
        }
    }
    match state.trigger(
        p,
        actions.pressed(Action::Fire),
        actions.just_pressed(Action::Fire),
    ) {
        Trigger::Nothing => {}
        Trigger::Empty => {
            say(format!("weapons: {id} is empty"));
            if let Some(s) = &p.empty_sound {
                play(&mut commands, &lib, s, 1.0);
            }
        }
        Trigger::Reloading => {
            say(format!(
                "weapons: {id} magazine empty, reloading ({} s)",
                p.reload_time
            ));
            if let Some(s) = &p.reload_sound {
                play(&mut commands, &lib, s, 1.0);
            }
        }
        Trigger::Fired => {
            let mut start = shot::fire_start(eye, axes, p.fire_offset);
            // If something is between the eye and the muzzle (standing
            // against a wall or a droid), fire from the eye so the shot
            // hits it, like the game's `CreateProximityProjectile`.
            if trace(level, &targets, eye, start, Some(me)).is_some() {
                start = eye;
            }
            let aim_point = shot::aim_at_crosshair(start, view, crosshair);
            shots.write(FireShot {
                shooter: me,
                origin: to_bevy(start),
                direction: dir_to_bevy((aim_point - start).normalize_or_zero()),
                weapon: id.clone(),
            });
            let turn = std::f32::consts::TAU / ROTATION_UNITS;
            kicks.write(ViewKick {
                pitch: p.kick[0] * turn,
                yaw: 2.0 * p.kick[1] * (rng.0.next_f32() - 0.5) * turn,
            });
        }
    }
}

/// Turn every `FireShot` into a bolt in flight or an instant hit.
#[allow(clippy::too_many_arguments)]
pub(crate) fn launch_shots(
    mut commands: Commands,
    mut fired: MessageReader<FireShot>,
    lib: Res<WeaponLibrary>,
    targets_q: Query<(Entity, &GlobalTransform, &Hittable), Without<Dead>>,
    players: Query<(), With<Player>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    level: Option<Res<LevelCollision>>,
    mut rng: ResMut<CombatRng>,
    mut damage: MessageWriter<Damage>,
    mut landed: MessageWriter<ShotLanded>,
    mut lobbed: MessageWriter<crate::throw::ThrowGrenade>,
    looks: Option<Res<crate::view::BoltLooks>>,
    tables: Query<&DamageTable>,
    zoom: Option<Res<crate::melee::Zoom>>,
    log: Option<Res<GameLog>>,
) {
    let say = |s: String| match &log {
        Some(l) => l.line(s),
        None => println!("{s}"),
    };
    let level = level.as_deref();
    let mut targets: Option<Vec<Target>> = None;
    let listener = camera.single().ok().map(|c| to_unreal(c.translation()));
    for f in fired.read() {
        let Some(p) = lib.weapons.get(&f.weapon) else {
            say(format!(
                "weapons: FireShot with unknown weapon `{}`",
                f.weapon
            ));
            continue;
        };
        let targets = targets.get_or_insert_with(|| gather_targets(&targets_q));
        // Zoomed in, the player's shots do more (`ZoomDamageMultiplier`).
        let zoomed = players.contains(f.shooter)
            && zoom
                .as_ref()
                .and_then(|z| z.zoomed())
                .is_some_and(|w| w == f.weapon);
        let base = if zoomed {
            p.damage * p.zoom_damage
        } else {
            p.damage
        };
        let start = to_unreal(f.origin);
        let dir = dir_to_unreal(f.direction).normalize_or_zero();
        let range = match p.shot {
            ShotKind::Instant { range } => range,
            ShotKind::Bolt { .. } | ShotKind::Lobbed { .. } => CROSSHAIR_RANGE,
        };
        // Aim error is added at the point the shot would land.
        let end = start + dir * range;
        let aim = trace(level, targets, start, end, Some(f.shooter))
            .map_or(end, |h| start + (end - start) * h.t);
        let aim = shot::add_aim_error(start, aim, p.aim_error, &mut rng.0);
        let dir = (aim - start).normalize_or_zero();

        // The fire sound: full volume for the player, quieter with
        // distance for everyone else. **Our choice**, not the game's
        // (which uses its 3D sound system).
        if !p.fire_sounds.is_empty() {
            let pick = (rng.0.next_f32() * p.fire_sounds.len() as f32) as usize;
            let name = &p.fire_sounds[pick.min(p.fire_sounds.len() - 1)];
            let volume = if players.contains(f.shooter) {
                1.0
            } else {
                listener.map_or(0.5, |l| 1.0 / (1.0 + (l - start).length() / 1000.0))
            };
            play(&mut commands, &lib, name, volume);
        }

        match p.shot {
            ShotKind::Lobbed { speed } => {
                // The round flies like a grenade: the throw systems take it
                // from here (and its blast does the damage).
                let v = dir_to_bevy(dir) * speed / UU_PER_UNIT;
                lobbed.write(crate::throw::ThrowGrenade {
                    thrower: f.shooter,
                    origin: f.origin,
                    velocity: v,
                    grenade: p.grenade.clone().unwrap_or_default(),
                });
            }
            ShotKind::Instant { range } => {
                let end = start + dir * range;
                let hit = trace(level, targets, start, end, Some(f.shooter));
                let t = hit.as_ref().map_or(1.0, |h| h.t);
                let point = start + (end - start) * t;
                land(
                    p,
                    base,
                    &lib,
                    &tables,
                    &log,
                    f.shooter,
                    start,
                    point,
                    dir,
                    hit.as_ref().map(|h| (h.entity, true)),
                    &mut damage,
                    &mut landed,
                );
            }
            ShotKind::Bolt {
                accel,
                max_speed,
                life_span,
            } => {
                let mut e = commands.spawn((
                    BoltFlight {
                        bolt: Bolt {
                            position: start,
                            direction: dir,
                            speed: 0.0,
                            accel,
                            max_speed,
                            age: 0.0,
                            life_span,
                        },
                        shooter: f.shooter,
                        weapon: f.weapon.clone(),
                        damage: base,
                        kind: p.damage_kind,
                        damage_type: p.damage_type.clone(),
                        start,
                    },
                    Transform::from_translation(to_bevy(start))
                        .looking_to(dir_to_bevy(dir), Vec3::Y),
                    DespawnOnExit(GameState::InGame),
                ));
                if let Some(looks) = &looks {
                    looks.attach(&mut e, &f.weapon);
                }
            }
        }
    }
}

/// A shot ended at `point`. `hit` is `None` for nothing hit, or the hit
/// character (`Some(Some(e))`) or the level (`Some(None)`).
#[allow(clippy::too_many_arguments)]
fn land(
    p: &WeaponParams,
    base: f32,
    lib: &WeaponLibrary,
    tables: &Query<&DamageTable>,
    log: &Option<Res<GameLog>>,
    shooter: Entity,
    start: Vec3,
    point: Vec3,
    dir: Vec3,
    hit: Option<(Option<Entity>, bool)>,
    damage: &mut MessageWriter<Damage>,
    landed: &mut MessageWriter<ShotLanded>,
) {
    let target = hit.and_then(|h| h.0);
    if let Some(t) = target {
        let amount = against(lib, tables, t, &p.damage_type, base);
        if amount != base {
            if let Some(l) = log {
                l.line(format!(
                    "weapons: {} hit {t:?}: {base} {} x {} = {amount}",
                    p.id,
                    p.damage_type,
                    amount / base.max(f32::EPSILON)
                ));
            }
        }
        damage.write(Damage {
            target: t,
            amount,
            kind: p.damage_kind,
            instigator: Some(shooter),
            point: to_bevy(point),
            direction: dir_to_bevy(dir),
        });
    }
    landed.write(ShotLanded {
        shooter,
        weapon: p.id.clone(),
        point: to_bevy(point),
        target,
        on_level: hit.is_some() && target.is_none(),
        distance: (point - start).length(),
    });
}

/// Move every bolt and see what it hits on the way.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fly_bolts(
    mut commands: Commands,
    time: Res<Time>,
    mut bolts: Query<(Entity, &mut BoltFlight, &mut Transform)>,
    targets_q: Query<(Entity, &GlobalTransform, &Hittable), Without<Dead>>,
    level: Option<Res<LevelCollision>>,
    lib: Res<WeaponLibrary>,
    tables: Query<&DamageTable>,
    mut damage: MessageWriter<Damage>,
    mut landed: MessageWriter<ShotLanded>,
) {
    let dt = time.delta_secs();
    if bolts.is_empty() || dt <= 0.0 {
        return;
    }
    let targets = gather_targets(&targets_q);
    let level = level.as_deref();
    for (e, mut b, mut tf) in &mut bolts {
        let (from, to) = b.bolt.advance(dt);
        let hit = trace(level, &targets, from, to, Some(b.shooter));
        if let Some(h) = hit {
            let point = from + (to - from) * h.t;
            if let Some(target) = h.entity {
                damage.write(Damage {
                    target,
                    amount: against(&lib, &tables, target, &b.damage_type, b.damage),
                    kind: b.kind,
                    instigator: Some(b.shooter),
                    point: to_bevy(point),
                    direction: dir_to_bevy(b.bolt.direction),
                });
            }
            landed.write(ShotLanded {
                shooter: b.shooter,
                weapon: b.weapon.clone(),
                point: to_bevy(point),
                target: h.entity,
                on_level: h.entity.is_none(),
                distance: (point - b.start).length(),
            });
            commands.entity(e).despawn();
        } else if b.bolt.expired() {
            landed.write(ShotLanded {
                shooter: b.shooter,
                weapon: b.weapon.clone(),
                point: to_bevy(to),
                target: None,
                on_level: false,
                distance: (to - b.start).length(),
            });
            commands.entity(e).despawn();
        } else {
            tf.translation = to_bevy(to);
        }
    }
}

/// The ammo counter and crosshair for the on-screen display.
pub(crate) fn weapon_readout(
    lib: Res<WeaponLibrary>,
    players: Query<&Arsenal, With<Player>>,
    crosshair: Option<Res<Crosshair>>,
    mut out: ResMut<WeaponReadout>,
) {
    let Ok(arsenal) = players.single() else {
        return;
    };
    let Some((id, state)) = arsenal.current() else {
        *out = WeaponReadout::default();
        return;
    };
    let Some(p) = lib.weapons.get(id) else {
        return;
    };
    let new = WeaponReadout {
        name: p.name.clone(),
        clip: Some(state.shots_in_clip()),
        clip_size: Some(if p.recharges() {
            p.max_ammo
        } else {
            p.clip_size
        }),
        reserve: (!p.recharges()).then_some(state.reserve),
        grenade: out.grenade.clone(),
        // **Our choice:** the crosshair's spread shown as the aim error
        // over 50 (the widest weapon so far is 15).
        spread: (p.aim_error / 50.0).clamp(0.0, 1.0),
        on_target: crosshair.is_some_and(|c| c.on_target),
        zoom_fov: out.zoom_fov,
    };
    if *out != new {
        *out = new;
    }
}
