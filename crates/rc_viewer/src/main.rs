//! rc_viewer: the first Bevy program. Opens a window and shows one texture,
//! or one 3D model, read from the installed game.
//!
//! Usage:
//!   rc_viewer [package] [texture name] [--frames N]
//!   rc_viewer --mesh [package] [mesh name] [--frames N]
//!
//! Defaults to the texture `DC15Blaster` from `GameData/Textures/weapontextures.utx`,
//! or with `--mesh` the model `DC15Blaster` from `GameData/StaticMeshes/weapons.usx`,
//! in the game folder (RC_GAME_DIR or the default Steam path). `--frames N`
//! closes the window by itself after N frames, for automated tests.
//!
//! Everything it does is logged to `logs/rc_viewer.log` (`logs/rc_viewer_mesh.log`
//! with `--mesh`), so a run can be checked from the numbers without looking at
//! the window. On frame `CHECK_FRAME` it also captures what is actually on
//! screen and compares it with what should be there, pixel by pixel.

mod mesh;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::{PrimaryWindow, WindowResolution};
use rc_image::{PixelFormat, Rgba};
use rc_package::texture::{self, format_name};
use rc_package::{ObjectRef, Package};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";
const DEFAULT_TEXTURE: &str = "DC15Blaster";
const LOG_PATH: &str = "logs/rc_viewer.log";
const MESH_LOG_PATH: &str = "logs/rc_viewer_mesh.log";
const DEFAULT_MESH: &str = "DC15Blaster";
const WINDOW_SIZE: u32 = 800;
/// The frame on which the on-screen check captures the window. A few frames
/// in, so the window and the texture upload have settled.
const CHECK_FRAME: u32 = 10;
/// Background colour, as 0-255 sRGB values (38 = 0.15).
const BACKGROUND: [u8; 3] = [38, 38, 38];

/// Our own log file, shared with Bevy's systems.
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

/// The decoded texture, handed from `main` to the startup system.
#[derive(Resource)]
struct LoadedTexture {
    name: String,
    image: Rgba,
}

#[derive(Resource)]
struct ExitAfter(Option<u32>);

/// What the window shows, which decides how the on-screen check works.
#[derive(Resource, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Texture,
    Mesh,
}

/// State of the on-screen check. `--frames` waits for it before closing.
#[derive(Resource, Default)]
struct ScreenCheck {
    /// Captures asked for so far.
    attempts: u32,
    /// Frame on which to ask for the next capture, if one is due.
    next_frame: Option<u32>,
    /// `Some(true)` once a capture passed, `Some(false)` once the check failed.
    passed: Option<bool>,
}

/// Captures that come back completely black are retried this many times.
/// In the cloud test machine (software rendering, no real screen) the first
/// capture is sometimes all black: the window had not been drawn yet.
const CHECK_ATTEMPTS: u32 = 5;

/// Things drawn in the scene: a sprite (texture view) or a 3D model.
type SceneObject = Or<(With<Sprite>, With<Mesh3d>)>;

#[derive(Resource, Default)]
struct FrameStats {
    frames: u32,
    last_report: f64,
    frames_since_report: u32,
}

fn pixel_format(format: u8) -> Option<PixelFormat> {
    Some(match format {
        0 => PixelFormat::P8,
        3 => PixelFormat::Dxt1,
        5 => PixelFormat::Bgra8,
        7 => PixelFormat::Dxt3,
        8 => PixelFormat::Dxt5,
        10 => PixelFormat::G16,
        _ => return None,
    })
}

/// Read the package and decode the named texture's full-size mipmap.
fn load_texture(log: &RunLog, path: &PathBuf, wanted: &str) -> Result<LoadedTexture, String> {
    let t = Instant::now();
    let data = fs::read(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    let pkg = Package::parse(&data).map_err(|e| e.to_string())?;
    log.line(format!(
        "package read: {} bytes, {} exports, {:.1} ms",
        data.len(),
        pkg.exports.len(),
        t.elapsed().as_secs_f64() * 1000.0
    ));
    let export = pkg
        .exports
        .iter()
        .find(|e| {
            pkg.export_class_name(e) == "Texture"
                && pkg.name(e.object_name).eq_ignore_ascii_case(wanted)
        })
        .ok_or_else(|| format!("no texture called {wanted} in this package"))?;
    let tex = texture::read_texture(&pkg, &data, export).map_err(|e| e.to_string())?;
    let format = pixel_format(tex.format)
        .ok_or_else(|| format!("format {} can't be decoded yet", tex.format))?;
    let palette = match tex.palette_ref(&pkg).map(ObjectRef::from_index) {
        Some(ObjectRef::Export(i)) => {
            let mut colors =
                texture::read_palette(&pkg, &data, &pkg.exports[i]).map_err(|e| e.to_string())?;
            if tex.masked(&pkg) {
                colors[0][3] = 0;
            }
            Some(colors)
        }
        Some(_) => return Err("palette is in another package (not supported yet)".into()),
        None => None,
    };
    let mip = tex.mips.first().ok_or("texture has no mipmaps")?;
    let bytes = &data[mip.data_offset..mip.data_offset + mip.data_len];
    let image = rc_image::decode(
        format,
        mip.width as usize,
        mip.height as usize,
        bytes,
        palette.as_deref(),
    )
    .map_err(|e| e.to_string())?;
    let [r, g, b, a] = image.mean();
    log.line(format!(
        "texture decoded: {} {}x{} {}, {} mips, average colour R {r:.0} G {g:.0} B {b:.0} A {a:.0}, {:.1} ms total",
        pkg.name(export.object_name),
        image.width,
        image.height,
        format_name(tex.format),
        tex.mips.len(),
        t.elapsed().as_secs_f64() * 1000.0
    ));
    Ok(LoadedTexture {
        name: pkg.name(export.object_name).to_string(),
        image,
    })
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    loaded: Res<LoadedTexture>,
    log: Res<RunLog>,
) {
    let img = &loaded.image;
    let image = Image::new(
        Extent3d {
            width: img.width as u32,
            height: img.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        img.pixels.clone(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    let handle = images.add(image);
    // Scale the picture to fit the window, keeping its shape.
    let fit = (WINDOW_SIZE as f32 * 0.9) / img.width.max(img.height) as f32;
    let size = Vec2::new(img.width as f32 * fit, img.height as f32 * fit);
    commands.spawn(Camera2d);
    commands.spawn(Sprite {
        custom_size: Some(size),
        ..Sprite::from_image(handle)
    });
    log.line(format!(
        "scene set up: camera + sprite of {} drawn at {:.0}x{:.0} pixels",
        loaded.name, size.x, size.y
    ));
}

// Bevy hands each system the parts of the game world it asks for, so a
// system that touches many parts has many arguments.
#[allow(clippy::too_many_arguments)]
fn count_frames(
    time: Res<Time>,
    log: Res<RunLog>,
    exit_after: Res<ExitAfter>,
    mut stats: ResMut<FrameStats>,
    windows: Query<&Window, With<PrimaryWindow>>,
    view: Res<ViewMode>,
    objects: Query<(), SceneObject>,
    mut exit: MessageWriter<AppExit>,
    mut commands: Commands,
    mut check: ResMut<ScreenCheck>,
) {
    stats.frames += 1;
    stats.frames_since_report += 1;
    let now = time.elapsed_secs_f64();
    if stats.frames == 1 {
        let window = windows
            .single()
            .map(|w| format!("{}x{}", w.width(), w.height()))
            .unwrap_or_else(|_| "none".into());
        log.line(format!(
            "first frame: window {window}, objects in scene: {}",
            objects.iter().count()
        ));
        stats.last_report = now;
    } else if now - stats.last_report >= 1.0 {
        let fps = stats.frames_since_report as f64 / (now - stats.last_report);
        log.line(format!(
            "frames so far: {}, about {fps:.0} frames per second",
            stats.frames
        ));
        stats.last_report = now;
        stats.frames_since_report = 0;
    }
    if stats.frames == 1 {
        check.next_frame = Some(CHECK_FRAME);
    }
    if check.next_frame == Some(stats.frames) {
        check.next_frame = None;
        check.attempts += 1;
        log.line(format!(
            "frame {}: capturing the window for the on-screen check (attempt {} of {CHECK_ATTEMPTS})",
            stats.frames, check.attempts
        ));
        let mut shot = commands.spawn(Screenshot::primary_window());
        match *view {
            ViewMode::Texture => shot.observe(check_screen),
            ViewMode::Mesh => shot.observe(mesh::check_screen),
        };
    }
    // Wait for the on-screen check, but give up on it after 300 extra frames.
    let check_settled =
        check.passed.is_some() || exit_after.0.is_some_and(|n| stats.frames >= n + 300);
    if exit_after.0.is_some_and(|n| stats.frames >= n) && check_settled {
        if check.passed.is_none() {
            log.line("on-screen check FAIL: no usable capture arrived");
        }
        log.line(format!(
            "closing after {} frames as asked (--frames)",
            stats.frames
        ));
        // Exit code 1 if the check failed, so a script can tell.
        exit.write(if check.passed == Some(true) {
            AppExit::Success
        } else {
            AppExit::from_code(1)
        });
    }
}

/// A window capture as plain sRGB colours, row by row from the top left.
struct Capture {
    w: usize,
    h: usize,
    pixels: Vec<[u8; 3]>,
    format: TextureFormat,
}

/// Turn a screenshot into a `Capture`. Returns `None` (and records the
/// outcome in `check`) if it can't be used: no data, an unknown pixel format,
/// or completely black, which is retried a few frames later.
fn read_capture(
    shot: &Image,
    log: &RunLog,
    frames: &FrameStats,
    check: &mut ScreenCheck,
) -> Option<Capture> {
    let (w, h) = (shot.width() as usize, shot.height() as usize);
    let format = shot.texture_descriptor.format;
    let Some(data) = shot.data.as_ref() else {
        log.line("on-screen check FAIL: the capture had no pixel data");
        check.passed = Some(false);
        return None;
    };
    let bgra = matches!(
        format,
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
    );
    if !bgra
        && !matches!(
            format,
            TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb
        )
    {
        log.line(format!(
            "on-screen check FAIL: capture format {format:?} not handled"
        ));
        check.passed = Some(false);
        return None;
    }
    let pixels: Vec<[u8; 3]> = data
        .as_chunks::<4>()
        .0
        .iter()
        .take(w * h)
        .map(|p| {
            if bgra {
                [p[2], p[1], p[0]]
            } else {
                [p[0], p[1], p[2]]
            }
        })
        .collect();
    if pixels.iter().all(|p| *p == [0, 0, 0]) {
        log.line("on-screen check: the capture is completely black (window not drawn yet)");
        if check.attempts < CHECK_ATTEMPTS {
            check.next_frame = Some(frames.frames + CHECK_FRAME);
        } else {
            log.line(format!(
                "on-screen check FAIL: still black after {CHECK_ATTEMPTS} captures"
            ));
            check.passed = Some(false);
        }
        return None;
    }
    Some(Capture {
        w,
        h,
        pixels,
        format,
    })
}

/// Compares the captured window with the decoded texture. The sprite fills a
/// square in the middle of the window, so each screen pixel there is compared
/// with the texture pixel it should show. A small average difference means
/// the texture really is on screen, the right way up and in the right colours.
fn check_screen(
    captured: On<ScreenshotCaptured>,
    loaded: Res<LoadedTexture>,
    log: Res<RunLog>,
    frames: Res<FrameStats>,
    mut check: ResMut<ScreenCheck>,
) {
    let Some(Capture {
        w,
        h,
        pixels,
        format,
    }) = read_capture(&captured.image, &log, &frames, &mut check)
    else {
        return;
    };
    let screen = |x: usize, y: usize| pixels[y * w + x];
    let tex = &loaded.image;
    let side = (w.min(h) as f32 * 0.9) as usize;
    let (x0, y0) = ((w - side) / 2, (h - side) / 2);
    // Skip a 2-pixel border, where the sprite's edge blends with the background.
    let (mut diff, mut count) = (0u64, 0u64);
    let mut mean = [0u64; 3];
    for sy in 2..side.saturating_sub(2) {
        for sx in 2..side.saturating_sub(2) {
            let s = screen(x0 + sx, y0 + sy);
            let tx = (sx * tex.width / side).min(tex.width - 1);
            let ty = (sy * tex.height / side).min(tex.height - 1);
            let t = &tex.pixels[(ty * tex.width + tx) * 4..];
            let expected = over_background(t);
            for c in 0..3 {
                diff += (s[c] as i32 - expected[c] as i32).unsigned_abs() as u64;
                mean[c] += s[c] as u64;
            }
            count += 1;
        }
    }
    let count = count.max(1);
    let avg_diff = diff as f64 / (count * 3) as f64;
    let mean = mean.map(|m| m as f64 / count as f64);
    let corner = screen(2, 2);
    log.line(format!(
        "on-screen check: capture {w}x{h} {format:?}, texture area {side}x{side} at ({x0}, {y0})"
    ));
    log.line(format!(
        "on-screen check: texture area average colour R {:.0} G {:.0} B {:.0}, corner pixel {:?} (background should be {:?})",
        mean[0], mean[1], mean[2], corner, BACKGROUND
    ));
    check.passed = Some(avg_diff < 10.0);
    let verdict = if avg_diff < 10.0 { "PASS" } else { "FAIL" };
    log.line(format!(
        "on-screen check {verdict}: average difference from the texture {avg_diff:.2} of 255 per channel (under 10 passes)"
    ));
}

/// What a texture pixel should look like on screen: see-through pixels are
/// mixed with the background by their alpha. The mixing happens on linear
/// light values (as the GPU does for sRGB images), not on the 0-255 numbers.
fn over_background(t: &[u8]) -> [u8; 3] {
    let to_linear = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let to_srgb = |v: f32| {
        let v = if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    let a = t[3] as f32 / 255.0;
    std::array::from_fn(|c| to_srgb(to_linear(t[c]) * a + to_linear(BACKGROUND[c]) * (1.0 - a)))
}

fn main() -> AppExit {
    let mut positional = Vec::new();
    let mut frames = None;
    let mut view = ViewMode::Texture;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--frames" {
            frames = args.next().and_then(|n| n.parse().ok());
        } else if arg == "--mesh" {
            view = ViewMode::Mesh;
        } else {
            positional.push(arg);
        }
    }
    let game = std::env::var_os("RC_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_DIR));
    let (folder, file, default_name, log_path) = match view {
        ViewMode::Texture => ("Textures", "weapontextures.utx", DEFAULT_TEXTURE, LOG_PATH),
        ViewMode::Mesh => ("StaticMeshes", "weapons.usx", DEFAULT_MESH, MESH_LOG_PATH),
    };
    let package = positional
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| game.join("GameData").join(folder).join(file));
    let wanted = positional
        .get(1)
        .cloned()
        .unwrap_or_else(|| default_name.to_string());

    // Rule: never write inside the game install. Refuse to run from in there.
    let cwd = std::env::current_dir().expect("can't read the current folder");
    if let (Ok(c), Ok(g)) = (cwd.canonicalize(), game.canonicalize()) {
        if c.starts_with(&g) {
            eprintln!("Refusing to run from inside the game folder ({}), because the log would be written there.", g.display());
            eprintln!("Run it from the project folder instead.");
            return AppExit::from_code(2);
        }
    }

    fs::create_dir_all("logs").expect("can't create the logs folder");
    let log = RunLog {
        file: Mutex::new(fs::File::create(log_path).expect("can't create the log file")),
        start: Instant::now(),
    };
    log.line(format!("rc_viewer {}", env!("CARGO_PKG_VERSION")));
    log.line(format!("package: {}", package.display()));

    let mut app = App::new();
    let title = match view {
        ViewMode::Texture => {
            log.line(format!("texture: {wanted}"));
            let loaded = match load_texture(&log, &package, &wanted) {
                Ok(t) => t,
                Err(e) => {
                    log.line(format!("FAILED: {e}"));
                    return AppExit::from_code(1);
                }
            };
            let title = format!("Republic Commando texture: {}", loaded.name);
            app.insert_resource(loaded).add_systems(Startup, setup);
            title
        }
        ViewMode::Mesh => {
            log.line(format!("mesh: {wanted}"));
            let loaded = match mesh::load(&log, &game, &package, &wanted) {
                Ok(m) => m,
                Err(e) => {
                    log.line(format!("FAILED: {e}"));
                    return AppExit::from_code(1);
                }
            };
            let title = format!("Republic Commando model: {wanted}");
            app.insert_resource(loaded)
                .add_systems(Startup, mesh::setup)
                .add_systems(Update, mesh::spin);
            title
        }
    };

    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title,
            resolution: WindowResolution::new(WINDOW_SIZE, WINDOW_SIZE),
            ..default()
        }),
        ..default()
    }))
    .insert_resource(ClearColor(Color::srgb(0.15, 0.15, 0.15)))
    .insert_resource(view)
    .insert_resource(ExitAfter(frames))
    .insert_resource(FrameStats::default())
    .insert_resource(ScreenCheck::default())
    .insert_resource(log)
    .add_systems(Update, count_frames)
    .run()
}
