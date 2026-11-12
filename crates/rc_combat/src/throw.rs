//! Throwing grenades, flying them, and their blasts: the Bevy systems for
//! the rules in `grenade.rs`.

use crate::damage_types::DamageTable;
use crate::fire::{to_bevy, to_unreal};
use crate::grenade::{blast_scale, Flight, GrenadeParams, Grenades, Touch};
use crate::health::{Damage, DamageKind, Dead};
use crate::library::WeaponLibrary;
use crate::shot;
use crate::status::{SpawnStatusEffect, StatusFrom};
use crate::Hittable;
use bevy::audio::{PlaybackSettings, Volume};
use bevy::prelude::*;
use rc_game_core::{
    Action, ActionState, GameLog, GameState, LevelCollision, Player, PlayerCamera, WeaponReadout,
};

/// The player's field of view for "is the player looking at the flash",
/// degrees (the original's default `FovAngle`).
pub(crate) const PLAYER_FOV: f32 = 85.0;

/// "Throw this grenade from here at this speed." Anyone can send it (the
/// player's button does; squad mates and enemies can too). Bevy space,
/// `velocity` in Bevy units per second.
#[derive(Message, Debug, Clone)]
pub struct ThrowGrenade {
    pub thrower: Entity,
    pub origin: Vec3,
    pub velocity: Vec3,
    /// A grenade file name without `.toml`, like `thermal_detonator`.
    pub grenade: String,
}

/// A grenade went off. Sent for effects, sounds, AI hearing and tests.
/// Bevy space; `radius` in Unreal units.
#[derive(Message, Debug, Clone)]
pub struct Blast {
    pub grenade: String,
    pub point: Vec3,
    pub radius: f32,
    pub instigator: Option<Entity>,
    /// Characters it hurt or stunned.
    pub hit: u32,
}

/// "Something blew up here": a barrel, a fuel dispenser, anything that
/// isn't a thrown grenade. Every character within `radius` whose centre
/// the blast can see takes `damage` falling off with distance (the
/// engine's `HurtRadius`, as for grenades), times its damage table for
/// `damage_type`. A [`Blast`] is sent too (named `what`), so the AI hears
/// it. Bevy space; `radius` in Unreal units.
#[derive(Message, Debug, Clone)]
pub struct Explode {
    pub point: Vec3,
    pub damage: f32,
    pub radius: f32,
    /// A damage type class name, like `CTDamageExplosionGeoBarrel`.
    pub damage_type: String,
    pub kind: DamageKind,
    pub instigator: Option<Entity>,
    /// What went off, for the log and the `Blast`.
    pub what: String,
}

/// What kind of body a character has (for effects and the log). Characters
/// without one count as `Organic`. How much damage a character takes is its
/// `DamageTable`.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Body {
    Organic,
    Droid,
}

impl Body {
    pub fn name(self) -> &'static str {
        match self {
            Body::Organic => "organic",
            Body::Droid => "droid",
        }
    }
}

/// A grenade in the air, on the ground or stuck to something.
#[derive(Component, Debug, Clone)]
pub struct GrenadeFlight {
    pub flight: Flight,
    pub grenade: String,
    pub thrower: Entity,
    /// Stuck to this character, this far from its feet (Unreal units).
    pub stuck_on: Option<(Entity, Vec3)>,
}

fn say(log: &Option<Res<GameLog>>, s: String) {
    match log {
        Some(l) => l.line(s),
        None => println!("{s}"),
    }
}

fn play(commands: &mut Commands, lib: &WeaponLibrary, name: &Option<String>, volume: f32) {
    if let Some(h) = name.as_ref().and_then(|n| lib.sounds.get(n)?.pick()) {
        commands.spawn((
            AudioPlayer(h),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)),
        ));
    }
}

/// The player's grenade buttons: next grenade, and throw.
pub(crate) fn player_grenades(
    actions: Res<ActionState>,
    time: Res<Time>,
    lib: Res<WeaponLibrary>,
    mut players: Query<(Entity, &mut Grenades), (With<Player>, Without<Dead>)>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    mut throws: MessageWriter<ThrowGrenade>,
    log: Option<Res<GameLog>>,
) {
    let Ok((me, mut g)) = players.single_mut() else {
        return;
    };
    g.cooldown = (g.cooldown - time.delta_secs()).max(0.0);
    if actions.just_pressed(Action::NextGrenade) && g.select_next() {
        let (id, n) = g.selected().cloned().unwrap_or_default();
        say(&log, format!("grenades: selected {id} ({n} left)"));
    }
    if !actions.just_pressed(Action::ThrowGrenade) {
        return;
    }
    let Ok(cam) = camera.single() else { return };
    let Some(id) = g.take() else {
        say(&log, "grenades: none left of the selected kind".into());
        return;
    };
    let Some(p) = lib.grenades.get(&id) else {
        return;
    };
    g.cooldown = p.throw_delay;
    let eye = to_unreal(cam.translation());
    let f = cam.forward();
    let view = Vec3::new(f.x, f.z, f.y).normalize_or_zero();
    let axes = shot::view_axes(view.y.atan2(view.x), view.z.clamp(-1.0, 1.0).asin());
    let start = shot::fire_start(eye, axes, p.throw_offset);
    let v = p.throw_velocity(view);
    let left = g.selected().map_or(0, |s| s.1);
    say(
        &log,
        format!(
            "grenades: player threw {id} at {:.0} uu/s ({left} left)",
            v.length()
        ),
    );
    throws.write(ThrowGrenade {
        thrower: me,
        origin: to_bevy(start),
        velocity: Vec3::new(v.x, v.z, v.y) / rc_game_core::units::UU_PER_UNIT,
        grenade: id,
    });
}

/// Turn each `ThrowGrenade` into a grenade in flight.
pub(crate) fn launch_grenades(
    mut commands: Commands,
    mut throws: MessageReader<ThrowGrenade>,
    lib: Res<WeaponLibrary>,
    looks: Option<Res<crate::view::GrenadeLooks>>,
    log: Option<Res<GameLog>>,
) {
    for t in throws.read() {
        let Some(p) = lib.grenades.get(&t.grenade) else {
            say(&log, format!("grenades: unknown grenade `{}`", t.grenade));
            continue;
        };
        let v = t.velocity * rc_game_core::units::UU_PER_UNIT;
        let flight = Flight::new(p, to_unreal(t.origin), Vec3::new(v.x, v.z, v.y));
        let mut e = commands.spawn((
            GrenadeFlight {
                flight,
                grenade: t.grenade.clone(),
                thrower: t.thrower,
                stuck_on: None,
            },
            Transform::from_translation(t.origin),
            DespawnOnExit(GameState::InGame),
        ));
        if let Some(looks) = &looks {
            looks.attach(&mut e, &t.grenade);
        }
        play(&mut commands, &lib, &p.throw_sound, 1.0);
    }
}

/// A character a blast may reach, in Unreal space.
struct Victim {
    entity: Entity,
    centre: Vec3,
    hit: Hittable,
    table: Option<String>,
    body: Body,
}

/// Fly every grenade; the ones whose fuse is done (or that touched what sets
/// them off, or sense a character nearby) go off.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fly_grenades(
    mut commands: Commands,
    time: Res<Time>,
    lib: Res<WeaponLibrary>,
    mut grenades: Query<(Entity, &mut GrenadeFlight, &mut Transform)>,
    targets: Query<
        (
            Entity,
            &GlobalTransform,
            &Hittable,
            Option<&Body>,
            Option<&DamageTable>,
        ),
        Without<Dead>,
    >,
    level: Option<Res<LevelCollision>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    mut damage: MessageWriter<Damage>,
    mut blasts: MessageWriter<Blast>,
    mut status: MessageWriter<SpawnStatusEffect>,
    log: Option<Res<GameLog>>,
) {
    let dt = time.delta_secs();
    if grenades.is_empty() || dt <= 0.0 {
        return;
    }
    let listener = camera.single().ok().map(|c| to_unreal(c.translation()));
    let v = |p: Vec3| rc_movement::Vec3::new(p.x, p.y, p.z);
    let victims: Vec<Victim> = targets
        .iter()
        .map(|(e, gt, hit, body, table)| Victim {
            entity: e,
            centre: to_unreal(gt.translation()) + Vec3::Z * hit.center_up,
            hit: *hit,
            table: table.map(|t| t.0.clone()),
            body: body.copied().unwrap_or(Body::Organic),
        })
        .collect();
    let feet_of = |e: Entity| {
        targets
            .get(e)
            .ok()
            .map(|(_, gt, ..)| to_unreal(gt.translation()))
    };
    let sees = |a: Vec3, b: Vec3| {
        !level.as_deref().is_some_and(|l| {
            l.0.sweep(rc_movement::Vec3::ZERO, v(a), v(b))
                .is_some_and(|h| h.time < 0.999)
        })
    };
    for (e, mut g, mut tf) in &mut grenades {
        let Some(p) = lib.grenades.get(&g.grenade) else {
            commands.entity(e).despawn();
            continue;
        };
        // Stuck to a character: ride along (or drop if it's gone).
        if let Some((who, offset)) = g.stuck_on {
            match feet_of(who) {
                Some(feet) => g.flight.position = feet + offset,
                None => {
                    g.stuck_on = None;
                    g.flight.stuck = false;
                }
            }
        }
        let thrower = g.thrower;
        let mut touched: Option<Entity> = None;
        let mut touched_level = false;
        let r = g.flight.step(p, dt, |a, b| {
            let mut best = level.as_deref().and_then(|l| {
                l.0.sweep(rc_movement::Vec3::ZERO, v(a), v(b))
                    .map(|h| Touch {
                        t: h.time,
                        normal: Vec3::new(h.normal.x, h.normal.y, h.normal.z),
                        actor: false,
                    })
            });
            let mut who = None;
            for vic in &victims {
                // The thrower can't be hit by their own grenade (the
                // original ignores its instigator), not just at first.
                if vic.entity == thrower {
                    continue;
                }
                let c = vic.centre;
                if let Some(t) =
                    shot::line_hits_cylinder(a, b, c, vic.hit.radius, vic.hit.half_height)
                {
                    if best.is_none_or(|bt| t < bt.t) {
                        best = Some(Touch {
                            t,
                            normal: Vec3::new(a.x - c.x, a.y - c.y, 0.0).normalize_or(Vec3::Z),
                            actor: true,
                        });
                        who = Some(vic.entity);
                    }
                }
            }
            if best.is_some_and(|b| b.actor) {
                touched = who;
            }
            touched_level = best.is_some_and(|b| !b.actor);
            best
        });
        if r.stuck && r.hit_actor {
            if let (Some(who), Some(feet)) = (touched, touched.and_then(feet_of)) {
                g.stuck_on = Some((who, g.flight.position - feet));
            }
        }
        if r.stuck {
            say(
                &log,
                format!(
                    "grenades: {} stuck to {} after {:.2} s",
                    g.grenade,
                    if r.hit_actor {
                        "a character"
                    } else {
                        "the level"
                    },
                    g.flight.age
                ),
            );
        }
        if r.bounced {
            if let Some(l) = listener {
                let vol = 1.0 / (1.0 + (l - g.flight.position).length() / 1000.0);
                play(&mut commands, &lib, &p.bounce_sound, vol);
            }
        }
        let mut explode = r.explode;
        let mut direct = if r.explode && r.hit_actor {
            touched
        } else {
            None
        };
        // Why it went off, for the log: a grenade that goes off right in
        // front of the thrower should say what it ran into.
        let mut why = if direct.is_some() {
            "on touching a character"
        } else if r.explode && touched_level {
            "on touching the level"
        } else {
            "when its fuse ran out"
        };
        // Stuck proximity grenades: a character it can see comes close.
        if !explode && g.flight.proximity_due(p, dt) {
            let at = g.flight.position;
            if let Some(vic) = victims
                .iter()
                .find(|vic| (vic.centre - at).length() <= p.proximity && sees(at, vic.centre))
            {
                explode = true;
                direct = Some(vic.entity);
                why = "when a character came near";
            }
        }
        if !explode {
            tf.translation = to_bevy(g.flight.position);
            continue;
        }
        let hit_count = blast(
            &lib,
            p,
            &g,
            direct,
            &victims,
            &sees,
            &mut damage,
            &mut status,
            &log,
            why,
        );
        let point = g.flight.position;
        let vol = listener.map_or(1.0, |l| 1.0 / (1.0 + (l - point).length() / 2000.0));
        play(&mut commands, &lib, &p.blast_sound, vol);
        blasts.write(Blast {
            grenade: g.grenade.clone(),
            point: to_bevy(point),
            radius: p.radius,
            instigator: Some(thrower),
            hit: hit_count,
        });
        commands.entity(e).despawn();
    }
}

/// A grenade goes off. The rules are the original's (`CTProjExplosive`'s
/// explosion, the engine's `HurtRadius`, and the status effects), in our
/// words:
/// - a character it went off on takes the full damage directly;
/// - everyone else within reach whose centre the blast can see takes
///   damage falling off with distance (`blast_scale`);
/// - its status effect starts where it went off (`crate::status`);
/// - every hit is scaled by the target's damage table.
#[allow(clippy::too_many_arguments)]
fn blast(
    lib: &WeaponLibrary,
    p: &GrenadeParams,
    g: &GrenadeFlight,
    direct: Option<Entity>,
    victims: &[Victim],
    sees: &dyn Fn(Vec3, Vec3) -> bool,
    damage: &mut MessageWriter<Damage>,
    status: &mut MessageWriter<SpawnStatusEffect>,
    log: &Option<Res<GameLog>>,
    why: &str,
) -> u32 {
    let point = g.flight.position;
    let types = &lib.damage_types;
    let mut hit_count = 0;
    for vic in victims {
        let dist = (vic.centre - point).length();
        let is_direct = direct == Some(vic.entity);
        let scale = if is_direct {
            1.0
        } else {
            blast_scale(dist, vic.hit.radius, p.radius)
        };
        let visible = is_direct || sees(point, vic.centre);
        let mut what = Vec::new();
        if scale > 0.0 && visible && p.damage > 0.0 {
            let m = types.multiplier(vic.table.as_deref(), &p.damage_type);
            let amount = p.damage * scale * m;
            if amount > 0.0 {
                damage.write(Damage {
                    target: vic.entity,
                    amount,
                    kind: p.damage_kind,
                    instigator: Some(g.thrower),
                    point: to_bevy(point),
                    direction: to_bevy(vic.centre - point).normalize_or_zero(),
                });
            }
            what.push(format!(
                "{} damage {amount:.1} (x{scale:.2} by distance, x{m} by its table)",
                if is_direct { "direct" } else { "blast" }
            ));
        }
        if !what.is_empty() {
            hit_count += 1;
            say(
                log,
                format!(
                    "grenades: {} reached {:?} ({}, table {}) at {dist:.0} uu: {}",
                    g.grenade,
                    vic.entity,
                    vic.body.name(),
                    vic.table.as_deref().unwrap_or("none"),
                    what.join(", ")
                ),
            );
        }
    }
    // Its lasting effect, from where it went off (`CreateStatusEffects`
    // from the grenade, reaching `DamageRadius + StatusEffectRadiusModifier`).
    if let Some(st) = &p.status {
        status.write(SpawnStatusEffect {
            class: st.class.clone(),
            from: StatusFrom::Point(to_bevy(point)),
            radius: p.radius + st.radius_change,
            max_time: st.max_time,
            damage_per_sec: st.damage_per_sec,
            instigator: Some(g.thrower),
            by: format!("grenade {}", g.grenade),
        });
    }
    say(
        log,
        format!(
            "grenades: {} went off {why} after {:.2} s and {} bounces at ({:.0}, {:.0}, {:.0}), radius {}, {hit_count} characters reached",
            g.grenade, g.flight.age, g.flight.bounces, point.x, point.y, point.z, p.radius
        ),
    );
    hit_count
}

/// Carry out every [`Explode`]: hurt the characters around it.
pub(crate) fn explosions(
    mut explode: MessageReader<Explode>,
    lib: Res<WeaponLibrary>,
    targets: Query<(Entity, &GlobalTransform, &Hittable, Option<&DamageTable>), Without<Dead>>,
    level: Option<Res<LevelCollision>>,
    mut damage: MessageWriter<Damage>,
    mut blasts: MessageWriter<Blast>,
    log: Option<Res<GameLog>>,
) {
    let v = |p: Vec3| rc_movement::Vec3::new(p.x, p.y, p.z);
    for x in explode.read() {
        let point = to_unreal(x.point);
        let mut hit = 0;
        for (e, gt, h, table) in &targets {
            let centre = to_unreal(gt.translation()) + Vec3::Z * h.center_up;
            let dist = (centre - point).length();
            let scale = blast_scale(dist, h.radius, x.radius);
            if scale <= 0.0 {
                continue;
            }
            // Walls in between keep it off.
            let blocked = level.as_deref().is_some_and(|l| {
                l.0.sweep(rc_movement::Vec3::ZERO, v(point), v(centre))
                    .is_some_and(|hit| hit.time < 0.999)
            });
            if blocked {
                continue;
            }
            let m = lib
                .damage_types
                .multiplier(table.map(|t| t.0.as_str()), &x.damage_type);
            let amount = x.damage * scale * m;
            if amount <= 0.0 {
                continue;
            }
            hit += 1;
            damage.write(Damage {
                target: e,
                amount,
                kind: x.kind,
                instigator: x.instigator,
                point: x.point,
                direction: to_bevy(centre - point).normalize_or_zero(),
            });
            say(
                &log,
                format!(
                    "explosions: {} reached {e:?} at {dist:.0} uu: {amount:.1} damage (x{scale:.2} by distance, x{m} by its table)",
                    x.what
                ),
            );
        }
        say(
            &log,
            format!(
                "explosions: {} went off at ({:.0}, {:.0}, {:.0}), damage {}, radius {}, {hit} characters reached",
                x.what, point.x, point.y, point.z, x.damage, x.radius
            ),
        );
        blasts.write(Blast {
            grenade: x.what.clone(),
            point: x.point,
            radius: x.radius,
            instigator: x.instigator,
            hit,
        });
    }
}

/// The selected grenade and how many are left, for the display.
pub(crate) fn grenade_readout(
    lib: Res<WeaponLibrary>,
    players: Query<&Grenades, With<Player>>,
    mut out: ResMut<WeaponReadout>,
) {
    let Ok(g) = players.single() else { return };
    let new = g.selected().map(|(id, n)| {
        let name = lib.grenades.get(id).map_or(id.clone(), |p| p.name.clone());
        (name, *n)
    });
    if out.grenade != new {
        out.grenade = new;
    }
}
