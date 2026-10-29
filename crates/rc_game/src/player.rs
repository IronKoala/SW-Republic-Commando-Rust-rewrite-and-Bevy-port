//! The player: a body that walks with the original game's movement
//! (`rc_movement`, as in `rc_walk`) and a camera at its eyes.
//!
//! The body only reads `ActionState`; the camera only follows the body. So a
//! VR mod can replace the camera (and feed head and hand aim into
//! `ActionState`) without touching the movement.

use bevy::camera::Exposure;
use bevy::prelude::*;
use bevy::render::view::Tonemapping;
use rc_game_core::units::to_bevy;
use rc_game_core::{
    playing, Action, ActionState, GameLog, GamePaths, GameSet, GameState, LevelCollision, Player,
    PlayerCamera, PlayerStart, ViewKick,
};
use rc_movement::{MoveInput, MoveParams, Pawn, Vec3 as UVec3, World};
use std::sync::Arc;

/// Field of view across the screen, degrees (from the settings file).
#[derive(Resource, Debug, Clone, Copy)]
pub struct FieldOfView(pub f32);

/// The player's movement and look, in Unreal terms.
#[derive(Component)]
pub struct Body {
    pub pawn: Pawn,
    /// Facing, radians (Unreal yaw).
    pub yaw: f32,
    /// Up/down look, radians, positive is up.
    pub pitch: f32,
    /// Simulated time, seconds.
    pub time: f32,
}

/// Nothing to collide with: used when no level plugin gave `LevelCollision`.
struct Nothing;
impl World for Nothing {
    fn sweep(&self, _: UVec3, _: UVec3, _: UVec3) -> Option<rc_movement::Hit> {
        None
    }
}

pub struct PlayerPlugin;

/// The player's movement. Parts that follow the player (the squad) order
/// themselves after it, so they see this frame's position.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayerMoveSet;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::InGame), spawn_player)
            .add_systems(
                Update,
                move_body
                    .in_set(GameSet::Simulate)
                    .in_set(PlayerMoveSet)
                    .run_if(playing),
            )
            .add_systems(
                Update,
                place_camera
                    .in_set(GameSet::View)
                    .run_if(in_state(GameState::InGame)),
            );
    }
}

/// Unreal yaw and pitch to a Bevy rotation (Bevy cameras look along -Z).
pub fn view_rotation(yaw: f32, pitch: f32) -> Quat {
    Quat::from_rotation_y(-(yaw + std::f32::consts::FRAC_PI_2)) * Quat::from_rotation_x(pitch)
}

fn movement_params(paths: &GamePaths, log: &GameLog) -> MoveParams {
    let base = MoveParams::player_commando();
    let Some(path) = paths.data_file("movement/player_commando.toml") else {
        log.line("player: no movement/player_commando.toml, using built-in values");
        return base;
    };
    match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|t| base.clone().with_overrides(&t))
    {
        Ok(p) => {
            log.line(format!("player: movement settings from {}", path.display()));
            p
        }
        Err(e) => {
            log.line(format!(
                "player: ERROR in {}: {e}; using built-in values",
                path.display()
            ));
            base
        }
    }
}

fn spawn_player(
    mut commands: Commands,
    paths: Res<GamePaths>,
    start: Option<Res<PlayerStart>>,
    collision: Option<Res<LevelCollision>>,
    log: Res<GameLog>,
) {
    let params = movement_params(&paths, &log);
    let (feet, yaw) = start.map(|s| (s.feet, s.yaw)).unwrap_or((UVec3::ZERO, 0.0));
    if collision.is_none() {
        commands.insert_resource(LevelCollision(Arc::new(Nothing)));
        log.line("player: no level collision; nothing is solid");
    }
    log.line(format!(
        "player: spawned, feet ({:.1}, {:.1}, {:.1}) facing {:.0} deg",
        feet.x,
        feet.y,
        feet.z,
        yaw.to_degrees()
    ));
    commands.spawn((
        Player,
        Body {
            pawn: Pawn::new(params, feet),
            yaw,
            pitch: 0.0,
            time: 0.0,
        },
        Transform::from_translation(to_bevy(feet)),
        DespawnOnExit(GameState::InGame),
    ));
    // Same camera settings as the level viewer, which the baked light is
    // matched to.
    commands.spawn((
        PlayerCamera,
        Camera3d::default(),
        Tonemapping::None,
        Exposure::default(),
        Projection::Perspective(PerspectiveProjection {
            near: 0.05,
            far: 5000.0,
            ..default()
        }),
        Transform::default(),
        DespawnOnExit(GameState::InGame),
    ));
}

fn move_body(
    time: Res<Time>,
    actions: Res<ActionState>,
    collision: Res<LevelCollision>,
    mut kicks: MessageReader<ViewKick>,
    mut body: Query<(&mut Body, &mut Transform, Has<rc_combat::Dead>), With<Player>>,
    log: Res<GameLog>,
) {
    let Ok((mut body, mut t, down)) = body.single_mut() else {
        return;
    };
    let dt = time.delta_secs().min(0.25);
    // Recoil adds to the look. (The original springs back afterwards; not
    // done yet.)
    let kick = kicks
        .read()
        .fold(Vec2::ZERO, |k, v| k + Vec2::new(v.yaw, v.pitch));
    body.yaw = (body.yaw + actions.look.x + kick.x).rem_euclid(std::f32::consts::TAU);
    body.pitch = (body.pitch + actions.look.y + kick.y).clamp(-1.5, 1.5);
    // Down (health at 0, waiting for a squad mate to revive): can look
    // around but not move.
    let input = if down {
        MoveInput {
            yaw: body.yaw,
            ..Default::default()
        }
    } else {
        MoveInput {
            forward: actions.movement.y,
            strafe: actions.movement.x,
            yaw: body.yaw,
            jump: actions.pressed(Action::Jump),
            crouch: actions.pressed(Action::Crouch),
            walk: actions.pressed(Action::Walk),
        }
    };
    let events = body.pawn.step(dt, &input, collision.0.as_ref());
    body.time += dt;
    for e in &events {
        log.line(format!("player: t={:.2} {e:?}", body.time));
    }
    t.translation = to_bevy(body.pawn.feet());
    t.rotation = Quat::from_rotation_y(-(body.yaw + std::f32::consts::FRAC_PI_2));
}

fn place_camera(
    fov: Res<FieldOfView>,
    weapon: Option<Res<rc_game_core::WeaponReadout>>,
    body: Query<&Body, With<Player>>,
    window: Query<&Window>,
    log: Res<GameLog>,
    mut logged: Local<(f32, f32)>,
    mut view: Query<(&mut Transform, &mut Projection), With<PlayerCamera>>,
) {
    let (Ok(body), Ok((mut t, mut proj))) = (body.single(), view.single_mut()) else {
        return;
    };
    t.translation = to_bevy(body.pawn.eye());
    t.rotation = view_rotation(body.yaw, body.pitch);
    // Bevy's field of view is up and down, and it widens the sides to fit
    // the window's shape: that is "Hor+" as long as the up-and-down view
    // doesn't change with the shape.
    if let Projection::Perspective(p) = &mut *proj {
        // A zoomed weapon (the sniper attachment) narrows the view.
        let across = weapon.and_then(|w| w.zoom_fov).unwrap_or(fov.0);
        p.fov = vertical_fov(across);
        // Log the angles whenever the window's shape or the setting changes.
        if let Some(w) = window.iter().next() {
            let aspect = w.width() / w.height().max(1.0);
            if (aspect, fov.0) != *logged {
                *logged = (aspect, fov.0);
                log.line(format!(
                    "player: view {:.0}x{:.0} (aspect {aspect:.3}): {:.1} degrees across, {:.1} up and down",
                    w.width(),
                    w.height(),
                    horizontal_fov(p.fov, aspect),
                    p.fov.to_degrees()
                ));
            }
        }
    }
}

/// The setting is the original's: degrees across a 4:3 screen (the only
/// shape the original's resolutions came in). Turn it into the up-and-down
/// angle, radians, which then stays the same at every screen shape.
pub fn vertical_fov(horizontal_4_3_degrees: f32) -> f32 {
    let half = (horizontal_4_3_degrees.to_radians() / 2.0).tan();
    2.0 * (half / (4.0 / 3.0)).atan()
}

/// The angle across a screen of this shape (width / height), degrees.
pub fn horizontal_fov(vertical: f32, aspect: f32) -> f32 {
    (2.0 * ((vertical / 2.0).tan() * aspect).atan()).to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hor_plus_field_of_view() {
        // 85 across 4:3 is 69.0 up and down, at every shape.
        let v = vertical_fov(85.0);
        assert!((v.to_degrees() - 69.0).abs() < 0.05, "{}", v.to_degrees());
        assert!((horizontal_fov(v, 4.0 / 3.0) - 85.0).abs() < 0.01);
        // Wider screens see more at the sides.
        assert!((horizontal_fov(v, 16.0 / 9.0) - 101.4).abs() < 0.1);
        assert!((horizontal_fov(v, 21.0 / 9.0) - 116.1).abs() < 0.1);
        assert!((horizontal_fov(v, 32.0 / 9.0) - 135.5).abs() < 0.1);
    }
}
