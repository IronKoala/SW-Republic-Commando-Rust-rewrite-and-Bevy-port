//! `--script`: a fixed route through the game, in place of the keyboard and
//! mouse. It presses the same actions a player would (menu down, accept,
//! pause...), checks the result of each step, logs `CHECK ... PASS/FAIL`,
//! and exits with code 0 only if every check passed. The real mouse is
//! ignored, so moving it over the window doesn't change the route.
//!
//! The route: the original main menu (colours checked against the
//! original's pictures), CAMPAIGN MAP, Geonosis, its first level, loading,
//! the level's walls and sky (looking up), walk forward, pause (nothing
//! moves), resume, saving (the level-start checkpoint, a checkpoint, a
//! quicksave), walking away, quickload (the player is back where they
//! saved, and saving again gives the same numbers), the level exit (the
//! next map loads with the player's health and ammo, and its start is the
//! new checkpoint), quit to the main menu
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
use rc_game_core::save::{CheckpointReached, RestoringSave, SaveSlot};
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
    /// On the pause menu, press menu-down until the button is focused: on
    /// the original's page the one calling this function, on the
    /// placeholder the item at this index.
    FocusPauseItem(&'static str, usize),
    /// Press an action, then check which original menu button has focus.
    FocusAfter(Action, &'static str),
    /// On an original page, press menu-down until the widget with this
    /// name has focus.
    FocusOn(&'static str),
    /// Press and release a key on the keyboard itself (for choosing a new
    /// key, which reads the keyboard, not the actions).
    Key(KeyCode),
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
    /// Near-white pixels within 7% of the screen height of the centre
    /// (the original's reticle picture, which is white and grey).
    centre_white: u32,
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
    /// Squad mates' feet last frame, and for how many frames in a row none
    /// of them moved.
    squad_feet: Vec<Vec3>,
    squad_still_frames: u32,
    music_volume: f32,
    /// The player's centre and facing when the quicksave was made.
    saved: Option<(rc_movement::Vec3, f32)>,
    /// The player's health and magazine when walking into the level exit,
    /// and the map left.
    leaving: Option<(f32, f32, String)>,
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
        .add_systems(Update, run.in_set(GameSet::Input))
        .add_systems(
            PreUpdate,
            ignore_pointer
                .after(bevy::input::InputSystems)
                .before(bevy::ui::UiSystems::Focus),
        );
    }
}

/// The script alone drives the menus: a real mouse moving over the window
/// (or resting on a button) must not change what is selected, or the route
/// goes astray. So the menus never see where the pointer is or a click.
fn ignore_pointer(mut windows: Query<&mut Window>, mut mouse: ResMut<ButtonInput<MouseButton>>) {
    for mut w in &mut windows {
        if w.physical_cursor_position().is_some() {
            w.set_physical_cursor_position(None);
        }
    }
    mouse.reset_all();
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

fn turn(w: &mut World, by: f32) {
    let mut q = w.query_filtered::<&mut Body, With<Player>>();
    for mut b in q.iter_mut(w) {
        b.yaw += by;
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
        Check("level drawn with the crosshair at the centre", |w| {
            let s = shot(w, "in_game")?;
            // The original's white reticle picture when the install has
            // it, else the placeholder's amber bars.
            let original = crate::original_hud::reticle_showing(w);
            let ok = s.lit > 0.3
                && if original {
                    s.centre_white > 100
                } else {
                    s.centre_amber > 4
                };
            if ok {
                Ok(format!(
                    "{:.0}% lit, {}",
                    s.lit * 100.0,
                    if original {
                        format!(
                            "original reticle: {} white pixels at the centre",
                            s.centre_white
                        )
                    } else {
                        format!("{} amber pixels at the centre", s.centre_amber)
                    }
                ))
            } else {
                Err(format!("original reticle {original}: {s:?}"))
            }
        }),
        Check("the first-person arms hold the weapon in hand", |w| {
            let arms = w
                .get_resource::<rc_combat::ArmsReadout>()
                .cloned()
                .ok_or("no arms readout")?;
            let mut q = w.query_filtered::<&rc_combat::Arsenal, With<Player>>();
            let held = q.iter(w).next().and_then(|a| a.current()).map(|c| c.0.clone());
            let got = format!(
                "holding {held:?}; the arms show {:?} playing {}{}",
                arms.weapon,
                arms.animation,
                if arms.hidden { " (hidden)" } else { "" }
            );
            if held.is_some() && arms.weapon == held && !arms.hidden {
                Ok(got)
            } else {
                Err(got)
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
                if i.collision_triangles > 10_000
                    && i.props > 0
                    && i.class_props > 0
                    && i.has_player_start
                {
                    Ok(got)
                } else {
                    Err(got)
                }
            },
        ),
        Check("the map's doors and lifts run", |w| {
            let Some(m) = w.get_resource::<rc_movers_plugin::LevelMovers>() else {
                return Err("no LevelMovers".into());
            };
            let set = m.read();
            let got = format!(
                "{} movers ({} solid), {} triggers, clock {:.1} s",
                set.movers.len(),
                set.movers.iter().filter(|m| m.collision.is_some()).count(),
                set.active_triggers().count(),
                set.time
            );
            if !set.movers.is_empty() && set.time > 0.5 {
                Ok(got)
            } else {
                Err(got)
            }
        }),
        Check("the sky camera turns with the player's camera", |w| {
            let sky = count::<rc_level_plugin::SkyCamera>(w);
            let mut q = w
                .query_filtered::<&Camera, (With<PlayerCamera>, With<rc_level_plugin::SkyFollows>)>(
                );
            let follows = q
                .iter(w)
                .filter(|c| matches!(c.clear_color, ClearColorConfig::None))
                .count();
            if sky == 1 && follows == 1 {
                Ok("1 sky camera; the player's camera lets it through".into())
            } else {
                Err(format!(
                    "{sky} sky cameras, {follows} player cameras following"
                ))
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
        // Turn round to where the squad stands (behind the player) and
        // capture logs/squad.png for a person to look at. The squad's
        // colour check is rc_squad_look_check, which sees them up close.
        Note(|w| turn(w, std::f32::consts::PI)),
        Wait(0.5),
        Shot("squad"),
        Check(
            "the squad mates wear their original models and armour",
            |w| {
                let mut q = w.query::<(
                    &rc_squad_plugin::SquadMate,
                    &rc_squad_plugin::look::MateModel,
                )>();
                let mut got: Vec<String> = q
                    .iter(w)
                    .map(|(m, mm)| {
                        let skins: Vec<&str> = mm
                            .model
                            .sections
                            .iter()
                            .map(|s| s.skin.as_ref().map_or("none", |k| k.texture.as_str()))
                            .collect();
                        format!("{} {} {}", m.name, mm.model.name, skins.join("+"))
                    })
                    .collect();
                got.sort();
                let mates = count::<rc_squad_plugin::SquadMate>(w);
                let all_skinned = got.iter().all(|g| !g.contains("none"));
                if mates == 3 && got.len() == 3 && all_skinned {
                    Ok(got.join(", "))
                } else {
                    Err(format!("{mates} squad mates, with models: {got:?}"))
                }
            },
        ),
        Note(|w| turn(w, std::f32::consts::PI)),
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
        // The level's own scripts (see level_script_* below): geo_01a's
        // landing-zone barricade opens once the player heals at the bacta
        // dispenser the scripts switch on.
        Check("the level's scripts run", level_script_running),
        Until(
            "the scripts start playing the level's lines",
            |w| level_script_lines(w).is_ok(),
            60.0,
        ),
        Check(
            "the scripts play the game's own lines, as long as each one lasts",
            level_script_lines,
        ),
        Check("the start barricade blocks the way", |w| level_script_way(w, true)),
        Until("the scripts switch the bacta dispenser on", level_script_dispenser_on, 300.0),
        Note(level_script_stand_at_dispenser),
        Wait(0.5),
        Hold(Action::Use, 5.0),
        Until("the barricade opens", level_script_barricade_gone, 60.0),
        Wait(3.0),
        Check(
            "the barricade is gone: not solid, not drawn, the way through is open",
            |w| level_script_way(w, false),
        ),
        Check(
            "the scripts' events, damage, healing and checkpoint",
            level_script_chain,
        ),
        // Enemy spawners run by the level's events (rc_enemies): send the
        // event that starts geo_01a's small-bunker droids, as the map's own
        // trigger would later, then destroy them: each sends its death
        // event back to the scripts.
        Check(
            "no enemy spawner started before its event",
            level_spawn_none_yet,
        ),
        Note(level_spawn_fire),
        Until(
            "the level's event switches the bunker spawners on",
            level_spawn_appeared,
            10.0,
        ),
        Check("the droids stand at their spawners", level_spawn_where),
        Note(level_spawn_destroy),
        Until(
            "each destroyed droid's event reaches the scripts",
            level_spawn_deaths_heard,
            10.0,
        ),
        Check("spawners and scripts agree", level_spawn_summary),
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
            // The level's own checkpoints (geo_01a's first comes 19 s in)
            // may have saved over it since; the start is then the first
            // checkpoint reached.
            let first = f
                .part("checkpoints")
                .and_then(|c| c["reached"].as_array()?.first()?.as_str().map(String::from));
            let got = format!(
                "{} at {:?}, first checkpoint {first:?}",
                f.header.map, f.header.checkpoint
            );
            if f.header.checkpoint.as_deref() == Some(rc_save::LEVEL_START)
                || first.as_deref() == Some(rc_save::LEVEL_START)
            {
                Ok(got)
            } else {
                Err(got)
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
        // Save while nobody is walking: a squad mate saved mid-step walks on
        // after the load, and the "same state" check below would see how far
        // they got.
        Until("squad mates standing still", squad_still, 15.0),
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
        Check("no droid spots the player just after the load", |w| {
            // Droids look for the player at the camera. It used to lag a
            // frame behind the body after a load, so droids could spot the
            // player from across the map.
            let quick = save_file(w, &SaveSlot::Quick)?;
            let again = rc_save::collect(w, None)?;
            let before = droid_modes(quick.parts.get("enemies"));
            let after = droid_modes(again.parts.get("enemies"));
            let got = format!("droids saved {before:?}, now {after:?}");
            let woke = before
                .iter()
                .zip(&after)
                .any(|(b, a)| b == "idle" && a != "idle");
            if woke || before.len() != after.len() {
                Err(got)
            } else {
                Ok(got)
            }
        }),
        Check("quickload gives back the ammo used before saving", |w| {
            let quick = save_file(w, &SaveSlot::Quick)?;
            let c = quick.part("combat").ok_or("no [combat] section")?;
            let saved = c["weapons"]
                .as_array()
                .and_then(|a| a.first())
                .ok_or("no weapons")?;
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
            let got = format!(
                "{id}: {:.0} in the magazine now, {saved_clip:.0} saved, {full:.0} when full",
                now.clip
            );
            if (now.clip - saved_clip).abs() < 0.01 && saved_clip < full {
                Ok(got)
            } else {
                Err(got)
            }
        }),
        Check(
            "quickload keeps the scripts' progress: no level start again, barricade still gone",
            level_script_after_load,
        ),
        Check("quickload puts the doors and lifts back", movers_after_load),
        Check("saving again after the load gives the same state", |w| {
            let quick = save_file(w, &SaveSlot::Quick)?;
            let again = rc_save::collect(w, None)?;
            let mut notes = vec![];
            for (k, v) in &quick.parts {
                let Some(b) = again.parts.get(k) else {
                    return Err(format!("[{k}] missing after the load"));
                };
                // Doors, lifts and scripts run on clocks (looping movers,
                // random timers), so a few frames later their numbers
                // differ; the two checks above compare what matters.
                if matches!(k.as_str(), "movers" | "scripts") {
                    notes.push(format!("[{k}] checked above"));
                    continue;
                }
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
        // The level exit: geo_01a's arrows lead to geo_01b.
        Check("the level has an open exit to the next map", level_exit_found),
        Note(level_exit_walk_in),
        Until(
            "the next map loads",
            |w| {
                in_state(w, GameState::InGame)
                    && w.resource::<CurrentLevel>().map.as_deref() == Some("geo_01b")
                    && count::<Player>(w) == 1
            },
            LOAD_TIMEOUT,
        ),
        Wait(1.0),
        Check("the next map has the player's health and ammo", level_exit_carried),
        Check("the player arrives at the next map's start", level_exit_arrived),
        Check("the next map's start is saved as a checkpoint, with the health", level_exit_checkpoint),
        Press(Action::Pause),
        Wait(0.2),
        Check(
            "the pause menu is the original's when the install has it",
            |w| {
                let m = w.resource::<OriginalMenus>();
                match (m.active(), m.pages.contains_key("paused")) {
                    (true, true) => Ok("original CTPausePCMenu".into()),
                    (false, _) => Ok("no original menus: placeholder".into()),
                    (true, false) => Err("placeholder, see the log for why".into()),
                }
            },
        ),
        Shot("paused"),
        FocusPauseItem("EndMissionSelected", 2),
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
        Check(
            "the original main menu is back, focus on NEW GAME",
            |w| match (w.resource::<OriginalMenus>().active(), focus(w)) {
                (true, Some(f)) if f == "MenuOptions[0]" => Ok(f),
                other => Err(format!("{other:?}")),
            },
        ),
        Note(|w| {
            // Each run saves the lowered volume to the script's own settings
            // file; start from full so it never runs down to zero.
            w.resource_mut::<Settings>().audio.music_volume = 1.0;
            w.resource_mut::<Noted>().music_volume = 1.0;
        }),
        // MULTIPLAYER, LOAD GAME, PROFILES and EXTRAS are greyed out and
        // skipped.
        FocusAfter(Action::MenuDown, "MenuOptions[2]"),
        FocusAfter(Action::MenuDown, "MenuOptions[4]"),
        Press(Action::MenuAccept), // OPTIONS
        Wait(0.3),
        Check("OPTIONS opens the original GAME options page", |w| {
            match (page(w), original_page_shown(w)) {
                (Page::Options, Some(k)) if k == "options" => Ok("options (original)".into()),
                other => Err(format!("{other:?}")),
            }
        }),
        Check(
            "mouse sensitivity shows the setting (default 6)",
            |w| match button_text(w, "Options[0]") {
                Some(t) if t == "6" => Ok(t),
                other => Err(format!("{other:?}")),
            },
        ),
        Shot("options_game"),
        Check("GAME options page matches the original's pictures", |w| {
            compare(w, "options_game", 0.03)
        }),
        FocusOn("Sound"),
        Press(Action::MenuAccept), // SOUND
        Wait(0.3),
        Check(
            "SOUND switches to the original SOUND page",
            |w| match page(w) {
                Page::OptionsSound => Ok("options_sound".into()),
                p => Err(format!("{p:?}")),
            },
        ),
        FocusOn("Options[2]"),   // MUSIC VOLUME
        Press(Action::MenuLeft), // music volume down a step
        Wait(0.2),
        Check("left lowers the music volume", |w| {
            let before = w.resource::<Noted>().music_volume;
            let now = w.resource::<Settings>().audio.music_volume;
            let shown = button_text(w, "Options[2]").unwrap_or_default();
            if now < before && shown == format!("{:.0}", now * 10.0) {
                Ok(format!("{before:.1} -> {now:.1}, shows {shown}"))
            } else {
                Err(format!("{before:.1} -> {now:.1}, shows {shown}"))
            }
        }),
        Check("greyed-out rows can't be reached (VOICES, EAX...)", |w| {
            let focusable = original_focusable(w);
            if focusable
                .iter()
                .any(|n| n == "Options[0]" || n == "Options[5]")
            {
                Err(format!("{focusable:?}"))
            } else {
                Ok(format!("{} buttons reachable", focusable.len()))
            }
        }),
        // Only the arrows (and left and right) change a value.
        Note(|w| {
            let v = w.resource::<Settings>().audio.music_volume;
            w.resource_mut::<Noted>().music_volume = v;
        }),
        FocusOn("Options[2]"),
        Press(Action::MenuAccept),
        Wait(0.2),
        Check(
            "choosing a value doesn't change it (only the arrows do)",
            |w| {
                let before = w.resource::<Noted>().music_volume;
                let now = w.resource::<Settings>().audio.music_volume;
                if now == before {
                    Ok(format!("music volume stays {now:.1}"))
                } else {
                    Err(format!("{before:.1} -> {now:.1}"))
                }
            },
        ),
        // The screen size's value opens a drop-down list.
        FocusOn("Graphics"),
        Press(Action::MenuAccept), // GRAPHICS
        Wait(0.3),
        FocusOn("Options[2]"), // VIDEO RESOLUTION
        Press(Action::MenuAccept),
        Wait(0.3),
        Check("choosing the screen size opens its drop-down list", |w| {
            let v = w.resource::<Settings>().video.clone();
            let m = w.resource::<OriginalMenus>();
            let l = m.list.as_ref().ok_or("no list")?;
            let at = l.focused_item().map(|i| l.items[i].clone());
            let now = format!("{}x{}", v.width, v.height);
            if at.as_deref() == Some(now.as_str()) {
                Ok(format!("{} sizes, on {now}", l.items.len()))
            } else {
                Err(format!("{} sizes, focus {at:?}, size {now}", l.items.len()))
            }
        }),
        Shot("options_list"),
        Press(Action::MenuDown),
        Wait(0.2),
        Press(Action::MenuBack),
        Wait(0.3),
        Check("Back closes the list without changing the size", |w| {
            let v = w.resource::<Settings>().video.clone();
            let open = w.resource::<OriginalMenus>().list.is_some();
            let shown = button_text(w, "Options[2]").unwrap_or_default();
            let now = format!("{}x{}", v.width, v.height);
            if !open && shown.eq_ignore_ascii_case(&now) {
                Ok(format!("closed, still {now}"))
            } else {
                Err(format!("open {open}, shows {shown}, size {now}"))
            }
        }),
        // The port's own settings, in the original GAME page's look.
        FocusOn("PortSettings"),
        Press(Action::MenuAccept), // PORT +
        Wait(0.3),
        Check(
            "PORT + opens the port's settings in the original's look",
            |w| {
                let fov = w.resource::<Settings>().video.fov;
                match (
                    page(w),
                    original_page_shown(w),
                    button_text(w, "Options[0]"),
                ) {
                    (Page::MoreSettings, Some(k), Some(t))
                        if k == "more_settings" && t == format!("{fov:.0}") =>
                    {
                        Ok(format!("original look, FIELD OF VIEW shows {t}"))
                    }
                    other => Err(format!("{other:?}")),
                }
            },
        ),
        Shot("options_port"),
        FocusOn("Controls"),
        Press(Action::MenuAccept), // CONTROLS
        Wait(0.3),
        Check("CONTROLS shows each action's keys", |w| {
            match (page(w), button_text(w, "OptionSet0[7]")) {
                (Page::OptionsControls, Some(t)) if t.contains("SPACE") => {
                    Ok(format!("JUMP is {t}"))
                }
                other => Err(format!("{other:?}")),
            }
        }),
        Shot("options_controls"),
        Check("CONTROLS page matches the original's pictures", |w| {
            compare(w, "options_controls", 0.03)
        }),
        FocusOn("OptionSet0[7]"), // JUMP
        Press(Action::MenuAccept),
        Wait(0.3),
        Check(
            "choosing JUMP opens the original reassign box",
            |w| match &w.resource::<OriginalMenus>().rebind {
                Some(r) if !r.chosen && r.action == Action::Jump => Ok("waiting for a key".into()),
                Some(r) => Err(format!("{:?} chosen={}", r.action, r.chosen)),
                None => Err("no box".into()),
            },
        ),
        Key(KeyCode::KeyK),
        Wait(0.3),
        Check("pressing K makes K the jump key", |w| {
            let chosen = w
                .resource::<OriginalMenus>()
                .rebind
                .as_ref()
                .map(|r| r.chosen);
            match (chosen, keys_of(w, Action::Jump).as_slice()) {
                (Some(true), [KeyCode::KeyK]) => Ok("K, ACCEPT/CANCEL/CLEAR showing".into()),
                other => Err(format!("{other:?}")),
            }
        }),
        Press(Action::MenuBack), // CANCEL
        Wait(0.3),
        Check("CANCEL puts SPACE back and closes the box", |w| {
            let open = w.resource::<OriginalMenus>().rebind.is_some();
            match (open, keys_of(w, Action::Jump).as_slice()) {
                (false, [KeyCode::Space]) => Ok("Space".into()),
                other => Err(format!("{other:?}")),
            }
        }),
        Press(Action::MenuBack), // DONE
        Wait(0.3),
        Check("Back returns to the original main menu", |w| {
            match page(w) {
                Page::Main if w.resource::<OriginalMenus>().active() => Ok("main".into()),
                p => Err(format!("{p:?}")),
            }
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

fn original_page_shown(w: &mut World) -> Option<String> {
    let state = *w.resource::<State<GameState>>().get();
    let play = w.get_resource::<State<Play>>().map(|p| *p.get());
    let menu = w.resource::<Menu>();
    match w
        .resource::<OriginalMenus>()
        .shown(&state, play.as_ref(), menu)
    {
        Some(original_menu::Shown::Page(k)) => Some(k),
        _ => None,
    }
}

/// The buttons keys and controllers can reach on the original page
/// showing.
fn original_focusable(w: &mut World) -> Vec<String> {
    let Some(k) = original_page_shown(w) else {
        return vec![];
    };
    let m = w.resource::<OriginalMenus>();
    m.pages[&k].screen.reachable_names()
}

fn pause_item_focused(w: &mut World, func: &str, index: usize) -> bool {
    let state = *w.resource::<State<GameState>>().get();
    let play = w.get_resource::<State<Play>>().map(|p| *p.get());
    let menu = w.resource::<Menu>();
    let m = w.resource::<OriginalMenus>();
    match m.shown(&state, play.as_ref(), menu) {
        Some(original_menu::Shown::Page(k)) => {
            let screen = &m.pages[&k].screen;
            screen.focus.is_some_and(|i| screen.on_select(i) == func)
        }
        _ => menu.selected == index,
    }
}

/// The focused button of the original screen showing: a box over the
/// page (question, reassign button), else the page, else the main menu.
fn focus(w: &mut World) -> Option<String> {
    let state = *w.resource::<State<GameState>>().get();
    let play = w.get_resource::<State<Play>>().map(|p| *p.get());
    let menu = w.resource::<Menu>();
    let m = w.resource::<OriginalMenus>();
    if let Some(q) = &m.question {
        return q.focused_name().map(str::to_string);
    }
    if let Some(r) = &m.rebind {
        return r.screen.focused_name().map(str::to_string);
    }
    if let Some(l) = &m.list {
        return l.screen.focused_name().map(str::to_string);
    }
    match m.shown(&state, play.as_ref(), menu) {
        Some(original_menu::Shown::Page(k)) => {
            m.pages[&k].screen.focused_name().map(str::to_string)
        }
        _ => m
            .main
            .as_ref()
            .and_then(|s| s.focused_name().map(str::to_string)),
    }
}

/// The text a button on the original page showing has now.
fn button_text(w: &mut World, name: &str) -> Option<String> {
    let state = *w.resource::<State<GameState>>().get();
    let play = w.get_resource::<State<Play>>().map(|p| *p.get());
    let menu = w.resource::<Menu>();
    let m = w.resource::<OriginalMenus>();
    let Some(original_menu::Shown::Page(k)) = m.shown(&state, play.as_ref(), menu) else {
        return None;
    };
    match &m.pages[&k].screen.layout.widget(name)?.kind {
        rc_menus::layout::Kind::Button(b) => Some(b.blurred.text.clone()),
        _ => None,
    }
}

/// The keys bound to an action on the keyboard.
fn keys_of(w: &mut World, action: Action) -> Vec<KeyCode> {
    w.resource::<crate::input::Bindings>()
        .keys
        .iter()
        .filter(|(a, _)| *a == action)
        .map(|(_, k)| *k)
        .collect()
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
    let reference = original_menu::software_picture(&mut menus, W, H).ok_or("no original menu")?;
    let mut total = 0.0f64;
    let mut worst = 0.0f64;
    let mut far = 0usize;
    for (a, b) in ours
        .pixels
        .chunks_exact(4)
        .zip(reference.pixels.chunks_exact(4))
    {
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
    let (mut amber, mut lit, mut centre, mut white) = (0u32, 0u32, 0u32, 0u32);
    let r = (h as f32 * 0.03) as i64;
    let r_white = (h as f32 * 0.07) as i64;
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
        let min = pr.min(pg).min(pb);
        if min > 170
            && pr.max(pg).max(pb) - min < 40
            && (x - w as i64 / 2).abs() <= r_white
            && (y - h as i64 / 2).abs() <= r_white
        {
            white += 1;
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
        centre_white: white,
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

/// True once no squad mate has moved for a few frames in a row.
fn squad_still(w: &mut World) -> bool {
    let mut q = w.query_filtered::<&Transform, With<rc_squad_plugin::SquadMate>>();
    let feet: Vec<Vec3> = q.iter(w).map(|t| t.translation).collect();
    let mut n = w.resource_mut::<Noted>();
    // Bevy units are metres-ish; 0.01 is about half an Unreal unit.
    let moved = feet.len() != n.squad_feet.len()
        || feet
            .iter()
            .zip(&n.squad_feet)
            .any(|(a, b)| a.distance(*b) > 0.01);
    n.squad_still_frames = if moved { 0 } else { n.squad_still_frames + 1 };
    n.squad_feet = feet;
    n.squad_still_frames >= 3
}

/// Each droid's mode ("idle", "alerted", ...) in an `[enemies]` section.
fn droid_modes(section: Option<&toml::Value>) -> Vec<String> {
    let droids = section
        .and_then(|e| e.get("droids"))
        .and_then(|d| d.as_array());
    droids
        .map(|a| {
            a.iter()
                .map(|d| d.get("mode").and_then(|m| m.as_str()).unwrap_or("?"))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
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
            Step::FocusPauseItem(func, index) => {
                if !pause_item_focused(world, func, index) {
                    // Down on even frames, released on odd ones.
                    if frames % 2 == 0 {
                        held.insert(Action::MenuDown);
                    }
                    done = frames > 200;
                    if done {
                        log.line(format!(
                            "CHECK focus {func} on the pause menu: not reached -> FAIL"
                        ));
                        world.resource_mut::<Script>().failed += 1;
                    }
                }
            }
            Step::FocusOn(want) => {
                if focus(world).as_deref() != Some(want) {
                    if frames % 2 == 0 {
                        held.insert(Action::MenuDown);
                    }
                    done = frames > 200;
                    if done {
                        log.line(format!("CHECK focus reaches {want}: not reached -> FAIL"));
                        world.resource_mut::<Script>().failed += 1;
                    }
                }
            }
            Step::Key(key) => {
                use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
                use bevy::input::ButtonState;
                let window = world
                    .query_filtered::<Entity, With<bevy::window::PrimaryWindow>>()
                    .iter(world)
                    .next();
                if let (Some(window), true) = (window, frames < 2) {
                    world.write_message(KeyboardInput {
                        key_code: key,
                        logical_key: Key::Unidentified(NativeKey::Unidentified),
                        state: if frames == 0 {
                            ButtonState::Pressed
                        } else {
                            ButtonState::Released
                        },
                        text: None,
                        repeat: false,
                        window,
                    });
                }
                done = frames >= 2;
            }
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
                Step::Press(_)
                    | Step::Hold(..)
                    | Step::Wait(_)
                    | Step::FocusAfter(..)
                    | Step::Key(_)
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

// Level scripts (`rc_scripts_plugin`), checked on geo_01a.

const BARRICADE_VOLUME: &str = "BactaBarricadeBlockingVolume";
const BARRICADE_PROP: &str = "PathToGunshipBarricade";

fn level_script_running(w: &mut World) -> Result<String, String> {
    let Some(s) = w.get_resource::<rc_scripts_plugin::LevelScript>() else {
        return Err("no LevelScript".into());
    };
    let d = &s.director;
    let got = format!(
        "{} scripted triggers, {} sequences, {} actions; {} events sent so far, first {:?}",
        d.stats.scripts,
        d.stats.sequences,
        d.stats.actions,
        d.stats.events_sent,
        d.history.first().map(|h| &h.1)
    );
    if d.stats.scripts > 10
        && d.history
            .iter()
            .any(|h| h.1.eq_ignore_ascii_case("levelstart"))
    {
        Ok(got)
    } else {
        Err(got)
    }
}

/// The level's lines and sounds (`rc_scripts_plugin::sounds`): every sound
/// geo_01a's scripts name was read from the game's packages with its
/// length, at least two lines have played and reached the sound card, and
/// nothing asked for was missing.
fn level_script_lines(w: &mut World) -> Result<String, String> {
    let Some(sounds) = w.get_resource::<rc_scripts_plugin::ScriptSounds>() else {
        return Err("no ScriptSounds (sounds not read)".into());
    };
    let (read, measured, missing, packages) = (
        sounds.sounds.len(),
        sounds.lengths.len(),
        sounds.missing.len(),
        format!("{:?}", sounds.packages),
    );
    let Some(s) = w.get_resource::<rc_scripts_plugin::LevelScript>() else {
        return Err("no LevelScript".into());
    };
    let st = &s.stats;
    let line = st
        .sounds_played
        .iter()
        .find(|n| sounds.speech.contains(&n.to_ascii_lowercase()))
        .cloned()
        .unwrap_or_default();
    let got = format!(
        "{read} sounds read from {packages}, {measured} measured, {missing} missing; played {:?}, {} started on the sound card, {} not loaded; first line of speech {line} lasts {:.1} s",
        st.sounds_played,
        st.sounds_started,
        st.sounds_missing.len(),
        s.director.sound_length(&line),
    );
    if read > 50
        && missing == 0
        && !line.is_empty()
        && st.sounds_played.len() >= 2
        && st.sounds_started >= 2
        && st.sounds_missing.is_empty()
    {
        Ok(got)
    } else {
        Err(got)
    }
}

/// Sweep a small box straight through the barricade, across the blocking
/// volume's thin side, where the prop the scripts blow up stands (the
/// barricade's other piece, `Blockade02`, stays, as in the original).
/// `blocked`: whether it should stop.
fn level_script_way(w: &mut World, blocked: bool) -> Result<String, String> {
    let Some(parts) = w.get_resource::<rc_level_plugin::LevelParts>().cloned() else {
        return Err("no LevelParts".into());
    };
    let Some((lo, hi)) = parts.bounds(BARRICADE_VOLUME).first().copied() else {
        return Err(format!("no {BARRICADE_VOLUME} in the level parts"));
    };
    let size = hi - lo;
    let Some((plo, phi)) = parts.bounds(BARRICADE_PROP).first().copied() else {
        return Err(format!("no {BARRICADE_PROP} in the level parts"));
    };
    let c = (plo + phi) * 0.5;
    let along_x = size.x < size.y;
    let reach = if along_x { size.x } else { size.y } * 0.5 + 60.0;
    let dir = if along_x {
        rc_movement::Vec3::new(1.0, 0.0, 0.0)
    } else {
        rc_movement::Vec3::new(0.0, 1.0, 0.0)
    };
    let (a, b) = (c - dir * reach, c + dir * reach);
    let ext = rc_movement::Vec3::new(10.0, 10.0, 10.0);
    let world = w.resource::<LevelCollision>().0.clone();
    let hit = world.sweep(ext, a, b);
    let solid: Vec<bool> = parts
        .list()
        .into_iter()
        .filter(|p| p.2.eq_ignore_ascii_case(BARRICADE_VOLUME))
        .map(|p| p.3)
        .collect();
    let mut hidden = 0;
    let mut drawn = 0;
    let props: Vec<usize> = parts
        .list()
        .into_iter()
        .filter(|p| p.2.eq_ignore_ascii_case(BARRICADE_PROP))
        .map(|p| p.0)
        .collect();
    let mut q = w.query::<(&rc_level_viewer::draw::MoverPart, &Visibility)>();
    for (p, v) in q.iter(w) {
        if props.contains(&p.0) {
            drawn += 1;
            hidden += (*v == Visibility::Hidden) as usize;
        }
    }
    let got = format!(
        "{BARRICADE_VOLUME} {:.0}x{:.0}x{:.0}, solid {solid:?}; sweep through it at {BARRICADE_PROP} ({:.0}, {:.0}, {:.0}) {}; {BARRICADE_PROP}: {hidden} of {drawn} drawn parts hidden",
        size.x,
        size.y,
        size.z,
        c.x,
        c.y,
        c.z,
        match hit {
            Some(h) => format!("stops at {:.0}%", h.time * 100.0),
            None => "passes".into(),
        }
    );
    let ok = if blocked {
        hit.is_some() && solid.iter().all(|s| *s) && hidden == 0
    } else {
        // Something else may lie beyond (the floor slopes); the volume
        // itself must no longer be what stops the box near its middle.
        hit.is_none_or(|h| h.time > 0.5) && solid.iter().all(|s| !*s) && hidden == drawn
    };
    if ok {
        Ok(got)
    } else {
        Err(got)
    }
}

/// After the quickload: the scripts carried on from the save. The level
/// start isn't sent again (the director is rebuilt at the load, so its
/// history only holds what was sent since), the barricade blown up before
/// the save is still gone, and every script finished in the save is still
/// finished.
fn level_script_after_load(w: &mut World) -> Result<String, String> {
    let quick = save_file(w, &SaveSlot::Quick)?;
    let saved = quick
        .part("scripts")
        .ok_or("no [scripts] section in the quicksave")?;
    let saved_finished: Vec<String> = saved
        .get("director")
        .and_then(|d| d.get("script"))
        .and_then(|s| s.as_array())
        .ok_or("no scripts in [scripts.director]")?
        .iter()
        .filter(|s| s.get("finished").and_then(|f| f.as_bool()) == Some(true))
        .filter_map(|s| s.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .collect();
    let gone = level_script_barricade_gone(w);
    let way = level_script_way(w, false);
    let Some(s) = w.get_resource::<rc_scripts_plugin::LevelScript>() else {
        return Err("no LevelScript".into());
    };
    let d = &s.director;
    let starts: Vec<&String> = d
        .history
        .iter()
        .map(|h| &h.1)
        .filter(|e| d.start_events().iter().any(|s| s.eq_ignore_ascii_case(e)))
        .collect();
    let states = d.script_states();
    let unfinished: Vec<&String> = saved_finished
        .iter()
        .filter(|n| {
            !states
                .iter()
                .any(|(name, _, what)| name == *n && what == "finished")
        })
        .collect();
    let got = format!(
        "clock {:.1} s; {} events since the load, level start sent {} times; {} scripts finished in the save, {} not finished now; barricade gone {gone}; {}",
        d.time,
        d.history.len(),
        starts.len(),
        saved_finished.len(),
        unfinished.len(),
        way.as_ref().unwrap_or_else(|e| e)
    );
    if starts.is_empty() && unfinished.is_empty() && gone && way.is_ok() {
        Ok(got)
    } else {
        Err(got)
    }
}

/// After the quickload: every mover the quicksave had standing still is at
/// the same key, and the movers' clock carried on from the save's.
fn movers_after_load(w: &mut World) -> Result<String, String> {
    let quick = save_file(w, &SaveSlot::Quick)?;
    let saved = quick
        .part("movers")
        .ok_or("no [movers] section in the quicksave")?;
    let saved_time = saved.get("time").and_then(|t| t.as_float()).unwrap_or(0.0) as f32;
    let list = saved
        .get("movers")
        .and_then(|m| m.as_array())
        .ok_or("no movers in [movers]")?
        .clone();
    let Some(m) = w.get_resource::<rc_movers_plugin::LevelMovers>() else {
        return Err("no LevelMovers".into());
    };
    let set = m.read();
    let (mut resting, mut same, mut open, mut wrong) = (0, 0, 0, Vec::new());
    for s in &list {
        let (Some(name), Some(key), Some(phase)) = (
            s.get("name").and_then(|v| v.as_str()),
            s.get("key").and_then(|v| v.as_integer()),
            s.get("phase").and_then(|v| v.as_str()),
        ) else {
            return Err(format!("unreadable mover {s}"));
        };
        if phase != "rest" {
            continue;
        }
        resting += 1;
        open += (key > 0) as usize;
        match set.movers.iter().find(|m| m.info.place.name == name) {
            Some(m) if m.key as i64 == key => same += 1,
            Some(m) => wrong.push(format!("{name} key {key} saved, {} now", m.key)),
            None => wrong.push(format!("{name} missing")),
        }
    }
    let got = format!(
        "{} movers saved, {resting} standing still ({open} of them open), {same} at the same key now; clock {saved_time:.1} s saved, {:.1} s now{}",
        list.len(),
        set.time,
        if wrong.is_empty() { String::new() } else { format!("; {}", wrong.join(", ")) }
    );
    if !list.is_empty() && wrong.is_empty() && set.time >= saved_time && set.time < saved_time + 5.0
    {
        Ok(got)
    } else {
        Err(got)
    }
}

fn level_script_dispenser_on(w: &mut World) -> bool {
    let Some(s) = w.get_resource::<rc_scripts_plugin::LevelScript>() else {
        return false;
    };
    let told = s
        .director
        .history
        .iter()
        .any(|h| h.1.eq_ignore_ascii_case("StartBactaReminderTimer"));
    told && s
        .dispensers()
        .iter()
        .any(|d| d.tag.eq_ignore_ascii_case("BactaDispenserLZ") && d.enabled)
}

/// Put the player where a squad mate would stand to use the dispenser.
fn level_script_stand_at_dispenser(w: &mut World) {
    let Some(s) = w.get_resource::<rc_scripts_plugin::LevelScript>() else {
        return;
    };
    let Some(d) = s
        .dispensers()
        .into_iter()
        .find(|d| d.tag.eq_ignore_ascii_case("BactaDispenserLZ"))
    else {
        return;
    };
    let spot = d.stand.unwrap_or(d.location);
    let log = w.resource::<GameLog>().clone();
    let mut q = w.query_filtered::<&mut Body, With<Player>>();
    for mut b in q.iter_mut(w) {
        b.pawn.location = rc_movement::Vec3::new(spot[0], spot[1], spot[2] + 10.0);
        log.line(format!(
            "script: player moved to {}'s marker ({:.0}, {:.0}, {:.0})",
            d.name, spot[0], spot[1], spot[2]
        ));
    }
}

fn level_script_barricade_gone(w: &mut World) -> bool {
    w.get_resource::<rc_scripts_plugin::LevelScript>()
        .is_some_and(|s| {
            s.stats
                .props_destroyed
                .iter()
                .any(|p| p.eq_ignore_ascii_case(BARRICADE_PROP))
        })
}

fn level_script_chain(w: &mut World) -> Result<String, String> {
    let Some(s) = w.get_resource::<rc_scripts_plugin::LevelScript>() else {
        return Err("no LevelScript".into());
    };
    let h = &s.director.history;
    let when = |e: &str| h.iter().find(|x| x.1.eq_ignore_ascii_case(e)).map(|x| x.0);
    let chain = [
        "levelstart",
        "Rappel",
        "CheckpointLevelStart",
        "StartBactaTraining",
        "BactaDispenserActive",
        "BactaUsed",
        "RetreatToGammaSector",
        "DialogGetToGunship",
    ];
    let times: Vec<String> = chain
        .iter()
        .map(|e| format!("{e} {}", when(e).map_or("-".into(), |t| format!("{t:.1}s"))))
        .collect();
    let in_order = chain.windows(2).all(|p| match (when(p[0]), when(p[1])) {
        (Some(a), Some(b)) => a <= b,
        _ => false,
    });
    let got = format!(
        "{}; player hurt {:.0}, healed {:.0}; switched off {:?}; destroyed {:?}; checkpoints {:?}; {} actions not done yet",
        times.join(", "),
        s.stats.player_hurt,
        s.stats.bacta_healed,
        s.stats.parts_switched_off,
        s.stats.props_destroyed,
        s.stats.checkpoints,
        s.stats.not_done
    );
    if in_order
        && s.stats.player_hurt > 0.0
        && s.stats.bacta_healed > 0.0
        && s.stats.checkpoints.iter().any(|c| c == "checkpoint0")
    {
        Ok(got)
    } else {
        Err(got)
    }
}

// Enemy spawners started by the level's events, checked on geo_01a. Its
// small bunker's spawners (tag `SpawnSmBunkerDroids`) each make one battle
// droid at the spawner itself; three of them send `DeadGunnerDroid` when
// their droid is destroyed.

const SPAWN_EVENT: &str = "SpawnSmBunkerDroids";
const SPAWN_DEATH_EVENT: &str = "DeadGunnerDroid";

/// The bunker spawners, and the living droids they made: (id, feet,
/// spawner location).
fn bunker_droids(
    w: &World,
) -> Result<(usize, Vec<(u32, rc_movement::Vec3, rc_movement::Vec3)>), String> {
    let fight = w
        .get_resource::<rc_enemies::Fight>()
        .ok_or("no fight on this map")?;
    let b = &fight.battle;
    let spawners = b
        .spawners
        .iter()
        .filter(|s| s.factory.tag.eq_ignore_ascii_case(SPAWN_EVENT))
        .count();
    let droids = b
        .alive()
        .filter_map(|f| {
            let sp = &b.spawners[b.spawner_of(f.id)?];
            sp.factory
                .tag
                .eq_ignore_ascii_case(SPAWN_EVENT)
                .then(|| (f.id, f.pawn.feet(), sp.factory.location))
        })
        .collect();
    Ok((spawners, droids))
}

fn level_spawn_none_yet(w: &mut World) -> Result<String, String> {
    let (n, droids) = bunker_droids(w)?;
    let fight = w
        .get_resource::<rc_enemies::Fight>()
        .ok_or("no fight on this map")?;
    let b = &fight.battle;
    let on: Vec<&str> = b
        .spawners
        .iter()
        .filter(|s| s.active)
        .map(|s| s.factory.tag.as_str())
        .collect();
    let got = format!(
        "scripted {}; {n} bunker spawners, {} of their droids; spawners on {on:?}",
        b.scripted,
        droids.len()
    );
    if b.scripted && n >= 3 && droids.is_empty() {
        Ok(got)
    } else {
        Err(got)
    }
}

fn level_spawn_fire(w: &mut World) {
    if let Some(mut s) = w.get_resource_mut::<rc_scripts_plugin::LevelScript>() {
        s.director.fire(SPAWN_EVENT, "self-test");
    }
}

fn level_spawn_appeared(w: &mut World) -> bool {
    bunker_droids(w).is_ok_and(|(n, d)| n > 0 && d.len() >= n)
}

fn level_spawn_where(w: &mut World) -> Result<String, String> {
    let (_, droids) = bunker_droids(w)?;
    let off: Vec<f32> = droids
        .iter()
        .map(|(_, feet, at)| (*feet - *at).length_2d())
        .collect();
    let got = format!(
        "{} droids, each this far from its spawner: {off:.0?} uu",
        droids.len()
    );
    if !off.is_empty() && off.iter().all(|d| *d < 50.0) {
        Ok(got)
    } else {
        Err(got)
    }
}

fn level_spawn_destroy(w: &mut World) {
    let Ok((_, droids)) = bunker_droids(w) else {
        return;
    };
    let entities: Vec<Entity> = {
        let fight = w.resource::<rc_enemies::Fight>();
        droids
            .iter()
            .filter_map(|(id, ..)| fight.entities.get(id).copied())
            .collect()
    };
    for e in entities {
        w.write_message(rc_combat::Damage {
            target: e,
            amount: 100_000.0,
            kind: rc_combat::DamageKind::Other,
            instigator: None,
            point: Vec3::ZERO,
            direction: Vec3::NEG_Y,
        });
    }
}

fn deaths_heard(w: &mut World) -> usize {
    w.get_resource::<rc_scripts_plugin::LevelScript>()
        .map_or(0, |s| {
            s.director
                .history
                .iter()
                .filter(|h| h.1.eq_ignore_ascii_case(SPAWN_DEATH_EVENT))
                .count()
        })
}

fn level_spawn_deaths_heard(w: &mut World) -> bool {
    deaths_heard(w) >= 3
}

fn level_spawn_summary(w: &mut World) -> Result<String, String> {
    let heard = deaths_heard(w);
    let (n, alive) = bunker_droids(w)?;
    let fight = w.resource::<rc_enemies::Fight>();
    let made: u32 = fight
        .battle
        .spawners
        .iter()
        .filter(|s| s.factory.tag.eq_ignore_ascii_case(SPAWN_EVENT))
        .map(|s| s.made)
        .sum();
    let destroyed = fight.stats.droids_destroyed;
    let raised = w
        .get_resource::<rc_scripts_plugin::LevelScript>()
        .map(|s| s.stats.raised.clone())
        .unwrap_or_default();
    let got = format!(
        "{n} spawners made {made}, {} still alive, {destroyed} droids destroyed in all; the scripts heard {SPAWN_DEATH_EVENT} {heard} times; events from droids and spawners {raised:?}",
        alive.len()
    );
    if made as usize == n && alive.is_empty() && heard == 3 {
        Ok(got)
    } else {
        Err(got)
    }
}

/// The first open level exit: (name, URL, centre in Unreal units).
fn level_exit(w: &mut World) -> Option<(String, String, [f32; 3])> {
    let s = w.get_resource::<rc_scripts_plugin::LevelScript>()?;
    s.director
        .exits()
        .into_iter()
        .find(|e| e.3)
        .map(|(n, u, at, _)| (n, u, at))
}

fn level_exit_found(w: &mut World) -> Result<String, String> {
    let (name, url, at) = level_exit(w).ok_or("no open level exit")?;
    let got = format!("{name} at ({:.0}, {:.0}, {:.0}) to {url}", at[0], at[1], at[2]);
    match rc_scripts::level_url(&url) {
        Some((m, _)) if m == "geo_01b" => Ok(got),
        _ => Err(got),
    }
}

/// Note the health and magazine, take a little health off (so carrying it
/// over shows), and put the player in the exit.
fn level_exit_walk_in(w: &mut World) {
    let Some((name, _, at)) = level_exit(w) else {
        return;
    };
    let mut q = w.query_filtered::<(&mut rc_combat::Health, &rc_combat::Arsenal), With<Player>>();
    let Some((mut h, a)) = q.iter_mut(w).next() else {
        return;
    };
    h.current = (h.current - 13.0).max(1.0);
    let health = h.current;
    let clip = a.weapons.first().map_or(0.0, |(_, s)| s.clip);
    let map = w.resource::<CurrentLevel>().map.clone().unwrap_or_default();
    w.resource_mut::<Noted>().leaving = Some((health, clip, map));
    let mut q = w.query_filtered::<(&mut Body, &mut Transform), With<Player>>();
    for (mut b, mut t) in q.iter_mut(w) {
        b.pawn.location = rc_movement::Vec3::new(at[0], at[1], at[2]);
        b.pawn.velocity = rc_movement::Vec3::ZERO;
        t.translation = rc_game_core::units::to_bevy(b.pawn.feet());
    }
    w.resource::<GameLog>().line(format!(
        "script: into level exit {name} at ({:.0}, {:.0}, {:.0}) with health {health:.0}, {clip:.0} in the magazine",
        at[0], at[1], at[2]
    ));
}

fn level_exit_carried(w: &mut World) -> Result<String, String> {
    let (health, clip, from) = w
        .resource::<Noted>()
        .leaving
        .clone()
        .ok_or("didn't walk into an exit")?;
    let mut q = w.query_filtered::<(&rc_combat::Health, &rc_combat::Arsenal), With<Player>>();
    let (h, a) = q.iter(w).next().ok_or("no player")?;
    let now_clip = a.weapons.first().map_or(0.0, |(_, s)| s.clip);
    let got = format!(
        "{from} -> geo_01b: health {health:.0} -> {:.0} (of {:.0}), magazine {clip:.0} -> {now_clip:.0}",
        h.current, h.max
    );
    if (h.current - health).abs() < 0.5 && h.current < h.max && (now_clip - clip).abs() < 0.01 {
        Ok(got)
    } else {
        Err(got)
    }
}

fn level_exit_arrived(w: &mut World) -> Result<String, String> {
    let start = *w.get_resource::<rc_game_core::PlayerStart>().ok_or("no PlayerStart")?;
    let f = feet(w).ok_or("no player")?;
    let d = ((f.x - start.feet.x).powi(2) + (f.y - start.feet.y).powi(2)).sqrt();
    let got = format!(
        "feet ({:.0}, {:.0}, {:.0}), {d:.0} uu from the start ({:.0}, {:.0}, {:.0})",
        f.x, f.y, f.z, start.feet.x, start.feet.y, start.feet.z
    );
    // A second of standing (or the squad nudging) moves the player a little.
    if d < 200.0 {
        Ok(got)
    } else {
        Err(got)
    }
}

fn level_exit_checkpoint(w: &mut World) -> Result<String, String> {
    let (health, _, _) = w.resource::<Noted>().leaving.clone().ok_or("didn't leave")?;
    let cp = save_file(w, &SaveSlot::Checkpoint)?;
    let combat = cp.part("combat").ok_or("no [combat] in the checkpoint")?;
    let saved = combat
        .get("health")
        .and_then(|h| h.as_float())
        .ok_or("no health in the checkpoint")? as f32;
    // The new map's own list of checkpoints starts with its start. (Its
    // scripts may reach their first checkpoint right away: geo_01b's
    // "CheckpointLevelStart" does.)
    let reached: Vec<String> = cp
        .part("checkpoints")
        .and_then(|c| c.get("reached").and_then(|r| r.as_array()).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let got = format!(
        "map {}, checkpoint {:?}, reached {reached:?}, health {saved:.0}",
        cp.header.map, cp.header.checkpoint
    );
    if cp.header.map == "geo_01b"
        && reached.first().map(String::as_str) == Some(rc_save::LEVEL_START)
        && (saved - health).abs() < 0.5
    {
        Ok(got)
    } else {
        Err(got)
    }
}
