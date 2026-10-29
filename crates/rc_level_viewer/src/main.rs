//! rc_level_viewer: loads one Republic Commando map and lets you fly around
//! it with a free camera.
//!
//! Usage:
//!   rc_level_viewer [map name or .ctm path] [--frames N]
//!
//! Defaults to `geo_01a` (the first Geonosis level) from `GameData/Maps` in
//! the game folder (RC_GAME_DIR or the default Steam path). `--frames N`
//! closes the window by itself after N frames, for automated tests.
//!
//! Everything is logged to `logs/rc_level_viewer.log`: what was read, what
//! couldn't be shown and why, frame rates, camera position, and an on-screen
//! check of what was actually drawn.

mod camera;
mod materials;
mod scene;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::render::view::Tonemapping;
use bevy::window::WindowResolution;
use camera::{fly, yaw_from_unreal, FlyCamera};
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
/// Sky-blue-grey background, as 0-255 sRGB values.
const BACKGROUND: [u8; 3] = [64, 72, 89];

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

fn to_image(p: &materials::Picture) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: p.image.width as u32,
            height: p.image.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        p.image.pixels.clone(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    // Level textures repeat across big surfaces.
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    loaded: Res<LoadedScene>,
    log: Res<RunLog>,
) {
    let Some(scene) = loaded.0.lock().unwrap().take() else {
        return;
    };
    let mut triangles = 0;
    let mut textured = 0;
    for b in &scene.batches {
        if b.indices.is_empty() {
            continue;
        }
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, b.positions.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, b.normals.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, b.uvs.clone());
        mesh.insert_indices(Indices::U32(b.indices.clone()));
        triangles += b.indices.len() / 3;
        let alpha_mode = if b.picture.as_ref().is_some_and(|p| p.additive) {
            AlphaMode::Add
        } else if b.translucent {
            AlphaMode::Blend
        } else if b.picture.as_ref().is_some_and(|p| p.has_alpha) {
            AlphaMode::Mask(0.5)
        } else {
            AlphaMode::Opaque
        };
        let material = StandardMaterial {
            // Lighting is a later step: draw textures at full brightness.
            unlit: true,
            base_color: if b.picture.is_some() {
                Color::WHITE
            } else {
                // No texture: a flat grey, so the shape still shows.
                Color::srgb(0.55, 0.55, 0.55)
            },
            base_color_texture: b.picture.as_ref().map(|p| {
                textured += 1;
                images.add(to_image(p))
            }),
            alpha_mode,
            double_sided: b.two_sided,
            cull_mode: if b.two_sided {
                None
            } else {
                Some(bevy::render::render_resource::Face::Back)
            },
            ..default()
        };
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(material)),
            Transform::default(),
        ));
    }
    let s = &scene.spawn;
    let yaw = yaw_from_unreal(s.yaw);
    let cam = FlyCamera {
        yaw,
        pitch: 0.0,
        speed: 10.0,
    };
    let rotation = cam.rotation();
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
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
        "collision triangles (solid BSP plus props, from rc_level::collision): {}",
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
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--frames" {
            frames = args.next().and_then(|n| n.parse().ok());
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
        .insert_resource(ExitAfter(frames))
        .insert_resource(FrameStats::default())
        .insert_resource(ScreenCheck::default())
        .insert_resource(log)
        .add_systems(Startup, setup)
        .add_systems(Update, (fly, count_frames))
        .run()
}
