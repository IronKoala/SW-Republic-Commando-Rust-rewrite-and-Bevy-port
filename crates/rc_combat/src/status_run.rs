//! The Bevy side of status effects (see `crate::status` for the rules):
//! start the ones asked for with [`SpawnStatusEffect`], hurt with them each
//! frame, and take them away when their time is up.

use crate::damage_types::DamageTable;
use crate::fire::{to_unreal, CombatRng};
use crate::health::{Damage, Dead, Invulnerable};
use crate::library::WeaponLibrary;
use crate::status::{
    add_status, in_reach, status_time, Added, Commando, SpawnStatusEffect, StatusEffect,
    StatusFrom, Target,
};
use crate::Hittable;
use bevy::prelude::*;
use rc_game_core::{GameLog, LevelCollision, Player, PlayerCamera};

fn say(log: &Option<Res<GameLog>>, s: String) {
    if let Some(l) = log {
        l.line(s);
    }
}

type Targets<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static GlobalTransform,
        &'static Hittable,
        Option<&'static DamageTable>,
        Has<Player>,
        Has<Commando>,
        Has<Invulnerable>,
        Option<&'static StatusEffect>,
        Has<Dead>,
    ),
>;

/// Start every status effect asked for this frame.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_status_effects(
    mut commands: Commands,
    mut asks: MessageReader<SpawnStatusEffect>,
    lib: Res<WeaponLibrary>,
    mut rng: ResMut<CombatRng>,
    targets: Targets,
    level: Option<Res<LevelCollision>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    log: Option<Res<GameLog>>,
) {
    let v = |p: Vec3| rc_movement::Vec3::new(p.x, p.y, p.z);
    // A line from the start to a character's middle meets no level
    // geometry or mover (the native check's trace).
    let sees = |a: Vec3, b: Vec3| {
        !level.as_deref().is_some_and(|l| {
            l.0.sweep(rc_movement::Vec3::ZERO, v(a), v(b))
                .is_some_and(|h| h.time < 0.999)
        })
    };
    let player_view = camera.single().ok().map(|c| {
        let f = c.forward();
        Vec3::new(f.x, f.z, f.y)
    });
    // What this frame has given so far, so two asks in one frame see each
    // other's effects (the original spawns them at once).
    let mut given: Vec<(Entity, StatusEffect)> = Vec::new();
    for ask in asks.read() {
        let Some(class) = lib.status.get(&ask.class) else {
            say(
                &log,
                format!(
                    "status effects: {} asked for {}, which has no file in status_effects; nothing happens",
                    ask.by, ask.class
                ),
            );
            continue;
        };
        let middle = |e: Entity| {
            targets
                .get(e)
                .ok()
                .map(|(_, gt, h, ..)| to_unreal(gt.translation()) + Vec3::Z * h.center_up)
        };
        let (start, origin, also_origin) = match ask.from {
            StatusFrom::Point(p) => (to_unreal(p), None, false),
            StatusFrom::Character { entity, also_them } => match middle(entity) {
                Some(m) => (m, Some(entity), also_them),
                None => {
                    say(
                        &log,
                        format!(
                            "status effects: {} {}: its character is gone; nothing happens",
                            ask.by, class.class
                        ),
                    );
                    continue;
                }
            },
        };
        // (who, seconds, damage a second, how far)
        let mut list: Vec<(Entity, f32, f32, f32)> = Vec::new();
        if let (Some(o), true) = (origin, also_origin) {
            // `AffectTaggedActor`: the full time, no random part.
            list.push((o, ask.max_time, ask.damage_per_sec, 0.0));
        }
        for (e, gt, hit, ..) in &targets {
            if Some(e) == origin {
                continue;
            }
            let centre = to_unreal(gt.translation()) + Vec3::Z * hit.center_up;
            let dist = (centre - start).length();
            if !in_reach(dist, ask.radius) || !sees(start, centre) {
                continue;
            }
            let secs = status_time(ask.max_time, dist, hit.radius, ask.radius);
            let dps = if class.random_damage {
                ask.damage_per_sec + (rng.0.next_f32() - 0.5)
            } else {
                ask.damage_per_sec
            };
            list.push((e, secs, dps, dist));
        }
        let mut lines = Vec::new();
        for (e, secs, dps, dist) in list {
            let Ok((_, _, _, table, player, commando, invulnerable, existing, dead)) =
                targets.get(e)
            else {
                continue;
            };
            if player && class.player_must_face {
                let centre = middle(e).unwrap_or(start);
                let facing = player_view.is_some_and(|f| {
                    f.normalize_or_zero()
                        .dot((start - centre).normalize_or_zero())
                        > (crate::throw::PLAYER_FOV.to_radians() * 0.5).cos()
                });
                if !facing {
                    lines.push(format!("{e:?} (player) looked away"));
                    continue;
                }
            }
            let multiplier = lib
                .damage_types
                .multiplier(table.map(|t| t.0.as_str()), &class.damage_type);
            let now = given
                .iter()
                .rev()
                .find(|(g, _)| *g == e)
                .map(|(_, s)| s.clone())
                .or_else(|| existing.cloned());
            let target = Target {
                player,
                commando,
                multiplier,
                invulnerable,
                existing: now.as_ref(),
            };
            let who = format!(
                "{e:?}{}{}{} at {dist:.0} uu",
                if player { " (player)" } else { "" },
                if commando { " (commando)" } else { "" },
                if dead { " (dead)" } else { "" }
            );
            match add_status(&lib.status, class, target, secs, dps, ask.instigator) {
                Added::Immune => lines.push(format!("{who}: immune (x{multiplier})")),
                Added::HasAnother(c) => lines.push(format!("{who}: already has {c}")),
                Added::NoTime => lines.push(format!("{who}: no time")),
                Added::MoreTime(t) => {
                    let mut s = now.clone().unwrap();
                    s.left += t;
                    lines.push(format!("{who}: {t:.2} s more ({:.2} s left)", s.left));
                    commands.entity(e).insert(s.clone());
                    given.push((e, s));
                }
                Added::New(s) => {
                    lines.push(format!(
                        "{who}: {:.2} s at {:.2} damage a second{}",
                        s.left,
                        s.damage_per_sec,
                        if s.stops { ", stopped" } else { "" }
                    ));
                    commands.entity(e).insert(s.clone());
                    given.push((e, s));
                }
            }
        }
        say(
            &log,
            format!(
                "status effects: {} starts {} at ({:.0}, {:.0}, {:.0}), radius {}, up to {} s, {} damage a second: {}",
                ask.by,
                class.class,
                start.x,
                start.y,
                start.z,
                ask.radius,
                ask.max_time,
                ask.damage_per_sec,
                if lines.is_empty() {
                    "nobody in reach".to_string()
                } else {
                    lines.join("; ")
                }
            ),
        );
    }
}

/// Status effects hurt while they last, and end when their time is up.
/// A dead character keeps its effect until then (it does no more harm),
/// as in the original.
pub(crate) fn wear_off_status_effects(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut StatusEffect, Has<Dead>)>,
    mut damage: MessageWriter<Damage>,
    transforms: Query<&GlobalTransform>,
    log: Option<Res<GameLog>>,
) {
    let dt = time.delta_secs();
    for (e, mut s, dead) in &mut q {
        if s.damage_per_sec > 0.0 && !dead {
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
            say(
                &log,
                format!("status effects: {e:?}'s {} wore off", s.class),
            );
            commands.entity(e).remove::<StatusEffect>();
        }
    }
}
