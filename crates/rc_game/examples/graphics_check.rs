//! Checks the GRAPHICS options without the game's files: a stand-in scene
//! with the game's stack of cameras (sky, level view, arms, then menus and
//! display) is drawn with each option on, and every picture is measured.
//!
//! Run (Linux, no screen needed):
//! `xvfb-run -a -s "-screen 0 1280x1024x24" cargo run -p rc_game --example graphics_check`
//! It ends `GRAPHICS CHECK: N passed, 0 failed`.

#[path = "../src/graphics.rs"]
mod graphics;
#[path = "../src/settings.rs"]
#[allow(dead_code)]
mod settings;

use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use rc_game_core::{GameLog, PlayerCamera};
use settings::Settings;
use std::sync::{Arc, Mutex};

/// One step: the settings to draw with.
struct Step {
    name: &'static str,
    change: fn(&mut Settings),
}

const STEPS: &[Step] = &[
    // The first picture under Xvfb is often black; it isn't checked.
    Step {
        name: "warm-up",
        change: |_| {},
    },
    Step {
        name: "defaults",
        change: |_| {},
    },
    Step {
        name: "brightness 1",
        change: |s| s.video.brightness = 1.0,
    },
    Step {
        name: "brightness 0",
        change: |s| s.video.brightness = 0.0,
    },
    Step {
        name: "fsaa 4",
        change: |s| s.video.fsaa = 4,
    },
    Step {
        name: "bloom",
        change: |s| s.video.bloom = true,
    },
    Step {
        name: "bloom + brightness 1",
        change: |s| {
            s.video.bloom = true;
            s.video.brightness = 1.0;
        },
    },
    Step {
        name: "blur",
        change: |s| s.video.blur = true,
    },
    Step {
        name: "shadows",
        change: |s| s.video.squad_shadows = true,
    },
    Step {
        name: "everything",
        change: |s| {
            s.video.bloom = true;
            s.video.blur = true;
            s.video.fsaa = 8;
            s.video.squad_shadows = true;
            s.video.brightness = 0.8;
        },
    },
    Step {
        name: "back to defaults",
        change: |_| {},
    },
];

const FRAMES_PER_STEP: u32 = 25;

/// A step's name, the middle's and the sky's average brightness, and the
/// share of the display box drawn.
type Measured = (&'static str, f32, f32, f32);

#[derive(Resource, Default)]
struct Run {
    frame: u32,
    step: usize,
    /// Average brightness of the middle and of the top-left corner (the
    /// sky), and the share of pixels drawn by the 2D camera's red box.
    results: Arc<Mutex<Vec<Measured>>>,
}

fn main() {
    let log = GameLog::terminal();
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                resolution: (640u32, 360u32).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(log)
        .insert_resource(Settings::default())
        .init_resource::<Run>()
        .add_plugins(graphics::GraphicsOptionsPlugin)
        .add_systems(Startup, scene)
        .add_systems(Update, (spin, drive))
        .add_observer(measure)
        .run();
}

#[derive(Component)]
struct Spin;

fn scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // The sky: its own camera, drawn first with a background colour.
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: -1,
            clear_color: ClearColorConfig::Custom(Color::srgb(0.1, 0.2, 0.5)),
            ..default()
        },
        Tonemapping::None,
        Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // The level view, drawn over the sky.
    commands
        .spawn((
            PlayerCamera,
            Camera3d::default(),
            Camera {
                order: 0,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            Tonemapping::None,
            Transform::from_xyz(0.0, 1.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        ))
        .with_children(|c| {
            // The arms, on their own layer.
            c.spawn((
                Camera3d::default(),
                Camera {
                    order: 1,
                    clear_color: ClearColorConfig::None,
                    ..default()
                },
                Tonemapping::None,
                RenderLayers::layer(1),
            ));
        });
    // Menus and the display.
    commands.spawn((
        Camera2d,
        Camera {
            order: 2,
            clear_color: ClearColorConfig::None,
            ..default()
        },
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(0.0),
            bottom: Val::Px(0.0),
            width: Val::Px(64.0),
            height: Val::Px(64.0),
            ..default()
        },
        BackgroundColor(Color::srgb(1.0, 0.0, 0.0)),
    ));
    // A lit floor and a spinning box (moves, for the blur), a bright light.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(20.0, 20.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.5, 0.5, 0.5))),
        Transform::from_xyz(0.0, -1.0, 0.0),
    ));
    commands.spawn((
        Spin,
        Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            emissive: LinearRgba::rgb(2.0, 2.0, 2.0),
            ..default()
        })),
    ));
    // The arms stand-in: a small box in front of the arms camera.
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.2, 0.2, 0.2))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.0, 1.0, 0.0),
            unlit: true,
            ..default()
        })),
        Transform::from_xyz(0.4, 0.6, 4.0),
        RenderLayers::layer(1),
    ));
    commands.spawn((
        PointLight {
            intensity: 2_000_000.0,
            range: 30.0,
            ..default()
        },
        Transform::from_xyz(2.0, 3.0, 2.0),
    ));
}

fn spin(time: Res<Time>, mut q: Query<&mut Transform, With<Spin>>) {
    for mut t in &mut q {
        t.rotate_y(time.delta_secs() * 3.0);
    }
}

fn drive(
    mut commands: Commands,
    mut run: ResMut<Run>,
    mut settings: ResMut<Settings>,
    mut exit: MessageWriter<AppExit>,
) {
    run.frame += 1;
    let f = run.frame % FRAMES_PER_STEP;
    if f == 1 {
        let Some(step) = STEPS.get(run.step) else {
            // Wait for the last picture to arrive (at most 4 more steps).
            if run.results.lock().unwrap().len() < STEPS.len() && run.step < STEPS.len() + 4 {
                run.step += 1;
                return;
            }
            report(&run);
            exit.write(if failures(&run).is_empty() {
                AppExit::Success
            } else {
                AppExit::from_code(1)
            });
            return;
        };
        let mut s = Settings::default();
        (step.change)(&mut s);
        *settings = s;
        println!("step: {}", step.name);
    }
    if f == 0 {
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
    let px = |x: usize, y: usize| -> [f32; 3] {
        let p = &data[(y * w + x) * 4..];
        let (r, g, b) = if bgra {
            (p[2], p[1], p[0])
        } else {
            (p[0], p[1], p[2])
        };
        [r as f32, g as f32, b as f32]
    };
    let mean = |x0: usize, y0: usize, x1: usize, y1: usize| -> f32 {
        let mut sum = 0.0;
        for y in y0..y1 {
            for x in x0..x1 {
                let p = px(x, y);
                sum += (p[0] + p[1] + p[2]) / 3.0;
            }
        }
        sum / ((x1 - x0) * (y1 - y0)) as f32
    };
    let middle = mean(w / 4, h / 2, 3 * w / 4, h * 9 / 10);
    let sky = mean(0, 0, w / 8, h / 8);
    let mut red = 0;
    for y in h - 60..h - 4 {
        for x in w - 60..w - 4 {
            let p = px(x, y);
            if p[0] > 200.0 && p[1] < 60.0 && p[2] < 60.0 {
                red += 1;
            }
        }
    }
    let red_share = red as f32 / (56.0 * 56.0);
    let name = STEPS[run.results.lock().unwrap().len().min(STEPS.len() - 1)].name;
    println!(
        "  {name}: middle {middle:.1}, sky {sky:.1}, display box {:.0}%",
        red_share * 100.0
    );
    run.results
        .lock()
        .unwrap()
        .push((name, middle, sky, red_share));
}

fn failures(run: &Run) -> Vec<String> {
    let r = run.results.lock().unwrap();
    let mut out = vec![];
    if r.len() != STEPS.len() {
        out.push(format!("{} pictures of {}", r.len(), STEPS.len()));
        return out;
    }
    let get = |n: &str| r.iter().find(|x| x.0 == n).unwrap();
    let base = get("defaults");
    for x in r.iter().skip(1) {
        if x.3 < 0.95 {
            out.push(format!(
                "{}: the display box is missing ({:.0}%)",
                x.0,
                x.3 * 100.0
            ));
        }
        if x.2 < 5.0 {
            out.push(format!("{}: the sky is missing ({:.1})", x.0, x.2));
        }
        if x.1 < 5.0 {
            out.push(format!("{}: the level is missing ({:.1})", x.0, x.1));
        }
    }
    if get("brightness 1").1 <= base.1 + 3.0 {
        out.push("brightness 1 isn't brighter".into());
    }
    if get("brightness 0").1 >= base.1 - 3.0 {
        out.push("brightness 0 isn't darker".into());
    }
    // With bloom on, brightness must still apply once, not once per
    // camera. (The sky here is the background colour, which only the bloom
    // path brightens; in the game the sky is drawn, so it is brightened
    // either way.)
    let (b1, bb1) = (get("brightness 1"), get("bloom + brightness 1"));
    if (bb1.1 - b1.1).abs() > 6.0 {
        out.push(format!(
            "with bloom the level's brightness differs ({:.1} against {:.1})",
            bb1.1, b1.1
        ));
    }
    let back = get("back to defaults");
    if (back.1 - base.1).abs() > 3.0 || (back.2 - base.2).abs() > 1.0 {
        out.push("going back to the defaults doesn't give the same picture".into());
    }
    out
}

fn report(run: &Run) {
    let f = failures(run);
    for x in &f {
        println!("FAIL: {x}");
    }
    let checks = 3 * (STEPS.len() - 1) + 4;
    println!(
        "GRAPHICS CHECK: {} passed, {} failed",
        checks - f.len().min(checks),
        f.len()
    );
}
