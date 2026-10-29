//! The player's melee hit and the sniper attachment's zoom.

use crate::damage_types::DamageTable;
use crate::fire::{to_bevy, to_unreal};
use crate::health::{Damage, DamageKind, Dead};
use crate::library::WeaponLibrary;
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

/// Whether the player is zoomed in: the weapon, and which of its zoom
/// steps (0 is the first).
#[derive(Resource, Default, Debug)]
pub struct Zoom {
    pub weapon: Option<(String, usize)>,
}

impl Zoom {
    /// The weapon zoomed in, if any.
    pub fn zoomed(&self) -> Option<&str> {
        self.weapon.as_ref().map(|(w, _)| w.as_str())
    }
}

/// The zoom button steps through a weapon's zoom levels and back out;
/// switching weapon or reloading leaves zoom.
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
    let steps = lib.weapons.get(id).map_or(0, |p| p.zoom_fovs.len());
    if zoom
        .weapon
        .as_ref()
        .is_some_and(|(w, _)| w != id || state.reloading.is_some())
    {
        zoom.weapon = None;
        say(&log, "weapons: zoom off".into());
    }
    if actions.just_pressed(Action::Zoom) && steps > 0 {
        zoom.weapon = match zoom.weapon.take() {
            None => Some((id.clone(), 0)),
            Some((w, i)) if i + 1 < steps => Some((w, i + 1)),
            Some(_) => None,
        };
        match &zoom.weapon {
            Some((w, i)) => say(&log, format!("weapons: zoom step {} ({w})", i + 1)),
            None => say(&log, "weapons: zoom off".into()),
        }
    }
}

pub(crate) fn zoom_readout(
    lib: Res<WeaponLibrary>,
    zoom: Option<Res<Zoom>>,
    mut out: ResMut<WeaponReadout>,
) {
    let fov = zoom.and_then(|z| z.weapon.clone()).and_then(|(w, i)| {
        lib.weapons
            .get(&w)
            .and_then(|p| p.zoom_fovs.get(i).copied())
    });
    if out.zoom_fov != fov {
        out.zoom_fov = fov;
    }
}

/// The melee button: hit the first character within reach in front of the
/// eye with the weapon in hand's melee (`MeleeRange`, `MeleeDamage`), or
/// the player's own. **Rule** (the original's weapon melee): a straight
/// line from the eye along the view, that long, against the characters'
/// cylinders (walls block it); the damage type is `CTDamageMelee`, scaled
/// by the target's damage table (battle droids take 3 times).
#[allow(clippy::too_many_arguments)]
pub(crate) fn player_melee(
    mut commands: Commands,
    actions: Res<ActionState>,
    time: Res<Time>,
    lib: Res<WeaponLibrary>,
    players: Query<(Entity, Option<&Arsenal>), (With<Player>, Without<Dead>)>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    targets: Query<(Entity, &GlobalTransform, &Hittable, Option<&DamageTable>), Without<Dead>>,
    level: Option<Res<rc_game_core::LevelCollision>>,
    mut cooldown: Local<f32>,
    mut damage: MessageWriter<Damage>,
    log: Option<Res<GameLog>>,
) {
    *cooldown = (*cooldown - time.delta_secs()).max(0.0);
    if !actions.just_pressed(Action::Melee) || *cooldown > 0.0 {
        return;
    }
    let (Ok((me, arsenal)), Ok(cam)) = (players.single(), camera.single()) else {
        return;
    };
    let m = &lib.player.melee;
    let (range, base) = arsenal
        .and_then(|a| a.current())
        .and_then(|(id, _)| lib.weapons.get(id))
        .and_then(|p| p.melee)
        .unwrap_or((m.range, m.damage));
    *cooldown = m.delay;
    let eye = to_unreal(cam.translation());
    let f = cam.forward();
    let view = Vec3::new(f.x, f.z, f.y).normalize_or_zero();
    let end = eye + view * range;
    let wall = level.as_deref().and_then(|l| {
        let v = |p: Vec3| rc_movement::Vec3::new(p.x, p.y, p.z);
        l.0.sweep(rc_movement::Vec3::ZERO, v(eye), v(end))
            .map(|h| h.time)
    });
    let mut best: Option<(f32, Entity, Option<String>)> = None;
    for (e, gt, hit, table) in &targets {
        if e == me {
            continue;
        }
        let c = to_unreal(gt.translation()) + Vec3::Z * hit.center_up;
        if let Some(t) = shot::line_hits_cylinder(eye, end, c, hit.radius, hit.half_height) {
            if wall.is_none_or(|w| t < w) && best.as_ref().is_none_or(|(bt, ..)| t < *bt) {
                best = Some((t, e, table.map(|d| d.0.clone())));
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
        Some((t, e, table)) => {
            let point = eye + (end - eye) * t;
            let mult = lib
                .damage_types
                .multiplier(table.as_deref(), "CTDamageMelee");
            let amount = base * mult;
            say(
                &log,
                format!(
                    "weapons: melee hit {e:?} at {:.0} uu for {amount} ({base} x{mult} by its table)",
                    t * range,
                ),
            );
            damage.write(Damage {
                target: e,
                amount,
                kind: DamageKind::Melee,
                instigator: Some(me),
                point: to_bevy(point),
                direction: Vec3::new(view.x, view.z, view.y),
            });
        }
        None => say(&log, "weapons: melee hit nothing".into()),
    }
}
