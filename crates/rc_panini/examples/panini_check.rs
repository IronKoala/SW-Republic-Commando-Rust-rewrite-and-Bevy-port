//! panini_check: draws a ring of equal white cubes around the camera on a
//! 32:9 window, first with the plain view and then with the Panini view at
//! strength 0.2, takes a screenshot of each, and measures each cube's width
//! in pixels along the middle row. Passes (exit code 0) if the middle cube
//! keeps its size and the outer cubes are much less stretched.
//!
//! Run: cargo run -p rc_panini --example panini_check
//! (In the cloud sandbox: under `xvfb-run -a -s "-screen 0 2560x720x24"`.)
//! Writes logs/panini_plain.png and logs/panini_bent.png.

use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::WindowResolution;
use rc_panini::{Fit, PaniniPlugin, PaniniView};

/// 69 degrees up and down, the game's default.
const V: f32 = 1.2043;
/// The strongest bend that still keeps the middle's size on 32:9 (above
/// about 0.22 the drawn view would pass its widest and the middle grows).
const STRENGTH: f32 = 0.2;

#[derive(Resource, Default)]
struct Run {
    frame: u32,
    waiting: bool,
    plain: Option<Vec<u32>>,
    bent: Option<Vec<u32>>,
}

fn main() -> AppExit {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                resolution: WindowResolution::new(2560, 720),
                title: "panini_check".into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(PaniniPlugin)
        .insert_resource(ClearColor(Color::srgb(0.0, 0.0, 0.2)))
        .init_resource::<Run>()
        .add_systems(Startup, setup)
        .add_systems(Update, step)
        .run()
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: V,
            ..default()
        }),
        Transform::default(),
    ));
    let white = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        unlit: true,
        ..default()
    });
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    for deg in [-62.0f32, -50.0, -30.0, 0.0, 30.0, 50.0, 62.0] {
        let a = deg.to_radians();
        let pos = Vec3::new(a.sin(), 0.0, -a.cos()) * 10.0;
        commands.spawn((
            Mesh3d(cube.clone()),
            MeshMaterial3d(white.clone()),
            Transform::from_translation(pos).looking_at(Vec3::ZERO, Vec3::Y),
        ));
    }
}

fn step(
    mut commands: Commands,
    mut run: ResMut<Run>,
    mut cam: Query<(Entity, &mut Projection), With<Camera3d>>,
    mut exit: MessageWriter<AppExit>,
) {
    run.frame += 1;
    if run.waiting || run.frame < 30 {
        return;
    }
    if run.plain.is_some() && run.bent.is_none() && run.frame == 30 {
        // Switch to the Panini view and wait for it to draw.
        let (e, mut p) = cam.single_mut().unwrap();
        let fit = Fit::new(V, 2560.0 / 720.0, STRENGTH);
        if let Projection::Perspective(p) = &mut *p {
            p.fov = fit.source_vertical_fov;
        }
        commands.entity(e).insert(fit.view);
        println!(
            "panini_check: Panini {STRENGTH}: drawn {:.1} deg up and down, {:.1} across",
            fit.source_vertical_fov.to_degrees(),
            fit.seen_across_degrees
        );
        return;
    }
    if let (Some(plain), Some(bent)) = (&run.plain, &run.bent) {
        let ok = judge(plain, bent);
        exit.write(if ok {
            AppExit::Success
        } else {
            AppExit::from_code(1)
        });
        return;
    }
    run.waiting = true;
    commands.spawn(Screenshot::primary_window()).observe(shot);
}

fn shot(captured: On<ScreenshotCaptured>, mut run: ResMut<Run>, has: Query<(), With<PaniniView>>) {
    run.waiting = false;
    let img = &captured.image;
    let (w, h) = (img.width() as usize, img.height() as usize);
    let bgra = matches!(
        img.texture_descriptor.format,
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
    );
    let Some(data) = img.data.as_ref() else {
        return;
    };
    let rgb = |i: usize| {
        let p = &data[i * 4..i * 4 + 4];
        if bgra {
            [p[2], p[1], p[0]]
        } else {
            [p[0], p[1], p[2]]
        }
    };
    // The cubes' widths along the middle row (white runs).
    let row = h / 2;
    let mut runs = Vec::new();
    let mut len = 0u32;
    let mut lit = 0;
    for x in 0..w {
        let c = rgb(row * w + x);
        if c[2] > 20 {
            lit += 1;
        }
        if c[0] > 180 && c[1] > 180 {
            len += 1;
        } else if len > 0 {
            runs.push(len);
            len = 0;
        }
    }
    if len > 0 {
        runs.push(len);
    }
    if lit == 0 {
        println!("panini_check: screenshot all black, trying again");
        run.frame = 20;
        return;
    }
    let bent = !has.is_empty();
    let name = if bent { "bent" } else { "plain" };
    println!("panini_check: {name}: cube widths along the middle row {runs:?}");
    let mut out = Vec::with_capacity(w * h * 3);
    for i in 0..w * h {
        out.extend(rgb(i));
    }
    let _ = std::fs::create_dir_all("logs");
    if let Some(png) = image::RgbImage::from_raw(w as u32, h as u32, out) {
        let _ = png.save(format!("logs/panini_{name}.png"));
    }
    if bent {
        run.bent = Some(runs);
    } else {
        run.plain = Some(runs);
    }
    run.frame = 0;
}

fn judge(plain: &[u32], bent: &[u32]) -> bool {
    if plain.len() != 7 || bent.len() != 7 {
        println!(
            "panini_check: FAIL: expected 7 cubes, saw {} and {}",
            plain.len(),
            bent.len()
        );
        return false;
    }
    let stretch = |r: &[u32]| r[0] as f32 / r[3] as f32;
    let middle = bent[3] as f32 / plain[3] as f32;
    println!(
        "panini_check: middle cube {} -> {} px ({:.0}%); outer cube is {:.2}x the middle plain, {:.2}x with Panini",
        plain[3],
        bent[3],
        middle * 100.0,
        stretch(plain),
        stretch(bent)
    );
    let ok = (middle - 1.0).abs() < 0.08 && stretch(bent) < stretch(plain) * 0.75;
    println!("panini_check: {}", if ok { "PASS" } else { "FAIL" });
    ok
}
