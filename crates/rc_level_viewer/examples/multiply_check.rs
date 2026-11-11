//! Checks multiplying materials (a `Combiner` with `CO_Multiply`, see
//! `materials::Layer`), without the game's files.
//!
//! Two unlit "props" (flat squares with no baked light, like geo_01a's
//! arena tiers, which are `bUnlit`) are drawn side by side through
//! `draw::spawn_level`, both with a plain grey picture (200):
//! - left: the picture alone (the yardstick);
//! - right: the same picture multiplied by a second one laid on the second
//!   texture coordinates. That second picture is dark (64) on its left
//!   half and white on its right half, and every corner's second
//!   coordinates point into the dark half, while the first coordinates
//!   cover the whole picture.
//!
//! So the right square should come out about 200 x 64 / 255 = 50 all over.
//! If the multiply were missing it would show 200; if the second picture
//! were read with the first coordinates, half of it would show 200.
//!
//! Run (Linux, no screen needed):
//! `xvfb-run -a -s "-screen 0 1280x1024x24" cargo run -p rc_level_viewer --example multiply_check`
//! It ends `MULTIPLY CHECK: N passed, 0 failed` and saves
//! `logs/multiply_check.png`.

use bevy::camera::Exposure;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::render::view::Tonemapping;
use rc_image::Rgba;
use rc_level_viewer::bump::{BumpLitMaterial, BumpMapPlugin};
use rc_level_viewer::draw;
use rc_level_viewer::lighting::LightingSettings;
use rc_level_viewer::materials::{Layer, Picture};
use rc_level_viewer::prop_light::{PropLightPlugin, PropMaterial};
use rc_level_viewer::scene::{Batch, Scene, Spawn, Stats};
use std::sync::{Arc, Mutex};

const W: u32 = 640;
const H: u32 = 360;
const LEFT_X: f32 = -1.2;
const RIGHT_X: f32 = 1.2;
const HALF: f32 = 0.9;
/// Pictures taken (the first ones under Xvfb are often black).
const SHOTS: usize = 4;
const FRAMES_PER_SHOT: u32 = 30;

#[derive(Resource, Default)]
struct Run {
    frame: u32,
    shots: usize,
    results: Arc<Mutex<Vec<[f32; 4]>>>,
}

fn main() -> AppExit {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                resolution: (W, H).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((PropLightPlugin, BumpMapPlugin))
        .init_resource::<Run>()
        .add_systems(Startup, setup)
        .add_systems(Update, drive)
        .run()
}

fn grey(v: u8) -> Rgba {
    Rgba {
        width: 4,
        height: 4,
        pixels: [v, v, v, 255].repeat(16),
    }
}

fn square(x: f32, multiply: bool) -> Batch {
    let (a, b) = (x - HALF, x + HALF);
    // Second picture: left half 64, right half white.
    let layer = Rgba {
        width: 2,
        height: 1,
        pixels: vec![64, 64, 64, 255, 255, 255, 255, 255],
    };
    Batch {
        from_mesh: true,
        positions: vec![[a, -1.0, 0.0], [b, -1.0, 0.0], [b, 1.0, 0.0], [a, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        uvs: vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
        layer_uvs: if multiply {
            vec![[0.25, 0.5]; 4]
        } else {
            vec![]
        },
        indices: vec![0, 1, 2, 0, 2, 3],
        picture: Some(Picture {
            image: grey(200),
            has_alpha: false,
            alpha_pixels: false,
            additive: false,
            blend: false,
            hologram: false,
            bump: None,
            uv_channel: 0,
            multiply: multiply.then(|| {
                Arc::new(Layer {
                    image: layer,
                    uv_channel: 1,
                    scale: 1.0,
                    name: "test layer".into(),
                })
            }),
            falloff: None,
        }),
        ..default()
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut props: ResMut<Assets<PropMaterial>>,
    mut bumps: ResMut<Assets<BumpLitMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        Tonemapping::None,
        Exposure::default(),
        Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let scene = Scene {
        batches: vec![square(LEFT_X, false), square(RIGHT_X, true)],
        lightmap_pages: vec![],
        lights: vec![],
        stats: Stats::default(),
        spawn: Spawn {
            position: [0.0; 3],
            yaw: 0,
            from: "test".into(),
        },
        sky: None,
        fog: Default::default(),
        fog_error: None,
    };
    let r = draw::spawn_level(
        &mut commands,
        &mut meshes,
        &mut images,
        &mut materials,
        Some(&mut bumps),
        &mut props,
        &scene,
        &LightingSettings::default(),
    );
    println!(
        "{} batches at full brightness, {} of them multiplied",
        r.full_bright, r.multiplied
    );
}

fn drive(mut commands: Commands, mut run: ResMut<Run>, mut exit: MessageWriter<AppExit>) {
    run.frame += 1;
    if run.frame % FRAMES_PER_SHOT != 0 {
        return;
    }
    if run.shots < SHOTS {
        let results = run.results.clone();
        commands
            .spawn(Screenshot::primary_window())
            .observe(move |c: On<ScreenshotCaptured>| measure(&c, &results));
        run.shots += 1;
        return;
    }
    if run.frame < FRAMES_PER_SHOT * (SHOTS as u32 + 3) {
        return;
    }
    let r = run.results.lock().unwrap();
    let Some(m) = r.last() else {
        println!("MULTIPLY CHECK: 0 passed, 1 failed (no picture arrived)");
        exit.write(AppExit::from_code(1));
        return;
    };
    let [left, right_a, right_b, background] = *m;
    let checks = [
        (
            format!("plain square shows its picture ({left:.1}, expected 200 +- 4)"),
            (left - 200.0).abs() <= 4.0,
        ),
        (
            format!("multiplied square, its left part: {right_a:.1}, expected about 50 (200 x 64 / 255) +- 8"),
            (right_a - 50.0).abs() <= 8.0,
        ),
        (
            format!("multiplied square, its right part: {right_b:.1}, the same (the second picture uses the second coordinates)"),
            (right_b - 50.0).abs() <= 8.0,
        ),
        (
            format!("background stays black ({background:.1})"),
            background < 3.0,
        ),
    ];
    let mut failed = 0;
    for (what, ok) in &checks {
        println!("{} {what}", if *ok { "PASS" } else { "FAIL" });
        failed += !ok as usize;
    }
    println!(
        "MULTIPLY CHECK: {} passed, {failed} failed",
        checks.len() - failed
    );
    exit.write(if failed == 0 {
        AppExit::Success
    } else {
        AppExit::from_code(1)
    });
}

fn measure(captured: &ScreenshotCaptured, results: &Mutex<Vec<[f32; 4]>>) {
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
    let unit = h as f32 / 2.0 / (22.5f32.to_radians().tan() * 5.0);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let average = |x: f32, y: f32| {
        let mut sum = 0.0;
        for dy in -3..=3 {
            for dx in -3..=3 {
                let p = px(cx + x * unit + dx as f32, cy - y * unit + dy as f32);
                sum += (p[0] + p[1] + p[2]) / 3.0;
            }
        }
        sum / 49.0
    };
    let m = [
        average(LEFT_X, 0.0),
        average(RIGHT_X - HALF * 0.6, 0.0),
        average(RIGHT_X + HALF * 0.6, 0.0),
        average(0.0, 1.6),
    ];
    println!(
        "  picture: plain {:.1}, multiplied {:.1} / {:.1}, background {:.1}",
        m[0], m[1], m[2], m[3]
    );
    let _ = std::fs::create_dir_all("logs");
    let mut pixels = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let p = px(x as f32, y as f32);
            pixels.extend_from_slice(&[p[0] as u8, p[1] as u8, p[2] as u8, 255]);
        }
    }
    let img = Rgba {
        width: w,
        height: h,
        pixels,
    };
    let _ = std::fs::write("logs/multiply_check.png", rc_image::png::encode(&img));
    results.lock().unwrap().push(m);
}
