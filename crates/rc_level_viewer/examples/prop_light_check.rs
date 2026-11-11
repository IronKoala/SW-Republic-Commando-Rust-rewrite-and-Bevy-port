//! Checks that real-time lights reach props, without the game's files.
//!
//! Three "props" (flat squares carrying baked corner colours, as the map's
//! props do) are drawn side by side through `draw::spawn_level`:
//! - left: the normal settings (`live_light_on_props = 1`);
//! - middle: the same, but with a (flat) bump map and a stored light
//!   direction, so it is drawn by the bump-map shader (`bump_lit.wesl`);
//! - right: `live_light_on_props = 0`, which draws exactly the old way
//!   (baked colours only), so it is the yardstick.
//!
//! A real-time ("bDynamicLight") point light from the left one's map shines
//! on all three. The steps switch it off, on, move it away and back, and
//! measure the colour in the middle of each square:
//! - light off: left = right (the baked light looks the same as before);
//! - light on: left and middle clearly brighter, right unchanged;
//! - light moved far away: left and middle back to their unlit colour;
//! - light moved next to the right square: the left one gets less.
//!
//! Run (Linux, no screen needed):
//! `xvfb-run -a -s "-screen 0 1280x1024x24" cargo run -p rc_level_viewer --example prop_light_check`
//! It ends `PROP LIGHT CHECK: N passed, 0 failed`. Pictures are saved as
//! `logs/prop_light_check_<step>.png`.

use bevy::camera::Exposure;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::render::view::Tonemapping;
use rc_image::Rgba;
use rc_level_viewer::bump::{BumpLitMaterial, BumpMapPlugin};
use rc_level_viewer::draw;
use rc_level_viewer::lighting::LightingSettings;
use rc_level_viewer::materials::{Bump, Picture};
use rc_level_viewer::prop_light::{PropLightPlugin, PropMaterial};
use rc_level_viewer::scene::{Batch, Scene, Spawn, Stats};
use rc_package::light::{Light, LightKind};
use std::sync::{Arc, Mutex};

const FRAMES_PER_STEP: u32 = 30;
const W: u32 = 640;
const H: u32 = 360;
/// Where the squares' middles are, in Bevy units along x.
const LEFT_X: f32 = -1.8;
const MIDDLE_X: f32 = 0.0;
const RIGHT_X: f32 = 1.8;
/// Half a square's width.
const HALF: f32 = 0.75;
/// The baked corner colour (0-255, the game's brightness curve).
const BAKED: [u8; 4] = [70, 60, 50, 255];

#[derive(Clone, Copy, Debug)]
enum LampAt {
    Off,
    /// In front of the middle, between both squares.
    Middle,
    Far,
    /// In front of the right square.
    ByRight,
}

const STEPS: &[(&str, LampAt)] = &[
    // The first pictures under Xvfb are often black while the shaders are
    // built; they aren't checked.
    ("warm-up", LampAt::Off),
    ("warm-up 2", LampAt::Off),
    ("off", LampAt::Off),
    ("on", LampAt::Middle),
    ("far away", LampAt::Far),
    ("by the right square", LampAt::ByRight),
    ("on again", LampAt::Middle),
];

#[derive(Debug, Clone, Copy, Default)]
struct Measured {
    left: f32,
    middle: f32,
    right: f32,
}

#[derive(Resource, Default)]
struct Run {
    frame: u32,
    step: usize,
    results: Arc<Mutex<Vec<(usize, Measured)>>>,
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

/// One flat square facing the camera, centred at `x`, with baked colours.
fn square(x: f32) -> Batch {
    let (a, b) = (x - HALF, x + HALF);
    Batch {
        two_sided: true,
        from_mesh: true,
        positions: vec![[a, -1.0, 0.0], [b, -1.0, 0.0], [b, 1.0, 0.0], [a, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        uvs: vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
        indices: vec![0, 1, 2, 0, 2, 3],
        colours: vec![BAKED; 4],
        ..default()
    }
}

/// A square drawn by the bump-map shader: a grey picture, a flat bump map
/// (so only the real-time light can change it) and a stored light coming
/// straight at it.
fn bump_square(x: f32) -> Batch {
    let solid = |c: [u8; 4]| Rgba {
        width: 4,
        height: 4,
        pixels: c.repeat(16),
    };
    let mut b = square(x);
    b.two_sided = false;
    b.picture = Some(Picture {
        image: solid([200, 200, 200, 255]),
        has_alpha: false,
        alpha_pixels: false,
        additive: false,
        blend: false,
        hologram: false,
        bump: Some(Arc::new(Bump {
            image: solid([128, 128, 255, 255]),
            uv_scale: 1.0,
            two_byte_format: false,
            name: "flat test bump".into(),
        })),
        uv_channel: 0,
        multiply: None,
    });
    b.bump_lit = true;
    b.light_dirs = vec![[0.0, 0.0, 1.0]; 4];
    b.light_colours = vec![[0.1, 0.1, 0.1]; 4];
    b
}

fn scene(batches: Vec<Batch>, lights: Vec<Light>) -> Scene {
    Scene {
        batches,
        lightmap_pages: vec![],
        lights,
        stats: Stats::default(),
        spawn: Spawn {
            position: [0.0; 3],
            yaw: 0,
            from: "test".into(),
        },
        sky: None,
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
    // A real-time light as the map gives it: Unreal units, 50 per Bevy
    // unit, Unreal (x, y, z) = Bevy (x, z, y). This is 2 units in front.
    let lamp = Light {
        kind: LightKind::Point,
        location: [0.0, 100.0, 0.0],
        rotation: [0; 3],
        brightness: 128.0,
        hue: 0,
        saturation: 255,
        radius: 400.0,
        radius_inner: None,
        cone: 128,
        light_type: 1,
        dynamic: true,
    };
    let on = LightingSettings::default();
    let off = LightingSettings {
        live_light_on_props: 0.0,
        ..default()
    };
    let l = draw::spawn_level(
        &mut commands,
        &mut meshes,
        &mut images,
        &mut materials,
        Some(&mut bumps),
        &mut props,
        &scene(vec![square(LEFT_X), bump_square(MIDDLE_X)], vec![lamp]),
        &on,
    );
    let r = draw::spawn_level(
        &mut commands,
        &mut meshes,
        &mut images,
        &mut materials,
        None,
        &mut props,
        &scene(vec![square(RIGHT_X)], vec![]),
        &off,
    );
    println!(
        "left and middle: {} coloured batches ({} bump-lit), {} lit by real-time lights, {} point lights; right: {} coloured, {} lit by real-time lights",
        l.coloured, l.bump_lit, l.coloured_live, l.lights.point, r.coloured, r.coloured_live
    );
}

fn drive(
    mut commands: Commands,
    mut run: ResMut<Run>,
    mut lamps: Query<(&mut Transform, &mut Visibility), With<PointLight>>,
    mut exit: MessageWriter<AppExit>,
) {
    run.frame += 1;
    let f = run.frame % FRAMES_PER_STEP;
    if f == 1 {
        let Some(&(name, at)) = STEPS.get(run.step) else {
            // Wait a few steps for the last pictures to arrive.
            if run.results.lock().unwrap().len() < STEPS.len() && run.step < STEPS.len() + 4 {
                run.step += 1;
                return;
            }
            let failed = report(&run);
            exit.write(if failed == 0 {
                AppExit::Success
            } else {
                AppExit::from_code(1)
            });
            return;
        };
        for (mut t, mut v) in &mut lamps {
            *v = if matches!(at, LampAt::Off) {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
            t.translation = match at {
                LampAt::Off | LampAt::Middle => Vec3::new(0.0, 0.0, 2.0),
                LampAt::Far => Vec3::new(0.0, 0.0, 60.0),
                LampAt::ByRight => Vec3::new(RIGHT_X + 1.2, 0.0, 1.0),
            };
        }
        println!("step: {name} ({at:?})");
    }
    if f == 0 && run.step < STEPS.len() {
        // Each picture remembers which step it belongs to: they can
        // arrive a few frames late.
        let (step, results) = (run.step, run.results.clone());
        commands
            .spawn(Screenshot::primary_window())
            .observe(move |c: On<ScreenshotCaptured>| measure(&c, step, &results));
        run.step += 1;
    }
}

fn measure(captured: &ScreenshotCaptured, step: usize, results: &Mutex<Vec<(usize, Measured)>>) {
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
    // 1 Bevy unit at the squares' distance (5) in pixels, for Bevy's
    // default 45 degree view.
    let unit = h as f32 / 2.0 / (22.5f32.to_radians().tan() * 5.0);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let average = |x: f32| {
        let mut sum = 0.0;
        for dy in -4..=4 {
            for dx in -4..=4 {
                let p = px(cx + x * unit + dx as f32, cy + dy as f32);
                sum += (p[0] + p[1] + p[2]) / 3.0;
            }
        }
        sum / 81.0
    };
    let m = Measured {
        left: average(LEFT_X),
        middle: average(MIDDLE_X),
        right: average(RIGHT_X),
    };
    let name = STEPS.get(step).map(|s| s.0).unwrap_or("?");
    println!(
        "  {name}: left {:.1}, middle (bump) {:.1}, right {:.1}",
        m.left, m.middle, m.right
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
        "logs/prop_light_check_{}.png",
        name.replace([' ', '-'], "_")
    );
    let _ = std::fs::write(file, rc_image::png::encode(&img));
    results.lock().unwrap().push((step, m));
}

/// Prints every check and returns how many failed.
fn report(run: &Run) -> usize {
    let r = run.results.lock().unwrap();
    let get = |n: &str| {
        let i = STEPS.iter().position(|s| s.0 == n)?;
        r.iter().find(|x| x.0 == i).map(|x| x.1)
    };
    let mut checks: Vec<(String, bool)> = vec![];
    let mut check = |what: String, ok: bool| checks.push((what, ok));
    match (
        get("off"),
        get("on"),
        get("far away"),
        get("by the right square"),
        get("on again"),
    ) {
        (Some(off), Some(on), Some(far), Some(right), Some(again)) => {
            check(
                format!("props still show their baked light (off: left {:.1}, right {:.1}, both above 5)", off.left, off.right),
                off.left > 5.0 && off.right > 5.0,
            );
            check(
                format!(
                    "light off: left looks like the old way (within 2 of right: {:.1} vs {:.1})",
                    off.left, off.right
                ),
                (off.left - off.right).abs() <= 2.0,
            );
            check(
                format!(
                    "light on: left brighter by 20+ ({:.1} -> {:.1})",
                    off.left, on.left
                ),
                on.left - off.left >= 20.0,
            );
            check(
                format!(
                    "light on: bump-mapped middle brighter by 20+ ({:.1} -> {:.1})",
                    off.middle, on.middle
                ),
                on.middle - off.middle >= 20.0,
            );
            check(
                format!(
                    "light far away: middle back to its unlit colour ({:.1} vs {:.1})",
                    far.middle, off.middle
                ),
                (far.middle - off.middle).abs() <= 2.0 && off.middle > 5.0,
            );
            check(
                format!(
                    "light on: right (live light off) unchanged ({:.1} -> {:.1})",
                    off.right, on.right
                ),
                (on.right - off.right).abs() <= 2.0,
            );
            check(
                format!(
                    "light far away: left back to baked ({:.1} vs {:.1})",
                    far.left, off.left
                ),
                (far.left - off.left).abs() <= 2.0,
            );
            check(
                format!(
                    "light moved right: left gets less ({:.1} < {:.1})",
                    right.left, on.left
                ),
                right.left + 10.0 < on.left,
            );
            check(
                format!(
                    "light back: left as bright as before ({:.1} vs {:.1})",
                    again.left, on.left
                ),
                (again.left - on.left).abs() <= 2.0,
            );
        }
        _ => check(
            format!("only {} of {} pictures arrived", r.len(), STEPS.len()),
            false,
        ),
    }
    let failed = checks.iter().filter(|c| !c.1).count();
    for (what, ok) in &checks {
        println!("{} {what}", if *ok { "PASS" } else { "FAIL" });
    }
    println!(
        "PROP LIGHT CHECK: {} passed, {failed} failed",
        checks.len() - failed
    );
    failed
}
