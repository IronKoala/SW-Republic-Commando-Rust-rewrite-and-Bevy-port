//! rc_walk: walk around a test room in first person, with the original
//! game's movement (crate `rc_movement`).
//!
//! Usage:
//!   rc_walk [--map NAME] [--settings FILE] [--script] [--frames N] [--ride]
//!
//! Controls are the game's defaults: W A S D move, mouse looks, Space jumps,
//! hold C to crouch, hold Shift to walk. Click the window to capture the
//! mouse, Escape to let it go.
//!
//! `--map` walks a real map instead of the test room: a map name from
//! `GameData/Maps` (like `geo_01a`) or a path to a `.ctm` file. The game
//! folder is RC_GAME_DIR or the default Steam path. The map is drawn the
//! level viewer's way (textures, baked light, real-time lights, read from
//! `lighting.cfg` if there is one); collision comes from `level.rs`.
//!
//! `--settings` picks the movement settings file (default
//! `data/movement/player_commando.toml`). `--script` replaces the keyboard
//! and mouse with a fixed list of moves and checks the results, then exits
//! with code 0 if every check passed. `--frames N` closes after N frames.
//! Everything is logged to `logs/rc_walk.log`.
//!
//! On a map, doors and lifts move (crate `rc_movers`, settings in
//! `data/movers/movers.toml`): walk into a door or a trigger and it
//! opens; stand on a lift and it carries you. `--ride` starts the player
//! on the map's first lift and sets it going, and with `--script` checks
//! that the player rides it all the way.
//!
//! The pieces are kept apart on purpose, so one can be swapped without
//! touching the others (for example a VR headset for the camera, or a
//! tracked hand for aiming):
//! - input (keyboard and mouse, or the script) only writes `PlayerIntent`;
//! - the body only reads `PlayerIntent` and moves the `Body`;
//! - the camera only reads the `Body` and `PlayerIntent`'s look direction.

mod script;

use bevy::asset::RenderAssetUsages;
use bevy::camera::Exposure;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::view::Tonemapping;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use rc_level_viewer::draw::{self, BACKGROUND};
use rc_level_viewer::lighting::LightingSettings;
use rc_level_viewer::{materials, scene, sky};
use rc_movement::room::test_room;
use rc_level_viewer::draw::MoverPart;
use rc_movement::{
    Brush, BrushWorld, Event, MoveInput, MoveParams, Pawn, TriangleWorld, Vec3 as UVec3, World,
};
use rc_movers::{MoverEvent, MoverSet};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::fs;
use std::io::Write;
use std::sync::Mutex;
use std::time::Instant;

const LOG_PATH: &str = "logs/rc_walk.log";
const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";
const DEFAULT_SETTINGS: &str = "data/movement/player_commando.toml";
const MOVER_SETTINGS: &str = "data/movers/movers.toml";
/// Unreal units per Bevy unit. Shared with the level and weapon viewers.
const UU_PER_UNIT: f32 = 50.0;
/// The game's horizontal field of view, degrees (PlayerController defaults).
const FOV_DEGREES: f32 = 85.0;
/// Mouse turn per count of mouse movement, radians. Our own choice: the
/// game's conversion lives in its compiled engine and hasn't been measured.
const MOUSE_RADIANS_PER_COUNT: f32 = 0.0025;
/// Physics step used by `--script`, so its results are the same every run.
const SCRIPT_DT: f32 = 1.0 / 60.0;
/// Log the player's state every this many frames.
const LOG_EVERY: u32 = 6;

/// Unreal position (Z up, left-handed) to Bevy (Y up, right-handed):
/// swap Y and Z, then scale.
fn to_bevy(v: UVec3) -> Vec3 {
    Vec3::new(v.x, v.z, v.y) / UU_PER_UNIT
}

/// Bevy rotation for an Unreal yaw (turning from +X toward +Y) and pitch
/// (positive looks up). Bevy cameras look along -Z.
fn view_rotation(yaw: f32, pitch: f32) -> Quat {
    Quat::from_rotation_y(-(yaw + std::f32::consts::FRAC_PI_2)) * Quat::from_rotation_x(pitch)
}

#[derive(Resource)]
struct RunLog {
    file: Mutex<fs::File>,
    start: Instant,
}

impl RunLog {
    fn line(&self, s: impl AsRef<str>) {
        let s = format!("[{:>8.3}s] {}", self.start.elapsed().as_secs_f64(), s.as_ref());
        println!("{s}");
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "{s}");
        }
    }
}

/// What the player wants to do this frame. Written by input, read by the
/// body (moving) and the camera (looking).
#[derive(Resource, Default)]
struct PlayerIntent {
    movement: MoveInput,
    /// Up/down look angle, radians, positive is up.
    pitch: f32,
}

/// The player's body, driven by `rc_movement`.
#[derive(Component)]
struct Body {
    pawn: Pawn,
    /// Simulated time, seconds.
    time: f32,
}

#[derive(Component)]
struct PlayerView;

/// What the player collides with: the test room or a real map.
enum Ground {
    Room(BrushWorld),
    Map(TriangleWorld),
}

#[derive(Resource)]
struct TestWorld(Ground);

/// A map's textured, lit look, built before the window opens and spawned
/// once by `setup`.
#[derive(Resource)]
struct MapLook(Mutex<Option<(scene::Scene, LightingSettings)>>);

impl TestWorld {
    fn world(&self) -> &dyn World {
        match &self.0 {
            Ground::Room(w) => w,
            Ground::Map(w) => w,
        }
    }
}

/// Slowest physics step so far, for the log (matters on big maps).
#[derive(Resource, Default)]
struct StepTiming {
    worst_ms: f64,
    total_ms: f64,
    steps: u32,
}

/// The map's doors and lifts, and which mover each export index is.
#[derive(Resource)]
struct Movers {
    set: MoverSet,
    by_export: HashMap<usize, usize>,
    /// `--ride`: the lift the player started on, its height then, and the
    /// player's feet height then.
    ride: Option<(usize, f32, f32)>,
}

#[derive(Resource)]
struct ExitAfter(Option<u32>);

#[derive(Resource, Default)]
struct FrameCount(u32);

/// State of the `--script` run.
#[derive(Resource)]
struct Script {
    steps: Vec<script::Step>,
    next: usize,
    /// Time left on the current `Hold`.
    hold_left: f32,
    stats: script::Stats,
    passed: u32,
    failed: u32,
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
enum Stage {
    Input,
    Move,
    View,
    Report,
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    world: Res<TestWorld>,
    look: Option<Res<MapLook>>,
    log: Res<RunLog>,
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
    let mut has_sky = false;
    match &world.0 {
        Ground::Room(room) => {
            for (i, b) in room.brushes.iter().enumerate() {
                let mat = if i == 0 { floor.clone() } else { block.clone() };
                commands.spawn((Mesh3d(meshes.add(brush_mesh(b))), MeshMaterial3d(mat)));
            }
            log.line(format!(
                "test room: {} solids; 1 Bevy unit = {UU_PER_UNIT} uu",
                room.brushes.len()
            ));
            add_room_lights(&mut commands);
        }
        Ground::Map(tw) => {
            let built = look.and_then(|l| l.0.lock().unwrap().take());
            match built {
                Some((sc, settings)) => {
                    let r = draw::spawn_level(
                        &mut commands,
                        &mut meshes,
                        &mut images,
                        &mut materials,
                        &sc,
                        &settings,
                    );
                    if let Some(s) = &sc.sky {
                        let bg = Color::srgb_u8(BACKGROUND[0], BACKGROUND[1], BACKGROUND[2]);
                        sky::spawn_sky_camera(&mut commands, s, bg);
                        has_sky = true;
                        log.line(format!("sky: camera inside the sky box at {} ({} sky batches)", s.from, r.sky_batches));
                    } else {
                        log.line("sky: this map has no SkyZoneInfo, so no sky");
                    }
                    log.line(format!(
                        "map drawn: {} triangles, {} batches with a texture; {} with a lightmap, {} with baked corner colours, {} at full brightness; real-time lights: {} point, {} spot, {} sun",
                        r.triangles,
                        r.textured,
                        r.lightmapped,
                        r.coloured,
                        r.full_bright,
                        r.lights.point,
                        r.lights.spot,
                        r.lights.directional
                    ));
                }
                None => {
                    // The textured look couldn't be built: show the
                    // collision triangles in grey instead.
                    commands.spawn((
                        Mesh3d(meshes.add(triangles_mesh(&tw.triangles))),
                        MeshMaterial3d(materials.add(StandardMaterial {
                            base_color: Color::srgb(0.6, 0.6, 0.58),
                            perceptual_roughness: 0.9,
                            double_sided: true,
                            cull_mode: None,
                            ..default()
                        })),
                    ));
                    add_room_lights(&mut commands);
                }
            }
            let (lo, hi) = tw.bounds();
            log.line(format!(
                "map: {} collision triangles, bounds ({:.0}, {:.0}, {:.0}) to ({:.0}, {:.0}, {:.0})",
                tw.triangles.len(),
                lo.x,
                lo.y,
                lo.z,
                hi.x,
                hi.y,
                hi.z
            ));
        }
    }
    // Same camera settings as the level viewer, which the baked light is
    // matched to.
    commands.spawn((
        Camera3d::default(),
        if has_sky {
            sky::viewer_camera()
        } else {
            Camera::default()
        },
        sky::SkyViewer,
        Tonemapping::None,
        Exposure::default(),
        Projection::Perspective(PerspectiveProjection {
            near: 0.05,
            far: 5000.0,
            ..default()
        }),
        Transform::default(),
        PlayerView,
    ));
}

/// A sun and some ambient light, for the test room (it has no lights of
/// its own).
fn add_room_lights(commands: &mut Commands) {
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
}

/// Build the map's look with the level viewer's code: textures, baked light
/// and lights. `None` (logged) if that fails; the walk still works.
fn build_look(
    map_path: &Path,
    game_data: &Path,
    log: &RunLog,
) -> Option<(scene::Scene, LightingSettings)> {
    let t = Instant::now();
    let lighting_path = Path::new("lighting.cfg");
    let (settings, notes) = LightingSettings::load(lighting_path);
    for n in notes {
        log.line(format!("  lighting settings problem: {n}"));
    }
    let map = match fs::read(map_path)
        .map_err(|e| e.to_string())
        .and_then(|d| rc_package::Package::parse(&d).map(|p| (p, d)).map_err(|e| e.to_string()))
    {
        Ok(m) => Rc::new(m),
        Err(e) => {
            log.line(format!("map look not built ({e}); drawing collision in grey"));
            return None;
        }
    };
    let mut mats = materials::Materials::new(game_data);
    match scene::build(map, &mut mats) {
        Ok(sc) => {
            log.line(format!(
                "map look built in {:.0} ms ({} draw batches, {} lightmap pages, {} lights); lighting settings {}",
                t.elapsed().as_secs_f64() * 1000.0,
                sc.batches.len(),
                sc.lightmap_pages.len(),
                sc.lights.len(),
                if lighting_path.exists() { "from lighting.cfg" } else { "default" }
            ));
            Some((sc, settings))
        }
        Err(e) => {
            log.line(format!("map look not built ({e}); drawing collision in grey"));
            None
        }
    }
}

/// One grey mesh for a map's collision triangles.
fn triangles_mesh(tris: &[[UVec3; 3]]) -> Mesh {
    let positions: Vec<[f32; 3]> = tris
        .iter()
        .flat_map(|t| t.iter().map(|&p| to_bevy(p).to_array()))
        .collect();
    let indices = (0..positions.len() as u32).collect();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_indices(Indices::U32(indices))
        .with_duplicated_vertices()
        .with_computed_flat_normals()
}

/// A Bevy mesh for one solid of the test room (a box, or a ramp along +X).
fn brush_mesh(b: &Brush) -> Mesh {
    let (lo, hi) = (b.min, b.max);
    // Corners in Unreal units. A ramp's two top corners at the low end sit
    // on the floor of the box.
    let ramp = b.planes.len() > 6;
    let low_top = if ramp { lo.z } else { hi.z };
    let c = |x: f32, y: f32, z: f32| to_bevy(UVec3::new(x, y, z));
    let p = [
        c(lo.x, lo.y, lo.z),
        c(hi.x, lo.y, lo.z),
        c(hi.x, hi.y, lo.z),
        c(lo.x, hi.y, lo.z),
        c(lo.x, lo.y, low_top),
        c(hi.x, lo.y, hi.z),
        c(hi.x, hi.y, hi.z),
        c(lo.x, hi.y, low_top),
    ];
    // Six faces as quads, each wound to face outward in Bevy's space.
    let faces: [[usize; 4]; 6] = [
        [0, 1, 2, 3], // bottom
        [4, 7, 6, 5], // top (or slope)
        [0, 4, 5, 1], // -Y side
        [2, 6, 7, 3], // +Y side
        [1, 5, 6, 2], // +X end
        [0, 3, 7, 4], // -X end
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
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_indices(Indices::U32(indices))
        .with_duplicated_vertices()
        .with_computed_flat_normals()
}

/// Keyboard and mouse to `PlayerIntent`, with the game's default keys.
fn keyboard_mouse_input(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mut intent: ResMut<PlayerIntent>,
) {
    let Ok(mut cursor) = cursor.single_mut() else {
        return;
    };
    if buttons.just_pressed(MouseButton::Left) {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
    let axis = |plus: KeyCode, minus: KeyCode| {
        (keys.pressed(plus) as i32 - keys.pressed(minus) as i32) as f32
    };
    let m = &mut intent.movement;
    m.forward = axis(KeyCode::KeyW, KeyCode::KeyS);
    m.strafe = axis(KeyCode::KeyD, KeyCode::KeyA);
    m.jump = keys.pressed(KeyCode::Space);
    m.crouch = keys.pressed(KeyCode::KeyC);
    m.walk = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    if cursor.grab_mode != CursorGrabMode::None {
        let d = motion.delta * MOUSE_RADIANS_PER_COUNT;
        m.yaw = (m.yaw + d.x).rem_euclid(std::f32::consts::TAU);
        intent.pitch = (intent.pitch - d.y).clamp(-1.5, 1.5);
    }
}

/// The `--script` run's stand-in for the keyboard and mouse.
fn script_input(
    mut script: ResMut<Script>,
    mut intent: ResMut<PlayerIntent>,
    mut movers: ResMut<Movers>,
    mut body: Query<&mut Body>,
    mut exit: MessageWriter<AppExit>,
    log: Res<RunLog>,
) {
    let Ok(mut body) = body.single_mut() else {
        return;
    };
    let s = &mut *script;
    // Use up this frame's share of the current hold, or run steps that take
    // no time (place, check) until the next hold.
    while s.hold_left <= 0.0 {
        let Some(step) = s.steps.get(s.next) else {
            log.line(format!(
                "SCRIPT DONE: {} checks passed, {} failed",
                s.passed, s.failed
            ));
            exit.write(if s.failed == 0 {
                AppExit::Success
            } else {
                AppExit::from_code(1)
            });
            intent.movement = MoveInput::default();
            return;
        };
        s.next += 1;
        match step {
            script::Step::Place { feet, yaw } => {
                let params = body.pawn.params.clone();
                body.pawn = Pawn::new(params, *feet);
                intent.movement = MoveInput { yaw: *yaw, ..Default::default() };
                intent.pitch = 0.0;
                let z = body.pawn.feet().z;
                s.stats = script::Stats {
                    start_feet_z: z,
                    peak_feet_z: z,
                    lowest_feet_z: z,
                    ..Default::default()
                };
                log.line(format!(
                    "place feet ({:.0}, {:.0}, {:.0}) yaw {:.0} deg",
                    feet.x,
                    feet.y,
                    feet.z,
                    yaw.to_degrees()
                ));
                // Let it settle onto the floor before measuring jumps.
                s.hold_left = 0.0;
            }
            script::Step::StartRide => {
                let m = &mut *movers;
                if let Some((i, _, _)) = m.ride {
                    let lift_z = m.set.movers[i].pose.pos.z;
                    m.ride = Some((i, lift_z, body.pawn.feet().z));
                    let ev = m.set.open(i);
                    log_mover_events(&m.set, &ev, body.time, &log);
                    log.line(format!(
                        "ride: {} set going with the player on it, lift z {lift_z:.1}, feet z {:.1}",
                        m.set.movers[i].info.place.name,
                        body.pawn.feet().z
                    ));
                }
            }
            script::Step::Turn { degrees } => {
                let yaw = intent.movement.yaw + degrees.to_radians();
                intent.movement.yaw = yaw.rem_euclid(std::f32::consts::TAU);
                log.line(format!("turn {degrees:.0} deg"));
            }
            script::Step::AtLeast { name, measure, min } => {
                let got = measure(&body.pawn, &s.stats);
                let ok = got >= *min;
                if ok {
                    s.passed += 1;
                } else {
                    s.failed += 1;
                }
                log.line(format!(
                    "CHECK {name}: got {got:.2}, want at least {min:.2} -> {}",
                    if ok { "PASS" } else { "FAIL" }
                ));
            }
            script::Step::Hold { secs, input } => {
                s.hold_left = *secs;
                let yaw = intent.movement.yaw;
                intent.movement = MoveInput { yaw, ..*input };
                log.line(format!("hold {:.1} s: {:?}", secs, input));
            }
            script::Step::Check { name, measure, want, tolerance } => {
                let got = measure(&body.pawn, &s.stats);
                let ok = (got - want).abs() <= *tolerance;
                if ok {
                    s.passed += 1;
                } else {
                    s.failed += 1;
                }
                log.line(format!(
                    "CHECK {name}: got {got:.2}, want {want:.2} +/- {tolerance} -> {}",
                    if ok { "PASS" } else { "FAIL" }
                ));
            }
        }
    }
    s.hold_left -= SCRIPT_DT;
}

/// Move the body with this frame's intent.
#[allow(clippy::too_many_arguments)]
fn move_body(
    time: Res<Time>,
    intent: Res<PlayerIntent>,
    world: Res<TestWorld>,
    mut movers: ResMut<Movers>,
    script: Option<ResMut<Script>>,
    mut body: Query<&mut Body>,
    mut timing: ResMut<StepTiming>,
    log: Res<RunLog>,
) {
    let Ok(mut body) = body.single_mut() else {
        return;
    };
    let dt = if script.is_some() { SCRIPT_DT } else { time.delta_secs().min(0.25) };
    let t0 = Instant::now();
    // Doors and lifts move first, carrying or pushing the player.
    let mover_events = movers.set.step(dt, world.world(), &mut body.pawn);
    log_mover_events(&movers.set, &mover_events, body.time + dt, &log);
    let w = movers.set.world(world.world());
    let events = body.pawn.step(dt, &intent.movement, &w);
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    timing.steps += 1;
    timing.total_ms += ms;
    if ms > timing.worst_ms {
        timing.worst_ms = ms;
    }
    if timing.steps % 600 == 0 {
        log.line(format!(
            "physics time per frame: average {:.3} ms, worst {:.3} ms ({} frames)",
            timing.total_ms / timing.steps as f64,
            timing.worst_ms,
            timing.steps
        ));
    }
    body.time += dt;
    let feet = body.pawn.feet();
    for e in &events {
        log.line(format!("t={:.3} event {e:?} feet z {:.2}", body.time, feet.z));
    }
    // `--ride`: how far the lift and the player have gone since it started
    // (NaN until the script sets it going).
    let ride = movers.ride.and_then(|(i, lift0, feet0)| {
        let lift = movers.set.movers[i].pose.pos.z - lift0;
        lift.is_finite().then_some((lift, feet.z - feet0))
    });
    if let Some((lift, moved)) = ride {
        let before = body.time - dt;
        if (body.time / 0.25).floor() != (before / 0.25).floor() {
            log.line(format!("t={:.3} ride: lift {lift:+.1} uu, feet {moved:+.1} uu", body.time));
        }
    }
    if let Some(mut s) = script {
        if let Some((lift, moved)) = ride {
            s.stats.lift_moved = lift;
            s.stats.feet_moved = moved;
            s.stats.worst_ride_gap = s.stats.worst_ride_gap.max((lift - moved).abs());
        }
        s.stats.peak_feet_z = s.stats.peak_feet_z.max(feet.z);
        s.stats.lowest_feet_z = s.stats.lowest_feet_z.min(feet.z);
        s.stats.events.extend(events.iter().cloned());
    }
    if events.iter().any(|e| matches!(e, Event::Landed { damage, .. } if *damage > 0.0)) {
        log.line("  (the original game would take this much health; not applied yet)");
    }
}

fn log_mover_events(set: &MoverSet, events: &[MoverEvent], t: f32, log: &RunLog) {
    let name = |i: usize| set.movers.get(i).map_or("?", |m| m.info.place.name.as_str());
    for e in events {
        let text = match e {
            MoverEvent::StartedOpening { mover } => format!("{} starts opening", name(*mover)),
            MoverEvent::Opened { mover, secs } => format!("{} open after {secs:.2} s", name(*mover)),
            MoverEvent::StartedClosing { mover } => format!("{} starts closing", name(*mover)),
            MoverEvent::Closed { mover, secs } => format!("{} closed after {secs:.2} s", name(*mover)),
            MoverEvent::Blocked { mover, what } => format!("{} blocked by the player ({what:?})", name(*mover)),
            MoverEvent::Sound { mover, sound } => format!("{} sound {sound} (not played yet)", name(*mover)),
            MoverEvent::Triggered { trigger, event, movers } => {
                format!("trigger {trigger} sends {event:?} to {movers} movers")
            }
            MoverEvent::Event { mover, event, movers } => {
                format!("{} sends {event:?} to {movers} movers", name(*mover))
            }
            MoverEvent::Boarded { mover } => format!("player stands on {}", name(*mover)),
            MoverEvent::LeftBoard { mover } => format!("player steps off {}", name(*mover)),
        };
        log.line(format!("t={t:.3} mover: {text}"));
    }
}

/// `--ride` without `--script`: set the lift going once the player stands
/// on it.
fn start_ride_when_standing(mut movers: ResMut<Movers>, body: Query<&Body>, log: Res<RunLog>) {
    let Ok(body) = body.single() else { return };
    let m = &mut *movers;
    if let Some((i, lift0, _)) = m.ride {
        if lift0.is_nan() && m.set.riding == Some(i) {
            let lift_z = m.set.movers[i].pose.pos.z;
            m.ride = Some((i, lift_z, body.pawn.feet().z));
            let ev = m.set.open(i);
            log_mover_events(&m.set, &ev, body.time, &log);
        }
    }
}

/// Move each mover's drawn triangles to where the mover is now. Unreal
/// to Bevy swaps Y and Z and scales, so the turn's rows and columns 1 and
/// 2 swap too.
fn place_movers(movers: Res<Movers>, mut parts: Query<(&MoverPart, &mut Transform)>) {
    for (part, mut t) in &mut parts {
        let Some(&i) = movers.by_export.get(&part.0) else { continue };
        let (m, shift) = movers.set.movers[i].motion().matrix();
        let s = [0usize, 2, 1];
        let col = |c: usize| Vec3::new(m[s[0]][s[c]], m[s[1]][s[c]], m[s[2]][s[c]]);
        let rot = Mat3::from_cols(col(0), col(1), col(2));
        t.rotation = Quat::from_mat3(&rot).normalize();
        t.translation = to_bevy(shift);
    }
}

/// Put the camera at the body's eyes, looking where the player looks.
fn place_view(
    intent: Res<PlayerIntent>,
    body: Query<&Body>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut view: Query<(&mut Transform, &mut Projection), With<PlayerView>>,
) {
    let (Ok(body), Ok((mut t, mut proj))) = (body.single(), view.single_mut()) else {
        return;
    };
    t.translation = to_bevy(body.pawn.eye());
    t.rotation = view_rotation(intent.movement.yaw, intent.pitch);
    // Unreal's field of view is across the screen; Bevy's is up and down.
    if let (Ok(w), Projection::Perspective(p)) = (window.single(), &mut *proj) {
        let aspect = w.width() / w.height().max(1.0);
        let half = (FOV_DEGREES.to_radians() / 2.0).tan();
        p.fov = 2.0 * (half / aspect).atan();
    }
}

fn report(
    mut frames: ResMut<FrameCount>,
    exit_after: Res<ExitAfter>,
    mut exit: MessageWriter<AppExit>,
    body: Query<&Body>,
    view: Query<&Transform, With<PlayerView>>,
    log: Res<RunLog>,
) {
    frames.0 += 1;
    if let (Ok(body), Ok(t)) = (body.single(), view.single()) {
        if frames.0 % LOG_EVERY == 1 {
            let p = &body.pawn;
            let f = p.feet();
            let expected_eye = to_bevy(p.eye());
            let camera_ok = (t.translation - expected_eye).length() < 1e-4;
            log.line(format!(
                "t={:.2} feet ({:.1}, {:.1}, {:.1}) speed {:.1} vz {:.1} {:?}{} eye {:.1} camera ({:.3}, {:.3}, {:.3}){}",
                body.time,
                f.x,
                f.y,
                f.z,
                p.velocity.length_2d(),
                p.velocity.z,
                p.mode,
                if p.crouched { " crouched" } else { "" },
                p.eye_height,
                t.translation.x,
                t.translation.y,
                t.translation.z,
                if camera_ok { "" } else { " CAMERA NOT AT EYE" },
            ));
        }
    }
    if let Some(n) = exit_after.0 {
        if frames.0 >= n {
            log.line(format!("{n} frames done, closing"));
            exit.write(AppExit::Success);
        }
    }
}

fn main() -> AppExit {
    let mut settings_path = DEFAULT_SETTINGS.to_string();
    let mut use_script = false;
    let mut map_name: Option<String> = None;
    let mut frames = None;
    let mut ride = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--settings" => settings_path = args.next().unwrap_or_default(),
            "--map" => map_name = args.next(),
            "--script" => use_script = true,
            "--frames" => frames = args.next().and_then(|n| n.parse().ok()),
            "--ride" => ride = true,
            other => {
                eprintln!("unknown option {other}");
                eprintln!("usage: rc_walk [--map NAME] [--settings FILE] [--script] [--frames N] [--ride]");
                return AppExit::from_code(2);
            }
        }
    }

    let _ = fs::create_dir_all("logs");
    let file = match fs::File::create(LOG_PATH) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("can't create {LOG_PATH}: {e}");
            return AppExit::from_code(1);
        }
    };
    let log = RunLog { file: Mutex::new(file), start: Instant::now() };
    log.line(format!("rc_walk {}", env!("CARGO_PKG_VERSION")));

    let params = match fs::read_to_string(&settings_path) {
        Ok(text) => match MoveParams::player_commando().with_overrides(&text) {
            Ok(p) => {
                log.line(format!("movement settings from {settings_path}"));
                p
            }
            Err(e) => {
                log.line(format!("ERROR in {settings_path}: {e}"));
                return AppExit::from_code(1);
            }
        },
        Err(e) => {
            log.line(format!("no settings file at {settings_path} ({e}); using built-in values"));
            MoveParams::player_commando()
        }
    };
    for (name, value) in params.fields() {
        log.line(format!("  {name} = {value}"));
    }

    let mover_settings = match fs::read_to_string(MOVER_SETTINGS) {
        Ok(text) => match rc_movers::Settings::default().with_overrides(&text) {
            Ok(s) => {
                log.line(format!("door and lift settings from {MOVER_SETTINGS}"));
                s
            }
            Err(e) => {
                log.line(format!("ERROR in {MOVER_SETTINGS}: {e}"));
                return AppExit::from_code(1);
            }
        },
        Err(_) => {
            log.line(format!("no {MOVER_SETTINGS}; built-in door and lift settings"));
            rc_movers::Settings::default()
        }
    };
    let mut movers = Movers {
        set: MoverSet::new(Default::default(), mover_settings.clone(), |_| Vec::new()),
        by_export: HashMap::new(),
        ride: None,
    };

    let mut look = None;
    let (ground, start, steps) = match &map_name {
        None => (Ground::Room(test_room()), None, script::steps()),
        Some(name) => {
            let game = std::env::var_os("RC_GAME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_DIR));
            let game_data = game.join("GameData");
            let path = if name.to_ascii_lowercase().ends_with(".ctm") {
                PathBuf::from(name)
            } else {
                game_data.join("Maps").join(format!("{name}.ctm"))
            };
            let t = Instant::now();
            let lvl = match rc_level::map_collision::load(&path, &game_data, params.collision_height) {
                Ok(l) => l,
                Err(e) => {
                    log.line(format!("ERROR loading {}: {e}", path.display()));
                    return AppExit::from_code(1);
                }
            };
            log.line(format!(
                "loaded {}: {} BSP nodes, {} static mesh props ({} with their class's model, {} couldn't be read), {} blocking volumes ({} couldn't be read), {:.0} ms",
                path.display(),
                lvl.bsp_nodes,
                lvl.mesh_actors,
                lvl.class_mesh_actors,
                lvl.mesh_failures,
                lvl.blocking_volumes,
                lvl.volume_failures,
                t.elapsed().as_secs_f64() * 1000.0
            ));
            let t = Instant::now();
            let v = |p: [f32; 3]| UVec3::new(p[0], p[1], p[2]);
            let tris = lvl.triangles.iter().map(|t| [v(t[0]), v(t[1]), v(t[2])]).collect();
            let mt = &lvl.mover_triangles;
            let set = MoverSet::new(lvl.movers.clone(), mover_settings.clone(), |e| {
                mt.get(&e)
                    .map(|ts| ts.iter().map(|t| [v(t[0]), v(t[1]), v(t[2])]).collect())
                    .unwrap_or_default()
            });
            log.line(format!(
                "doors and lifts: {} movers ({} solid), {} triggers that run, {} that don't yet",
                set.movers.len(),
                set.movers.iter().filter(|m| m.collision.is_some()).count(),
                set.active_triggers().count(),
                set.unhandled_triggers.len()
            ));
            for (i, m) in set.movers.iter().enumerate() {
                log.line(format!(
                    "  mover {i} {} ({}): {} keys, opens in {:.2} s, StayOpenTime {:.2} s, state {:?} -> {}, tag {:?}, at ({:.0}, {:.0}, {:.0})",
                    m.info.place.name,
                    m.info.chain[0],
                    m.info.num_keys,
                    m.info.open_time(),
                    m.info.stay_open_time,
                    m.info.state,
                    m.behaviour.name(),
                    m.info.tag,
                    m.pose.pos.x,
                    m.pose.pos.y,
                    m.pose.pos.z
                ));
            }
            movers.by_export = set
                .movers
                .iter()
                .enumerate()
                .map(|(i, m)| (m.info.place.export, i))
                .collect();
            movers.set = set;
            let tw = TriangleWorld::new(tris);
            log.line(format!(
                "collision grid built in {:.0} ms",
                t.elapsed().as_secs_f64() * 1000.0
            ));
            let start = lvl.start.map(|(f, yaw)| (v(f), yaw)).unwrap_or_else(|| {
                let (lo, hi) = tw.bounds();
                log.line("no PlayerStart: starting above the middle of the map");
                (UVec3::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0, hi.z), 0.0)
            });
            log.line(format!(
                "start: feet ({:.1}, {:.1}, {:.1}), facing {:.0} deg",
                start.0.x,
                start.0.y,
                start.0.z,
                start.1.to_degrees()
            ));
            look = build_look(&path, &game_data, &log);
            let lowest = tw.bounds().0.z;
            let mut steps = script::map_steps(start.0, start.1, lowest);
            let mut start = start;
            if ride {
                let r = params.collision_radius;
                let ext = UVec3::new(r, r, params.collision_height);
                let lift = (0..movers.set.movers.len()).find_map(|i| {
                    let m = &movers.set.movers[i];
                    let (spot, clear) = m.ride_clearance(&tw, ext).filter(|_| m.is_lift(r))?;
                    (clear >= 0.999).then_some((i, spot))
                });
                match lift {
                    Some((i, spot)) => {
                        let m = &movers.set.movers[i];
                        let secs = m.info.open_time();
                        log.line(format!(
                            "ride: starting on {} at ({:.0}, {:.0}, {:.0}); it takes {secs:.2} s",
                            m.info.place.name, spot.x, spot.y, spot.z
                        ));
                        start = (spot, 0.0);
                        steps = script::ride_steps(spot, secs);
                        movers.ride = Some((i, f32::NAN, f32::NAN));
                    }
                    None => {
                        log.line("ERROR: --ride, but this map has no lift to stand on");
                        return AppExit::from_code(1);
                    }
                }
            }
            (Ground::Map(tw), Some(start), steps)
        }
    };

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: match &map_name {
                Some(m) => format!("Republic Commando: walk {m}"),
                None => "Republic Commando: walk test room".into(),
            },
            ..default()
        }),
        ..default()
    }))
    .insert_resource(ClearColor(if look.is_some() {
        Color::srgb_u8(BACKGROUND[0], BACKGROUND[1], BACKGROUND[2])
    } else {
        Color::srgb(0.35, 0.45, 0.6)
    }))
    .insert_resource(MapLook(Mutex::new(look)))
    .insert_resource(TestWorld(ground))
    .insert_resource(movers)
    .init_resource::<StepTiming>()
    .insert_resource(ExitAfter(frames))
    .init_resource::<FrameCount>()
    .init_resource::<PlayerIntent>()
    .configure_sets(Update, (Stage::Input, Stage::Move, Stage::View, Stage::Report).chain())
    .add_plugins(sky::SkyPlugin)
    .add_systems(Startup, setup)
    .add_systems(Update, move_body.in_set(Stage::Move))
    .add_systems(Update, (place_view, place_movers).in_set(Stage::View))
    .add_systems(Update, report.in_set(Stage::Report));
    if use_script {
        log.line("input: --script (keyboard and mouse ignored)");
        app.insert_resource(Script {
            steps,
            next: 0,
            hold_left: 0.0,
            stats: Default::default(),
            passed: 0,
            failed: 0,
        })
        .add_systems(Update, script_input.in_set(Stage::Input));
    } else {
        log.line("input: keyboard and mouse (click the window to look around)");
        if ride {
            // No script to set the lift going: it goes as soon as the
            // player lands on it.
            app.add_systems(Update, start_ride_when_standing.in_set(Stage::Input));
        }
        app.add_systems(Update, keyboard_mouse_input.in_set(Stage::Input));
    }
    app.insert_resource(log);
    let (feet, yaw) = start.unwrap_or((UVec3::ZERO, 0.0));
    app.world_mut().resource_mut::<PlayerIntent>().movement.yaw = yaw;
    app.world_mut().spawn(Body {
        pawn: Pawn::new(params, feet),
        time: 0.0,
    });
    app.run()
}
