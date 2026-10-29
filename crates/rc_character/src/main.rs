//! rc_character: shows one character from the game in a 3D window, playing
//! one of its animations.
//!
//! Usage:
//!   rc_character [sequence] [--set NAME] [--mesh NAME] [--package FILE]
//!                [--list] [--frames N]
//!
//! Defaults to the `CloneTrooper` mesh in `GameData/Animations/clone.ukx`,
//! its first linked animation set, playing `WalkForward` from
//! `CloneCommandoSet`. `--list` logs every sequence in the set. `--frames N`
//! closes the window by itself after N frames, for automated tests.
//!
//! The skin texture is found by following the mesh's material into the
//! texture package (see `load.rs`).
//!
//! How the animation is drawn: each frame, `rc_anim` poses the skeleton and
//! moves the mesh's points on the CPU ("skinning"), and the new positions are
//! copied into the Bevy mesh. The root bone's sideways movement is removed so
//! a walk stays in place in front of the camera.
//!
//! Everything is logged to `logs/rc_character.log`. On frame `CHECK_FRAME`
//! the window is captured and the character's outline on screen is compared
//! with the outline we expect from our own numbers.

mod check;
mod load;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::Screenshot;
use bevy::render::view::{DebandDither, Tonemapping};
use bevy::window::{PrimaryWindow, WindowResolution};
use rc_anim::globals;
use rc_package::skeletal::SkeletalMesh;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";
const DEFAULT_SET: &str = "CloneCommandoSet";
const DEFAULT_SEQUENCE: &str = "WalkForward";
const LOG_PATH: &str = "logs/rc_character.log";
const WINDOW_SIZE: u32 = 800;
/// The frame on which the on-screen check captures the window.
const CHECK_FRAME: u32 = 30;
/// Background colour, 0-255 sRGB. Blue-grey, so both the white armour and
/// the black body suit stand out from it.
pub(crate) const BACKGROUND: [u8; 3] = [40, 60, 90];

/// Our own log file, shared with Bevy's systems.
#[derive(Resource)]
pub(crate) struct RunLog {
    file: Mutex<fs::File>,
    start: Instant,
}

impl RunLog {
    pub(crate) fn line(&self, s: impl AsRef<str>) {
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

/// Converts a point from the game's axes to Bevy's.
///
/// The game (Unreal Engine): X forward, Y right, Z up, "left-handed".
/// Bevy: X right, Y up, the camera looks along -Z, "right-handed".
/// So Bevy X = game Y, Bevy Y = game Z, Bevy Z = -game X. This is a mirror
/// image swap as well as a turn, which is what changing handedness needs;
/// it also means triangles come out the other way round (see `load.rs`).
/// The mesh's own scale, origin and turn (yaw only) are applied first.
pub(crate) fn to_bevy(mesh: &SkeletalMesh, p: [f32; 3]) -> [f32; 3] {
    let s = mesh.mesh_scale;
    let o = mesh.mesh_origin;
    let q = [
        (p[0] - o[0]) * s[0],
        (p[1] - o[1]) * s[1],
        (p[2] - o[2]) * s[2],
    ];
    let yaw = mesh.rot_origin[1] as f32 * std::f32::consts::TAU / 65536.0;
    let (sin, cos) = yaw.sin_cos();
    let q = [q[0] * cos - q[1] * sin, q[0] * sin + q[1] * cos, q[2]];
    [q[1], q[2], -q[0]]
}

#[derive(Resource)]
struct Loaded(load::Character);

/// The live state of the animation.
#[derive(Resource)]
pub(crate) struct Playback {
    mesh: Handle<Mesh>,
    /// Current frame of the sequence (fractional).
    pub(crate) frame: f32,
    /// Bevy-space positions drawn this frame, one per point.
    pub(crate) points: Vec<[f32; 3]>,
    /// Pause while a capture is on its way, so the screen matches `points`.
    pub(crate) paused: bool,
    min_height: f32,
    max_height: f32,
}

#[derive(Resource)]
struct ExitAfter(Option<u32>);

#[derive(Resource, Default)]
struct FrameStats {
    frames: u32,
    last_report: f64,
    frames_since_report: u32,
}

/// Corner order for each triangle, as wedge numbers (one Bevy vertex per wedge).
fn indices(c: &load::Character) -> Vec<u32> {
    let mut out = Vec::with_capacity(c.mesh.triangles.len() * 3);
    for t in &c.mesh.triangles {
        let [a, b, d] = t.wedges.map(|w| w as u32);
        if c.flip_winding {
            out.extend([a, d, b]);
        } else {
            out.extend([a, b, d]);
        }
    }
    out
}

/// Smooth normals (the direction each point's surface faces, for lighting):
/// add up the facing of every triangle touching a point, then shorten to
/// length 1. Returned per wedge.
fn wedge_normals(c: &load::Character, points: &[[f32; 3]], idx: &[u32]) -> Vec<[f32; 3]> {
    use rc_anim::math::{cross, length, sub};
    let point_of = |w: u32| c.mesh.wedges[w as usize].point as usize;
    let mut n = vec![[0f32; 3]; points.len()];
    for tri in idx.as_chunks::<3>().0 {
        let [a, b, d] = [point_of(tri[0]), point_of(tri[1]), point_of(tri[2])];
        let f = cross(sub(points[b], points[a]), sub(points[d], points[a]));
        for p in [a, b, d] {
            for k in 0..3 {
                n[p][k] += f[k];
            }
        }
    }
    for v in &mut n {
        let l = length(*v);
        *v = if l > 0.0 {
            v.map(|x| x / l)
        } else {
            [0.0, 1.0, 0.0]
        };
    }
    c.mesh.wedges.iter().map(|w| n[w.point as usize]).collect()
}

/// Pose the skeleton at `frame`, skin the points and convert to Bevy axes.
fn posed_points(c: &load::Character, frame: f32) -> Vec<[f32; 3]> {
    let mut local = c.clip.sample(&c.skeleton, frame);
    // Keep the character in place: undo the root bone's sideways movement
    // (the game moves the character itself while it walks).
    if let (Some(l), Some(rest)) = (local.first_mut(), c.skeleton.local.first()) {
        l.pos[0] = rest.pos[0];
        l.pos[1] = rest.pos[1];
    }
    let pose = globals(&c.skeleton.parents, &local);
    rc_anim::skin(&c.mesh.points, &c.weights, &c.skeleton, &pose)
        .into_iter()
        .map(|p| to_bevy(&c.mesh, p))
        .collect()
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    loaded: Res<Loaded>,
    log: Res<RunLog>,
) {
    let c = &loaded.0;
    let points = posed_points(c, 0.0);
    let idx = indices(c);
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    let wedge_pos: Vec<[f32; 3]> = c
        .mesh
        .wedges
        .iter()
        .map(|w| points[w.point as usize])
        .collect();
    let uvs: Vec<[f32; 2]> = c.mesh.wedges.iter().map(|w| [w.u, w.v]).collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, wedge_normals(c, &points, &idx));
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, wedge_pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(idx));
    let handle = meshes.add(mesh);

    let base_color_texture = c.skin.as_ref().map(|(_, img)| {
        images.add(Image::new(
            Extent3d {
                width: img.width as u32,
                height: img.height as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            img.pixels.clone(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        ))
    });
    let material = materials.add(StandardMaterial {
        base_color: if base_color_texture.is_some() {
            Color::WHITE
        } else {
            Color::srgb(0.6, 0.6, 0.6)
        },
        base_color_texture,
        perceptual_roughness: 0.7,
        ..default()
    });
    // The points move every frame, so the outline Bevy measured at the start
    // would be wrong; tell it not to skip drawing based on that outline.
    commands.spawn((
        Mesh3d(handle.clone()),
        MeshMaterial3d(material),
        NoFrustumCulling,
    ));

    // Camera: in front of the character (the game's "forward" is Bevy's -Z),
    // a little to the side and above, looking at its middle.
    let (lo, hi) = rc_package::skeletal::bounds(&points);
    let centre = Vec3::new(
        (lo[0] + hi[0]) / 2.0,
        (lo[1] + hi[1]) / 2.0,
        (lo[2] + hi[2]) / 2.0,
    );
    let height = hi[1] - lo[1];
    let eye = centre + Vec3::new(height * 0.6, height * 0.15, -height * 1.6);
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(eye).looking_at(centre, Vec3::Y),
        // Bevy's default colour grading ("tonemapping") needs lookup tables
        // from the `tonemapping_luts` feature, which this build leaves out
        // to share one Bevy build with rc_viewer. Without them every lit
        // pixel comes out magenta, so switch it off (rc_viewer does too).
        Tonemapping::None,
        // Dithering only works with tonemapping on; say so to avoid a warning.
        DebandDither::Disabled,
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 8000.0,
            ..default()
        },
        Transform::from_translation(centre + Vec3::new(height, height * 2.0, -height * 1.5))
            .looking_at(centre, Vec3::Y),
    ));
    log.line(format!(
        "scene set up: camera at ({:.0}, {:.0}, {:.0}) looking at ({:.0}, {:.0}, {:.0}), character {:.1} units tall, skin {}",
        eye.x,
        eye.y,
        eye.z,
        centre.x,
        centre.y,
        centre.z,
        height,
        c.skin.as_ref().map_or("none (grey)".to_string(), |(n, _)| n.clone())
    ));
    commands.insert_resource(Playback {
        mesh: handle,
        frame: 0.0,
        points,
        paused: false,
        min_height: f32::INFINITY,
        max_height: f32::NEG_INFINITY,
    });
}

/// Advance the animation and copy the new point positions into the mesh.
fn animate(
    time: Res<Time>,
    loaded: Res<Loaded>,
    mut play: ResMut<Playback>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let c = &loaded.0;
    if play.paused {
        return;
    }
    play.frame = (play.frame + time.delta_secs() * c.clip.rate).rem_euclid(c.clip.num_frames);
    let points = posed_points(c, play.frame);
    let (lo, hi) = rc_package::skeletal::bounds(&points);
    play.min_height = play.min_height.min(hi[1] - lo[1]);
    play.max_height = play.max_height.max(hi[1] - lo[1]);
    let idx = indices(c);
    let normals = wedge_normals(c, &points, &idx);
    let wedge_pos: Vec<[f32; 3]> = c
        .mesh
        .wedges
        .iter()
        .map(|w| points[w.point as usize])
        .collect();
    if let Some(mut mesh) = meshes.get_mut(&play.mesh) {
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, wedge_pos);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    }
    play.points = points;
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
    cameras: Query<(&Camera, &GlobalTransform)>,
    mut play: ResMut<Playback>,
    mut exit: MessageWriter<AppExit>,
    mut commands: Commands,
    mut check: ResMut<check::ScreenCheck>,
) {
    stats.frames += 1;
    stats.frames_since_report += 1;
    let now = time.elapsed_secs_f64();
    let (lo, hi) = rc_package::skeletal::bounds(&play.points);
    if stats.frames == 1 {
        let window = windows
            .single()
            .map(|w| format!("{}x{}", w.width(), w.height()))
            .unwrap_or_else(|_| "none".into());
        log.line(format!("first frame: window {window}"));
        stats.last_report = now;
        check.next_frame = Some(CHECK_FRAME);
    } else if now - stats.last_report >= 1.0 {
        let fps = stats.frames_since_report as f64 / (now - stats.last_report);
        log.line(format!(
            "frames so far: {}, about {fps:.0} per second; animation frame {:.1}; character box ({:.1}, {:.1}, {:.1}) to ({:.1}, {:.1}, {:.1}); height so far {:.1} to {:.1}",
            stats.frames, play.frame, lo[0], lo[1], lo[2], hi[0], hi[1], hi[2], play.min_height, play.max_height
        ));
        stats.last_report = now;
        stats.frames_since_report = 0;
    }
    if check.next_frame == Some(u32::MAX) {
        // A black capture asked for a retry; do it 10 frames from now.
        check.next_frame = Some(stats.frames + 10);
    }
    if check.next_frame == Some(stats.frames) {
        check.next_frame = None;
        check.attempts += 1;
        let (Ok(window), Ok((camera, cam_tf))) = (windows.single(), cameras.single()) else {
            log.line("on-screen check FAIL: no window or camera");
            check.passed = Some(false);
            return;
        };
        // Where every point should land on screen, in window pixels.
        let projected: Vec<Option<Vec2>> = play
            .points
            .iter()
            .map(|p| camera.world_to_viewport(cam_tf, Vec3::from_array(*p)).ok())
            .collect();
        check.expected = Some(check::Expected {
            projected,
            window: Vec2::new(window.width(), window.height()),
        });
        play.paused = true;
        log.line(format!(
            "frame {}: capturing the window for the on-screen check (attempt {} of {}), animation paused at frame {:.1}",
            stats.frames,
            check.attempts,
            check::CHECK_ATTEMPTS,
            play.frame
        ));
        commands
            .spawn(Screenshot::primary_window())
            .observe(check::check_screen);
    }
    if check.passed.is_some() || check.next_frame.is_some() {
        play.paused = false;
    }
    let check_settled =
        check.passed.is_some() || exit_after.0.is_some_and(|n| stats.frames >= n + 300);
    if exit_after.0.is_some_and(|n| stats.frames >= n) && check_settled {
        if check.passed.is_none() {
            log.line("on-screen check FAIL: no usable capture arrived");
        }
        log.line(format!(
            "closing after {} frames as asked (--frames); character height during the run {:.1} to {:.1}",
            stats.frames, play.min_height, play.max_height
        ));
        exit.write(if check.passed == Some(true) {
            AppExit::Success
        } else {
            AppExit::from_code(1)
        });
    }
}

fn main() -> AppExit {
    let mut positional = Vec::new();
    let (mut frames, mut set, mut mesh, mut package, mut list) = (None, None, None, None, false);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--frames" => frames = args.next().and_then(|n| n.parse().ok()),
            "--set" => set = args.next(),
            "--mesh" => mesh = args.next(),
            "--package" => package = args.next().map(PathBuf::from),
            "--list" => list = true,
            _ => positional.push(arg),
        }
    }
    let game = std::env::var_os("RC_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_DIR));
    let using_defaults = package.is_none();
    let wanted = load::Wanted {
        package: package
            .unwrap_or_else(|| game.join("GameData").join("Animations").join("clone.ukx")),
        mesh: mesh.or_else(|| using_defaults.then(|| "CloneTrooper".to_string())),
        set: set.or_else(|| using_defaults.then(|| DEFAULT_SET.to_string())),
        sequence: positional
            .first()
            .cloned()
            .or_else(|| using_defaults.then(|| DEFAULT_SEQUENCE.to_string())),
        list,
    };

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
        file: Mutex::new(fs::File::create(LOG_PATH).expect("can't create the log file")),
        start: Instant::now(),
    };
    log.line(format!("rc_character {}", env!("CARGO_PKG_VERSION")));
    log.line(format!("package: {}", wanted.package.display()));

    let character = match load::load(&log, &game, &wanted) {
        Ok(c) => c,
        Err(e) => {
            log.line(format!("FAILED: {e}"));
            return AppExit::from_code(1);
        }
    };
    let title = format!(
        "Republic Commando: {} playing {} ({})",
        character.mesh_name, character.clip.name, character.set_name
    );

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title,
                resolution: WindowResolution::new(WINDOW_SIZE, WINDOW_SIZE),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb_u8(
            BACKGROUND[0],
            BACKGROUND[1],
            BACKGROUND[2],
        )))
        .insert_resource(Loaded(character))
        .insert_resource(ExitAfter(frames))
        .insert_resource(FrameStats::default())
        .insert_resource(check::ScreenCheck::default())
        .insert_resource(log)
        .add_systems(Startup, setup)
        .add_systems(Update, (animate, count_frames).chain())
        .run()
}
