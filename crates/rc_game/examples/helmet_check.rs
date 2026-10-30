//! Checks the helmet frame on the GPU against the same frame painted in
//! software (`rc_menus::helmet::paint`), at several screen shapes, and that
//! the HELMET setting hides it. Needs the game's files (the model and its
//! picture), not a level: a plain background stands in for the world.
//!
//! Run (Linux, no screen needed), with RC_GAME_DIR set to the game folder:
//! `xvfb-run -a -s "-screen 0 1920x1080x24" cargo run -p rc_game --example helmet_check`
//! On Windows the same without `xvfb-run -a -s "..."`. It ends
//! `HELMET CHECK: N passed, 0 failed`.

#[path = "../src/helmet.rs"]
mod helmet;
#[path = "../src/settings.rs"]
#[allow(dead_code)]
mod settings;

use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::PrimaryWindow;
use rc_game_core::{GameLog, GamePaths, GameState, PlayerCamera};
use settings::Settings;
use std::sync::{Arc, Mutex};

/// The world's stand-in colour behind the helmet.
const BACKGROUND: [u8; 3] = [90, 110, 140];

/// One step: a window size and whether the HELMET setting is on.
struct Step {
    name: &'static str,
    size: (u32, u32),
    helmet: bool,
}

const STEPS: &[Step] = &[
    // The first picture under Xvfb is often black; it isn't checked.
    Step {
        name: "warm-up",
        size: (960, 720),
        helmet: true,
    },
    Step {
        name: "4:3",
        size: (960, 720),
        helmet: true,
    },
    Step {
        name: "16:9",
        size: (1280, 720),
        helmet: true,
    },
    Step {
        name: "21:9",
        size: (1680, 720),
        helmet: true,
    },
    Step {
        name: "32:9",
        size: (1792, 504),
        helmet: true,
    },
    Step {
        name: "5:4",
        size: (900, 720),
        helmet: true,
    },
    Step {
        name: "phone 20:9",
        size: (1600, 720),
        helmet: true,
    },
    Step {
        name: "helmet off",
        size: (1280, 720),
        helmet: false,
    },
    Step {
        name: "helmet on again",
        size: (1280, 720),
        helmet: true,
    },
];

const FRAMES_PER_STEP: u32 = 30;

/// A step's name and what was measured.
#[derive(Debug, Clone)]
struct Measured {
    name: &'static str,
    size: (usize, usize),
    /// Mean colour difference (0 to 255) on the pixels the software paint
    /// says are helmet, and on the rest.
    on_helmet: f32,
    elsewhere: f32,
    /// Share of the screen the software paint covers.
    painted: f32,
}

#[derive(Resource, Default)]
struct Run {
    frame: u32,
    step: usize,
    results: Arc<Mutex<Vec<Measured>>>,
}

fn main() {
    let game = std::env::var("RC_GAME_DIR").unwrap_or_else(|_| {
        "C:\\Games\\Steam\\steamapps\\common\\Star Wars Republic Commando".into()
    });
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                resolution: (960u32, 720u32).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(GameLog::terminal())
        .insert_resource(GamePaths::new(game, vec!["data".into()]))
        .insert_resource(Settings::default())
        .init_state::<GameState>()
        .init_resource::<Run>()
        .add_plugins(helmet::HelmetPlugin)
        .add_systems(Startup, scene)
        .add_systems(Update, drive)
        .add_observer(measure)
        .run();
}

fn scene(mut commands: Commands, mut next: ResMut<NextState<GameState>>) {
    next.set(GameState::InGame);
    commands.spawn((
        PlayerCamera,
        Camera3d::default(),
        Camera {
            order: 0,
            clear_color: ClearColorConfig::Custom(Color::srgb_u8(
                BACKGROUND[0],
                BACKGROUND[1],
                BACKGROUND[2],
            )),
            ..default()
        },
        bevy::render::view::Tonemapping::None,
    ));
}

fn drive(
    mut commands: Commands,
    mut run: ResMut<Run>,
    mut settings: ResMut<Settings>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    helmet: Res<helmet::Helmet>,
    mut exit: MessageWriter<AppExit>,
) {
    if let Some(Err(e)) = &helmet.model {
        println!("FAIL: the helmet couldn't be read: {e}");
        println!("HELMET CHECK: 0 passed, 1 failed");
        exit.write(AppExit::from_code(1));
        return;
    }
    run.frame += 1;
    let f = run.frame % FRAMES_PER_STEP;
    if f == 1 {
        let Some(step) = STEPS.get(run.step) else {
            if run.results.lock().unwrap().len() < STEPS.len() && run.step < STEPS.len() + 4 {
                run.step += 1;
                return;
            }
            let fails = failures(&run, &helmet);
            for x in &fails {
                println!("FAIL: {x}");
            }
            let checks = STEPS.len() - 1;
            println!(
                "HELMET CHECK: {} passed, {} failed",
                checks - fails.len().min(checks),
                fails.len()
            );
            exit.write(if fails.is_empty() {
                AppExit::Success
            } else {
                AppExit::from_code(1)
            });
            return;
        };
        settings.game.helmet = step.helmet;
        if let Ok(mut w) = windows.single_mut() {
            w.resolution.set(step.size.0 as f32, step.size.1 as f32);
        }
        println!("step: {}", step.name);
    }
    if f == 0 {
        commands.spawn(Screenshot::primary_window());
        run.step += 1;
    }
}

fn measure(captured: On<ScreenshotCaptured>, run: Res<Run>, helmet: Res<helmet::Helmet>) {
    let shot = &captured.image;
    let (w, h) = (shot.width() as usize, shot.height() as usize);
    let bgra = matches!(
        shot.texture_descriptor.format,
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
    );
    let Some(data) = shot.data.as_ref() else {
        return;
    };
    let Some(Ok(model)) = &helmet.model else {
        return;
    };
    let index = run.results.lock().unwrap().len().min(STEPS.len() - 1);
    let step = &STEPS[index];
    // What it should look like.
    let points = rc_menus::helmet::fit(model, &helmet.plan, w as f32 / h as f32);
    let want = rc_menus::helmet::paint(model, &points, helmet.plan.brightness, w, h);
    let (mut on, mut on_n, mut off, mut off_n) = (0.0f64, 0usize, 0.0f64, 0usize);
    for i in 0..w * h {
        let p = &data[i * 4..i * 4 + 4];
        let got = if bgra {
            [p[2], p[1], p[0]]
        } else {
            [p[0], p[1], p[2]]
        };
        let q = &want.pixels[i * 4..i * 4 + 4];
        let helmet_here = q[3] > 0;
        // With the setting off, the painting's helmet pixels should show
        // the background.
        let expect = if helmet_here && step.helmet {
            [q[0], q[1], q[2]]
        } else {
            BACKGROUND
        };
        let d = (0..3)
            .map(|k| (got[k] as f64 - expect[k] as f64).abs())
            .sum::<f64>()
            / 3.0;
        if helmet_here {
            on += d;
            on_n += 1;
        } else {
            off += d;
            off_n += 1;
        }
    }
    let m = Measured {
        name: step.name,
        size: (w, h),
        on_helmet: (on / on_n.max(1) as f64) as f32,
        elsewhere: (off / off_n.max(1) as f64) as f32,
        painted: on_n as f32 / (w * h) as f32,
    };
    println!(
        "  {}: {}x{}, helmet covers {:.1}%, difference on the helmet {:.1}, elsewhere {:.1}",
        m.name,
        w,
        h,
        m.painted * 100.0,
        m.on_helmet,
        m.elsewhere
    );
    if let Ok(f) = std::fs::create_dir_all("logs")
        .and_then(|_| std::fs::File::create(format!("logs/helmet_check_{}.png", index)))
    {
        let mut rgba = Vec::with_capacity(w * h * 4);
        for i in 0..w * h {
            let p = &data[i * 4..i * 4 + 4];
            if bgra {
                rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
            } else {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        let _ = rc_image::png::write(
            &rc_image::Rgba {
                width: w,
                height: h,
                pixels: rgba,
            },
            f,
        );
    }
    run.results.lock().unwrap().push(m);
}

fn failures(run: &Run, _helmet: &helmet::Helmet) -> Vec<String> {
    let r = run.results.lock().unwrap();
    let mut out = vec![];
    if r.len() != STEPS.len() {
        out.push(format!("{} pictures of {}", r.len(), STEPS.len()));
        return out;
    }
    for (m, step) in r.iter().zip(STEPS).skip(1) {
        if (m.size.0 as u32, m.size.1 as u32) != step.size {
            out.push(format!(
                "{}: the window is {}x{}, not {}x{}",
                m.name, m.size.0, m.size.1, step.size.0, step.size.1
            ));
        }
        // Edges of triangles land a pixel apart now and then; the colours
        // must match on the whole.
        if m.on_helmet > 20.0 || m.elsewhere > 6.0 || m.painted < 0.05 {
            out.push(format!(
                "{}: the picture doesn't match the painting (helmet {:.1}, elsewhere {:.1}, covers {:.1}%)",
                m.name,
                m.on_helmet,
                m.elsewhere,
                m.painted * 100.0
            ));
        }
    }
    out
}
