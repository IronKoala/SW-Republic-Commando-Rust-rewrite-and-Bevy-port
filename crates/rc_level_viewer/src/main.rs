//! rc_level_viewer: loads one Republic Commando map and lets you fly around
//! it with a free camera.
//!
//! Usage:
//!   rc_level_viewer [map name or .ctm path] [--frames N] [--lighting FILE]
//!                   [--capture FILE.bmp] [--pitch DEG] [--no-sky]
//!
//! Defaults to `geo_01a` (the first Geonosis level) from `GameData/Maps` in
//! the game folder (RC_GAME_DIR or the default Steam path). `--frames N`
//! closes the window by itself after N frames, for automated tests.
//! `--lighting FILE` reads lighting settings (default `lighting.cfg` in the
//! current folder, if there is one; see `lighting.rs` and docs/DESIGN.md).
//! `--capture FILE.bmp` saves the frame used for the on-screen check as a
//! picture, for comparing looks without watching the window. `--pitch DEG`
//! starts the camera looking up (positive) or down. `--no-sky` leaves the
//! sky out (the background colour shows instead).
//!
//! Everything is logged to `logs/rc_level_viewer.log`: what was read, what
//! couldn't be shown and why, frame rates, camera position, and an on-screen
//! check of what was actually drawn.

use bevy::camera::Exposure;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::render::view::Tonemapping;
use bevy::window::WindowResolution;
use rc_level_viewer::camera::{fly, yaw_from_unreal, FlyCamera};
use rc_level_viewer::draw::{self, BACKGROUND};
use rc_level_viewer::lighting::LightingSettings;
use rc_level_viewer::{materials, scene, sky};
use rc_package::Package;
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::Instant;

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";
const DEFAULT_MAP: &str = "geo_01a";
const LOG_PATH: &str = "logs/rc_level_viewer.log";
const CHECK_FRAME: u32 = 30;
const CHECK_ATTEMPTS: u32 = 5;

#[derive(Resource)]
struct RunLog {
    file: Mutex<fs::File>,
    start: Instant,
}

impl RunLog {
    fn line(&self, s: impl AsRef<str>) {
        let s = format!(
            "[{:>8.3}s] {}",
            self.start.elapsed().as_secs_f64(),
            s.as_ref()
        );
        println!("{s}");
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "{s}");
        }
    }
}

/// What `main` built, handed to the startup system once.
#[derive(Resource)]
struct LoadedScene(Mutex<Option<scene::Scene>>);

#[derive(Resource)]
struct ExitAfter(Option<u32>);

#[derive(Resource)]
struct CapturePath(Option<PathBuf>);

/// `--pitch DEG` (start looking up or down) and `--no-sky`.
#[derive(Resource)]
struct ViewOptions {
    pitch_degrees: f32,
    sky: bool,
}

/// Save red, green, blue pixels (top row first) as a 24-bit .bmp file, a
/// format every system can open, written by hand to avoid another library.
fn write_bmp(
    path: &std::path::Path,
    w: usize,
    h: usize,
    px: impl Fn(usize) -> [u8; 3],
) -> std::io::Result<()> {
    let row = (w * 3).div_ceil(4) * 4;
    let size = 54 + row * h;
    let mut out = Vec::with_capacity(size);
    out.extend(b"BM");
    out.extend((size as u32).to_le_bytes());
    out.extend([0u8; 4]);
    out.extend(54u32.to_le_bytes());
    out.extend(40u32.to_le_bytes());
    out.extend((w as i32).to_le_bytes());
    out.extend((h as i32).to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(24u16.to_le_bytes());
    out.extend([0u8; 24]);
    // Rows go bottom to top, each pixel blue, green, red.
    for y in (0..h).rev() {
        for x in 0..w {
            let [r, g, b] = px(y * w + x);
            out.extend([b, g, r]);
        }
        out.resize(out.len() + row - w * 3, 0);
    }
    fs::write(path, out)
}

#[derive(Resource, Default)]
struct FrameStats {
    frames: u32,
    last_report: f64,
    frames_since_report: u32,
}

#[derive(Resource, Default)]
struct ScreenCheck {
    attempts: u32,
    next_frame: Option<u32>,
    passed: Option<bool>,
}

#[allow(clippy::too_many_arguments)]
fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    loaded: Res<LoadedScene>,
    settings: Res<LightingSettings>,
    log: Res<RunLog>,
    view: Res<ViewOptions>,
) {
    let Some(scene) = loaded.0.lock().unwrap().take() else {
        return;
    };
    let exposure = Exposure::default();
    let r = draw::spawn_level(
        &mut commands,
        &mut meshes,
        &mut images,
        &mut materials,
        &scene,
        &settings,
    );
    let baked = settings.baked_light;
    let (triangles, textured, lightmapped, coloured, full_bright, report) = (
        r.triangles,
        r.textured,
        r.lightmapped,
        r.coloured,
        r.full_bright,
        r.lights.clone(),
    );
    let background = Color::srgb_u8(BACKGROUND[0], BACKGROUND[1], BACKGROUND[2]);
    let has_sky = match scene.sky.as_ref().filter(|_| view.sky) {
        Some(sky) => {
            sky::spawn_sky_camera(&mut commands, sky, background);
            log.line(format!(
                "sky: camera at {:?} (Bevy units) from {}; {} sky batches, {} sky box polygons, {} sky props, {} backdrop polygons left open",
                sky.position,
                sky.from,
                r.sky_batches,
                scene.stats.sky_polygons,
                scene.stats.sky_props,
                scene.stats.backdrop_polygons
            ));
            true
        }
        None if !view.sky => {
            log.line("sky: switched off (--no-sky)");
            false
        }
        None => {
            log.line("sky: this map has no SkyZoneInfo, so no sky");
            false
        }
    };
    log.line(format!(
        "lighting: baked light {}; draw batches: {lightmapped} with a lightmap, {coloured} with baked corner colours, {full_bright} at full brightness; lightmap gain {} (x{:.2} in linear light), corner colour gain {}; real-time lights: {} point, {} spot, {} sun, {} left to the baked light",
        if baked { "on" } else { "off" },
        settings.lightmap_gain,
        LightingSettings::linear_gain(settings.lightmap_gain),
        settings.vertex_colour_gain,
        report.point,
        report.spot,
        report.directional,
        report.skipped
    ));
    let s = &scene.spawn;
    let yaw = yaw_from_unreal(s.yaw);
    let cam = FlyCamera {
        yaw,
        pitch: view.pitch_degrees.to_radians(),
        speed: 10.0,
    };
    let rotation = cam.rotation();
    commands.spawn((
        Camera3d::default(),
        if has_sky {
            sky::viewer_camera()
        } else {
            Camera::default()
        },
        sky::SkyViewer,
        Tonemapping::None,
        exposure,
        Projection::Perspective(PerspectiveProjection {
            fov: 75f32.to_radians(),
            near: 0.05,
            far: 5000.0,
            ..default()
        }),
        Transform::from_translation(Vec3::from(s.position)).with_rotation(rotation),
        cam,
    ));
    log.line(format!(
        "scene set up: {} draw batches ({} with a texture), {triangles} triangles; camera at {:?} (Bevy units) from {}, facing yaw {:.1} degrees",
        scene.batches.len(),
        textured,
        s.position,
        s.from,
        yaw.to_degrees()
    ));
}

#[allow(clippy::too_many_arguments)]
fn count_frames(
    time: Res<Time>,
    log: Res<RunLog>,
    exit_after: Res<ExitAfter>,
    mut stats: ResMut<FrameStats>,
    cams: Query<&Transform, With<FlyCamera>>,
    mut exit: MessageWriter<AppExit>,
    mut commands: Commands,
    mut check: ResMut<ScreenCheck>,
) {
    stats.frames += 1;
    stats.frames_since_report += 1;
    let now = time.elapsed_secs_f64();
    if stats.frames == 1 {
        stats.last_report = now;
        check.next_frame = Some(CHECK_FRAME);
        log.line("first frame drawn");
    } else if now - stats.last_report >= 1.0 {
        let fps = stats.frames_since_report as f64 / (now - stats.last_report);
        let pos = cams.single().map(|t| t.translation).unwrap_or_default();
        log.line(format!(
            "frames so far: {}, about {fps:.0} frames per second, camera at ({:.1}, {:.1}, {:.1})",
            stats.frames, pos.x, pos.y, pos.z
        ));
        stats.last_report = now;
        stats.frames_since_report = 0;
    }
    if check.next_frame == Some(stats.frames) {
        check.next_frame = None;
        check.attempts += 1;
        log.line(format!(
            "frame {}: capturing the window for the on-screen check (attempt {} of {CHECK_ATTEMPTS})",
            stats.frames, check.attempts
        ));
        commands
            .spawn(Screenshot::primary_window())
            .observe(check_screen);
    }
    let settled = check.passed.is_some() || exit_after.0.is_some_and(|n| stats.frames >= n + 300);
    if exit_after.0.is_some_and(|n| stats.frames >= n) && settled {
        if check.passed.is_none() {
            log.line("on-screen check FAIL: no usable capture arrived");
        }
        log.line(format!(
            "closing after {} frames as asked (--frames)",
            stats.frames
        ));
        exit.write(if check.passed == Some(true) {
            AppExit::Success
        } else {
            AppExit::from_code(1)
        });
    }
}

/// Without looking at the picture, we can still tell whether the level was
/// drawn: from the spawn point, most of the window should be covered by
/// level surfaces (not the background colour) and show many different
/// colours (textures, not flat grey).
fn check_screen(
    captured: On<ScreenshotCaptured>,
    log: Res<RunLog>,
    frames: Res<FrameStats>,
    mut check: ResMut<ScreenCheck>,
    capture: Res<CapturePath>,
) {
    let shot = &captured.image;
    let (w, h) = (shot.width() as usize, shot.height() as usize);
    let format = shot.texture_descriptor.format;
    let Some(data) = shot.data.as_ref() else {
        log.line("on-screen check FAIL: the capture had no pixel data");
        check.passed = Some(false);
        return;
    };
    let bgra = matches!(
        format,
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
    );
    let px = |i: usize| -> [u8; 3] {
        let p = &data[i * 4..];
        if bgra {
            [p[2], p[1], p[0]]
        } else {
            [p[0], p[1], p[2]]
        }
    };
    let n = w * h;
    let mut black = 0;
    let mut background = 0;
    let mut colours = HashSet::new();
    let mut mean = [0u64; 3];
    for i in 0..n {
        let c = px(i);
        if c == [0, 0, 0] {
            black += 1;
        }
        if c.iter().zip(BACKGROUND).all(|(a, b)| a.abs_diff(b) <= 2) {
            background += 1;
        }
        // Count colours in coarse steps of 8, so noise doesn't inflate it.
        colours.insert([c[0] >> 3, c[1] >> 3, c[2] >> 3]);
        for k in 0..3 {
            mean[k] += c[k] as u64;
        }
    }
    // All black, or all background: the window or the level hadn't been
    // drawn yet (meshes and textures reach the GPU over the first frames).
    if black == n || background == n {
        log.line(format!(
            "on-screen check: the capture is completely {} (not drawn yet)",
            if black == n {
                "black"
            } else {
                "background colour"
            }
        ));
        if check.attempts < CHECK_ATTEMPTS {
            check.next_frame = Some(frames.frames + CHECK_FRAME);
        } else {
            log.line("on-screen check FAIL: still nothing drawn after 5 captures");
            check.passed = Some(false);
        }
        return;
    }
    if let Some(path) = &capture.0 {
        match write_bmp(path, w, h, px) {
            Ok(()) => log.line(format!("saved the capture to {}", path.display())),
            Err(e) => log.line(format!(
                "couldn't save the capture to {}: {e}",
                path.display()
            )),
        }
    }
    let covered = 100.0 * (n - background) as f64 / n as f64;
    let mean = mean.map(|m| m as f64 / n as f64);
    let pass = covered > 50.0 && colours.len() > 100;
    check.passed = Some(pass);
    log.line(format!(
        "on-screen check {}: capture {w}x{h}, {covered:.1}% of the window is level (not background), {} distinct colours, average colour R {:.0} G {:.0} B {:.0} (passes above 50% and 100 colours)",
        if pass { "PASS" } else { "FAIL" },
        colours.len(),
        mean[0],
        mean[1],
        mean[2]
    ));
}

fn log_stats(log: &RunLog, s: &scene::Stats) {
    if let Some(e) = &s.lightmap_error {
        log.line(format!("lightmaps NOT read (level drawn unlit): {e}"));
    }
    log.line(format!(
        "lightmaps: {} rectangles on {} pages; BSP polygons with a lightmap {}, without {}",
        s.lightmap_entries, s.lightmap_pages, s.bsp_polygons_lit, s.bsp_polygons_unlit
    ));
    if s.lightmap_checks.is_empty() {
        log.line("lightmap checks: all passed");
    } else {
        for c in &s.lightmap_checks {
            log.line(format!("lightmap check problem: {c}"));
        }
    }
    log.line(format!(
        "prop baked light: {} drawn props have corner colours, {} have none, {} have a colour list of the wrong length (not used)",
        s.mesh_actors_with_colours, s.mesh_actors_without_colours, s.colour_count_mismatch
    ));
    log.line(format!(
        "lights: {} lights read, {} of them real-time (bDynamicLight); {} zones with an ambient light",
        s.lights, s.dynamic_lights, s.zone_ambients
    ));
    log.line(format!(
        "BSP: {} polygons, {} triangles drawn, skipped {:?}; winding (backwards, unclear, forwards) {:?}",
        s.bsp_polygons, s.bsp_triangles, s.bsp_skipped, s.bsp_winding
    ));
    if s.bsp_checks.is_empty() {
        log.line("BSP checks: all passed");
    } else {
        for c in &s.bsp_checks {
            log.line(format!("BSP check problem: {c}"));
        }
    }
    log.line(format!(
        "actors: {} read, {} failed; {} with a static mesh, {} drawn, {} hidden; {} different meshes; {} mesh triangles",
        s.actors, s.actor_failures, s.mesh_actors, s.mesh_actors_drawn, s.hidden_actors, s.unique_meshes, s.mesh_triangles
    ));
    for (e, n) in &s.mesh_failures {
        log.line(format!("  mesh not shown ({n}x): {e}"));
    }
    log.line(format!(
        "materials: {} shown with a texture, {} not ({} reasons)",
        s.materials_ok,
        s.material_failures.values().sum::<usize>(),
        s.material_failures.len()
    ));
    for (e, n) in &s.material_failures {
        log.line(format!("  material not shown ({n}x): {e}"));
    }
    log.line(format!(
        "blocking volumes (invisible walls): {} read, {} stop players, {} stop the AI, {} unreadable, {} with a scale not applied; {} PathNodes/PlayerStarts inside one (should be 0 or close), {} volumes within 512 units of a path point",
        s.blocking_volumes,
        s.volumes_block_players,
        s.volumes_block_actors,
        s.volume_failures.len(),
        s.volumes_scaled,
        s.path_points_in_volumes,
        s.volumes_near_paths
    ));
    for f in &s.volume_failures {
        log.line(format!("  volume not read: {f}"));
    }
    log.line(format!(
        "collision triangles (solid BSP, props and blocking volumes, from rc_level::collision): {}",
        s.collision_triangles
    ));
    for (what, f) in [
        ("all geometry", s.floor_test),
        ("props only", s.floor_test_meshes),
    ] {
        log.line(format!(
            "floor test ({what}): of {} PathNodes/PlayerStarts, {} have a floor 0-150 units below (median {:.0}), {} have one under 80 units (should be 0 or close)",
            f.tested, f.near, f.median, f.too_close
        ));
    }
    log.line(format!(
        "level bounds in Bevy units: min ({:.1}, {:.1}, {:.1}) max ({:.1}, {:.1}, {:.1})",
        s.bounds_min[0],
        s.bounds_min[1],
        s.bounds_min[2],
        s.bounds_max[0],
        s.bounds_max[1],
        s.bounds_max[2]
    ));
}

fn main() -> AppExit {
    let mut positional = Vec::new();
    let mut frames = None;
    let mut lighting_path = PathBuf::from("lighting.cfg");
    let mut capture = None;
    let mut view = ViewOptions {
        pitch_degrees: 0.0,
        sky: true,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--frames" {
            frames = args.next().and_then(|n| n.parse().ok());
        } else if arg == "--pitch" {
            view.pitch_degrees = args.next().and_then(|n| n.parse().ok()).unwrap_or(0.0);
        } else if arg == "--no-sky" {
            view.sky = false;
        } else if arg == "--capture" {
            capture = args.next().map(PathBuf::from);
        } else if arg == "--lighting" {
            if let Some(p) = args.next() {
                lighting_path = PathBuf::from(p);
            }
        } else {
            positional.push(arg);
        }
    }
    let game = std::env::var_os("RC_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_DIR));
    let game_data = game.join("GameData");
    let map_arg = positional
        .first()
        .cloned()
        .unwrap_or_else(|| DEFAULT_MAP.to_string());
    let map_path = if map_arg.to_ascii_lowercase().ends_with(".ctm") {
        PathBuf::from(&map_arg)
    } else {
        game_data.join("Maps").join(format!("{map_arg}.ctm"))
    };

    // Rule: never write inside the game install. Refuse to run from in there.
    let cwd = std::env::current_dir().expect("can't read the current folder");
    if let (Ok(c), Ok(g)) = (cwd.canonicalize(), game.canonicalize()) {
        if c.starts_with(&g) {
            eprintln!("Refusing to run from inside the game folder ({}), because the log would be written there.", g.display());
            return AppExit::from_code(2);
        }
    }
    fs::create_dir_all("logs").expect("can't create the logs folder");
    let log = RunLog {
        file: Mutex::new(fs::File::create(LOG_PATH).expect("can't create the log file")),
        start: Instant::now(),
    };
    log.line(format!("rc_level_viewer {}", env!("CARGO_PKG_VERSION")));
    log.line(format!("map: {}", map_path.display()));
    let (settings, notes) = LightingSettings::load(&lighting_path);
    log.line(format!(
        "lighting settings: {} ({:?})",
        if lighting_path.exists() {
            format!("read from {}", lighting_path.display())
        } else {
            format!("defaults ({} not found)", lighting_path.display())
        },
        settings
    ));
    for n in notes {
        log.line(format!("  lighting settings problem: {n}"));
    }

    let t = Instant::now();
    let map = match fs::read(&map_path)
        .map_err(|e| e.to_string())
        .and_then(|d| {
            Package::parse(&d)
                .map(|p| (p, d))
                .map_err(|e| e.to_string())
        }) {
        Ok(m) => Rc::new(m),
        Err(e) => {
            log.line(format!("FAILED to read the map: {e}"));
            return AppExit::from_code(1);
        }
    };
    let mut mats = materials::Materials::new(&game_data);
    log.line(format!(
        "map read: {} bytes, {} objects; {} package files found under {}",
        map.1.len(),
        map.0.exports.len(),
        mats.package_files(),
        game_data.display()
    ));
    let scene = match scene::build(map, &mut mats) {
        Ok(s) => s,
        Err(e) => {
            log.line(format!("FAILED to build the level: {e}"));
            return AppExit::from_code(1);
        }
    };
    log_stats(&log, &scene.stats);
    log.line(format!(
        "level built in {:.0} ms",
        t.elapsed().as_secs_f64() * 1000.0
    ));
    log.line("controls: W A S D move, E/Space up, Q/Ctrl down, Shift faster, hold right mouse to look, wheel changes speed");

    let title = format!("Republic Commando level: {map_arg}");
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title,
                resolution: WindowResolution::new(1280, 720),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb_u8(
            BACKGROUND[0],
            BACKGROUND[1],
            BACKGROUND[2],
        )))
        .insert_resource(LoadedScene(Mutex::new(Some(scene))))
        .insert_resource(settings)
        .insert_resource(CapturePath(capture))
        .insert_resource(view)
        .insert_resource(ExitAfter(frames))
        .insert_resource(FrameStats::default())
        .insert_resource(ScreenCheck::default())
        .insert_resource(log)
        .add_plugins(sky::SkyPlugin)
        .add_systems(Startup, setup)
        .add_systems(Update, (fly, count_frames))
        .run()
}
