//! Throwing grenades, flying them, and their blasts: the Bevy systems for
//! the rules in `grenade.rs`.

use crate::fire::{to_bevy, to_unreal};
use crate::grenade::{blast_scale, Flight, Grenades};
use crate::health::{Damage, Dead};
use crate::library::WeaponLibrary;
use crate::shot;
use crate::Hittable;
use bevy::audio::{PlaybackSettings, Volume};
use bevy::prelude::*;
use rc_game_core::{
    Action, ActionState, GameLog, GameState, LevelCollision, Player, PlayerCamera, WeaponReadout,
};

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

/// What kind of body a character has, for damage tables (EMP hurts droids
/// most) and stuns. Characters without one count as `Organic`.
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

/// Can't act for `left` seconds (a flash or EMP blast). AI and the
/// player's controls read it; combat counts it down.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Stunned {
    pub left: f32,
}

/// A grenade in the air or on the ground.
#[derive(Component, Debug, Clone)]
pub struct GrenadeFlight {
    pub flight: Flight,
    pub grenade: String,
    pub thrower: Entity,
}

fn say(log: &Option<Res<GameLog>>, s: String) {
    match log {
        Some(l) => l.line(s),
        None => println!("{s}"),
    }
}

fn play(commands: &mut Commands, lib: &WeaponLibrary, name: &Option<String>, volume: f32) {
    if let Some(h) = name.as_ref().and_then(|n| lib.sounds.get(n)) {
        commands.spawn((
            AudioPlayer(h.clone()),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)),
        ));
    }
}

/// The player's grenade buttons: next grenade, and throw.
pub(crate) fn player_grenades(
    actions: Res<ActionState>,
    time: Res<Time>,
    lib: Res<WeaponLibrary>,
    mut players: Query<(Entity, &mut Grenades), (With<Player>, Without<Dead>, Without<Stunned>)>,
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
        let flight = Flight::new(to_unreal(t.origin), Vec3::new(v.x, v.z, v.y));
        let mut e = commands.spawn((
            GrenadeFlight {
                flight,
                grenade: t.grenade.clone(),
                thrower: t.thrower,
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

/// Fly every grenade; the ones whose fuse is done go off.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fly_grenades(
    mut commands: Commands,
    time: Res<Time>,
    lib: Res<WeaponLibrary>,
    mut grenades: Query<(Entity, &mut GrenadeFlight, &mut Transform)>,
    targets: Query<(Entity, &GlobalTransform, &Hittable, Option<&Body>), Without<Dead>>,
    level: Option<Res<LevelCollision>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    mut damage: MessageWriter<Damage>,
    mut blasts: MessageWriter<Blast>,
    log: Option<Res<GameLog>>,
) {
    let dt = time.delta_secs();
    if grenades.is_empty() || dt <= 0.0 {
        return;
    }
    let listener = camera.single().ok().map(|c| to_unreal(c.translation()));
    let v = |p: Vec3| rc_movement::Vec3::new(p.x, p.y, p.z);
    for (e, mut g, mut tf) in &mut grenades {
        let Some(p) = lib.grenades.get(&g.grenade) else {
            commands.entity(e).despawn();
            continue;
        };
        let thrower = g.thrower;
        let young = g.flight.age < 0.5;
        let r = g.flight.step(p, dt, |a, b| {
            // Characters stop impact grenades (and thrown ones bounce off
            // them, as off a wall: our choice).
            let mut best = level.as_deref().and_then(|l| {
                l.0.sweep(rc_movement::Vec3::ZERO, v(a), v(b))
                    .map(|h| (h.time, Vec3::new(h.normal.x, h.normal.y, h.normal.z)))
            });
            for (te, gt, hit, _) in &targets {
                if te == thrower && young {
                    continue;
                }
                let c = to_unreal(gt.translation()) + Vec3::Z * hit.center_up;
                if let Some(t) = shot::line_hits_cylinder(a, b, c, hit.radius, hit.half_height) {
                    if best.is_none_or(|(bt, _)| t < bt) {
                        let n = Vec3::new(a.x - c.x, a.y - c.y, 0.0).normalize_or(Vec3::Z);
                        best = Some((t, n));
                    }
                }
            }
            best
        });
        if r.bounced {
            if let Some(l) = listener {
                let vol = 1.0 / (1.0 + (l - g.flight.position).length() / 1000.0);
                play(&mut commands, &lib, &p.bounce_sound, vol);
            }
        }
        if !r.explode {
            tf.translation = to_bevy(g.flight.position);
            continue;
        }
        // The blast: the engine's HurtRadius, in our words. Everything
        // within reach whose centre the blast can see takes damage that
        // falls off with distance.
        let point = g.flight.position;
        let mut hit_count = 0;
        for (te, gt, hit, body) in &targets {
            let c = to_unreal(gt.translation()) + Vec3::Z * hit.center_up;
            let dist = (c - point).length();
            let scale = blast_scale(dist, hit.radius, p.radius);
            if scale <= 0.0 {
                continue;
            }
            let blocked = level.as_deref().is_some_and(|l| {
                l.0.sweep(rc_movement::Vec3::ZERO, v(point), v(c))
                    .is_some_and(|h| h.time < 0.999)
            });
            if blocked {
                continue;
            }
            let kind = body.copied().unwrap_or(Body::Organic).name();
            let amount = p.damage * scale * p.multiplier_vs(kind);
            if amount > 0.0 {
                damage.write(Damage {
                    target: te,
                    amount,
                    kind: p.damage_kind,
                    instigator: Some(thrower),
                    point: to_bevy(point),
                    direction: to_bevy(c - point).normalize_or_zero(),
                });
            }
            if p.stuns(kind) {
                commands.entity(te).insert(Stunned {
                    left: p.stun_time * scale.max(0.5),
                });
            }
            hit_count += 1;
            say(
                &log,
                format!(
                    "grenades: {} blast reached {te:?} ({kind}) at {dist:.0} uu: scale {scale:.2}, damage {amount:.1}{}",
                    g.grenade,
                    if p.stuns(kind) { format!(", stunned {:.1} s", p.stun_time * scale.max(0.5)) } else { String::new() }
                ),
            );
        }
        say(
            &log,
            format!(
                "grenades: {} went off after {:.2} s and {} bounces at ({:.0}, {:.0}, {:.0}), radius {}, {hit_count} characters in reach",
                g.grenade, g.flight.age, g.flight.bounces, point.x, point.y, point.z, p.radius
            ),
        );
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

/// Stuns wear off.
pub(crate) fn wear_off_stuns(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut Stunned)>,
) {
    for (e, mut s) in &mut q {
        s.left -= time.delta_secs();
        if s.left <= 0.0 {
            commands.entity(e).remove::<Stunned>();
        }
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
