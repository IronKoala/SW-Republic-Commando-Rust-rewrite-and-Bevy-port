//! Checks that holograms shimmer and flicker (rc_hologram), measured from
//! the picture on screen.
//!
//! One hologram ball (a level batch with an `HsHologram` picture, drawn
//! through `draw::spawn_level` like a map's holograms) sits in front of the
//! camera on black. Pictures are taken over a few seconds and the checks
//! are:
//! - the hologram got the shimmering material (not the steady glow);
//! - it shows, in the hologram's light blue;
//! - its edges, seen side-on, are clearly darker than its middle (the
//!   steady glow was the same everywhere);
//! - its brightness changes from picture to picture (the flicker);
//! - with the game's files: the speckle pattern moves between pictures
//!   (the jumping noise). Set RC_GAME_DIR to the game folder for this one.
//!
//! Run (Linux, no screen needed):
//! `xvfb-run -a -s "-screen 0 1280x1024x24" cargo run -p rc_level_viewer --example hologram_check`
//! It ends `HOLOGRAM CHECK: N passed, 0 failed`. Pictures are saved as
//! `logs/hologram_check_<n>.png`.

use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::render::view::Tonemapping;
use rc_hologram::{HologramGameData, HologramPlugin, HologramReport};
use rc_image::Rgba;
use rc_level_viewer::bump::{BumpLitMaterial, BumpMapPlugin};
use rc_level_viewer::draw;
use rc_level_viewer::lighting::LightingSettings;
use rc_level_viewer::materials::{Picture, HOLOGRAM_COLOUR};
use rc_level_viewer::prop_light::{PropLightPlugin, PropMaterial};
use rc_level_viewer::scene::{Batch, Scene, Spawn, Stats};
use std::sync::{Arc, Mutex};

const W: u32 = 640;
const H: u32 = 360;
const RADIUS: f32 = 1.2;
const DISTANCE: f32 = 5.0;
/// Frames before the first picture (shaders are built meanwhile).
const WARM_UP: u32 = 90;
const EVERY: u32 = 4;
const SHOTS: usize = 40;

#[derive(Debug, Clone)]
struct Shot {
    middle: f32,
    edge: f32,
    blue_over_red: f32,
    /// The middle 32 x 32 pixels' brightness, each divided by their
    /// average (the speckle pattern, without the overall flicker).
    pattern: Vec<f32>,
}

#[derive(Resource, Default)]
struct Run {
    frame: u32,
    taken: usize,
    swapped: usize,
    game_pictures: bool,
    shots: Arc<Mutex<Vec<(usize, Shot)>>>,
}

fn main() -> AppExit {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            resolution: (W, H).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins((PropLightPlugin, BumpMapPlugin, HologramPlugin))
    .init_resource::<Run>()
    .add_systems(Startup, setup)
    .add_systems(Update, drive);
    if let Some(dir) = std::env::var_os("RC_GAME_DIR") {
        app.insert_resource(HologramGameData(
            std::path::PathBuf::from(dir).join("GameData"),
        ));
    }
    app.run()
}

/// A ball, as a level batch with a hologram picture (plain white turned
/// into the default hologram colour, like `Holo_Blank`).
fn ball() -> Batch {
    let mesh = Sphere::new(RADIUS).mesh().uv(48, 32);
    let positions: Vec<[f32; 3]> = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(bevy::mesh::VertexAttributeValues::Float32x3(v)) => v.clone(),
        _ => vec![],
    };
    let normals: Vec<[f32; 3]> = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(bevy::mesh::VertexAttributeValues::Float32x3(v)) => v.clone(),
        _ => vec![],
    };
    let uvs: Vec<[f32; 2]> = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(bevy::mesh::VertexAttributeValues::Float32x2(v)) => v.clone(),
        _ => vec![],
    };
    let indices: Vec<u32> = mesh
        .indices()
        .map(|i| i.iter().map(|x| x as u32).collect())
        .unwrap_or_default();
    let white = Rgba {
        width: 4,
        height: 4,
        pixels: vec![255; 64],
    };
    let c = HOLOGRAM_COLOUR;
    Batch {
        positions,
        normals,
        uvs,
        indices,
        from_mesh: true,
        picture: Some(Picture {
            image: rc_image::hologram(&white, [c[0], c[1], c[2]]),
            has_alpha: false,
            alpha_pixels: false,
            additive: true,
            blend: false,
            hologram: true,
            bump: None,
            uv_channel: 0,
            multiply: None,
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
        Transform::from_xyz(0.0, 0.0, DISTANCE).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let scene = Scene {
        batches: vec![ball()],
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
        attached: Default::default(),
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
        "hologram ball: {} triangles, {} full-bright batches",
        r.triangles, r.full_bright
    );
}

fn drive(
    mut commands: Commands,
    mut run: ResMut<Run>,
    report: Res<HologramReport>,
    mut exit: MessageWriter<AppExit>,
) {
    run.frame += 1;
    run.swapped = report.drawn;
    run.game_pictures = report.game_pictures;
    if run.frame < WARM_UP || !(run.frame - WARM_UP).is_multiple_of(EVERY) {
        return;
    }
    if run.taken < SHOTS {
        let (n, shots) = (run.taken, run.shots.clone());
        commands
            .spawn(Screenshot::primary_window())
            .observe(move |c: On<ScreenshotCaptured>| measure(&c, n, &shots));
        run.taken += 1;
        return;
    }
    // Give the last pictures time to arrive.
    if run.frame < WARM_UP + EVERY * (SHOTS as u32 + 20) && run.shots.lock().unwrap().len() < SHOTS
    {
        return;
    }
    let failed = report_checks(&run, report.problem.as_deref());
    exit.write(if failed == 0 {
        AppExit::Success
    } else {
        AppExit::from_code(1)
    });
}

fn measure(captured: &ScreenshotCaptured, n: usize, shots: &Mutex<Vec<(usize, Shot)>>) {
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
        let (x, y) = (
            (x.max(0.0) as usize).min(w - 1),
            (y.max(0.0) as usize).min(h - 1),
        );
        let p = &data[(y * w + x) * 4..];
        let (r, g, b) = if bgra {
            (p[2], p[1], p[0])
        } else {
            (p[0], p[1], p[2])
        };
        [r as f32, g as f32, b as f32]
    };
    let grey = |p: [f32; 3]| (p[0] + p[1] + p[2]) / 3.0;
    let unit = h as f32 / 2.0 / (22.5f32.to_radians().tan() * DISTANCE);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    // The ball's outline on screen is a little wider than RADIUS x unit
    // (perspective); 0.9 of the radius is well inside, near the rim.
    let rim = RADIUS * unit * 0.9;
    let mut middle = 0.0;
    let mut sum = [0.0f32; 3];
    for dy in -6..=6 {
        for dx in -6..=6 {
            let p = px(cx + dx as f32, cy + dy as f32);
            middle += grey(p);
            for k in 0..3 {
                sum[k] += p[k];
            }
        }
    }
    middle /= 169.0;
    let mut edge = 0.0;
    for i in 0..24 {
        let a = i as f32 / 24.0 * std::f32::consts::TAU;
        edge += grey(px(cx + a.cos() * rim, cy + a.sin() * rim));
    }
    edge /= 24.0;
    let mut pattern = Vec::with_capacity(32 * 32);
    for dy in -16..16 {
        for dx in -16..16 {
            pattern.push(grey(px(cx + dx as f32, cy + dy as f32)));
        }
    }
    let avg = pattern.iter().sum::<f32>() / pattern.len() as f32;
    for p in &mut pattern {
        *p /= avg.max(1.0);
    }
    let s = Shot {
        middle,
        edge,
        blue_over_red: sum[2] / sum[0].max(1.0),
        pattern,
    };
    println!(
        "  picture {n}: middle {:.1}, rim {:.1}, blue/red {:.2}",
        s.middle, s.edge, s.blue_over_red
    );
    if n.is_multiple_of(10) {
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
        let _ = std::fs::write(
            format!("logs/hologram_check_{n}.png"),
            rc_image::png::encode(&img),
        );
    }
    shots.lock().unwrap().push((n, s));
}

/// Prints every check and returns how many failed.
fn report_checks(run: &Run, problem: Option<&str>) -> usize {
    let mut shots = run.shots.lock().unwrap().clone();
    shots.sort_by_key(|s| s.0);
    // Skip the first few: under Xvfb they can be black.
    let shots: Vec<Shot> = shots.into_iter().skip(4).map(|s| s.1).collect();
    let mut checks: Vec<(String, bool)> = vec![];
    let mut check = |what: String, ok: bool| checks.push((what, ok));
    check(
        format!(
            "the hologram got the shimmering material ({} swapped)",
            run.swapped
        ),
        run.swapped >= 1,
    );
    if shots.len() < 10 {
        check(format!("enough pictures arrived ({})", shots.len()), false);
    } else {
        let middles: Vec<f32> = shots.iter().map(|s| s.middle).collect();
        let top = middles.iter().cloned().fold(0.0, f32::max);
        let low = middles.iter().cloned().fold(f32::MAX, f32::min);
        let mut ratios: Vec<f32> = shots.iter().map(|s| s.edge / s.middle.max(1.0)).collect();
        ratios.sort_by(|a, b| a.total_cmp(b));
        let ratio = ratios[ratios.len() / 2];
        let blue = shots.iter().map(|s| s.blue_over_red).sum::<f32>() / shots.len() as f32;
        check(
            format!("it shows (brightest middle {top:.1}, above 30)"),
            top > 30.0,
        );
        check(
            format!("it is light blue (blue / red {blue:.2}, above 1.3)"),
            blue > 1.3,
        );
        check(
            format!("edges seen side-on are darker (rim / middle {ratio:.2}, below 0.6)"),
            top > 30.0 && ratio < 0.6,
        );
        check(
            format!(
                "brightness flickers between pictures (middle {low:.1} to {top:.1}, spread above 10%)"
            ),
            top > 0.0 && (top - low) / top > 0.10,
        );
        if run.game_pictures {
            // How much the speckle pattern differs between neighbouring
            // pictures, overall flicker taken out.
            let mut moved = 0.0;
            for p in shots.windows(2) {
                moved += p[0]
                    .pattern
                    .iter()
                    .zip(&p[1].pattern)
                    .map(|(a, b)| (a - b).abs())
                    .sum::<f32>()
                    / p[0].pattern.len() as f32;
            }
            moved /= (shots.len() - 1) as f32;
            check(
                format!("the noise speckle moves between pictures (average change {moved:.3}, above 0.01)"),
                moved > 0.01,
            );
        } else {
            println!(
                "  not checked: the noise speckle (game pictures not read: {})",
                problem.unwrap_or("RC_GAME_DIR not set")
            );
        }
    }
    let mut failed = 0;
    for (what, ok) in &checks {
        println!("{} {what}", if *ok { "PASS" } else { "FAIL" });
        if !ok {
            failed += 1;
        }
    }
    println!(
        "HOLOGRAM CHECK: {} passed, {failed} failed",
        checks.len() - failed
    );
    failed
}
