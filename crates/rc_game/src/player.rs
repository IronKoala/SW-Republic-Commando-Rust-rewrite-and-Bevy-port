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
    PlayerCamera, PlayerStart,
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

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::InGame), spawn_player)
            .add_systems(Update, move_body.in_set(GameSet::Simulate).run_if(playing))
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
    mut body: Query<(&mut Body, &mut Transform), With<Player>>,
    log: Res<GameLog>,
) {
    let Ok((mut body, mut t)) = body.single_mut() else {
        return;
    };
    let dt = time.delta_secs().min(0.25);
    body.yaw = (body.yaw + actions.look.x).rem_euclid(std::f32::consts::TAU);
    body.pitch = (body.pitch + actions.look.y).clamp(-1.5, 1.5);
    let input = MoveInput {
        forward: actions.movement.y,
        strafe: actions.movement.x,
        yaw: body.yaw,
        jump: actions.pressed(Action::Jump),
        crouch: actions.pressed(Action::Crouch),
        walk: actions.pressed(Action::Walk),
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
    body: Query<&Body, With<Player>>,
    window: Query<&Window>,
    mut view: Query<(&mut Transform, &mut Projection), With<PlayerCamera>>,
) {
    let (Ok(body), Ok((mut t, mut proj))) = (body.single(), view.single_mut()) else {
        return;
    };
    t.translation = to_bevy(body.pawn.eye());
    t.rotation = view_rotation(body.yaw, body.pitch);
    // The game's field of view is across the screen; Bevy's is up and down.
    if let (Some(w), Projection::Perspective(p)) = (window.iter().next(), &mut *proj) {
        let aspect = w.width() / w.height().max(1.0);
        let half = (fov.0.to_radians() / 2.0).tan();
        p.fov = 2.0 * (half / aspect).atan();
    }
}
