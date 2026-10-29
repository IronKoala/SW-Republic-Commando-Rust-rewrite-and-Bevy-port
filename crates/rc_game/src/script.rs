//! `--script`: a fixed route through the game, in place of the keyboard and
//! mouse. It presses the same actions a player would (menu down, accept,
//! pause...), checks the result of each step, logs `CHECK ... PASS/FAIL`,
//! and exits with code 0 only if every check passed.
//!
//! The route: main menu, Campaign, Geonosis, its first level,
//! loading, the level's walls and sky (looking up), walk forward, pause (nothing moves), resume, quit to the main
//! menu (the level is cleared), Options (music volume up), back, Quit.

use crate::levels::LevelList;
use crate::menu::{Menu, Page};
use crate::player::Body;
use crate::settings::Settings;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use rc_game_core::{
    Action, ActionState, CurrentLevel, GameLog, GameSet, GameState, LevelCollision, Play, Player,
    PlayerCamera,
};
use std::collections::HashSet;

/// How long to wait for a level to load before failing, seconds.
const LOAD_TIMEOUT: f32 = 600.0;

type Probe = fn(&mut World) -> Result<String, String>;

#[derive(Clone)]
enum Step {
    /// Press and release an action (one frame down, one up).
    Press(Action),
    /// Hold an action for some seconds.
    Hold(Action, f32),
    /// Wait some seconds with nothing pressed.
    Wait(f32),
    /// Wait until the game reaches a state (fails after the timeout).
    Until(&'static str, fn(&mut World) -> bool, f32),
    /// Check something; logs PASS or FAIL.
    Check(&'static str, Probe),
    /// Remember something for a later check.
    Note(fn(&mut World)),
    /// Press menu-down until the item the function names is selected.
    SelectItem(&'static str, fn(&mut World) -> Option<usize>),
    /// Capture the window, save it to `logs/<name>.png` and measure its
    /// colours (retried while the capture comes back black).
    Shot(&'static str),
    /// Log the result so far, and make the exit code an error if anything
    /// failed (before Quit closes the game with success).
    Summary,
}

#[derive(Resource)]
struct Script {
    steps: Vec<Step>,
    next: usize,
    /// Seconds into the current step.
    t: f32,
    /// Frames into the current step.
    frames: u32,
    passed: u32,
    failed: u32,
}

/// The player's feet at the last `Note`, Unreal units.
/// Colour measurements of one screenshot.
#[derive(Debug, Clone, Copy, Default)]
struct ShotStats {
    /// Share of pixels in the menus' amber, 0 to 1.
    amber: f32,
    /// Share of pixels that aren't near-black.
    lit: f32,
    /// Amber pixels within 3% of the screen height of the centre (the
    /// crosshair).
    centre_amber: u32,
    /// Share of pixels in the colour shown where nothing is drawn (no
    /// level and no sky), 0 to 1.
    empty: f32,
}

#[derive(Resource, Default)]
struct Shots {
    stats: std::collections::HashMap<&'static str, ShotStats>,
    pending: Option<&'static str>,
    attempts: u32,
}

#[derive(Resource, Default)]
struct Noted {
    feet: Option<rc_movement::Vec3>,
    music_volume: f32,
}

pub struct ScriptPlugin;

impl Plugin for ScriptPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Script {
            steps: steps(),
            next: 0,
            t: 0.0,
            frames: 0,
            passed: 0,
            failed: 0,
        })
        .init_resource::<Noted>()
        .init_resource::<Shots>()
        .add_systems(Update, run.in_set(GameSet::Input));
    }
}

fn in_state(w: &mut World, s: GameState) -> bool {
    *w.resource::<State<GameState>>().get() == s
}

fn page(w: &mut World) -> Page {
    w.resource::<Menu>().page
}

fn feet(w: &mut World) -> Option<rc_movement::Vec3> {
    let mut q = w.query_filtered::<&Body, With<Player>>();
    q.iter(w).next().map(|b| b.pawn.feet())
}

fn set_pitch(w: &mut World, pitch: f32) {
    let mut q = w.query_filtered::<&mut Body, With<Player>>();
    for mut b in q.iter_mut(w) {
        b.pitch = pitch;
    }
}

fn count<T: Component>(w: &mut World) -> usize {
    let mut q = w.query_filtered::<(), With<T>>();
    q.iter(w).count()
}

fn steps() -> Vec<Step> {
    use Step::*;
    vec![
        Until("main menu", |w| in_state(w, GameState::MainMenu), 10.0),
        Check("main menu page shows", |w| match page(w) {
            Page::Main => Ok("main".into()),
            p => Err(format!("{p:?}")),
        }),
        Wait(0.5),
        Shot("main_menu"),
        Check("main menu is drawn in amber", |w| {
            let s = shot(w, "main_menu")?;
            if s.amber > 0.002 && s.lit > 0.002 {
                Ok(format!(
                    "{:.2}% amber, {:.1}% lit",
                    s.amber * 100.0,
                    s.lit * 100.0
                ))
            } else {
                Err(format!("{s:?}"))
            }
        }),
        Press(Action::MenuAccept),
        Wait(0.2),
        Check("Campaign opens the campaign list", |w| match page(w) {
            Page::Campaigns => Ok("campaigns".into()),
            p => Err(format!("{p:?}")),
        }),
        // Geonosis' first level (the Prologue is a cut scene where the
        // player falls; the first Geonosis level is the one rc_walk tests).
        SelectItem("Geonosis", |w| {
            w.resource::<LevelList>()
                .campaigns
                .iter()
                .position(|c| c.name == "Geonosis")
        }),
        Press(Action::MenuAccept),
        Wait(0.2),
        Check("the campaign opens its levels", |w| match page(w) {
            Page::Levels(i) => Ok(format!("levels of campaign {i}")),
            p => Err(format!("{p:?}")),
        }),
        // Go down to the next item and up again, to test moving around.
        Press(Action::MenuDown),
        Press(Action::MenuUp),
        Check("selection back on the first level", |w| {
            let s = w.resource::<Menu>().selected;
            if s == 0 {
                Ok("0".into())
            } else {
                Err(s.to_string())
            }
        }),
        Press(Action::MenuAccept),
        Until("loading starts", |w| !in_state(w, GameState::MainMenu), 5.0),
        Check("a map was chosen", |w| {
            w.resource::<CurrentLevel>()
                .map
                .clone()
                .ok_or("none".into())
        }),
        Until(
            "level loaded",
            |w| in_state(w, GameState::InGame) || in_state(w, GameState::MainMenu),
            LOAD_TIMEOUT,
        ),
        Check("in the game", |w| {
            if in_state(w, GameState::InGame) {
                Ok("InGame".into())
            } else {
                Err(format!(
                    "back at the menu: {:?}",
                    w.resource::<Menu>().notice
                ))
            }
        }),
        Wait(1.0),
        Check("player, camera and collision exist", |w| {
            let (p, c, l) = (
                count::<Player>(w),
                count::<PlayerCamera>(w),
                w.contains_resource::<LevelCollision>(),
            );
            if p == 1 && c == 1 && l {
                Ok(format!("{p} player, {c} camera"))
            } else {
                Err(format!("{p} players, {c} cameras, collision {l}"))
            }
        }),
        Shot("in_game"),
        Check("level drawn with the crosshair in amber", |w| {
            let s = shot(w, "in_game")?;
            if s.lit > 0.3 && s.centre_amber > 4 {
                Ok(format!(
                    "{:.0}% lit, {} amber pixels at the centre",
                    s.lit * 100.0,
                    s.centre_amber
                ))
            } else {
                Err(format!("{s:?}"))
            }
        }),
        Check("the level part loaded walls, props, doors and invisible walls", |w| {
            let Some(i) = w.get_resource::<rc_level_plugin::LevelInfo>() else {
                return Err("no LevelInfo".into());
            };
            let got = format!(
                "{}: {} collision triangles, {} props ({} with their class's model), {} blocking volumes, start {}",
                i.map,
                i.collision_triangles,
                i.props,
                i.class_props,
                i.blocking_volumes,
                if i.has_player_start { "from the map" } else { "MISSING" }
            );
            if i.collision_triangles > 10_000 && i.props > 0 && i.class_props > 0 && i.has_player_start {
                Ok(got)
            } else {
                Err(got)
            }
        }),
        Check("the sky camera turns with the player's camera", |w| {
            let sky = count::<rc_level_plugin::SkyCamera>(w);
            let mut q = w.query_filtered::<&Camera, (
                With<PlayerCamera>,
                With<rc_level_plugin::SkyFollows>,
            )>();
            let follows = q
                .iter(w)
                .filter(|c| matches!(c.clear_color, ClearColorConfig::None))
                .count();
            if sky == 1 && follows == 1 {
                Ok("1 sky camera; the player's camera lets it through".into())
            } else {
                Err(format!("{sky} sky cameras, {follows} player cameras following"))
            }
        }),
        // Look up 60 degrees: geo_01a shows open sky above the spires there.
        Note(|w| set_pitch(w, 60f32.to_radians())),
        Wait(0.5),
        Shot("look_up"),
        Check("looking up shows the sky, not the empty colour", |w| {
            let s = shot(w, "look_up")?;
            let got = format!("{:.1}% empty, {:.0}% lit", s.empty * 100.0, s.lit * 100.0);
            if s.empty < 0.05 && s.lit > 0.3 {
                Ok(got)
            } else {
                Err(got)
            }
        }),
        Note(|w| set_pitch(w, 0.0)),
        Wait(0.2),
        Note(|w| {
            let f = feet(w);
            w.resource_mut::<Noted>().feet = f;
        }),
        Hold(Action::MoveForward, 1.0),
        Check("walking forward moves the player", |w| {
            let (Some(a), Some(b)) = (w.resource::<Noted>().feet, feet(w)) else {
                return Err("no player".into());
            };
            let d = (b - a).length_2d();
            if d > 100.0 {
                Ok(format!("{d:.0} uu"))
            } else {
                Err(format!("only {d:.0} uu"))
            }
        }),
        Press(Action::Pause),
        Wait(0.3),
        Check("pause key pauses", |w| {
            match w.get_resource::<State<Play>>().map(|s| *s.get()) {
                Some(Play::Paused) => Ok("paused".into()),
                p => Err(format!("{p:?}")),
            }
        }),
        Note(|w| {
            let f = feet(w);
            w.resource_mut::<Noted>().feet = f;
        }),
        Hold(Action::MoveForward, 0.5),
        Check("nothing moves while paused", |w| {
            let (Some(a), Some(b)) = (w.resource::<Noted>().feet, feet(w)) else {
                return Err("no player".into());
            };
            let d = (b - a).length();
            if d < 0.01 {
                Ok(format!("{d:.3} uu"))
            } else {
                Err(format!("moved {d:.1} uu"))
            }
        }),
        Press(Action::MenuAccept), // Resume
        Wait(0.3),
        Check("Resume goes back to play", |w| {
            match w.get_resource::<State<Play>>().map(|s| *s.get()) {
                Some(Play::Running) => Ok("running".into()),
                p => Err(format!("{p:?}")),
            }
        }),
        Press(Action::Pause),
        Wait(0.2),
        Press(Action::MenuDown),
        Press(Action::MenuDown),
        Press(Action::MenuAccept), // Quit to main menu
        Until("main menu again", |w| in_state(w, GameState::MainMenu), 5.0),
        Wait(0.3),
        Check("leaving the level clears it", |w| {
            let (p, c) = (count::<Player>(w), count::<PlayerCamera>(w));
            let meshes = count::<Mesh3d>(w);
            if p == 0 && c == 0 && meshes == 0 {
                Ok("no player, camera or level meshes left".into())
            } else {
                Err(format!("{p} players, {c} cameras, {meshes} meshes left"))
            }
        }),
        Note(|w| {
            let v = w.resource::<Settings>().audio.music_volume;
            w.resource_mut::<Noted>().music_volume = v;
        }),
        Press(Action::MenuDown),
        Press(Action::MenuAccept), // Options
        Wait(0.2),
        Press(Action::MenuLeft), // music volume down a step
        Wait(0.2),
        Check("left lowers the music volume", |w| {
            let before = w.resource::<Noted>().music_volume;
            let now = w.resource::<Settings>().audio.music_volume;
            if now < before {
                Ok(format!("{before:.1} -> {now:.1}"))
            } else {
                Err(format!("{before:.1} -> {now:.1}"))
            }
        }),
        Press(Action::MenuBack),
        Wait(0.2),
        Check("Back returns to the main menu", |w| match page(w) {
            Page::Main => Ok("main".into()),
            p => Err(format!("{p:?}")),
        }),
        Press(Action::MenuDown),
        Check("Quit is selected", |w| {
            let s = w.resource::<Menu>().selected;
            if s == 2 {
                Ok("item 2".into())
            } else {
                Err(format!("item {s}"))
            }
        }),
        Summary,
        Press(Action::MenuAccept), // Quit: the game should close now.
        Wait(5.0),
        Check("Quit closes the game", |_| {
            Err("still running 5 s later".into())
        }),
    ]
}

fn shot(w: &mut World, name: &str) -> Result<ShotStats, String> {
    w.resource::<Shots>()
        .stats
        .get(name)
        .copied()
        .ok_or("no screenshot".into())
}

fn measure_shot(captured: On<ScreenshotCaptured>, mut shots: ResMut<Shots>, log: Res<GameLog>) {
    let Some(name) = shots.pending.take() else {
        return;
    };
    let img = &captured.image;
    let (w, h) = (img.width() as usize, img.height() as usize);
    let bgra = matches!(
        img.texture_descriptor.format,
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
    );
    let Some(data) = img.data.as_ref() else {
        return;
    };
    let mut rgba = Vec::with_capacity(w * h * 4);
    for p in data.chunks_exact(4) {
        if bgra {
            rgba.extend([p[2], p[1], p[0], 255]);
        } else {
            rgba.extend([p[0], p[1], p[2], 255]);
        }
    }
    let (mut amber, mut lit, mut centre, mut empty) = (0u32, 0u32, 0u32, 0u32);
    let bg = rc_level_plugin::EMPTY_COLOUR.map(|c| c as i32);
    let r = (h as f32 * 0.03) as i64;
    for (i, p) in rgba.chunks_exact(4).enumerate() {
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        let (pr, pg, pb) = (p[0] as i32, p[1] as i32, p[2] as i32);
        // Near the theme's amber (#F2B544), allowing for blending.
        let is_amber = (pr - 242).abs() < 50 && (pg - 181).abs() < 50 && pb < 130;
        if is_amber {
            amber += 1;
            if (x - w as i64 / 2).abs() <= r && (y - h as i64 / 2).abs() <= r {
                centre += 1;
            }
        }
        if pr.max(pg).max(pb) > 30 {
            lit += 1;
        }
        if (pr - bg[0]).abs() <= 6 && (pg - bg[1]).abs() <= 6 && (pb - bg[2]).abs() <= 6 {
            empty += 1;
        }
    }
    if lit == 0 {
        log.line(format!(
            "script: screenshot {name} is all black (window not drawn yet), trying again"
        ));
        return;
    }
    let n = (w * h).max(1) as f32;
    let stats = ShotStats {
        amber: amber as f32 / n,
        lit: lit as f32 / n,
        centre_amber: centre,
        empty: empty as f32 / n,
    };
    let path = format!("logs/{name}.png");
    let saved = std::fs::write(
        &path,
        rc_image::png::encode(&rc_image::Rgba {
            width: w,
            height: h,
            pixels: rgba,
        }),
    );
    log.line(format!(
        "script: screenshot {name} {w}x{h}: {:.2}% amber, {:.1}% lit, {:.1}% empty, {centre} amber at the centre; {}",
        stats.amber * 100.0,
        stats.lit * 100.0,
        stats.empty * 100.0,
        match saved {
            Ok(()) => format!("saved to {path}"),
            Err(e) => format!("not saved ({e})"),
        }
    ));
    shots.stats.insert(name, stats);
}

fn run(world: &mut World) {
    let dt = world.resource::<Time>().delta_secs();
    let log = world.resource::<GameLog>().clone();
    let mut held = HashSet::new();
    loop {
        let (step, t, frames) = {
            let s = world.resource::<Script>();
            if s.next == usize::MAX - 1 {
                break;
            }
            let Some(step) = s.steps.get(s.next).cloned() else {
                let (p, f) = (s.passed, s.failed);
                log.line(format!("SCRIPT DONE: {p} checks passed, {f} failed"));
                world.write_message(if f == 0 {
                    AppExit::Success
                } else {
                    AppExit::from_code(1)
                });
                world.resource_mut::<Script>().next = usize::MAX - 1;
                break;
            };
            (step, s.t, s.frames)
        };
        let mut done = true;
        match step {
            Step::Press(a) => {
                if frames == 0 {
                    held.insert(a);
                    done = false;
                }
                // Frame 1: released.
            }
            Step::Hold(a, secs) => {
                if t < secs {
                    held.insert(a);
                    done = false;
                }
            }
            Step::Wait(secs) => done = t >= secs,
            Step::Until(what, cond, timeout) => {
                if !cond(world) {
                    if t < timeout {
                        done = false;
                    } else {
                        log.line(format!(
                            "CHECK {what}: timed out after {timeout:.0} s -> FAIL"
                        ));
                        world.resource_mut::<Script>().failed += 1;
                    }
                } else {
                    log.line(format!("script: {what} after {t:.2} s"));
                }
            }
            Step::Check(name, probe) => {
                let r = probe(world);
                let mut s = world.resource_mut::<Script>();
                match r {
                    Ok(got) => {
                        s.passed += 1;
                        log.line(format!("CHECK {name}: {got} -> PASS"));
                    }
                    Err(got) => {
                        s.failed += 1;
                        log.line(format!("CHECK {name}: {got} -> FAIL"));
                    }
                }
            }
            Step::Note(f) => f(world),
            Step::SelectItem(what, target) => match target(world) {
                None => {
                    log.line(format!("CHECK find {what} in the menu: not there -> FAIL"));
                    world.resource_mut::<Script>().failed += 1;
                }
                Some(i) => {
                    if world.resource::<Menu>().selected != i {
                        // Down on even frames, released on odd ones.
                        if frames % 2 == 0 {
                            held.insert(Action::MenuDown);
                        }
                        done = frames > 200;
                    }
                }
            },
            Step::Shot(name) => {
                let mut shots = world.resource_mut::<Shots>();
                if shots.stats.contains_key(name) {
                    shots.attempts = 0;
                } else {
                    done = false;
                    if shots.pending.is_none() && frames % 10 == 0 {
                        shots.attempts += 1;
                        if shots.attempts > 6 {
                            log.line(format!(
                                "CHECK screenshot {name}: no usable capture -> FAIL"
                            ));
                            world.resource_mut::<Script>().failed += 1;
                            done = true;
                        } else {
                            shots.pending = Some(name);
                            world
                                .spawn(Screenshot::primary_window())
                                .observe(measure_shot);
                        }
                    }
                }
            }
            Step::Summary => {
                let s = world.resource::<Script>();
                let (p, f) = (s.passed, s.failed);
                log.line(format!("SCRIPT DONE: {p} checks passed, {f} failed"));
                if f > 0 {
                    world.write_message(AppExit::from_code(1));
                }
            }
        }
        let mut s = world.resource_mut::<Script>();
        if done {
            s.next += 1;
            s.t = 0.0;
            s.frames = 0;
            // Steps that take no time run in the same frame; a key press or
            // wait ends this frame's turn.
            if matches!(step, Step::Press(_) | Step::Hold(..) | Step::Wait(_)) {
                break;
            }
        } else {
            s.t += dt;
            s.frames += 1;
            break;
        }
    }
    let mut movement = Vec2::ZERO;
    if held.contains(&Action::MoveForward) {
        movement.y = 1.0;
    }
    world
        .resource_mut::<ActionState>()
        .update(held, movement, Vec2::ZERO);
}
