//! Throwing grenades, flying them, and their blasts: the Bevy systems for
//! the rules in `grenade.rs`.

use crate::damage_types::DamageTable;
use crate::fire::{to_bevy, to_unreal};
use crate::grenade::{blast_scale, status_time, Flight, GrenadeParams, Grenades, Touch};
use crate::health::{Damage, DamageKind, Dead};
use crate::library::WeaponLibrary;
use crate::shot;
use crate::{Hittable, Team};
use bevy::audio::{PlaybackSettings, Volume};
use bevy::prelude::*;
use rc_game_core::{
    Action, ActionState, GameLog, GameState, LevelCollision, Player, PlayerCamera, WeaponReadout,
};

/// The player's field of view for "is the player looking at the flash",
/// degrees (the original's default `FovAngle`).
const PLAYER_FOV: f32 = 85.0;

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

/// Under a grenade's lasting effect for `left` seconds (the flash
/// grenade's stun, the EMP grenade's electrocution, the thermal's burn).
/// When `stops` is set, computer-controlled characters can't act while it
/// lasts; the player can, as in the original.
/// It may hurt `damage_per_sec` (already scaled by the character's damage
/// table) each second.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct Stunned {
    pub left: f32,
    pub damage_per_sec: f32,
    pub kind: DamageKind,
    pub stops: bool,
    pub instigator: Option<Entity>,
    /// Damage built up since the last quarter-second hit (sent in steps so
    /// the log isn't a line a frame).
    pub pending: f32,
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
    commando: bool,
    player: bool,
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
            Option<&Team>,
            Has<Player>,
        ),
        Without<Dead>,
    >,
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
    let player_view = camera.single().ok().map(|c| {
        let f = c.forward();
        Vec3::new(f.x, f.z, f.y)
    });
    let v = |p: Vec3| rc_movement::Vec3::new(p.x, p.y, p.z);
    let victims: Vec<Victim> = targets
        .iter()
        .map(|(e, gt, hit, body, table, team, player)| Victim {
            entity: e,
            centre: to_unreal(gt.translation()) + Vec3::Z * hit.center_up,
            hit: *hit,
            table: table.map(|t| t.0.clone()),
            body: body.copied().unwrap_or(Body::Organic),
            commando: team == Some(&Team::Republic),
            player,
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
        // Stuck proximity grenades: a character it can see comes close.
        if !explode && g.flight.proximity_due(p, dt) {
            let at = g.flight.position;
            if let Some(vic) = victims
                .iter()
                .find(|vic| (vic.centre - at).length() <= p.proximity && sees(at, vic.centre))
            {
                explode = true;
                direct = Some(vic.entity);
            }
        }
        if !explode {
            tf.translation = to_bevy(g.flight.position);
            continue;
        }
        let hit_count = blast(
            &mut commands,
            &lib,
            p,
            &g,
            direct,
            &victims,
            player_view,
            &sees,
            &mut damage,
            &log,
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
/// - its status effect reaches every character it can see within the
///   radius, for a time falling off with distance (`status_time`), unless
///   their damage table makes them immune; the player only when looking
///   towards a flash; clone commandos half as long for the EMP;
/// - every hit is scaled by the target's damage table.
#[allow(clippy::too_many_arguments)]
fn blast(
    commands: &mut Commands,
    lib: &WeaponLibrary,
    p: &GrenadeParams,
    g: &GrenadeFlight,
    direct: Option<Entity>,
    victims: &[Victim],
    player_view: Option<Vec3>,
    sees: &dyn Fn(Vec3, Vec3) -> bool,
    damage: &mut MessageWriter<Damage>,
    log: &Option<Res<GameLog>>,
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
        if let Some(st) = &p.status {
            let facing = !vic.player
                || !st.player_must_face
                || player_view.is_some_and(|f| {
                    f.normalize_or_zero()
                        .dot((point - vic.centre).normalize_or_zero())
                        > (PLAYER_FOV.to_radians() * 0.5).cos()
                });
            let m = types.multiplier(vic.table.as_deref(), &st.damage_type);
            let mut secs = status_time(
                st.max_time,
                dist,
                vic.hit.radius,
                p.radius + st.radius_change,
            );
            if st.half_on_commandos && vic.commando {
                secs *= 0.5;
            }
            if visible && facing && m != 0.0 && secs > 0.0 {
                commands.entity(vic.entity).insert(Stunned {
                    left: secs,
                    damage_per_sec: st.damage_per_sec * m,
                    kind: st.damage_kind,
                    stops: st.stops,
                    instigator: Some(g.thrower),
                    pending: 0.0,
                });
                what.push(format!(
                    "{:.1} s of its effect ({:.0} damage a second)",
                    secs,
                    st.damage_per_sec * m
                ));
            } else if visible && secs > 0.0 && m == 0.0 {
                what.push("immune to its effect".into());
            }
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
    say(
        log,
        format!(
            "grenades: {} went off after {:.2} s and {} bounces at ({:.0}, {:.0}, {:.0}), radius {}, {hit_count} characters reached",
            g.grenade, g.flight.age, g.flight.bounces, point.x, point.y, point.z, p.radius
        ),
    );
    hit_count
}

/// Stuns and electrocutions wear off; electrocution hurts while it lasts.
pub(crate) fn wear_off_stuns(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut Stunned), Without<Dead>>,
    mut damage: MessageWriter<Damage>,
    transforms: Query<&GlobalTransform>,
) {
    let dt = time.delta_secs();
    for (e, mut s) in &mut q {
        if s.damage_per_sec > 0.0 {
            s.pending += s.damage_per_sec * dt.min(s.left.max(0.0));
        }
        let last = s.left - dt <= 0.0;
        // Nothing to send for an effect that doesn't hurt (a stun): a 0-damage
        // hit would still hold off the shield's recharge.
        if s.pending > 0.0 && (s.pending >= s.damage_per_sec * 0.25 || last) {
            let amount = std::mem::take(&mut s.pending);
            damage.write(Damage {
                target: e,
                amount,
                kind: s.kind,
                instigator: s.instigator,
                point: transforms.get(e).map_or(Vec3::ZERO, |t| t.translation()),
                direction: Vec3::ZERO,
            });
        }
        s.left -= dt;
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
