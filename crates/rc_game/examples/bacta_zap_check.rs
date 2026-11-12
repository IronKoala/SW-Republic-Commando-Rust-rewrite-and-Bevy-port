//! Checks the bacta dispenser's zap effect (`BactaZap_Dispenser`, from the
//! game's `Properties/cteffects.u`) as the effects plugin draws it, on its
//! own, with pictures.
//!
//! The effect is started at the middle, turned as if hung on a dispenser
//! facing +X (Unreal). The camera looks at it from in front and a little
//! to the side. A grey wall stands between the camera and the right half
//! of the effect. Pictures are taken while it plays; the checks are:
//! - the effect and all its pictures were read;
//! - its parts have particles while the pictures are taken (all but
//!   `FlareA`, a single flash at the start);
//! - the hologram parts (`Holo_*`, `ZTest` off in the game's data) show
//!   over the wall, while the mist and beams (`ZTest` on) are hidden by it;
//! - the beam, spray and lightning parts (`UseDirectionAs` `PTDU_Up`) are
//!   drawn long and thin along their movement, not as squares.
//!
//! Run (Linux, no screen needed), with the game folder in `RC_GAME_DIR`:
//! `xvfb-run -a -s "-screen 0 1280x1024x24" cargo run -p rc_game --example bacta_zap_check`
//! It ends `BACTA ZAP CHECK: N passed, 0 failed`. Pictures:
//! `logs/bacta_zap_check_<n>.png`.

use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use rc_effects_plugin::{EffectStats, EffectsPlugin, RunningEffect, SpawnEffect};
use rc_game_core::{units, GameLog, GamePaths, GameState, Play, PlayerCamera};
use std::sync::{Arc, Mutex};

const W: u32 = 960;
const H: u32 = 540;
/// Frames before the effect starts (shaders are built meanwhile).
const START: u32 = 60;
/// Seconds after the start for each picture.
const SHOTS: [f32; 3] = [1.6, 2.6, 3.8];
const WALL: [u8; 3] = [90, 90, 90];
const BACKGROUND: [u8; 3] = [10, 10, 14];

#[derive(Resource, Default)]
struct Run {
    frame: u32,
    started: Option<f32>,
    taken: usize,
    pictures: Arc<Mutex<Vec<Measured>>>,
    /// Per shot: emitter name, sprites drawn, and the longest side over
    /// the shortest of its first sprite's axes.
    parts: Vec<Vec<(String, usize)>>,
    ended: bool,
}

#[derive(Debug, Clone, Default)]
struct Measured {
    /// Pixels in the wall's half clearly brighter than the wall.
    over_wall: usize,
    /// Pixels in the open half clearly brighter than the background.
    open: usize,
}

fn main() -> AppExit {
    let game = std::env::var("RC_GAME_DIR").unwrap_or_else(|_| {
        "C:\\Games\\Steam\\steamapps\\common\\Star Wars Republic Commando".into()
    });
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                resolution: (W, H).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(GameLog::terminal())
        .insert_resource(GamePaths::new(game, vec!["data".into()]))
        .init_state::<GameState>()
        .add_sub_state::<Play>()
        .init_resource::<Run>()
        .add_plugins(EffectsPlugin)
        .add_systems(Startup, scene)
        .add_systems(Update, drive)
        .add_observer(measure)
        .run()
}

/// Unreal units to Bevy.
fn b(x: f32, y: f32, z: f32) -> Vec3 {
    units::to_bevy(rc_movement::Vec3::new(x, y, z))
}

fn scene(
    mut commands: Commands,
    mut next: ResMut<NextState<GameState>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    next.set(GameState::InGame);
    commands.spawn((
        PlayerCamera,
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::srgb_u8(
                BACKGROUND[0],
                BACKGROUND[1],
                BACKGROUND[2],
            )),
            ..default()
        },
        bevy::render::view::Tonemapping::None,
        Transform::from_translation(b(300.0, 0.0, 60.0)).looking_at(b(40.0, 0.0, 50.0), Vec3::Y),
    ));
    // The wall: 150 uu in front of the effect, covering the half of the
    // view on the effect's +Y side.
    let c = WALL.map(|v| v as f32 / 255.0);
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.05, 8.0, 6.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(c[0], c[1], c[2]),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(b(150.0, 150.0, 0.0)),
    ));
}

fn drive(
    mut commands: Commands,
    mut run: ResMut<Run>,
    time: Res<Time>,
    mut spawn: MessageWriter<SpawnEffect>,
    effects: Query<&RunningEffect>,
    stats: Res<EffectStats>,
    mut exit: MessageWriter<AppExit>,
) {
    run.frame += 1;
    let now = time.elapsed_secs();
    // A picture of the empty scene first: where the wall is.
    if run.frame == START - 10 {
        commands.spawn(Screenshot::primary_window());
        return;
    }
    if run.frame == START {
        spawn.write(SpawnEffect {
            effect: "BactaZap_Dispenser".into(),
            at: Vec3::ZERO,
            by: "bacta_zap_check".into(),
            axes: Some([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
            ..default()
        });
        run.started = Some(now);
        return;
    }
    let Some(t0) = run.started else { return };
    if run.ended {
        return;
    }
    let shot = run.taken;
    if let Some(at) = SHOTS.get(shot) {
        if now - t0 >= *at {
            let parts = effects
                .iter()
                .flat_map(|r| {
                    r.state
                        .emitters
                        .iter()
                        .zip(r.def().emitters.iter())
                        .map(|(s, d)| (d.name.clone(), s.particles.len()))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            println!(
                "picture {shot} at {:.1} s: {} sprites drawn",
                now - t0,
                stats.sprites_drawn
            );
            for (n, c) in &parts {
                println!("  {n}: {c} alive");
            }
            run.parts.push(parts);
            commands.spawn(Screenshot::primary_window());
            run.taken += 1;
        }
        return;
    }
    // Wait for the last picture.
    if run.pictures.lock().unwrap().len() < SHOTS.len() + 1 {
        if now - t0 > SHOTS[SHOTS.len() - 1] + 5.0 {
            println!("FAIL: pictures never came");
            exit.write(AppExit::from_code(1));
            run.ended = true;
        }
        return;
    }
    run.ended = true;
    let mut fails = Vec::new();
    let mut passed = 0;
    let mut check = |ok: bool, what: String| {
        if ok {
            println!("PASS: {what}");
            passed += 1;
        } else {
            println!("FAIL: {what}");
            fails.push(what);
        }
    };
    check(
        stats.spawned.iter().any(|s| s == "BactaZap_Dispenser"),
        format!("the effect started ({:?})", stats.spawned),
    );
    let pictures: Vec<&String> = stats
        .missing
        .iter()
        .filter(|m| m.contains("picture"))
        .collect();
    check(
        pictures.is_empty(),
        format!("every picture was read (missing: {pictures:?})"),
    );
    let alive = |name: &str| {
        run.parts
            .iter()
            .flat_map(|p| p.iter())
            .filter(|(n, c)| n.contains(name) && *c > 0)
            .count()
    };
    // FlareA is left out: it sends one short flash (0.1-0.5 s) when the
    // effect starts and the pictures come later.
    for part in [
        "Mist",
        "Beams",
        "Holo_circle",
        "Holo_man",
        "Holo_Bar",
        "Holo_text",
        "ArmA",
        "ArmB",
        "Lightning",
        "CU_fx",
    ] {
        check(
            alive(part) > 0,
            format!("{part} had particles in a picture"),
        );
    }
    let shots: Vec<Measured> = run
        .pictures
        .lock()
        .unwrap()
        .iter()
        .skip(1)
        .cloned()
        .collect();
    for (i, m) in shots.iter().enumerate() {
        println!(
            "picture {i}: {} lit pixels over the wall, {} in the open half",
            m.over_wall, m.open
        );
    }
    // From 1.6 s the hologram parts are up: they must show over the wall.
    check(
        shots.iter().all(|m| m.over_wall > 200),
        "the hologram (ZTest off) shows over the wall in every picture".into(),
    );
    check(
        shots.iter().all(|m| m.open > 200),
        "the effect shows in the open half".into(),
    );
    println!("BACTA ZAP CHECK: {passed} passed, {} failed", fails.len());
    exit.write(if fails.is_empty() {
        AppExit::Success
    } else {
        AppExit::from_code(1)
    });
}

fn measure(captured: On<ScreenshotCaptured>, run: Res<Run>, mut wall: Local<Vec<bool>>) {
    let shot = &captured.image;
    let (w, h) = (shot.width() as usize, shot.height() as usize);
    let bgra = matches!(
        shot.texture_descriptor.format,
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
    );
    let Some(data) = shot.data.as_ref() else {
        return;
    };
    let index = run.pictures.lock().unwrap().len();
    let mut rgba = Vec::with_capacity(w * h * 4);
    let mut m = Measured::default();
    let first = wall.is_empty();
    for i in 0..w * h {
        let p = &data[i * 4..i * 4 + 4];
        let c = if bgra {
            [p[2], p[1], p[0]]
        } else {
            [p[0], p[1], p[2]]
        };
        rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
        let near = |want: [u8; 3], by: i32| {
            c.iter()
                .zip(want)
                .all(|(a, b)| (*a as i32 - b as i32).abs() < by)
        };
        if first {
            // The empty scene: remember where the wall is.
            wall.push(near(WALL, 4));
            continue;
        }
        let on_wall = wall.get(i).copied().unwrap_or(false);
        if on_wall && !near(WALL, 25) {
            m.over_wall += 1;
        } else if !on_wall && !near(BACKGROUND, 25) {
            m.open += 1;
        }
    }
    if let Ok(f) = std::fs::create_dir_all("logs")
        .and_then(|_| std::fs::File::create(format!("logs/bacta_zap_check_{index}.png")))
    {
        let _ = rc_image::png::write(
            &rc_image::Rgba {
                width: w,
                height: h,
                pixels: rgba,
            },
            f,
        );
    }
    run.pictures.lock().unwrap().push(m);
}
