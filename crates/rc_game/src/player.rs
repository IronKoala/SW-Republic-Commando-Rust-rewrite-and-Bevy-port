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

/// Field of view (from the settings file), degrees.
#[derive(Resource, Debug, Clone, Copy)]
pub struct FieldOfView {
    /// Across a 4:3 screen (`video.fov`).
    pub across_4_3: f32,
    /// The widest the view may get across any screen (`video.widest_fov`);
    /// 0 for no limit. Off while the Panini view is on, which takes the
    /// stretching out its own way.
    pub widest: f32,
}

impl FieldOfView {
    pub fn from_settings(s: &crate::settings::Settings) -> Self {
        Self {
            across_4_3: s.video.fov,
            widest: if s.video.panini > 0.0 {
                0.0
            } else {
                s.video.widest_fov
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
    mut logged: Local<(f32, f32, f32)>,
    mut view: Query<(&mut Transform, &mut Projection), With<PlayerCamera>>,
) {
    let (Ok(body), Ok((mut t, mut proj))) = (body.single(), view.single_mut()) else {
        return;
    };
    *t = eye_view(body);
    // Bevy's field of view is up and down, and it widens the sides to fit
    // the window's shape: that is "Hor+" as long as the up-and-down view
    // doesn't change with the shape. Past `widest` degrees across, a flat
    // view stretches things at the sides into a fish-eye look, so there
    // the angle across stays put and a wider window shows a little less up
    // and down instead.
    if let Projection::Perspective(p) = &mut *proj {
        let w = window.iter().next();
        let aspect = w.map_or(p.aspect_ratio, |w| w.width() / w.height().max(1.0));
        // A zoomed weapon (the sniper attachment) narrows the view.
        let across = weapon.and_then(|w| w.zoom_fov).unwrap_or(fov.across_4_3);
        p.fov = view_angle(across, fov.widest, aspect);
        // Log the angles whenever the window's shape or a setting changes.
        let Some(w) = w else {
            return;
        };
        if (aspect, fov.across_4_3, fov.widest) != *logged {
            *logged = (aspect, fov.across_4_3, fov.widest);
            let capped = p.fov < vertical_fov(across) - 1e-4;
            log.line(format!(
                "player: view {:.0}x{:.0} (aspect {aspect:.3}): {:.1} degrees across, {:.1} up and down (setting {:.0} across 4:3, widest {}{})",
                w.width(),
                w.height(),
                horizontal_fov(p.fov, aspect),
                p.fov.to_degrees(),
                fov.across_4_3,
                if fov.widest > 0.0 { format!("{:.0}", fov.widest) } else { "no limit".into() },
                if capped { ", held at the widest" } else { "" },
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
/// landscape screen sees up and down instead of a narrow slit. The longer
/// side stops at `widest` degrees (0 for no limit), past which a flat view
/// stretches the edges into a fish-eye look.
pub fn view_angle(across_4_3: f32, widest: f32, aspect: f32) -> f32 {
    let short_side = vertical_fov(across_4_3);
    let half = |deg_or_rad: f32| (deg_or_rad / 2.0).tan();
    let limit = if widest > 0.0 {
        half(widest.to_radians())
    } else {
        f32::INFINITY
    };
    if aspect >= 1.0 {
        // Landscape: up and down is the short side.
        let across = (half(short_side) * aspect).min(limit);
        2.0 * (across / aspect).atan()
    } else {
        // Upright: across is the short side; up and down is the long one.
        2.0 * (half(short_side) / aspect).min(limit).atan()
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
    fn widest_view_stops_the_fish_eye() {
        // 16:9 at the original's 85 is under 110 across: plain Hor+.
        assert_eq!(view_angle(85.0, 110.0, 16.0 / 9.0), vertical_fov(85.0));
        // Ultrawide and 32:9 stop at 110 across; the view up and down gets
        // a little shorter.
        for aspect in [21.0 / 9.0, 3440.0 / 1377.0, 32.0 / 9.0] {
            let v = view_angle(85.0, 110.0, aspect);
            assert!((horizontal_fov(v, aspect) - 110.0).abs() < 0.01);
            assert!(v < vertical_fov(85.0));
        }
        // 95 at 16:9 would be 111 across: held at 110.
        let v = view_angle(95.0, 110.0, 16.0 / 9.0);
        assert!((horizontal_fov(v, 16.0 / 9.0) - 110.0).abs() < 0.01);
        // 0 is no limit: 32:9 goes to 135.5 again.
        let v = view_angle(85.0, 0.0, 32.0 / 9.0);
        assert!((horizontal_fov(v, 32.0 / 9.0) - 135.5).abs() < 0.1);
        // A zoomed weapon is far narrower than the limit: untouched.
        assert_eq!(view_angle(30.0, 110.0, 32.0 / 9.0), vertical_fov(30.0));
    }

    /// Every common screen shape: (name, width, height, degrees across,
    /// degrees up and down) at the default 85 and widest 110. The same
    /// table is in docs/DISPLAY_AND_INPUT.md.
    const SHAPES: [(&str, f32, f32, f32, f32); 14] = [
        ("4:3 old monitor or iPad", 1024.0, 768.0, 85.0, 69.0),
        ("16:10 monitor or Steam Deck", 1280.0, 800.0, 95.4, 69.0),
        ("16:9 TV or monitor", 1920.0, 1080.0, 101.4, 69.0),
        ("16:9 maximized window", 1920.0, 1009.0, 105.2, 69.0),
        ("21:9 ultrawide", 2560.0, 1080.0, 110.0, 62.1),
        ("3440x1440 ultrawide", 3440.0, 1440.0, 110.0, 61.7),
        ("32:9 super ultrawide", 5120.0, 1440.0, 110.0, 43.8),
        ("phone landscape 19.5:9", 2340.0, 1080.0, 110.0, 66.8),
        ("phone upright 9:19.5", 1080.0, 2340.0, 66.8, 110.0),
        ("phone upright 9:16", 1080.0, 1920.0, 69.0, 101.4),
        ("flip phone open upright 9:22", 1080.0, 2640.0, 60.6, 110.0),
        ("flip phone cover screen", 1056.0, 1066.0, 69.0, 69.5),
        ("foldable opened", 2176.0, 1812.0, 79.1, 69.0),
        ("square", 1000.0, 1000.0, 69.0, 69.0),
    ];

    #[test]
    fn every_screen_shape() {
        for (name, w, h, across, up) in SHAPES {
            let aspect = w / h;
            let v = view_angle(85.0, 110.0, aspect);
            let got_across = horizontal_fov(v, aspect);
            let got_up = v.to_degrees();
            println!("{name} {w}x{h}: {got_across:.1} across, {got_up:.1} up and down");
            assert!(
                (got_across - across).abs() < 0.1 && (got_up - up).abs() < 0.1,
                "{name}: {got_across:.1} across, {got_up:.1} up and down"
            );
            // The short side never sees more than the original's 69, and
            // the long side never more than the limit. (Very long shapes,
            // 32:9 or a tall flip phone, give up some of the short side to
            // stay under the limit; the Panini view is the way to keep it.)
            assert!(
                got_across.min(got_up) < 69.01 && got_across.max(got_up) < 110.01,
                "{name}"
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
