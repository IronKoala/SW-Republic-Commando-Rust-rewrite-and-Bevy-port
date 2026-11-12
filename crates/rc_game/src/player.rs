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
    playing, Action, ActionState, Cutscene, GameLog, GamePaths, GameSet, GameState, LevelCollision,
    Manning, Player, PlayerCamera, PlayerStart, ShakeView, StopShake, TeleportPlayer, ViewKick,
    ViewShake,
};
use rc_movement::{Mode, MoveInput, MoveParams, Pawn, Vec3 as UVec3, World};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Field of view (from the settings file), degrees.
#[derive(Resource, Debug, Clone, Copy)]
pub struct FieldOfView {
    /// Across a 4:3 screen (`video.fov`).
    pub across_4_3: f32,
    /// The longest screen shape (long side over short side) the view keeps
    /// widening for (`video.widest_shape`); 0 for no limit. Off while the
    /// Panini view is on, which takes the stretching out its own way.
    pub widest_shape: f32,
}

impl FieldOfView {
    pub fn from_settings(s: &crate::settings::Settings) -> Self {
        Self {
            across_4_3: s.video.fov,
            widest_shape: if s.video.panini > 0.0 {
                0.0
            } else {
                s.video.widest_shape
            },
        }
    }
}

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
            .init_resource::<rc_game_core::PlayerPawn>()
            .add_systems(
                Update,
                (move_body, publish_pawn)
                    .chain()
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

/// Share the body's centre and look with the other parts
/// ([`rc_game_core::PlayerPawn`]).
fn publish_pawn(
    bodies: Query<&Body, With<rc_game_core::Player>>,
    mut pawn: ResMut<rc_game_core::PlayerPawn>,
) {
    if let Ok(b) = bodies.single() {
        let now = rc_game_core::PlayerPawn {
            centre: arr(b.pawn.location),
            yaw: b.yaw,
            pitch: b.pitch,
        };
        if *pawn != now {
            *pawn = now;
        }
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
    mut shake: ResMut<ViewShake>,
    log: Res<GameLog>,
) {
    // A new level has a new player controller: no shake.
    shake.reset();
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

#[allow(clippy::too_many_arguments)]
fn move_body(
    time: Res<Time>,
    actions: Res<ActionState>,
    collision: Res<LevelCollision>,
    movers: Option<Res<rc_movers_plugin::LevelMovers>>,
    mut kicks: MessageReader<ViewKick>,
    (cut, mut teleports, use_item): (
        Res<Cutscene>,
        MessageReader<TeleportPlayer>,
        Res<rc_game_core::UseItem>,
    ),
    (mut shake, mut shakes, mut stops, mut shake_log): (
        ResMut<ViewShake>,
        MessageReader<ShakeView>,
        MessageReader<StopShake>,
        Local<ShakeLog>,
    ),
    mut body: Query<
        (
            &mut Body,
            &mut Transform,
            Has<rc_combat::Dead>,
            Option<&Manning>,
        ),
        With<Player>,
    >,
    log: Res<GameLog>,
    mut faced: Local<bool>,
) {
    let Ok((mut body, mut t, down, manning)) = body.single_mut() else {
        return;
    };
    let dt = time.delta_secs().min(0.25);
    // A cutscene moves the player to another room, facing the way the
    // arrival point faces.
    for tp in teleports.read() {
        let [x, y, z] = tp.location;
        body.pawn.location = UVec3::new(x, y, z);
        body.pawn.velocity = UVec3::ZERO;
        body.yaw = tp.yaw.rem_euclid(std::f32::consts::TAU);
        body.pitch = 0.0;
        log.line(format!(
            "player: t={:.2} moved by the cutscene to ({x:.1}, {y:.1}, {z:.1}) facing {:.0} deg ({})",
            body.time,
            body.yaw.to_degrees(),
            tp.by
        ));
    }
    // On a mounted turret: the body is held where the turret part says and
    // looks where the turret points (the turret part turns it from the
    // look input, within the turret's limits).
    if let Some(m) = manning {
        kicks.clear();
        let [x, y, z] = m.body;
        body.pawn.location = UVec3::new(x, y, z);
        body.pawn.velocity = UVec3::ZERO;
        body.yaw = m.yaw.rem_euclid(std::f32::consts::TAU);
        body.pitch = m.pitch.clamp(-1.5, 1.5);
        body.time += dt;
        shake_view(
            &mut shake,
            &mut shakes,
            &mut stops,
            &mut body,
            dt,
            &mut shake_log,
            &log,
        );
        t.translation = to_bevy(body.pawn.feet());
        t.rotation = Quat::from_rotation_y(-(body.yaw + std::f32::consts::FRAC_PI_2));
        return;
    }
    // Recoil adds to the look. (The original springs back afterwards; not
    // done yet.)
    let kick = kicks
        .read()
        .fold(Vec2::ZERO, |k, v| k + Vec2::new(v.yaw, v.pitch));
    body.yaw = (body.yaw + actions.look.x + kick.x).rem_euclid(std::f32::consts::TAU);
    body.pitch = (body.pitch + actions.look.y + kick.y).clamp(-1.5, 1.5);
    // Held by a cutscene: looking stays inside its limits.
    if let Some(held) = &cut.held {
        let (yaw, pitch) = held.limit(body.yaw, body.pitch);
        body.yaw = yaw;
        body.pitch = pitch.clamp(-1.5, 1.5);
    }
    // The view shake comes right after the player's own turning and its
    // limits, as in the original (`UpdateRotation` calls `ViewShake`).
    shake_view(
        &mut shake,
        &mut shakes,
        &mut stops,
        &mut body,
        dt,
        &mut shake_log,
        &log,
    );
    // The cutscene stopped the body's physics: it stays exactly where it
    // is (the prologue's newborn clone doesn't fall).
    if cut.frozen {
        body.pawn.velocity = UVec3::ZERO;
        body.time += dt;
        t.translation = to_bevy(body.pawn.feet());
        t.rotation = Quat::from_rotation_y(-(body.yaw + std::f32::consts::FRAC_PI_2));
        return;
    }
    // Using something that holds the player (a bacta dispenser): pulled
    // to its spot and turned to face it, then held there. Looking around
    // stays; the body only falls or rides a lift.
    // Once turned to face it, the player may look around again.
    if let Some(lock) = &use_item.lock {
        let pawn = &body.pawn;
        let (to, yaw, there) = lock.pull(
            [pawn.location.x, pawn.location.y],
            body.yaw,
            lock.speed * dt,
            if *faced { 0.0 } else { lock.turn * dt },
        );
        body.pawn.location.x = to[0];
        body.pawn.location.y = to[1];
        body.pawn.velocity.x = 0.0;
        body.pawn.velocity.y = 0.0;
        if !*faced {
            body.yaw = yaw;
        }
        if there && !*faced {
            *faced = true;
            log.line(format!(
                "player: t={:.2} pulled into place at ({:.1}, {:.1}) facing {:.0} deg",
                body.time,
                to[0],
                to[1],
                yaw.to_degrees()
            ));
        }
    } else {
        *faced = false;
    }
    // Down (health at 0, waiting for a squad mate to revive): can look
    // around but not move.
    let input = if down || use_item.lock.is_some() {
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

/// What the log says about the shake running now.
#[derive(Default)]
struct ShakeLog {
    by: String,
    frames: u32,
    /// Biggest camera movement, Unreal units, and biggest turn in one
    /// frame (pitch, yaw), Unreal rotation units.
    biggest_move: f32,
    biggest_turn: (i32, i32),
    /// Total turn added (pitch, yaw), Unreal rotation units.
    total_turn: (i64, i64),
}

/// New shakes and stops, then one frame of shake (see
/// `rc_game_core::shake`): the turn goes into where the body looks (it
/// stays, as in the original), the movement into `ViewShake::offset` for
/// the camera.
fn shake_view(
    shake: &mut ViewShake,
    shakes: &mut MessageReader<ShakeView>,
    stops: &mut MessageReader<StopShake>,
    body: &mut Body,
    dt: f32,
    slog: &mut ShakeLog,
    log: &GameLog,
) {
    for _ in stops.read() {
        if shake.state.running() {
            log.line(format!("player: t={:.2} shake stopped", body.time));
        }
        shake.state.stop();
    }
    for s in shakes.read() {
        let fade = shake.state.fade();
        let taken = shake.state.request(s);
        log.line(format!(
            "player: t={:.2} shake from {}: in {} s, hold {} s, out {} s, move ({}, {}, {}) uu, rotation yaw {} pitch {}, frequency {}{}",
            body.time,
            s.by,
            s.in_time,
            s.sustain_time,
            s.out_time,
            s.offset[0],
            s.offset[1],
            s.offset[2],
            s.yaw_mag,
            s.pitch_mag,
            s.frequency,
            if taken {
                String::new()
            } else {
                format!(" -- ignored: the running shake ({}) is stronger now (fade {fade:.2})", slog.by)
            }
        ));
        if taken {
            *slog = ShakeLog {
                by: s.by.clone(),
                ..ShakeLog::default()
            };
        }
    }
    let was_running = shake.state.running() && shake.state.elapsed < shake.state.time_out;
    let Some(step) = shake.step(dt) else {
        return;
    };
    if was_running {
        slog.frames += 1;
        let m = step.offset.iter().map(|x| x * x).sum::<f32>().sqrt();
        slog.biggest_move = slog.biggest_move.max(m);
        slog.biggest_turn.0 = slog.biggest_turn.0.max(step.pitch.abs());
        slog.biggest_turn.1 = slog.biggest_turn.1.max(step.yaw.abs());
        slog.total_turn.0 += step.pitch as i64;
        slog.total_turn.1 += step.yaw as i64;
        if shake.state.elapsed >= shake.state.time_out {
            log.line(format!(
                "player: t={:.2} shake from {} over: {} frames, camera moved up to {:.2} uu, turned up to {} pitch / {} yaw a frame, {} / {} in all (65536 = full turn){}",
                body.time,
                slog.by,
                slog.frames,
                slog.biggest_move,
                slog.biggest_turn.0,
                slog.biggest_turn.1,
                slog.total_turn.0,
                slog.total_turn.1,
                if shake.turn_view { "" } else { "; turning switched off" }
            ));
        }
    }
    if shake.turn_view && (step.pitch != 0 || step.yaw != 0) {
        let unit = std::f32::consts::TAU / 65536.0;
        body.pitch = (body.pitch + step.pitch as f32 * unit).clamp(-1.5, 1.5);
        body.yaw = (body.yaw + step.yaw as f32 * unit).rem_euclid(std::f32::consts::TAU);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn place_camera(
    fov: Res<FieldOfView>,
    weapon: Option<Res<rc_game_core::WeaponReadout>>,
    body: Query<&Body, With<Player>>,
    area: Option<Res<crate::display::PlayArea>>,
    log: Res<GameLog>,
    mut logged: Local<(f32, f32, f32)>,
    mut view: Query<(&mut Transform, &mut Projection), With<PlayerCamera>>,
    shake: Res<ViewShake>,
) {
    let (Ok(body), Ok((mut t, mut proj))) = (body.single(), view.single_mut()) else {
        return;
    };
    *t = eye_view(body);
    // The view shake moves the camera off the eyes (world axes), as the
    // original adds `ShakeOffset` to the eye position.
    let [x, y, z] = shake.camera_offset();
    t.translation += to_bevy(UVec3::new(x, y, z));
    // Bevy's field of view is up and down, and it widens the sides to fit
    // the window's shape: that is "Hor+" as long as the up-and-down view
    // doesn't change with the shape. On screens longer than `widest_shape`,
    // a flat view would stretch things at the sides into a fish-eye look,
    // so there the angle across stays what it is at that shape and the
    // view shows a little less up and down instead.
    if let Projection::Perspective(p) = &mut *proj {
        // The picture's shape: the window's, or the box inside black bars.
        let w = area.as_deref().map(|a| a.size.as_vec2());
        let aspect = w.map_or(p.aspect_ratio, |w| w.x / w.y.max(1.0));
        // A zoomed weapon (the sniper attachment) narrows the view.
        let across = weapon.and_then(|w| w.zoom_fov).unwrap_or(fov.across_4_3);
        p.fov = view_angle(across, fov.widest_shape, aspect);
        // Log the angles whenever the window's shape or a setting changes.
        let Some(w) = w else {
            return;
        };
        if (aspect, fov.across_4_3, fov.widest_shape) != *logged {
            *logged = (aspect, fov.across_4_3, fov.widest_shape);
            let capped = p.fov < vertical_fov(across) - 1e-4;
            log.line(format!(
                "player: view {:.0}x{:.0} (aspect {aspect:.3}): {:.1} degrees across, {:.1} up and down (setting {:.0} across 4:3, widest {}{})",
                w.x,
                w.y,
                horizontal_fov(p.fov, aspect),
                p.fov.to_degrees(),
                fov.across_4_3,
                if fov.widest_shape > 0.0 { format!("shape {:.2}", fov.widest_shape) } else { "no limit".into() },
                if capped { ", wider screen than that: held" } else { "" },
            ));
        }
    }
}

/// The up-and-down angle, radians, for a setting of `across_4_3` degrees
/// across a 4:3 screen on a screen of this shape (width / height).
///
/// The screen's shorter side always sees the original's up-and-down angle
/// (69.0 degrees for 85), and the longer side widens to fit: "Hor+" on a
/// TV, monitor or ultrawide, and the same turned on its side for a phone
/// held upright or a tall flip phone, so those see as much across as a
/// landscape screen sees up and down instead of a narrow slit.
///
/// The longer side stops widening once the screen is `widest_shape` times
/// longer than it is short (0 for no limit): past that a flat view
/// stretches the edges into a fish-eye look, so a longer screen shows the
/// same angle along its long side and a little less along its short one.
/// The limit is a shape, not an angle, so the setting always changes the
/// view on every screen.
pub fn view_angle(across_4_3: f32, widest_shape: f32, aspect: f32) -> f32 {
    let short_side = (vertical_fov(across_4_3) / 2.0).tan();
    let limit = |long: f32| {
        if widest_shape > 0.0 {
            long.min(widest_shape)
        } else {
            long
        }
    };
    if aspect >= 1.0 {
        // Landscape: up and down is the short side.
        2.0 * (short_side * limit(aspect) / aspect).atan()
    } else {
        // Upright: across is the short side; up and down is the long one.
        2.0 * (short_side * limit(1.0 / aspect)).atan()
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
    fn a_shake_turns_the_view_and_moves_the_camera_then_ends() {
        use bevy::ecs::system::RunSystemOnce;
        use rc_game_core::{units::to_unreal, ShakeView, StopShake, TeleportPlayer, UseItem};
        let mut world = bevy::ecs::world::World::new();
        world.insert_resource(GameLog::terminal());
        world.insert_resource(Time::<()>::default());
        world.insert_resource(ActionState::default());
        world.insert_resource(Cutscene::default());
        world.insert_resource(UseItem::default());
        world.insert_resource(LevelCollision(Arc::new(Nothing)));
        world.insert_resource(ViewShake::default());
        world.insert_resource(FieldOfView {
            across_4_3: 85.0,
            widest_shape: 2.0,
        });
        world.init_resource::<Messages<ViewKick>>();
        world.init_resource::<Messages<TeleportPlayer>>();
        world.init_resource::<Messages<ShakeView>>();
        world.init_resource::<Messages<StopShake>>();
        world.spawn((
            Player,
            Body {
                pawn: Pawn::new(MoveParams::player_commando(), UVec3::ZERO),
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
                Projection::Perspective(PerspectiveProjection::default()),
            ))
            .id();
        // geo_01a's falling-droid shake: ACTION_ShakeView with rotation
        // Pitch 100, Yaw 100 and the class's other defaults.
        world.write_message(ShakeView {
            in_time: 0.3,
            sustain_time: 1.0,
            out_time: 0.7,
            offset: [3.0; 3],
            yaw_mag: 100.0,
            pitch_mag: 100.0,
            frequency: 1.0,
            by: "test".into(),
        });
        let mut moved = 0.0f32;
        let mut turned = false;
        // 2.1 s at 60 frames a second: past the 2 s shake.
        for _ in 0..126 {
            world
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_secs_f32(1.0 / 60.0));
            let before = {
                let mut q = world.query_filtered::<&Body, With<Player>>();
                let b = q.iter(&world).next().unwrap();
                (b.yaw, b.pitch)
            };
            world.run_system_once(move_body).unwrap();
            world.run_system_once(place_camera).unwrap();
            // A one-off system reads every message again: send it once.
            world.resource_mut::<Messages<ShakeView>>().clear();
            let mut q = world.query_filtered::<&Body, With<Player>>();
            let b = q.iter(&world).next().unwrap();
            turned |= (b.yaw, b.pitch) != before;
            let eye = eye_view(b).translation;
            let cam_at = world.get::<Transform>(cam).unwrap().translation;
            let off = to_unreal(cam_at - eye);
            let shake = world.resource::<ViewShake>().offset;
            assert!((off.x - shake[0]).abs() < 1e-2 && (off.z - shake[2]).abs() < 1e-2);
            assert!(shake.iter().all(|x| x.abs() <= 3.0 + 1e-4), "{shake:?}");
            moved = moved.max(off.length());
        }
        assert!(moved > 0.5, "the camera moved {moved} uu");
        assert!(turned, "the view never turned");
        // Over: the camera is back at the eyes and the view stays put.
        let v = world.resource::<ViewShake>();
        assert!(v.state.elapsed >= v.state.time_out);
        assert_eq!(v.offset, [0.0; 3]);
    }

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
    fn long_screens_stop_widening() {
        // 16:9 is under the 2.0 limit: plain Hor+.
        assert_eq!(view_angle(85.0, 2.0, 16.0 / 9.0), vertical_fov(85.0));
        // The user's maximized ultrawide (3440x1369) sees what a 2:1
        // screen sees across, 107.9, and a little less up and down.
        let aspect = 3440.0 / 1369.0;
        let v = view_angle(85.0, 2.0, aspect);
        assert!((horizontal_fov(v, aspect) - 107.9).abs() < 0.1);
        assert!(v < vertical_fov(85.0));
        // The setting still changes the view there (it didn't with the
        // first try, a fixed 110 degrees, which held 95 to 120 all at 110).
        let mut last = 0.0;
        for fov in [60.0, 85.0, 95.0, 100.0, 110.0, 120.0] {
            let across = horizontal_fov(view_angle(fov, 2.0, aspect), aspect);
            assert!(across > last + 1.0, "{fov}: {across}");
            last = across;
        }
        // 0 is no limit: 32:9 goes to 135.5 again.
        let v = view_angle(85.0, 0.0, 32.0 / 9.0);
        assert!((horizontal_fov(v, 32.0 / 9.0) - 135.5).abs() < 0.1);
    }

    /// Every common screen shape: (name, width, height, degrees across,
    /// degrees up and down) at the default 85 and widest shape 2.0. The
    /// same table is in docs/DISPLAY_AND_INPUT.md.
    const SHAPES: [(&str, f32, f32, f32, f32); 14] = [
        ("4:3 old monitor or iPad", 1024.0, 768.0, 85.0, 69.0),
        ("16:10 monitor or Steam Deck", 1280.0, 800.0, 95.4, 69.0),
        ("16:9 TV or monitor", 1920.0, 1080.0, 101.4, 69.0),
        ("16:9 maximized window", 1920.0, 1009.0, 105.2, 69.0),
        ("21:9 ultrawide", 2560.0, 1080.0, 107.9, 60.2),
        (
            "3440x1369 ultrawide, maximized",
            3440.0,
            1369.0,
            107.9,
            57.4,
        ),
        ("32:9 super ultrawide", 5120.0, 1440.0, 107.9, 42.3),
        ("phone sideways 19.5:9", 2340.0, 1080.0, 107.9, 64.8),
        ("phone upright 9:19.5", 1080.0, 2340.0, 64.8, 107.9),
        ("phone upright 9:16", 1080.0, 1920.0, 69.0, 101.4),
        ("flip phone open upright 9:22", 1080.0, 2640.0, 58.7, 107.9),
        ("flip phone cover screen", 1056.0, 1066.0, 69.0, 69.5),
        ("foldable opened", 2176.0, 1812.0, 79.1, 69.0),
        ("square", 1000.0, 1000.0, 69.0, 69.0),
    ];

    #[test]
    fn every_screen_shape() {
        for (name, w, h, across, up) in SHAPES {
            let aspect = w / h;
            let v = view_angle(85.0, 2.0, aspect);
            let got_across = horizontal_fov(v, aspect);
            let got_up = v.to_degrees();
            println!("{name} {w}x{h}: {got_across:.1} across, {got_up:.1} up and down");
            assert!(
                (got_across - across).abs() < 0.1 && (got_up - up).abs() < 0.1,
                "{name}: {got_across:.1} across, {got_up:.1} up and down"
            );
        }
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
