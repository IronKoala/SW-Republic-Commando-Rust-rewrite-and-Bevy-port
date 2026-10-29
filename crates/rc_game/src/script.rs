//! `--script`: a fixed route through the game, in place of the keyboard and
//! mouse. It presses the same actions a player would (menu down, accept,
//! pause...), checks the result of each step, logs `CHECK ... PASS/FAIL`,
//! and exits with code 0 only if every check passed.
//!
//! The route: the original main menu (colours checked against the
//! original's pictures), CAMPAIGN MAP, Geonosis, its first level, loading,
//! the level's walls and sky (looking up), walk forward, pause (nothing
//! moves), resume, saving (the level-start checkpoint, a checkpoint, a
//! quicksave), walking away, quickload (the player is back where they
//! saved, and saving again gives the same numbers), quit to the main menu
//! (the level is cleared), OPTIONS
//! (music volume down), back, QUIT, the original's "are you sure?" box (NO,
//! then QUIT again and YES).

use crate::levels::LevelList;
use crate::menu::{Menu, Page};
use crate::original_menu::{self, OriginalMenus};
use crate::player::Body;
use crate::settings::Settings;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use rc_game_core::{
    Action, ActionState, CurrentLevel, GameLog, GameSet, GameState, LevelCollision, Play, Player,
    PlayerCamera,
};
use rc_game_core::save::{CheckpointReached, RestoringSave, SaveSlot};
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
    /// Press an action, then check which original menu button has focus.
    FocusAfter(Action, &'static str),
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
}

#[derive(Resource, Default)]
struct Shots {
    stats: std::collections::HashMap<&'static str, ShotStats>,
    /// The captured pictures, for colour checks.
    pictures: std::collections::HashMap<&'static str, rc_image::Rgba>,
    pending: Option<&'static str>,
    attempts: u32,
}

#[derive(Resource, Default)]
struct Noted {
    feet: Option<rc_movement::Vec3>,
    music_volume: f32,
    /// The player's centre and facing when the quicksave was made.
    saved: Option<(rc_movement::Vec3, f32)>,
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

/// The player's collision-box centre and facing.
fn body(w: &mut World) -> Option<(rc_movement::Vec3, f32)> {
    let mut q = w.query_filtered::<&Body, With<Player>>();
    q.iter(w).next().map(|b| (b.pawn.location, b.yaw))
}

fn save_file(w: &mut World, slot: &SaveSlot) -> Result<rc_save::file::SaveFile, String> {
    let dir = w.resource::<rc_save::SaveFolder>().0.clone();
    rc_save::file::read(&rc_save::file::slot_path(&dir, slot))
        .map_err(|e| format!("{}: {e}", slot.file_name()))
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
        Wait(0.3),
        Check("the original main menu is drawn", |w| {
            let m = w.resource::<OriginalMenus>();
            if !m.active() {
                return Err("not loaded (placeholder menu shown)".into());
            }
            let focus = focus(w);
            let n = w.resource::<OriginalMenus>().drawn.len();
            if n > 100 && focus.as_deref() == Some("MenuOptions[0]") {
                Ok(format!("{n} pictures and letters, focus on {focus:?}"))
            } else {
                Err(format!("{n} pictures and letters, focus on {focus:?}"))
            }
        }),
        Wait(0.5),
        Shot("main_menu"),
        Check("main menu matches the original's pictures", |w| {
            compare(w, "main_menu", 0.2)
        }),
        FocusAfter(Action::MenuDown, "MenuOptions[2]"),
        FocusAfter(Action::MenuUp, "MenuOptions[0]"),
        FocusAfter(Action::MenuDown, "MenuOptions[2]"),
        Press(Action::MenuAccept),
        Wait(0.2),
        Check("CAMPAIGN MAP opens the campaign list", |w| match page(w) {
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
        // Without the sky that view is mostly black (17% lit when the sky
        // camera was switched off for a trial run; 72% with it).
        Note(|w| set_pitch(w, 60f32.to_radians())),
        Wait(0.5),
        Shot("look_up"),
        Check("looking up shows the sky", |w| {
            let s = shot(w, "look_up")?;
            let got = format!("{:.0}% lit", s.lit * 100.0);
            if s.lit > 0.45 {
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
        Check("the level's start was saved as a checkpoint", |w| {
            let f = save_file(w, &SaveSlot::Checkpoint)?;
            let got = format!("{} at {:?}", f.header.map, f.header.checkpoint);
            match f.header.checkpoint.as_deref() {
                Some(rc_save::LEVEL_START) => Ok(got),
                _ => Err(got),
            }
        }),
        Note(|w| {
            w.write_message(CheckpointReached {
                name: "script checkpoint".into(),
            });
        }),
        Wait(0.2),
        Check("reaching a checkpoint saves it", |w| {
            let f = save_file(w, &SaveSlot::Checkpoint)?;
            match f.header.checkpoint.as_deref() {
                Some("script checkpoint") => Ok("checkpoint.toml at \"script checkpoint\"".into()),
                c => Err(format!("{c:?}")),
            }
        }),
        // Use some ammo first, so the save differs from a fresh start.
        Hold(Action::Fire, 0.5),
        Wait(0.2),
        Note(|w| {
            let b = body(w);
            w.resource_mut::<Noted>().saved = b;
        }),
        Press(Action::QuickSave),
        Wait(0.2),
        Check("quicksave writes where the player is", |w| {
            let f = save_file(w, &SaveSlot::Quick)?;
            let (at, _) = w.resource::<Noted>().saved.ok_or("no player")?;
            let p = f.part("player").ok_or("no [player] section")?;
            let loc: Vec<f32> = p["location"]
                .as_array()
                .ok_or("no location")?
                .iter()
                .filter_map(|v| v.as_float().map(|f| f as f32))
                .collect();
            let d = (rc_movement::Vec3::new(loc[0], loc[1], loc[2]) - at).length();
            let got = format!(
                "{} sections, player at ({:.1}, {:.1}, {:.1}), {d:.2} uu from the live player",
                f.parts.len(),
                loc[0],
                loc[1],
                loc[2]
            );
            if d < 1.0 {
                Ok(got)
            } else {
                Err(got)
            }
        }),
        Hold(Action::MoveForward, 1.0),
        Check("walked away from the saved spot", |w| {
            let (at, _) = w.resource::<Noted>().saved.ok_or("no save")?;
            let (now, _) = body(w).ok_or("no player")?;
            let d = (now - at).length_2d();
            if d > 100.0 {
                Ok(format!("{d:.0} uu away"))
            } else {
                Err(format!("only {d:.0} uu"))
            }
        }),
        Press(Action::QuickLoad),
        Until(
            "quickload reloads the level",
            |w| !in_state(w, GameState::InGame),
            5.0,
        ),
        Until(
            "quickload finished",
            |w| in_state(w, GameState::InGame) && !w.contains_resource::<RestoringSave>(),
            LOAD_TIMEOUT,
        ),
        Wait(0.3),
        Check("quickload puts the player back where they saved", |w| {
            let (at, yaw) = w.resource::<Noted>().saved.ok_or("no save")?;
            let (now, now_yaw) = body(w).ok_or("no player")?;
            let d = (now - at).length();
            let turn = (now_yaw - yaw).abs();
            let got = format!("{d:.2} uu from the saved spot, facing {:.3} rad off", turn);
            if d < 2.0 && turn < 0.001 {
                Ok(got)
            } else {
                Err(got)
            }
        }),
        Check("quickload gives back the ammo used before saving", |w| {
            let quick = save_file(w, &SaveSlot::Quick)?;
            let c = quick.part("combat").ok_or("no [combat] section")?;
            let saved = c["weapons"].as_array().and_then(|a| a.first()).ok_or("no weapons")?;
            let saved_clip = saved["clip"].as_float().ok_or("no clip")? as f32;
            let mut q = w.query_filtered::<&rc_combat::Arsenal, With<Player>>();
            let a = q.iter(w).next().ok_or("player has no weapons")?;
            let (id, now) = a.weapons.first().ok_or("no weapons")?;
            let full = w
                .resource::<rc_combat::WeaponLibrary>()
                .weapons
                .get(id)
                .map(|p| rc_combat::WeaponState::new(p).clip)
                .unwrap_or(0.0);
            let got = format!("{id}: {:.0} in the magazine now, {saved_clip:.0} saved, {full:.0} when full", now.clip);
            if (now.clip - saved_clip).abs() < 0.01 && saved_clip < full {
                Ok(got)
            } else {
                Err(got)
            }
        }),
        Check("saving again after the load gives the same state", |w| {
            let quick = save_file(w, &SaveSlot::Quick)?;
            let again = rc_save::collect(w, None)?;
            let mut notes = vec![];
            for (k, v) in &quick.parts {
                let Some(b) = again.parts.get(k) else {
                    return Err(format!("[{k}] missing after the load"));
                };
                let diff = rc_save::file::largest_difference(v, b)
                    .ok_or(format!("[{k}] changed shape: {v} vs {b}"))?;
                notes.push(format!("[{k}] off by {diff:.3}"));
                if diff > 2.0 {
                    return Err(notes.join(", "));
                }
            }
            if again.header.map != quick.header.map
                || again.header.checkpoint != quick.header.checkpoint
            {
                return Err(format!("{:?} vs {:?}", again.header, quick.header));
            }
            Ok(notes.join(", "))
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
        Check("the original main menu is back, focus on NEW GAME", |w| {
            match (w.resource::<OriginalMenus>().active(), focus(w)) {
                (true, Some(f)) if f == "MenuOptions[0]" => Ok(f),
                other => Err(format!("{other:?}")),
            }
        }),
        Note(|w| {
            let v = w.resource::<Settings>().audio.music_volume;
            w.resource_mut::<Noted>().music_volume = v;
        }),
        // MULTIPLAYER, LOAD GAME, PROFILES and EXTRAS are greyed out and
        // skipped.
        FocusAfter(Action::MenuDown, "MenuOptions[2]"),
        FocusAfter(Action::MenuDown, "MenuOptions[4]"),
        Press(Action::MenuAccept), // OPTIONS
        Wait(0.2),
        Check("OPTIONS opens the options page", |w| match page(w) {
            Page::Options => Ok("options".into()),
            p => Err(format!("{p:?}")),
        }),
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
        Check("Back returns to the original main menu", |w| match page(w) {
            Page::Main if w.resource::<OriginalMenus>().active() => Ok("main".into()),
            p => Err(format!("{p:?}")),
        }),
        FocusAfter(Action::MenuDown, "MenuOptions[7]"),
        Press(Action::MenuAccept), // QUIT
        Wait(0.3),
        Check("QUIT asks the original's question, on NO", |w| {
            let m = w.resource::<OriginalMenus>();
            let q = m.question.as_ref().ok_or("no question box")?;
            match q.focused_name() {
                Some("BButton") => Ok("question box up, focus on NO (BButton)".into()),
                f => Err(format!("focus on {f:?}")),
            }
        }),
        Shot("quit_question"),
        Check("question box matches the original's pictures", |w| {
            // Mostly dark: the question, title, two buttons and the
            // background lines.
            compare(w, "quit_question", 0.03)
        }),
        Press(Action::MenuAccept), // NO
        Wait(0.2),
        Check("NO closes the question and stays", |w| {
            let m = w.resource::<OriginalMenus>();
            if m.question.is_none() && in_state(w, GameState::MainMenu) {
                Ok("back on the main menu".into())
            } else {
                Err("question still up".into())
            }
        }),
        Press(Action::MenuAccept), // QUIT again
        Wait(0.2),
        Press(Action::MenuRight),
        Check("right moves to YES", |w| {
            let m = w.resource::<OriginalMenus>();
            match m.question.as_ref().and_then(|q| q.focused_name()) {
                Some("AButton") => Ok("YES (AButton)".into()),
                f => Err(format!("focus on {f:?}")),
            }
        }),
        Summary,
        Press(Action::MenuAccept), // YES: the game should close now.
        Wait(5.0),
        Check("YES closes the game", |_| {
            Err("still running 5 s later".into())
        }),
    ]
}

fn focus(w: &mut World) -> Option<String> {
    w.resource::<OriginalMenus>()
        .main
        .as_ref()
        .and_then(|m| m.focused_name().map(str::to_string))
}

/// Compare the menu area of a screenshot with the same menu painted in
/// software straight from the original's textures (`rc_menus::render`).
/// Both are shrunk to 320x240 and compared pixel by pixel. `min_lit` is
/// how much of the reference must be lit, so that comparing two black
/// pictures can't pass.
fn compare(w: &mut World, name: &'static str, min_lit: f64) -> Result<String, String> {
    const W: usize = 320;
    const H: usize = 240;
    let shot = w
        .resource::<Shots>()
        .pictures
        .get(name)
        .cloned()
        .ok_or("no screenshot")?;
    let area = original_menu::area_for(Vec2::new(shot.width as f32, shot.height as f32));
    let ours = shrink(&shot, area, W, H);
    let mut menus = w.resource_mut::<OriginalMenus>();
    let reference =
        original_menu::software_picture(&mut menus, W, H).ok_or("no original menu")?;
    let mut total = 0.0f64;
    let mut worst = 0.0f64;
    let mut far = 0usize;
    for (a, b) in ours.pixels.chunks_exact(4).zip(reference.pixels.chunks_exact(4)) {
        let d = (0..3)
            .map(|k| (a[k] as f64 - b[k] as f64).abs())
            .sum::<f64>()
            / 3.0;
        total += d;
        worst = worst.max(d);
        if d > 40.0 {
            far += 1;
        }
    }
    let n = (W * H) as f64;
    let mean = total / n;
    let lit = reference
        .pixels
        .chunks_exact(4)
        .filter(|p| p[0].max(p[1]).max(p[2]) > 30)
        .count() as f64
        / n;
    let _ = std::fs::write(
        format!("logs/{name}_reference.png"),
        rc_image::png::encode(&reference),
    );
    let msg = format!(
        "mean difference {mean:.1} of 255, {:.1}% of pixels differ by more than 40, reference {:.0}% lit (saved logs/{name}_reference.png)",
        far as f64 / n * 100.0,
        lit * 100.0
    );
    // Software and GPU filtering and blending differ a little; a wrong
    // texture, colour or position shows up as large differences.
    if mean < 12.0 && (far as f64 / n) < 0.05 && lit > min_lit {
        Ok(msg)
    } else {
        Err(msg)
    }
}

/// Cut `area` (in pixels) out of `img` and shrink it to `w` x `h` by
/// averaging.
fn shrink(img: &rc_image::Rgba, area: Rect, w: usize, h: usize) -> rc_image::Rgba {
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let x0 = area.min.x + area.width() * x as f32 / w as f32;
            let x1 = area.min.x + area.width() * (x + 1) as f32 / w as f32;
            let y0 = area.min.y + area.height() * y as f32 / h as f32;
            let y1 = area.min.y + area.height() * (y + 1) as f32 / h as f32;
            let mut sum = [0u32; 3];
            let mut n = 0u32;
            for sy in (y0 as usize)..(y1.ceil() as usize).min(img.height) {
                for sx in (x0 as usize)..(x1.ceil() as usize).min(img.width) {
                    let i = (sy * img.width + sx) * 4;
                    for (k, s) in sum.iter_mut().enumerate() {
                        *s += img.pixels[i + k] as u32;
                    }
                    n += 1;
                }
            }
            let o = (y * w + x) * 4;
            for k in 0..3 {
                out[o + k] = (sum[k] / n.max(1)) as u8;
            }
            out[o + 3] = 255;
        }
    }
    rc_image::Rgba {
        width: w,
        height: h,
        pixels: out,
    }
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
    let (mut amber, mut lit, mut centre) = (0u32, 0u32, 0u32);
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
    };
    let path = format!("logs/{name}.png");
    let picture = rc_image::Rgba {
        width: w,
        height: h,
        pixels: rgba,
    };
    let saved = std::fs::write(&path, rc_image::png::encode(&picture));
    shots.pictures.insert(name, picture);
    log.line(format!(
        "script: screenshot {name} {w}x{h}: {:.2}% amber, {:.1}% lit, {centre} amber at the centre; {}",
        stats.amber * 100.0,
        stats.lit * 100.0,
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
            Step::FocusAfter(a, want) => {
                if frames == 0 {
                    held.insert(a);
                    done = false;
                } else {
                    let f = focus(world);
                    let mut s = world.resource_mut::<Script>();
                    if f.as_deref() == Some(want) {
                        s.passed += 1;
                        log.line(format!("CHECK {} moves focus to {want} -> PASS", a.name()));
                    } else {
                        s.failed += 1;
                        log.line(format!(
                            "CHECK {} moves focus to {want}: got {f:?} -> FAIL",
                            a.name()
                        ));
                    }
                }
            }
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
            if matches!(
                step,
                Step::Press(_) | Step::Hold(..) | Step::Wait(_) | Step::FocusAfter(..)
            ) {
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
