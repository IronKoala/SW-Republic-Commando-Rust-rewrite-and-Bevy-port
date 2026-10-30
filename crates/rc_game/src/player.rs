//! The player: a body that walks with the original game's movement
//! (`rc_movement`, as in `rc_walk`) and a camera at its eyes.
//!
//! The body only reads `ActionState`; the camera only follows the body. So a
//! VR mod can replace the camera (and feed head and hand aim into
//! `ActionState`) without touching the movement.

use bevy::camera::Exposure;
use bevy::prelude::*;
use bevy::render::view::Tonemapping;
use rc_game_core::save::{SaveAppExt, SavePart};
use rc_game_core::units::to_bevy;
use rc_game_core::{
    playing, Action, ActionState, GameLog, GamePaths, GameSet, GameState, LevelCollision, Player,
    PlayerCamera, PlayerStart, ViewKick,
};
use rc_movement::{Mode, MoveInput, MoveParams, Pawn, Vec3 as UVec3, World};
use serde::{Deserialize, Serialize};
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
        app.add_save_part::<PlayerSave>()
            .add_systems(OnEnter(GameState::InGame), spawn_player)
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

/// The player's `[player]` section in a save file: where the body is and
/// where it looks, in Unreal units and radians. Health and weapons are the
/// weapons part's own section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerSave {
    /// Centre of the collision box.
    pub location: [f32; 3],
    pub velocity: [f32; 3],
    /// On the ground (`true`) or in the air.
    pub walking: bool,
    pub crouched: bool,
    pub yaw: f32,
    pub pitch: f32,
}

fn arr(v: UVec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

impl SavePart for PlayerSave {
    const KEY: &'static str = "player";

    fn save(world: &mut bevy::ecs::world::World) -> Option<Self> {
        let mut q = world.query_filtered::<&Body, With<Player>>();
        let b = q.iter(world).next()?;
        Some(PlayerSave {
            location: arr(b.pawn.location),
            velocity: arr(b.pawn.velocity),
            walking: b.pawn.mode == Mode::Walking,
            crouched: b.pawn.crouched,
            yaw: b.yaw,
            pitch: b.pitch,
        })
    }

    fn load(world: &mut bevy::ecs::world::World, d: Self) -> Result<(), String> {
        let mut q = world.query_filtered::<(&mut Body, &mut Transform), With<Player>>();
        let (mut b, mut t) = q.iter_mut(world).next().ok_or("no player in the level")?;
        let [x, y, z] = d.location;
        b.pawn.location = UVec3::new(x, y, z);
        let [x, y, z] = d.velocity;
        b.pawn.velocity = UVec3::new(x, y, z);
        b.pawn.mode = if d.walking {
            Mode::Walking
        } else {
            Mode::Falling
        };
        b.pawn.crouched = d.crouched;
        b.pawn.eye_height = if d.crouched {
            b.pawn.params.crouched_eye_height()
        } else {
            b.pawn.params.base_eye_height
        };
        b.yaw = d.yaw;
        b.pitch = d.pitch;
        t.translation = to_bevy(b.pawn.feet());
        t.rotation = Quat::from_rotation_y(-(b.yaw + std::f32::consts::FRAC_PI_2));
        let feet = b.pawn.feet();
        let yaw = b.yaw;
        // Move the camera now too. It only follows the body at the end of
        // the frame, and droids look for the player at the camera; left
        // where the level start put it, they saw the player there for a
        // frame after every load (sometimes from across the map).
        let view = eye_view(&b);
        let mut q =
            world.query_filtered::<(&mut Transform, &mut GlobalTransform), With<PlayerCamera>>();
        for (mut t, mut g) in q.iter_mut(world) {
            *t = view;
            *g = GlobalTransform::from(view);
        }
        world.resource::<GameLog>().line(format!(
            "player: put back at feet ({:.1}, {:.1}, {:.1}) facing {:.0} deg",
            feet.x,
            feet.y,
            feet.z,
            yaw.to_degrees()
        ));
        Ok(())
    }
}

/// Where the camera goes for this body: at its eyes, looking where it looks.
fn eye_view(body: &Body) -> Transform {
    Transform::from_translation(to_bevy(body.pawn.eye()))
        .with_rotation(view_rotation(body.yaw, body.pitch))
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
    let body = Body {
        pawn: Pawn::new(params, feet),
        yaw,
        pitch: 0.0,
        time: 0.0,
    };
    // The camera starts at the body's eyes, not at the map's centre, so
    // nothing that looks at the camera before it first follows the body
    // (droids looking for the player, aiming) sees the player elsewhere.
    let view = eye_view(&body);
    commands.spawn((
        Player,
        body,
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
        view,
        GlobalTransform::from(view),
        DespawnOnExit(GameState::InGame),
    ));
}

fn move_body(
    time: Res<Time>,
    actions: Res<ActionState>,
    collision: Res<LevelCollision>,
    movers: Option<Res<rc_movers_plugin::LevelMovers>>,
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
    // Doors and lifts move first, carrying or pushing the player; the
    // player then moves among them (they are in `LevelCollision`).
    if let Some(m) = &movers {
        m.step(dt, &mut body.pawn);
    }
    let events = body.pawn.step(dt, &input, collision.0.as_ref());
    body.time += dt;
    for e in &events {
        log.line(format!("player: t={:.2} {e:?}", body.time));
    }
    t.translation = to_bevy(body.pawn.feet());
    t.rotation = Quat::from_rotation_y(-(body.yaw + std::f32::consts::FRAC_PI_2));
}

pub(crate) fn place_camera(
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
    *t = eye_view(body);
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

    #[test]
    fn loading_a_save_moves_the_camera_with_the_player() {
        // Droids look for the player at the camera. Before this, the camera
        // stayed where the level start put it for a frame after a load, and
        // droids could spot the player there (from thousands of uu).
        let mut world = bevy::ecs::world::World::new();
        world.insert_resource(GameLog::terminal());
        let start = UVec3::new(0.0, 0.0, 0.0);
        world.spawn((
            Player,
            Body {
                pawn: Pawn::new(MoveParams::player_commando(), start),
                yaw: 0.0,
                pitch: 0.0,
                time: 0.0,
            },
            Transform::default(),
        ));
        let cam = world
            .spawn((
                PlayerCamera,
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        let save = PlayerSave {
            location: [3000.0, -1200.0, 250.0],
            velocity: [0.0; 3],
            walking: true,
            crouched: false,
            yaw: 1.0,
            pitch: 0.2,
        };
        PlayerSave::load(&mut world, save).unwrap();
        let mut q = world.query_filtered::<&Body, With<Player>>();
        let want = eye_view(q.iter(&world).next().unwrap());
        let t = *world.get::<Transform>(cam).unwrap();
        let g = world.get::<GlobalTransform>(cam).unwrap().translation();
        assert!((t.translation - want.translation).length() < 1e-3, "{t:?}");
        assert!(t.rotation.angle_between(want.rotation) < 1e-4);
        assert!((g - want.translation).length() < 1e-3, "{g:?}");
        // The eye is at the saved spot, not near where the camera was.
        let eye = rc_game_core::units::to_unreal(t.translation);
        assert!((eye.x - 3000.0).abs() < 0.01 && (eye.y + 1200.0).abs() < 0.01);
    }
}
