//! rc_shoot: a firing range for the combat plugin. Stand in the movement
//! test room with the DC-17m and the DC-15s, and shoot battle droids. One
//! droid fires back, so you can watch your shield take the hits first.
//!
//! Usage:
//!   rc_shoot [--script] [--frames N]
//!
//! Controls: the mouse aims, left button fires, R reloads, Q or the mouse
//! wheel switches weapon. Click the window to capture the mouse, Escape to
//! let it go. The window's title shows the ammo, health and shield.
//!
//! Keys are mapped to the game's actions here only for this test program;
//! the game itself (rc_game) reads them from `data/input/bindings.toml`.
//!
//! The game folder is RC_GAME_DIR or the default Steam path; the weapon
//! files are read from `data` in the current folder. Without the game
//! folder the range still works, without sounds and the gun model.
//!
//! `--script` replaces the mouse and keys with a fixed list of actions and
//! checks the results (droids down, magazine counts, reload time, shield
//! before health, and a look at the picture), then exits with code 0 if
//! every check passed. `--frames N` closes after N frames. Everything is
//! logged to `logs/rc_shoot.log`. If RC_SHOOT_DUMP names a file, the
//! script's picture is also saved there (as a .ppm image).

use bevy::asset::RenderAssetUsages;
use bevy::camera::Exposure;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::render::view::Tonemapping;
use bevy::time::TimeUpdateStrategy;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use rc_combat::{
    Arsenal, CombatPlugin, Dead, Died, FireShot, Health, Hittable, Shield, ShotLanded, Team,
    ViewKick, WeaponLibrary,
};
use rc_game_core::units::{to_bevy, UU_PER_UNIT};
use rc_game_core::{
    Action, ActionState, CorePlugin, GameLog, GamePaths, GameSet, GameState, LevelCollision,
    Player, PlayerCamera, PlayerVitals, WeaponReadout,
};
use rc_movement::room::test_room;
use rc_movement::{Brush, Vec3 as UVec3};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const LOG_PATH: &str = "logs/rc_shoot.log";
const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";
/// The game's horizontal field of view, degrees (PlayerController defaults).
const FOV_DEGREES: f32 = 85.0;
/// Mouse turn per count of mouse movement, radians (same as rc_walk).
const MOUSE_RADIANS_PER_COUNT: f32 = 0.0025;
const SCRIPT_DT: f32 = 1.0 / 60.0;
/// Where the player stands (Unreal units) and how high the eye is.
const PLAYER_AT: (f32, f32) = (-500.0, 0.0);
const EYE_HEIGHT: f32 = 144.0;
/// The droid that fires back: how often (the E-5's own `ShotDelay` is
/// 0.55 s, but a droid in the game pauses between bursts; this is our
/// choice for the range) and how long a downed droid stays down.
const RETURN_FIRE_EVERY: f32 = 2.0;
const STAND_UP_AFTER: f32 = 4.0;
/// How quickly the view settles back after a shot's kick (per second).
/// **Our choice:** the game's recoil spring is in its compiled engine.
const KICK_SETTLE: f32 = 8.0;

/// Which way the player looks: Unreal yaw and pitch, radians, plus the
/// kick from recent shots on top.
#[derive(Resource, Default)]
struct Aim {
    yaw: f32,
    pitch: f32,
    kick_pitch: f32,
    kick_yaw: f32,
}

/// A target on the range.
#[derive(Component)]
struct Target {
    name: &'static str,
    /// Fires back at the player.
    shoots_back: bool,
    since_shot: f32,
    down_for: f32,
}

/// The visible body of a target (a child, so it can tip over).
#[derive(Component)]
struct TargetLook;

#[derive(Resource)]
struct Looks {
    standing: Handle<StandardMaterial>,
    down: Handle<StandardMaterial>,
}

#[derive(Resource)]
struct ExitAfter(Option<u32>);

#[derive(Resource, Default)]
struct FrameCount(u32);

/// Counts for the log and the checks.
#[derive(Resource, Default)]
struct Tally {
    shots: u32,
    kills: Vec<&'static str>,
}

/// One step of `--script`.
#[derive(Clone, Copy, Debug)]
enum Step {
    /// Hold these actions for this many seconds.
    Hold(&'static [Action], f32),
    /// Turn the view to this yaw and pitch (degrees).
    Face(f32, f32),
    Expect(Check),
    /// Capture the window and look at the picture.
    Picture,
}

#[derive(Clone, Copy, Debug)]
enum Check {
    Loaded,
    Down(&'static str),
    Magazine(&'static str, u32, u32),
    Reloading,
    ShieldFirst,
    Shots(u32),
}

#[derive(Resource)]
struct Script {
    steps: Vec<Step>,
    next: usize,
    hold_left: f32,
    held: &'static [Action],
    passed: u32,
    failed: u32,
    /// Waiting for the picture to come back.
    picture: Option<u32>,
}

fn script_steps() -> Vec<Step> {
    use Action::*;
    use Check::*;
    use Step::*;
    vec![
        Hold(&[], 0.5),
        Expect(Loaded),
        // The droid 1000 uu straight ahead: 20 damage a bolt, 100 health.
        Face(0.0, 0.0),
        Hold(&[Fire], 0.95),
        Hold(&[], 1.0),
        Expect(Down("droid ahead")),
        Expect(Magazine("DC-17m Blaster", 50, 60)),
        Expect(Shots(10)),
        // Reload: the game's 1.5 s animation.
        Hold(&[Reload], 0.1),
        Expect(Reloading),
        Hold(&[], 1.5),
        Expect(Magazine("DC-17m Blaster", 60, 50)),
        // The droid to the right, with the pistol: one shot per press,
        // 55 damage each, so two shots. The view turns a frame before the
        // trigger is pulled (the camera's position is read a frame late).
        // The pistol is last in the cycle (after the DC-17m's attachments),
        // so "previous weapon" reaches it.
        Hold(&[PrevWeapon], 0.1),
        Face(26.57, 0.0),
        Hold(&[], 0.1),
        Hold(&[Fire], 0.1),
        Hold(&[], 0.3),
        Hold(&[Fire], 0.1),
        Hold(&[], 1.0),
        Expect(Down("droid right")),
        Expect(Shots(12)),
        // Two charges used, recharged at 1.5 a second while we waited.
        Expect(Magazine("DC-15s Pistol", 5, 0)),
        // By now the far droid has fired at the player several times.
        Expect(ShieldFirst),
        Hold(&[NextWeapon], 0.1),
        Face(0.0, 0.0),
        Hold(&[], 0.5),
        Picture,
    ]
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut script = false;
    let mut frames = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--script" => script = true,
            "--frames" => frames = it.next().and_then(|n| n.parse::<u32>().ok()),
            other => {
                eprintln!("unknown option `{other}`; usage: rc_shoot [--script] [--frames N]");
                std::process::exit(2);
            }
        }
    }
    let log = GameLog::create(Path::new(LOG_PATH)).unwrap_or_else(|_| GameLog::terminal());
    let game_dir = std::env::var_os("RC_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_DIR));
    log.line(format!(
        "rc_shoot: game folder {} ({}), settings from data/",
        game_dir.display(),
        if game_dir.join("GameData").is_dir() {
            "found"
        } else {
            "not found: no sounds or gun model"
        }
    ));

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "rc_shoot".into(),
            resolution: (1280u32, 720u32).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins((CorePlugin, CombatPlugin))
    .insert_resource(GamePaths::new(game_dir, vec![PathBuf::from("data")]))
    .insert_resource(LevelCollision(Arc::new(test_room())))
    .insert_resource(log)
    .insert_resource(ClearColor(Color::srgb(0.08, 0.09, 0.11)))
    .insert_resource(ExitAfter(frames))
    .init_resource::<FrameCount>()
    .init_resource::<Aim>()
    .init_resource::<Tally>()
    .add_systems(Startup, setup)
    .add_systems(Update, turn_view.in_set(GameSet::Input))
    .add_systems(
        Update,
        (count_shots, targets_fall_and_stand, targets_shoot_back).in_set(GameSet::Simulate),
    )
    .add_systems(Update, (show_readouts, count_frames).in_set(GameSet::Hud));
    if script {
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            SCRIPT_DT,
        )))
        .insert_resource(Script {
            steps: script_steps(),
            next: 0,
            hold_left: 0.0,
            held: &[],
            passed: 0,
            failed: 0,
            picture: None,
        })
        .add_systems(Update, run_script.in_set(GameSet::Input).before(turn_view));
    } else {
        app.add_systems(
            Update,
            keyboard_mouse_input
                .in_set(GameSet::Input)
                .before(turn_view),
        );
    }
    app.world_mut()
        .resource_mut::<NextState<GameState>>()
        .set(GameState::InGame);
    let code = app.run();
    std::process::exit(if code.is_success() { 0 } else { 1 });
}

fn u(x: f32, y: f32, z: f32) -> Vec3 {
    to_bevy(UVec3::new(x, y, z))
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    log: Res<GameLog>,
) {
    let floor = materials.add(StandardMaterial {
        base_color: Color::srgb(0.45, 0.45, 0.42),
        perceptual_roughness: 0.9,
        ..default()
    });
    let block = materials.add(StandardMaterial {
        base_color: Color::srgb(0.55, 0.42, 0.30),
        perceptual_roughness: 0.8,
        ..default()
    });
    let room = test_room();
    for (i, b) in room.brushes.iter().enumerate() {
        let mat = if i == 0 { floor.clone() } else { block.clone() };
        commands.spawn((Mesh3d(meshes.add(brush_mesh(b))), MeshMaterial3d(mat)));
    }
    commands.spawn((
        DirectionalLight {
            illuminance: 8000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(-30.0, 60.0, -20.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight {
        brightness: 300.0,
        ..default()
    });

    // The player: feet on the floor, the camera at the eye.
    let (px, py) = PLAYER_AT;
    commands.spawn((
        Name::new("player"),
        Player,
        Transform::from_translation(u(px, py, 0.0)),
    ));
    let aspect = 1280.0 / 720.0;
    let vertical_fov = 2.0 * ((FOV_DEGREES.to_radians() * 0.5).tan() / aspect).atan();
    commands
        .spawn((
            Camera3d::default(),
            Tonemapping::None,
            Exposure::default(),
            Projection::Perspective(PerspectiveProjection {
                fov: vertical_fov,
                near: 0.05,
                far: 5000.0,
                ..default()
            }),
            Transform::from_translation(u(px, py, EYE_HEIGHT)),
            PlayerCamera,
        ))
        .with_children(|c| {
            // A small dot for a crosshair, floating in front of the eye.
            c.spawn((
                Mesh3d(meshes.add(Sphere::new(0.0025))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgb(0.2, 1.0, 0.3),
                    unlit: true,
                    ..default()
                })),
                Transform::from_xyz(0.0, 0.0, -0.5),
            ));
        });

    // Battle droids: 100 health, the game's commando-sized cylinder.
    let looks = Looks {
        standing: materials.add(StandardMaterial {
            base_color: Color::srgb(0.78, 0.66, 0.45),
            perceptual_roughness: 0.7,
            ..default()
        }),
        down: materials.add(StandardMaterial {
            base_color: Color::srgb(0.25, 0.22, 0.2),
            perceptual_roughness: 0.9,
            ..default()
        }),
    };
    let size = Hittable {
        radius: 40.0,
        half_height: 84.0,
        center_up: 84.0,
    };
    let body = meshes.add(Cylinder::new(
        size.radius / UU_PER_UNIT,
        2.0 * size.half_height / UU_PER_UNIT,
    ));
    let targets: [(&'static str, f32, f32, bool); 3] = [
        ("droid ahead", 500.0, 0.0, false),
        ("droid right", 0.0, 250.0, false),
        ("droid far", 2500.0, -1000.0, true),
    ];
    for (name, x, y, shoots_back) in targets {
        commands
            .spawn((
                Name::new(name),
                Target {
                    name,
                    shoots_back,
                    since_shot: 0.0,
                    down_for: 0.0,
                },
                Health::new(100.0),
                Team::Separatist,
                size,
                Transform::from_translation(u(x, y, 0.0)),
                Visibility::default(),
            ))
            .with_children(|c| {
                c.spawn((
                    TargetLook,
                    Mesh3d(body.clone()),
                    MeshMaterial3d(looks.standing.clone()),
                    Transform::from_xyz(0.0, size.center_up / UU_PER_UNIT, 0.0),
                ));
            });
        log.line(format!(
            "range: {name} at ({x}, {y}){}",
            if shoots_back { ", fires back" } else { "" }
        ));
    }
    commands.insert_resource(looks);
}

/// Mouse and keys to the game's actions (this program only).
fn keyboard_mouse_input(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mut actions: ResMut<ActionState>,
) {
    let Ok(mut cursor) = cursor.single_mut() else {
        return;
    };
    let captured = cursor.grab_mode != CursorGrabMode::None;
    if keys.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    } else if !captured && buttons.just_pressed(MouseButton::Left) {
        // The first click only captures the mouse.
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
        actions.update(HashSet::new(), Vec2::ZERO, Vec2::ZERO);
        return;
    }
    let mut now = HashSet::new();
    let mut look = Vec2::ZERO;
    if captured {
        if buttons.pressed(MouseButton::Left) {
            now.insert(Action::Fire);
        }
        if keys.pressed(KeyCode::KeyR) {
            now.insert(Action::Reload);
        }
        if keys.pressed(KeyCode::KeyQ) || scroll.delta.y < 0.0 {
            now.insert(Action::NextWeapon);
        }
        if scroll.delta.y > 0.0 {
            now.insert(Action::PrevWeapon);
        }
        look = Vec2::new(motion.delta.x, -motion.delta.y) * MOUSE_RADIANS_PER_COUNT;
    }
    actions.update(now, Vec2::ZERO, look);
}

/// Turn the camera from the look action, plus the kick from each shot,
/// which settles back over a fraction of a second.
fn turn_view(
    time: Res<Time>,
    actions: Res<ActionState>,
    mut kicks: MessageReader<ViewKick>,
    mut aim: ResMut<Aim>,
    mut camera: Query<&mut Transform, With<PlayerCamera>>,
) {
    aim.yaw += actions.look.x;
    aim.pitch = (aim.pitch + actions.look.y).clamp(-1.4, 1.4);
    for k in kicks.read() {
        aim.kick_pitch += k.pitch;
        aim.kick_yaw += k.yaw;
    }
    let settle = (-KICK_SETTLE * time.delta_secs()).exp();
    aim.kick_pitch *= settle;
    aim.kick_yaw *= settle;
    let (yaw, pitch) = (aim.yaw + aim.kick_yaw, aim.pitch + aim.kick_pitch);
    if let Ok(mut t) = camera.single_mut() {
        t.rotation = Quat::from_rotation_y(-(yaw + std::f32::consts::FRAC_PI_2))
            * Quat::from_rotation_x(pitch);
    }
}

#[allow(clippy::too_many_arguments)]
fn count_shots(
    mut shots: MessageReader<FireShot>,
    mut landed: MessageReader<ShotLanded>,
    names: Query<&Name>,
    log: Res<GameLog>,
    mut died: MessageReader<Died>,
    players: Query<(), With<Player>>,
    targets: Query<&Target>,
    mut tally: ResMut<Tally>,
) {
    for s in shots.read() {
        if players.contains(s.shooter) {
            tally.shots += 1;
            let d = s.direction;
            log.line(format!(
                "range: player fired {} toward yaw {:.2} pitch {:.2} degrees",
                s.weapon,
                d.z.atan2(d.x).to_degrees(),
                d.y.asin().to_degrees()
            ));
        }
    }
    for l in landed.read() {
        let name = |e: Entity| names.get(e).map_or("?".to_string(), |n| n.to_string());
        log.line(format!(
            "range: {}'s {} shot landed {} after {:.0} uu",
            name(l.shooter),
            l.weapon,
            match l.target {
                Some(t) => format!("on {}", name(t)),
                None if l.on_level => "on the room".to_string(),
                None => "nowhere".to_string(),
            },
            l.distance
        ));
    }
    for d in died.read() {
        if let Ok(t) = targets.get(d.entity) {
            tally.kills.push(t.name);
        }
    }
}

/// Downed droids tip over and go dark, then stand up again with full
/// health, so the range never runs out of targets. A downed player gets
/// up too.
#[allow(clippy::type_complexity)]
fn targets_fall_and_stand(
    mut commands: Commands,
    time: Res<Time>,
    looks: Res<Looks>,
    mut targets: Query<(Entity, &mut Target, &mut Health, Has<Dead>, &Children)>,
    mut bodies: Query<(&mut Transform, &mut MeshMaterial3d<StandardMaterial>), With<TargetLook>>,
    mut player: Query<
        (Entity, &mut Health, Option<&mut Shield>),
        (With<Player>, With<Dead>, Without<Target>),
    >,
    log: Res<GameLog>,
) {
    let dt = time.delta_secs();
    for (e, mut t, mut health, dead, children) in &mut targets {
        if !dead {
            continue;
        }
        t.down_for += dt;
        let up = t.down_for >= STAND_UP_AFTER;
        for c in children.iter() {
            if let Ok((mut tf, mut mat)) = bodies.get_mut(c) {
                let half = tf.translation.y.max(0.4);
                if up {
                    tf.rotation = Quat::IDENTITY;
                    tf.translation.y = Hittable::COMMANDO.center_up / UU_PER_UNIT;
                    mat.0 = looks.standing.clone();
                } else if tf.rotation == Quat::IDENTITY {
                    tf.rotation = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
                    tf.translation.y = (Hittable::COMMANDO.radius / UU_PER_UNIT).min(half);
                    mat.0 = looks.down.clone();
                }
            }
        }
        if up {
            t.down_for = 0.0;
            health.current = health.max;
            commands.entity(e).remove::<Dead>();
            log.line(format!("range: {} stands up again", t.name));
        }
    }
    if let Ok((e, mut health, shield)) = player.single_mut() {
        health.current = health.max;
        if let Some(mut s) = shield {
            s.current = s.max;
        }
        commands.entity(e).remove::<Dead>();
        log.line("range: the player is back up with full health");
    }
}

/// The droid that fires back aims an E-5 at the player's chest.
fn targets_shoot_back(
    time: Res<Time>,
    mut targets: Query<(Entity, &mut Target, &GlobalTransform), Without<Dead>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    mut shots: MessageWriter<FireShot>,
) {
    let Ok(eye) = camera.single() else {
        return;
    };
    let chest = eye.translation() - Vec3::Y * 40.0 / UU_PER_UNIT;
    for (e, mut t, at) in &mut targets {
        if !t.shoots_back {
            continue;
        }
        t.since_shot += time.delta_secs();
        if t.since_shot < RETURN_FIRE_EVERY {
            continue;
        }
        t.since_shot = 0.0;
        // The droid's gun, roughly at its chest and a little in front.
        let origin = at.translation() + Vec3::Y * 130.0 / UU_PER_UNIT;
        let toward = (chest - origin).normalize_or_zero();
        shots.write(FireShot {
            shooter: e,
            origin: origin + toward * 60.0 / UU_PER_UNIT,
            direction: toward,
            weapon: "e5_blaster".into(),
        });
    }
}

/// The window title as a simple display: weapon, ammo, health, shield.
fn show_readouts(
    weapon: Res<WeaponReadout>,
    vitals: Res<PlayerVitals>,
    tally: Res<Tally>,
    mut window: Query<&mut Window, With<PrimaryWindow>>,
) {
    let Ok(mut w) = window.single_mut() else {
        return;
    };
    let ammo = match (weapon.clip, weapon.reserve) {
        (Some(c), Some(r)) => format!("{c}/{} + {r}", weapon.clip_size.unwrap_or(0)),
        (Some(c), None) => format!("{c}/{}", weapon.clip_size.unwrap_or(0)),
        _ => String::new(),
    };
    let title = format!(
        "rc_shoot | {} {ammo}{} | health {:.0} shield {:.0} | droids down {}",
        weapon.name,
        if weapon.on_target { " [on target]" } else { "" },
        vitals.health,
        vitals.shield,
        tally.kills.len()
    );
    if w.title != title {
        w.title = title;
    }
}

fn count_frames(
    mut frames: ResMut<FrameCount>,
    exit_after: Res<ExitAfter>,
    script: Option<Res<Script>>,
    mut exit: MessageWriter<AppExit>,
    log: Res<GameLog>,
) {
    frames.0 += 1;
    if script.is_none() && exit_after.0.is_some_and(|n| frames.0 >= n) {
        log.line(format!(
            "closing after {} frames as asked (--frames)",
            frames.0
        ));
        exit.write(AppExit::Success);
    }
}

#[allow(clippy::too_many_arguments)]
fn run_script(
    mut commands: Commands,
    mut script: ResMut<Script>,
    mut actions: ResMut<ActionState>,
    mut aim: ResMut<Aim>,
    lib: Option<Res<WeaponLibrary>>,
    tally: Res<Tally>,
    weapon: Res<WeaponReadout>,
    players: Query<(&Health, &Shield, &Arsenal), With<Player>>,
    frames: Res<FrameCount>,
    log: Res<GameLog>,
    mut exit: MessageWriter<AppExit>,
) {
    if let Some(started) = script.picture {
        // The capture arrives a few frames later; give up after 5 s.
        if frames.0 > started + 300 {
            log.line("CHECK FAIL picture: no capture arrived");
            script.failed += 1;
            script.picture = None;
        } else {
            actions.update(HashSet::new(), Vec2::ZERO, Vec2::ZERO);
            return;
        }
    }
    while script.hold_left <= 0.0 {
        let Some(step) = script.steps.get(script.next).copied() else {
            let (p, f) = (script.passed, script.failed);
            log.line(format!(
                "script done: {p} of {} checks passed{}",
                p + f,
                if f == 0 {
                    ""
                } else {
                    " (see CHECK FAIL lines)"
                }
            ));
            exit.write(if f == 0 {
                AppExit::Success
            } else {
                AppExit::from_code(1)
            });
            actions.update(HashSet::new(), Vec2::ZERO, Vec2::ZERO);
            return;
        };
        script.next += 1;
        match step {
            Step::Hold(held, secs) => {
                script.held = held;
                script.hold_left = secs;
            }
            Step::Face(yaw, pitch) => {
                aim.yaw = yaw.to_radians();
                aim.pitch = pitch.to_radians();
            }
            Step::Picture => {
                log.line("capturing the window to look at the picture");
                script.picture = Some(frames.0);
                commands
                    .spawn(Screenshot::primary_window())
                    .observe(check_picture);
                actions.update(HashSet::new(), Vec2::ZERO, Vec2::ZERO);
                return;
            }
            Step::Expect(c) => {
                let player = players.single().ok();
                let (ok, what) = match c {
                    Check::Loaded => {
                        let ok = lib
                            .as_ref()
                            .is_some_and(|l| l.errors.is_empty() && l.weapons.len() >= 4)
                            && player.is_some();
                        let sounds = lib.as_ref().map_or(0, |l| l.sounds.len());
                        (
                            ok,
                            format!("weapon files read, player armed ({sounds} sounds loaded)"),
                        )
                    }
                    Check::Down(name) => (
                        tally.kills.contains(&name),
                        format!("{name} is down (down so far: {:?})", tally.kills),
                    ),
                    Check::Magazine(name, clip, reserve) => {
                        let ok = weapon.name == name
                            && weapon.clip == Some(clip)
                            && weapon.reserve.unwrap_or(0) == reserve;
                        (
                            ok,
                            format!(
                                "{name} shows {clip} + {reserve}; it shows {} {:?} + {:?}",
                                weapon.name, weapon.clip, weapon.reserve
                            ),
                        )
                    }
                    Check::Reloading => {
                        let r = player.and_then(|(_, _, a)| a.current().map(|w| w.1.reloading));
                        (
                            matches!(r, Some(Some(_))),
                            format!("reload started ({r:?} s left)"),
                        )
                    }
                    Check::ShieldFirst => {
                        let (h, s) = player.map_or((0.0, 0.0), |(h, s, _)| (h.current, s.current));
                        (
                            h == 200.0 && s < 100.0,
                            format!("the far droid's shots took shield {s}/100, health untouched {h}/200"),
                        )
                    }
                    Check::Shots(n) => (
                        tally.shots == n,
                        format!("{n} shots fired; counted {}", tally.shots),
                    ),
                };
                if ok {
                    script.passed += 1;
                    log.line(format!("CHECK PASS {what}"));
                } else {
                    script.failed += 1;
                    log.line(format!("CHECK FAIL {what}"));
                }
            }
        }
    }
    script.hold_left -= SCRIPT_DT;
    let now: HashSet<Action> = script.held.iter().copied().collect();
    actions.update(now, Vec2::ZERO, Vec2::ZERO);
}

/// Look at the picture without a person: the gun in the lower right should
/// be drawn in the game's colours (textured: many colours, not one flat
/// grey, and not the magenta a missing material shows), and the droid
/// ahead should cover the middle of the view.
fn check_picture(captured: On<ScreenshotCaptured>, mut script: ResMut<Script>, log: Res<GameLog>) {
    script.picture = None;
    let shot = &captured.image;
    let (w, h) = (shot.width() as usize, shot.height() as usize);
    let Some(data) = shot.data.as_ref() else {
        log.line("CHECK FAIL picture: the capture had no pixel data");
        script.failed += 1;
        return;
    };
    let bgra = matches!(
        shot.texture_descriptor.format,
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
    );
    let px = |x: usize, y: usize| -> [u8; 3] {
        let p = &data[(y * w + x) * 4..];
        if bgra {
            [p[2], p[1], p[0]]
        } else {
            [p[0], p[1], p[2]]
        }
    };
    if let Ok(path) = std::env::var("RC_SHOOT_DUMP") {
        let mut out = format!("P6 {w} {h} 255\n").into_bytes();
        for y in 0..h {
            for x in 0..w {
                out.extend(px(x, y));
            }
        }
        let _ = std::fs::write(path, out);
    }
    // The gun's corner: right half, bottom 40%.
    let mut colours = HashSet::new();
    let (mut magenta, mut n) = (0usize, 0usize);
    for y in (h * 6 / 10)..h {
        for x in (w / 2)..w {
            let c = px(x, y);
            colours.insert([c[0] >> 3, c[1] >> 3, c[2] >> 3]);
            if c[0] > 200 && c[2] > 200 && c[1] < 60 {
                magenta += 1;
            }
            n += 1;
        }
    }
    let mean_mid = {
        let mut m = [0u32; 3];
        let mut k = 0;
        for y in (h / 2 - 10)..(h / 2 + 10) {
            for x in (w / 2 - 30)..(w / 2 - 10) {
                let c = px(x, y);
                for i in 0..3 {
                    m[i] += c[i] as u32;
                }
                k += 1;
            }
        }
        m.map(|v| v / k)
    };
    let gun_ok = colours.len() >= 40 && magenta * 100 < n;
    // The droid's tan colour is brighter in red than blue; the far wall
    // behind is not.
    let droid_ok = mean_mid[0] > mean_mid[2] + 20;
    log.line(format!(
        "picture {w}x{h}: gun corner has {} colours, {:.1}% magenta; middle of the view is {:?}",
        colours.len(),
        100.0 * magenta as f32 / n.max(1) as f32,
        mean_mid
    ));
    for (ok, what) in [
        (
            gun_ok,
            "the gun is drawn in many colours, none of it magenta",
        ),
        (droid_ok, "the droid ahead is in the middle of the view"),
    ] {
        if ok {
            script.passed += 1;
            log.line(format!("CHECK PASS {what}"));
        } else {
            script.failed += 1;
            log.line(format!("CHECK FAIL {what}"));
        }
    }
}

/// A Bevy mesh for one solid of the test room (a box, or a ramp along +X).
/// Same as rc_walk's.
fn brush_mesh(b: &Brush) -> Mesh {
    let (lo, hi) = (b.min, b.max);
    let ramp = b.planes.len() > 6;
    let low_top = if ramp { lo.z } else { hi.z };
    let p = [
        u(lo.x, lo.y, lo.z),
        u(hi.x, lo.y, lo.z),
        u(hi.x, hi.y, lo.z),
        u(lo.x, hi.y, lo.z),
        u(lo.x, lo.y, low_top),
        u(hi.x, lo.y, hi.z),
        u(hi.x, hi.y, hi.z),
        u(lo.x, hi.y, low_top),
    ];
    let faces: [[usize; 4]; 6] = [
        [0, 1, 2, 3],
        [4, 7, 6, 5],
        [0, 4, 5, 1],
        [2, 6, 7, 3],
        [1, 5, 6, 2],
        [0, 3, 7, 4],
    ];
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for f in faces {
        let base = positions.len() as u32;
        for &i in &f {
            positions.push(p[i].to_array());
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_indices(Indices::U32(indices))
    .with_duplicated_vertices()
    .with_computed_flat_normals()
}
