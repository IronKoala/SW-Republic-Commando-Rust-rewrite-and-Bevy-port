//! Checks the tactical visor without the game: a grey ball (standing in for
//! Sev) and a grey box (a pickup) are drawn in each visor mode, and the
//! colours in every picture are measured.
//!
//! Run (Linux, no screen needed):
//! `xvfb-run -a -s "-screen 0 1280x1024x24" cargo run -p rc_visor --example visor_check`
//! It ends `VISOR CHECK: N passed, 0 failed`. With RC_GAME_DIR set to the
//! game's folder it uses the game's own colours; without, stand-in ones
//! (red for the ball, pale blue for the box). Pictures are saved as
//! `logs/visor_check_<step>.png`.

use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use rc_game_core::{Action, ActionState, CorePlugin, GameLog, GameSet, GameState, PlayerCamera};
use rc_visor::original::{self, Original, VisorFile};
use rc_visor::{
    TacticalGlow, VisorChoice, VisorLook, VisorMode, VisorPlugin, VisorReadout, VisorSettings,
    VisorState,
};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const FRAMES_PER_STEP: u32 = 20;
const W: u32 = 640;
const H: u32 = 360;

#[derive(Clone, Copy)]
enum Change {
    None,
    /// Set the options.
    Options(VisorChoice, f32),
    /// Press CYCLE VISOR MODE.
    Key,
}

const STEPS: &[(&str, Change)] = &[
    // The first picture under Xvfb is often black; it isn't checked.
    ("warm-up", Change::None),
    // OFF moves the visor out of tactical mode, as the original does.
    ("normal", Change::Options(VisorChoice::Off, 0.2)),
    ("tactical 0.2", Change::Options(VisorChoice::Cycle, 0.2)),
    ("tactical 1.0", Change::Options(VisorChoice::Cycle, 1.0)),
    ("key: low light", Change::Key),
    ("key: normal", Change::Key),
    ("key: tactical", Change::Key),
    ("intensity 0", Change::Options(VisorChoice::Cycle, 0.0)),
];

/// Per picture: red excess at the ball's edge and in its middle, blue
/// excess on the box's edge, floor brightness, visor mode and shells shown.
#[derive(Debug, Clone, Copy, Default)]
struct Measured {
    ball_edge_red: f32,
    ball_middle_red: f32,
    box_blue: f32,
    floor: f32,
    mode: Option<VisorMode>,
    shown: usize,
}

#[derive(Resource, Default)]
struct Run {
    frame: u32,
    step: usize,
    pending: Option<(usize, VisorMode, usize)>,
    results: Arc<Mutex<Vec<(usize, Measured)>>>,
}

fn main() {
    let log = GameLog::terminal();
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let file = VisorFile::read(&data.join(original::DATA_FILE)).expect("data file");
    let found = match std::env::var_os("RC_GAME_DIR") {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            println!("using the game's colours from {}", dir.display());
            original::read(&file, &|rel| rc_game_core::paths::resolve(&dir, rel))
        }
        None => {
            println!("RC_GAME_DIR not set: stand-in colours");
            Original {
                colours: BTreeMap::from([
                    ("sev".to_string(), [220, 30, 30]),
                    ("pickup".to_string(), [150, 200, 255]),
                ]),
                ..default()
            }
        }
    };
    println!(
        "colours: {:?}; problems: {:?}",
        found.colours, found.problems
    );
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                resolution: (W, H).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(CorePlugin)
        .insert_resource(log)
        .insert_resource(VisorLook {
            file,
            original: found,
        })
        .init_resource::<Run>()
        .add_plugins(VisorPlugin)
        .add_systems(Startup, scene)
        .add_systems(Update, drive.in_set(GameSet::Input))
        .add_observer(measure)
        .run();
}

fn scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut next: ResMut<NextState<GameState>>,
) {
    next.set(GameState::InGame);
    commands.spawn((
        PlayerCamera,
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let grey = materials.add(StandardMaterial {
        base_color: Color::srgb(0.4, 0.4, 0.4),
        perceptual_roughness: 0.9,
        ..default()
    });
    // The glow goes on the parent; the meshes are children, as the squad's
    // and pickups' are.
    commands
        .spawn((
            TacticalGlow("sev".into()),
            Transform::from_xyz(-1.2, 0.0, 0.0),
            Visibility::default(),
        ))
        .with_child((
            Mesh3d(meshes.add(Sphere::new(1.0).mesh().uv(48, 24))),
            MeshMaterial3d(grey.clone()),
        ));
    commands
        .spawn((
            TacticalGlow("pickup".into()),
            Transform::from_xyz(1.5, 0.0, 0.0).with_rotation(Quat::from_rotation_y(0.6)),
            Visibility::default(),
        ))
        .with_child((
            Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
            MeshMaterial3d(grey.clone()),
        ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(30.0, 30.0))),
        MeshMaterial3d(grey),
        Transform::from_xyz(0.0, -1.5, 0.0),
    ));
    commands.spawn((
        PointLight {
            intensity: 300_000.0,
            range: 30.0,
            ..default()
        },
        Transform::from_xyz(0.0, 3.0, 4.0),
    ));
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    mut run: ResMut<Run>,
    mut settings: ResMut<VisorSettings>,
    mut state: ResMut<VisorState>,
    mut actions: ResMut<ActionState>,
    readout: Res<VisorReadout>,
    mut exit: MessageWriter<AppExit>,
) {
    run.frame += 1;
    let f = run.frame % FRAMES_PER_STEP;
    // Release the key the frame after pressing it.
    actions.update(HashSet::new(), Vec2::ZERO, Vec2::ZERO);
    if f == 1 {
        let Some(&(name, change)) = STEPS.get(run.step) else {
            if run.results.lock().unwrap().len() < STEPS.len() && run.step < STEPS.len() + 4 {
                run.step += 1;
                return;
            }
            report(&run, &readout);
            exit.write(if failures(&run, &readout).is_empty() {
                AppExit::Success
            } else {
                AppExit::from_code(1)
            });
            return;
        };
        match change {
            Change::None => {}
            Change::Options(choice, intensity) => {
                *settings = VisorSettings { choice, intensity };
                if choice != VisorChoice::Off {
                    state.mode = VisorMode::Tactical;
                }
            }
            Change::Key => {
                actions.update(HashSet::from([Action::Flashlight]), Vec2::ZERO, Vec2::ZERO)
            }
        }
        println!("step: {name}");
    }
    if f == 0 && run.step < STEPS.len() {
        run.pending = Some((run.step, state.mode, readout.shown));
        commands.spawn(Screenshot::primary_window());
        run.step += 1;
    }
}

fn measure(captured: On<ScreenshotCaptured>, run: Res<Run>) {
    let shot = &captured.image;
    let (w, h) = (shot.width() as usize, shot.height() as usize);
    let bgra = matches!(
        shot.texture_descriptor.format,
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
    );
    let Some(data) = shot.data.as_ref() else {
        return;
    };
    let px = |x: f32, y: f32| -> [f32; 3] {
        let (x, y) = ((x as usize).min(w - 1), (y as usize).min(h - 1));
        let p = &data[(y * w + x) * 4..];
        let (r, g, b) = if bgra {
            (p[2], p[1], p[0])
        } else {
            (p[0], p[1], p[2])
        };
        [r as f32, g as f32, b as f32]
    };
    // Where things are on screen: 1 unit at the objects' distance is
    // h / 2 / tan(22.5 deg) / 5 pixels (Bevy's default 45 degree view).
    let unit = h as f32 / 2.0 / (22.5f32.to_radians().tan() * 5.0);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let ball = (cx - 1.2 * unit, cy);
    let red = |p: [f32; 3]| p[0] - (p[1] + p[2]) / 2.0;
    let blue = |p: [f32; 3]| p[2] - p[0];
    let mut edge = 0.0;
    for k in 0..24 {
        let a = k as f32 / 24.0 * std::f32::consts::TAU;
        let r = 0.93 * unit;
        edge += red(px(ball.0 + a.cos() * r, ball.1 + a.sin() * r));
    }
    edge /= 24.0;
    let mut middle = 0.0;
    for dy in -3..=3 {
        for dx in -3..=3 {
            middle += red(px(ball.0 + dx as f32, ball.1 + dy as f32));
        }
    }
    middle /= 49.0;
    // The box's left and right edges, half way up.
    let bx = cx + 1.5 * unit;
    let mut box_blue = 0.0;
    let mut n = 0.0;
    for dx in [-0.5f32, -0.45, 0.45, 0.5] {
        for dy in [-0.3f32, 0.0, 0.3] {
            box_blue += blue(px(bx + dx * unit, cy + dy * unit));
            n += 1.0;
        }
    }
    box_blue /= n;
    // The floor in front, below the objects.
    let mut floor = 0.0;
    for y in (h * 85 / 100)..(h * 95 / 100) {
        for x in (w * 40 / 100)..(w * 60 / 100) {
            let p = px(x as f32, y as f32);
            floor += (p[0] + p[1] + p[2]) / 3.0;
        }
    }
    floor /= ((h / 10) * (w / 5)) as f32;
    let (step, mode, shown) = run.pending.unwrap_or((usize::MAX, VisorMode::Normal, 0));
    let name = STEPS.get(step).map(|s| s.0).unwrap_or("?");
    let m = Measured {
        ball_edge_red: edge,
        ball_middle_red: middle,
        box_blue,
        floor,
        mode: Some(mode),
        shown,
    };
    println!(
        "  {name}: mode {:?}, shells shown {shown}, ball edge red {edge:.1}, middle {middle:.1}, box blue {box_blue:.1}, floor {floor:.1}",
        mode
    );
    let _ = std::fs::create_dir_all("logs");
    let mut pixels = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let p = px(x as f32, y as f32);
            pixels.extend_from_slice(&[p[0] as u8, p[1] as u8, p[2] as u8, 255]);
        }
    }
    let img = rc_image::Rgba {
        width: w,
        height: h,
        pixels,
    };
    let file = format!(
        "logs/visor_check_{}.png",
        name.replace([' ', ':', '.'], "_")
    );
    let _ = std::fs::write(file, rc_image::png::encode(&img));
    run.results.lock().unwrap().push((step, m));
}

fn failures(run: &Run, readout: &VisorReadout) -> Vec<String> {
    let r = run.results.lock().unwrap();
    let get = |n: &str| {
        let i = STEPS.iter().position(|s| s.0 == n)?;
        r.iter().find(|x| x.0 == i).map(|x| x.1)
    };
    let mut out = vec![];
    if readout.shells != 2 {
        out.push(format!(
            "{} glowing copies, want 2 (ball and box)",
            readout.shells
        ));
    }
    let (Some(normal), Some(t02), Some(t1), Some(low), Some(back_normal), Some(back_t), Some(zero)) = (
        get("normal"),
        get("tactical 0.2"),
        get("tactical 1.0"),
        get("key: low light"),
        get("key: normal"),
        get("key: tactical"),
        get("intensity 0"),
    ) else {
        out.push(format!("{} pictures of {}", r.len(), STEPS.len()));
        return out;
    };
    let mode = |m: &Measured, want: VisorMode, what: &str, out: &mut Vec<String>| {
        if m.mode != Some(want) {
            out.push(format!("{what}: mode {:?}, want {want:?}", m.mode));
        }
    };
    mode(&normal, VisorMode::Normal, "OFF option", &mut out);
    mode(&low, VisorMode::LowLight, "first key press", &mut out);
    mode(
        &back_normal,
        VisorMode::Normal,
        "second key press",
        &mut out,
    );
    mode(&back_t, VisorMode::Tactical, "third key press", &mut out);
    if normal.ball_edge_red > 10.0 {
        out.push(format!(
            "normal mode: the ball is already red ({:.1})",
            normal.ball_edge_red
        ));
    }
    if t02.ball_edge_red < normal.ball_edge_red + 8.0 {
        out.push(format!(
            "tactical 0.2: the ball's edge isn't red ({:.1} against {:.1})",
            t02.ball_edge_red, normal.ball_edge_red
        ));
    }
    if t1.ball_edge_red < t02.ball_edge_red + 10.0 {
        out.push(format!(
            "intensity 1.0 isn't stronger than 0.2 ({:.1} against {:.1})",
            t1.ball_edge_red, t02.ball_edge_red
        ));
    }
    if t1.ball_edge_red < t1.ball_middle_red + 10.0 {
        out.push(format!(
            "the edge doesn't glow more than the middle ({:.1} against {:.1})",
            t1.ball_edge_red, t1.ball_middle_red
        ));
    }
    if t1.box_blue < normal.box_blue + 8.0 {
        out.push(format!(
            "the pickup's edge isn't blue ({:.1} against {:.1})",
            t1.box_blue, normal.box_blue
        ));
    }
    if low.ball_edge_red > normal.ball_edge_red + 5.0 || low.shown != 0 {
        out.push(format!(
            "low light: still glowing (red {:.1}, {} shown)",
            low.ball_edge_red, low.shown
        ));
    }
    if low.floor < normal.floor + 5.0 {
        out.push(format!(
            "low light: the lamp doesn't light the floor ({:.1} against {:.1})",
            low.floor, normal.floor
        ));
    }
    if (back_normal.floor - normal.floor).abs() > 3.0
        || (back_normal.ball_edge_red - normal.ball_edge_red).abs() > 3.0
    {
        out.push("back in normal mode the picture differs from before".into());
    }
    if (back_t.ball_edge_red - t1.ball_edge_red).abs() > 6.0 {
        out.push(format!(
            "back in tactical mode the glow differs ({:.1} against {:.1})",
            back_t.ball_edge_red, t1.ball_edge_red
        ));
    }
    if zero.ball_edge_red > normal.ball_edge_red + 3.0 || zero.shown != 0 {
        out.push(format!(
            "intensity 0 still glows (red {:.1}, {} shown)",
            zero.ball_edge_red, zero.shown
        ));
    }
    out
}

fn report(run: &Run, readout: &VisorReadout) {
    let f = failures(run, readout);
    for x in &f {
        println!("FAIL: {x}");
    }
    let checks = 17;
    println!(
        "VISOR CHECK: {} passed, {} failed",
        checks - f.len().min(checks),
        f.len()
    );
}
