//! Checks character shadows without the game: a stand-in "character" made
//! of two overlapping boxes stands on a white floor, seen from straight
//! above, and the floor's brightness is measured where the shadow should
//! fall.
//!
//! Run (Linux, no screen needed):
//! `xvfb-run -a -s "-screen 0 1280x1024x24" cargo run -p rc_shadows --example shadow_check`
//! It ends `SHADOW CHECK: N passed, 0 failed`. Pictures are saved as
//! `logs/shadow_check_<step>.png`.
//!
//! What is checked:
//! - the shadow falls where the original's light (from Unreal (1,1,3))
//!   throws it, and nowhere on the other side;
//! - it is as dark as the data file says (about 0.75 for a body this tall);
//! - where the two boxes' shadows overlap it is no darker than where only
//!   one falls (each floor spot is darkened once);
//! - switching shadows off, and a camera beyond the original's 1200-unit
//!   cull distance, both remove it.

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use rc_game_core::units::UU_PER_UNIT;
use rc_game_core::{GameLog, PlayerCamera};
use rc_shadows::{CastsShadow, CharacterShadows, ShadowFile, ShadowReadout, ShadowsPlugin};
use std::sync::{Arc, Mutex};

const FRAMES_PER_STEP: u32 = 20;
const W: u32 = 640;
const H: u32 = 360;
/// Bevy units the picture shows from top to bottom.
const VIEW_HEIGHT: f32 = 4.0;
/// The camera's view, top to bottom: narrow, so the camera is high up
/// (about 300 Unreal units) and the boxes' tops don't look much bigger
/// than their feet.
const FOV: f32 = 0.27;

/// How high to put the camera to see `height` units of floor top to bottom.
fn camera_height(height: f32) -> f32 {
    height / 2.0 / (FOV / 2.0).tan()
}

/// A trooper: 156 Unreal units tall, its middle 78 above the feet.
const TALL: f32 = 156.0 / UU_PER_UNIT;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Step {
    WarmUp,
    On,
    Off,
    Far,
}

const STEPS: &[Step] = &[Step::WarmUp, Step::On, Step::Off, Step::Far];

/// Per picture: plain floor, only-one-box shadow, both-boxes shadow, the
/// spot on the light's side (no shadow), all 0..1 linear.
#[derive(Debug, Clone, Copy, Default)]
struct Measured {
    floor: f32,
    one: f32,
    both: f32,
    lit_side: f32,
    drawn: usize,
}

#[derive(Resource, Default)]
struct Run {
    frame: u32,
    step: usize,
    /// The step, characters drawn, camera height.
    pending: Option<(Step, usize, f32)>,
    results: Arc<Mutex<Vec<(Step, Measured)>>>,
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                resolution: (W, H).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(GameLog::terminal())
        .init_resource::<Run>()
        .add_plugins(ShadowsPlugin)
        .add_systems(Startup, scene)
        .add_systems(Update, drive)
        .add_observer(measure)
        .run();
}

fn scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Straight down, +x to the right and +z down the picture.
    commands.spawn((
        PlayerCamera,
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        // A normal (perspective) view as the game's, high enough that the
        // floor fills VIEW_HEIGHT units from top to bottom.
        Projection::Perspective(PerspectiveProjection {
            fov: FOV,
            ..default()
        }),
        Tonemapping::None,
        Transform::from_xyz(0.0, camera_height(VIEW_HEIGHT), 0.0)
            .looking_at(Vec3::ZERO, Vec3::NEG_Z),
    ));
    let white = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        unlit: true,
        ..default()
    });
    let body = materials.add(StandardMaterial {
        base_color: Color::srgb(0.2, 0.3, 0.8),
        unlit: true,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(30.0, 30.0))),
        MeshMaterial3d(white),
    ));
    // The character: its top entity at its feet, the meshes below it, as
    // the cast's are. Box A is the full height; box B, shorter and moved
    // to +x, overlaps it.
    commands
        .spawn((
            CastsShadow { middle: TALL / 2.0 },
            Transform::IDENTITY,
            Visibility::default(),
        ))
        .with_children(|c| {
            c.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.5, TALL, 0.5))),
                MeshMaterial3d(body.clone()),
                Transform::from_xyz(0.0, TALL / 2.0, 0.0),
            ));
            c.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.5, 1.0, 0.5))),
                MeshMaterial3d(body),
                Transform::from_xyz(0.2, 0.5, 0.0),
            ));
        });
}

fn drive(
    mut run: ResMut<Run>,
    mut commands: Commands,
    mut switch: ResMut<CharacterShadows>,
    mut camera: Query<&mut Transform, With<PlayerCamera>>,
    readout: Res<ShadowReadout>,
    eye: Query<&GlobalTransform, With<PlayerCamera>>,
    mut exit: MessageWriter<AppExit>,
) {
    run.frame += 1;
    let f = run.frame % FRAMES_PER_STEP;
    if f == FRAMES_PER_STEP / 2 {
        let Some(&step) = STEPS.get(run.step) else {
            exit.write(if report(&run) {
                AppExit::Success
            } else {
                AppExit::from_code(1)
            });
            return;
        };
        match step {
            Step::WarmUp | Step::On => switch.on = true,
            Step::Off => switch.on = false,
            Step::Far => {
                switch.on = true;
                // 30 Bevy units = 1500 Unreal units away, beyond the
                // original's 1200.
                if let Ok(mut t) = camera.single_mut() {
                    t.translation.y = 30.0;
                }
            }
        }
        println!("step: {step:?}");
    }
    if f == 0 && run.step < STEPS.len() {
        let height = eye.iter().next().map(|t| t.translation().y).unwrap_or(1.0);
        run.pending = Some((STEPS[run.step], readout.drawn, height));
        commands.spawn(Screenshot::primary_window());
        run.step += 1;
    }
}

fn to_linear(c: u8) -> f32 {
    let s = c as f32 / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
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
    let rgb = |x: usize, y: usize| -> [u8; 3] {
        let p = &data[(y.min(h - 1) * w + x.min(w - 1)) * 4..];
        if bgra {
            [p[2], p[1], p[0]]
        } else {
            [p[0], p[1], p[2]]
        }
    };
    let (step, drawn, height) = run.pending.unwrap_or((Step::WarmUp, 0, 1.0));
    // World (x, z) on the floor to the picture, averaged over 5x5 pixels,
    // as linear brightness.
    let unit = h as f32 / 2.0 / ((FOV / 2.0).tan() * height);
    let at = |x: f32, z: f32| -> f32 {
        let (px, py) = (w as f32 / 2.0 + x * unit, h as f32 / 2.0 + z * unit);
        let mut sum = 0.0;
        for dy in -2..=2 {
            for dx in -2..=2 {
                let c = rgb((px as i32 + dx) as usize, (py as i32 + dy) as usize);
                sum += (to_linear(c[0]) + to_linear(c[1]) + to_linear(c[2])) / 3.0;
            }
        }
        sum / 25.0
    };
    // The light is towards +x +z (from Unreal (1,1,3)), so the shadow falls
    // towards -x -z, reaching a third of the height away.
    let m = Measured {
        floor: at(1.5, 1.2),
        one: at(-0.4, -0.4),
        both: at(0.0, -0.4),
        lit_side: at(0.45, 0.45),
        drawn,
    };
    println!(
        "  {step:?}: floor {:.3}, one box's shadow {:.3}, both boxes' {:.3}, light side {:.3}, characters drawn {drawn}",
        m.floor, m.one, m.both, m.lit_side
    );
    let _ = std::fs::create_dir_all("logs");
    let mut pixels = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let c = rgb(x, y);
            pixels.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    let img = rc_image::Rgba {
        width: w,
        height: h,
        pixels,
    };
    let file = format!("logs/shadow_check_{step:?}.png").to_lowercase();
    let _ = std::fs::write(file, rc_image::png::encode(&img));
    run.results.lock().unwrap().push((step, m));
}

/// Prints every check; true when all pass.
fn report(run: &Run) -> bool {
    let results = run.results.lock().unwrap().clone();
    let get = |s: Step| results.iter().find(|(r, _)| *r == s).map(|(_, m)| *m);
    let want = ShadowFile::default().strength(TALL / 2.0);
    let mut checks: Vec<(String, bool)> = Vec::new();
    match get(Step::On) {
        Some(m) => {
            checks.push((format!("floor is white ({:.3})", m.floor), m.floor > 0.9));
            let dark = 1.0 - m.one / m.floor.max(1e-3);
            checks.push((
                format!("shadow darkens the floor by {dark:.2} (data file: {want:.2})"),
                (dark - want).abs() < 0.06,
            ));
            checks.push((
                format!(
                    "where both boxes' shadows overlap it is no darker ({:.3} vs {:.3})",
                    m.both, m.one
                ),
                (m.both - m.one).abs() < 0.02,
            ));
            checks.push((
                format!("no shadow on the light's side ({:.3})", m.lit_side),
                (m.lit_side - m.floor).abs() < 0.02,
            ));
            checks.push((format!("1 character drawn ({})", m.drawn), m.drawn == 1));
        }
        None => checks.push(("picture with shadows on".into(), false)),
    }
    match get(Step::Off) {
        Some(m) => checks.push((
            format!(
                "switched off: no shadow ({:.3} vs floor {:.3})",
                m.one, m.floor
            ),
            (m.one - m.floor).abs() < 0.02,
        )),
        None => checks.push(("picture with shadows off".into(), false)),
    }
    match get(Step::Far) {
        Some(m) => checks.push((
            format!(
                "camera 1500 uu away: not drawn ({:.3} vs floor {:.3}, drawn {})",
                m.one, m.floor, m.drawn
            ),
            (m.one - m.floor).abs() < 0.02 && m.drawn == 0,
        )),
        None => checks.push(("picture from far away".into(), false)),
    }
    let mut failed = 0;
    for (what, ok) in &checks {
        println!("{} {what}", if *ok { "PASS" } else { "FAIL" });
        if !ok {
            failed += 1;
        }
    }
    println!(
        "SHADOW CHECK: {} passed, {failed} failed",
        checks.len() - failed
    );
    failed == 0
}
