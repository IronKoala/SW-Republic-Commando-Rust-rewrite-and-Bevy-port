//! The player's melee hit and the sniper attachment's zoom.

use crate::fire::{to_bevy, to_unreal};
use crate::health::{Damage, DamageKind, Dead};
use crate::library::WeaponLibrary;
use crate::throw::Stunned;
use crate::{shot, Arsenal, Hittable};
use bevy::audio::{PlaybackSettings, Volume};
use bevy::prelude::*;
use rc_game_core::{Action, ActionState, GameLog, Player, PlayerCamera, WeaponReadout};

fn say(log: &Option<Res<GameLog>>, s: String) {
    match log {
        Some(l) => l.line(s),
        None => println!("{s}"),
    }
}

/// Whether the player is zoomed in, and the weapon it was for.
#[derive(Resource, Default, Debug)]
pub(crate) struct Zoom {
    pub weapon: Option<String>,
}

/// The zoom button toggles zoom on a weapon that has one; switching weapon
/// or reloading leaves zoom.
pub(crate) fn player_zoom(
    mut commands: Commands,
    actions: Res<ActionState>,
    lib: Res<WeaponLibrary>,
    zoom: Option<ResMut<Zoom>>,
    players: Query<&Arsenal, (With<Player>, Without<Dead>)>,
    log: Option<Res<GameLog>>,
) {
    let Some(mut zoom) = zoom else {
        commands.init_resource::<Zoom>();
        return;
    };
    let Ok(arsenal) = players.single() else {
        zoom.weapon = None;
        return;
    };
    let Some((id, state)) = arsenal.current() else {
        return;
    };
    let can = lib.weapons.get(id).and_then(|p| p.zoom_fov).is_some();
    if zoom
        .weapon
        .as_ref()
        .is_some_and(|w| w != id || state.reloading.is_some())
    {
        zoom.weapon = None;
        say(&log, "weapons: zoom off".into());
    }
    if actions.just_pressed(Action::Zoom) && can {
        if zoom.weapon.is_some() {
            zoom.weapon = None;
            say(&log, "weapons: zoom off".into());
        } else {
            zoom.weapon = Some(id.clone());
            say(&log, format!("weapons: zoom on ({id})"));
        }
    }
}

pub(crate) fn zoom_readout(
    lib: Res<WeaponLibrary>,
    zoom: Option<Res<Zoom>>,
    mut out: ResMut<WeaponReadout>,
) {
    let fov = zoom
        .and_then(|z| z.weapon.clone())
        .and_then(|w| lib.weapons.get(&w).and_then(|p| p.zoom_fov));
    if out.zoom_fov != fov {
        out.zoom_fov = fov;
    }
}

/// The melee button: hit the first character within reach in front of the
/// eye. **Rule:** a straight line from the eye along the view, `range` uu
/// long, against the characters' cylinders (walls block it).
#[allow(clippy::too_many_arguments)]
pub(crate) fn player_melee(
    mut commands: Commands,
    actions: Res<ActionState>,
    time: Res<Time>,
    lib: Res<WeaponLibrary>,
    players: Query<Entity, (With<Player>, Without<Dead>, Without<Stunned>)>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    targets: Query<(Entity, &GlobalTransform, &Hittable), Without<Dead>>,
    level: Option<Res<rc_game_core::LevelCollision>>,
    mut cooldown: Local<f32>,
    mut damage: MessageWriter<Damage>,
    log: Option<Res<GameLog>>,
) {
    *cooldown = (*cooldown - time.delta_secs()).max(0.0);
    if !actions.just_pressed(Action::Melee) || *cooldown > 0.0 {
        return;
    }
    let (Ok(me), Ok(cam)) = (players.single(), camera.single()) else {
        return;
    };
    let m = &lib.player.melee;
    *cooldown = m.delay;
    let eye = to_unreal(cam.translation());
    let f = cam.forward();
    let view = Vec3::new(f.x, f.z, f.y).normalize_or_zero();
    let end = eye + view * m.range;
    let wall = level.as_deref().and_then(|l| {
        let v = |p: Vec3| rc_movement::Vec3::new(p.x, p.y, p.z);
        l.0.sweep(rc_movement::Vec3::ZERO, v(eye), v(end))
            .map(|h| h.time)
    });
    let mut best: Option<(f32, Entity)> = None;
    for (e, gt, hit) in &targets {
        if e == me {
            continue;
        }
        let c = to_unreal(gt.translation()) + Vec3::Z * hit.center_up;
        if let Some(t) = shot::line_hits_cylinder(eye, end, c, hit.radius, hit.half_height) {
            if wall.is_none_or(|w| t < w) && best.is_none_or(|(bt, _)| t < bt) {
                best = Some((t, e));
            }
        }
    }
    if let Some(s) = m.sound.as_ref().and_then(|n| lib.sounds.get(n)) {
        commands.spawn((
            AudioPlayer(s.clone()),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(1.0)),
        ));
    }
    match best {
        Some((t, e)) => {
            let point = eye + (end - eye) * t;
            say(
                &log,
                format!(
                    "weapons: melee hit {e:?} at {:.0} uu for {}",
                    t * m.range,
                    m.damage
                ),
            );
            damage.write(Damage {
                target: e,
                amount: m.damage,
                kind: DamageKind::Melee,
                instigator: Some(me),
                point: to_bevy(point),
                direction: Vec3::new(view.x, view.z, view.y),
            });
        }
        None => say(&log, "weapons: melee hit nothing".into()),
    }
}
