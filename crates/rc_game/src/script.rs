//! `--script`: a fixed route through the game, in place of the keyboard and
//! mouse. It presses the same actions a player would (menu down, accept,
//! pause...), checks the result of each step, logs `CHECK ... PASS/FAIL`,
//! and exits with code 0 only if every check passed. The real mouse is
//! ignored, so moving it over the window doesn't change the route.
//!
//! The route: the original main menu (colours checked against the
//! original's pictures), CAMPAIGN MAP (the original campaign page: its
//! missions, the planets left and right, EVERY MAP +), geo_01a, loading,
//! the level's walls and sky (looking up), walk forward, pause (nothing
//! moves), resume, saving (the level-start checkpoint, a checkpoint, a
//! quicksave), walking away, quickload (the player is back where they
//! saved, and saving again gives the same numbers), the level exit (the
//! next map loads with the player's health and ammo, and its start is the
//! new checkpoint), quit to the main menu
//! (the level is cleared), OPTIONS
//! (music volume down), back, QUIT, the original's "are you sure?" box (NO,
//! then QUIT again and YES).

use crate::menu::{Menu, Page};
use crate::original_credits;
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
    /// Type text on the keyboard, one key a frame, as the keyboard sends
    /// it (with each key's text); `\u{8}` is Backspace and `\n` Enter.
    Type(&'static str),
    /// Capture the window, save it to `logs/<name>.png` and measure its
    /// colours (retried while the capture comes back black).
    Shot(&'static str),
    /// Hold an action for some seconds, then keep holding it while the
    /// window is captured as in `Shot` (the next step starts in the same
    /// frame, so a following `Hold` of the same action never lets go).
    ShotHolding(&'static str, Action, f32),
    /// Log the result so far, and make the exit code an error if anything
    /// failed (before Quit closes the game with success).
    Summary,
    /// Put the mouse pointer somewhere (window pixels) with the left button
    /// down or up, worked out from the game as it is now; `None` gives the
    /// pointer back to nobody (the menus see no pointer again).
    Pointer(fn(&mut World) -> Option<(Vec2, bool)>),
}

/// Where the script holds the mouse pointer, if anywhere (see
/// `Step::Pointer`).
#[derive(Resource, Default)]
struct ScriptPointer(Option<(Vec2, bool)>);

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
    /// Near-white or pale blue pixels within 7% of the screen height of
    /// the centre (the original's reticle picture, white and grey, drawn
    /// with the helmet's light blue tint).
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
    /// The ammo box walked into (by name).
    ammo_box: Option<String>,
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
        .init_resource::<ScriptPointer>()
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
fn ignore_pointer(
    mut windows: Query<&mut Window>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    pointer: Res<ScriptPointer>,
) {
    // Except where the script itself points (dragging a scroll bar).
    if let Some((at, down)) = pointer.0 {
        for mut w in &mut windows {
            w.set_physical_cursor_position(Some(at.as_dvec2()));
        }
        if down && !mouse.pressed(MouseButton::Left) {
            mouse.press(MouseButton::Left);
        } else if !down {
            mouse.release_all();
        }
        return;
    }
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
            // The profiles page may already be over it (no profile yet).
            Page::Main | Page::Profiles => Ok(format!("{:?}", page(w))),
            p => Err(format!("{p:?}")),
        }),
        Wait(0.3),
        // No profile yet (the script's save folder starts empty): the
        // original PROFILES page opens first, and a profile is made.
        Check(
            "with no profile, the original PROFILES page opens first, BACK hidden",
            |w| {
                let items = save_items(w, "profiles")?;
                let back = button_hidden(w, "profiles", "BButton");
                let got = format!("{:?} {:?} {items:?}, BACK hidden {back:?}, focus {:?}", page(w), shown_page(w), focus(w));
                match (page(w), shown_page(w).as_deref(), back, focus(w).as_deref()) {
                    (Page::Profiles, Some("profiles"), Some(true), Some("ProfileList[0]"))
                        if items == ["< NEW >"] => Ok(got),
                    _ => Err(got),
                }
            },
        ),
        Shot("profiles_first"),
        Press(Action::MenuBack),
        Wait(0.2),
        Check("BACK does nothing while no profile is chosen", |w| {
            match (page(w), shown_page(w)) {
                (Page::Profiles, Some(k)) if k == "profiles" => Ok("still on profiles".into()),
                other => Err(format!("{other:?}")),
            }
        }),
        Press(Action::MenuAccept), // < NEW >
        Wait(0.3),
        Check("< NEW > opens the name page, focus in its empty box", |w| {
            let text = button_text(w, "NameBox");
            match (page(w), focus(w).as_deref(), text.as_deref()) {
                (Page::NewProfile, Some("NameBox"), Some("")) => Ok("new_profile, NameBox".into()),
                other => Err(format!("{other:?}")),
            }
        }),
        Type("Script 1\u{8}2!"),
        Wait(0.1),
        Check(
            "typing fills the box (Backspace takes one off, ! isn't a letter or digit)",
            |w| match button_text(w, "NameBox").as_deref() {
                Some("Script 2") => Ok("Script 2".into()),
                other => Err(format!("{other:?}")),
            },
        ),
        Check("the box's cursor is drawn after the text", |w| {
            let m = w.resource::<OriginalMenus>();
            let quads: Vec<_> = m.drawn.iter().filter(|q| q.widget == "NameBox").collect();
            let white = quads
                .iter()
                .filter(|q| q.texture.eq_ignore_ascii_case("Engine.MenuWhite"))
                .count();
            // The cursor blinks: it may be off in this frame, but the page
            // must have its picture.
            let picture = m
                .pages
                .get(crate::original_profiles::NEW_PROFILE)
                .and_then(|p| p.screen.layout.white_texture.clone());
            match picture {
                Some(p) => Ok(format!(
                    "{} pictures and letters for NameBox, {white} cursor ({p})",
                    quads.len()
                )),
                None => Err("the page has no WhiteTexture for the cursor".into()),
            }
        }),
        Shot("new_profile"),
        Type("\n"),
        Wait(0.1),
        Check("Enter leaves the box for OK", |w| match focus(w).as_deref() {
            Some("AButton") => Ok("AButton".into()),
            other => Err(format!("{other:?}")),
        }),
        Press(Action::MenuAccept), // OK
        Wait(0.2),
        Check("OK shows the original's message", |w| {
            let m = w.resource::<OriginalMenus>();
            let text = m
                .question
                .as_ref()
                .and_then(|q| q.layout.widget("Message"))
                .and_then(|w| match &w.kind {
                    rc_menus::layout::Kind::Text(t) => Some(t.text.clone()),
                    _ => None,
                });
            match text.as_deref() {
                Some("CREATING PROFILE: Script_2") => Ok("CREATING PROFILE: Script_2".into()),
                other => Err(format!("{other:?}")),
            }
        }),
        Until(
            "the main menu after the message",
            |w| page(w) == Page::Main && w.resource::<OriginalMenus>().question.is_none(),
            5.0,
        ),
        Check("the profile is made and in use, its saves in its own folder", |w| {
            let saves = w.resource::<rc_save::SaveFolder>().0.clone();
            let remembered = w.resource::<Settings>().save.profile.clone();
            let got = format!("saves in {}, settings profile {remembered:?}", saves.display());
            if saves.ends_with("Profile_Script_2") && saves.is_dir() && remembered == "Script_2" {
                Ok(got)
            } else {
                Err(got)
            }
        }),
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
        Check("the main menu's background video plays", |w| {
            let v = w.resource::<crate::menu_video::MenuVideo>();
            let movie = w
                .resource::<OriginalMenus>()
                .main
                .as_ref()
                .map(|m| m.layout.background_movie.clone())
                .unwrap_or_default();
            if movie.is_empty() {
                return Err("the main menu names no background video".into());
            }
            match &v.player {
                None => {
                    let path = crate::menu_video::avi_for(w.resource::<rc_game_core::GamePaths>(), &movie);
                    if path.exists() {
                        Err(format!("{} is there but didn't play (see the log)", path.display()))
                    } else {
                        Ok(format!("{movie:?}: no {} in this install, skipped", path.display()))
                    }
                }
                Some(p) if v.wanted && v.frames_shown >= 3 => Ok(format!(
                    "{movie:?}: {} frames shown, now frame {:?} of {}, {} frames let go late",
                    v.frames_shown,
                    v.last_frame,
                    p.frame_count(),
                    v.frames_late
                )),
                Some(_) => Err(format!(
                    "{movie:?}: only {} frames shown (drawn: {})",
                    v.frames_shown, v.wanted
                )),
            }
        }),
        Shot("main_menu"),
        Check("main menu matches the original's pictures", |w| {
            compare(w, "main_menu", 0.2)
        }),
        FocusAfter(Action::MenuDown, "MenuOptions[1]"),
        FocusAfter(Action::MenuUp, "MenuOptions[0]"),
        FocusAfter(Action::MenuDown, "MenuOptions[1]"),
        Press(Action::MenuAccept), // LOAD GAME
        Wait(0.3),
        Check(
            "LOAD GAME opens the original load page (no saves yet)",
            |w| {
                let items = save_items(w, "load_game")?;
                match (page(w), shown_page(w)) {
                    (Page::LoadGame, Some(k)) if k == "load_game" && items.is_empty() => {
                        Ok("load_game (original), empty".into())
                    }
                    (p, k) => Err(format!("{p:?}, original page {k:?}, {items:?}")),
                }
            },
        ),
        Press(Action::MenuBack),
        Wait(0.2),
        FocusAfter(Action::MenuDown, "MenuOptions[2]"),
        Press(Action::MenuAccept),
        Wait(0.2),
        Check(
            "CAMPAIGN MAP opens the original campaign page",
            |w| match (page(w), shown_page(w)) {
                (Page::Campaigns, Some(k)) if k == "campaigns" => Ok("campaigns (original)".into()),
                (p, k) => Err(format!("{p:?}, original page {k:?}")),
            },
        ),
        Wait(0.5),
        Check("Geonosis is chosen and its missions are listed", |w| {
            let (title, items) = campaign_list(w)?;
            let focus = focus(w);
            if title == "GEONOSIS"
                && items.first().map(String::as_str) == Some("PROLOGUE")
                && focus.as_deref() == Some("LevelList[0]")
            {
                Ok(format!("{title}: {items:?}, focus on {focus:?}"))
            } else {
                Err(format!("{title}: {items:?}, focus on {focus:?}"))
            }
        }),
        Shot("campaign"),
        Check("campaign page matches the original's pictures", |w| {
            compare(w, "campaign", 0.2)
        }),
        FocusAfter(Action::MenuDown, "LevelList[1]"),
        FocusAfter(Action::MenuUp, "LevelList[0]"),
        Press(Action::MenuRight),
        Wait(0.1),
        Check("right goes to the assault ship", |w| {
            let (title, items) = campaign_list(w)?;
            if title == "ASSAULT SHIP" && items.len() == 4 {
                Ok(format!("{title}: {items:?}"))
            } else {
                Err(format!("{title}: {items:?}"))
            }
        }),
        Press(Action::MenuLeft),
        Wait(0.1),
        Check("left comes back to Geonosis", |w| {
            let (title, _) = campaign_list(w)?;
            if title == "GEONOSIS" {
                Ok(title)
            } else {
                Err(title)
            }
        }),
        // Down the six missions to SELECT, then round the buttons to the
        // port's EVERY MAP +, which lists every Geonosis map.
        Press(Action::MenuDown),
        Press(Action::MenuDown),
        Press(Action::MenuDown),
        Press(Action::MenuDown),
        FocusAfter(Action::MenuDown, "LevelList[5]"),
        FocusAfter(Action::MenuDown, "AButton"),
        FocusAfter(Action::MenuDown, "BButton"),
        FocusAfter(Action::MenuDown, "EveryMap"),
        Press(Action::MenuAccept),
        Wait(0.1),
        Check("EVERY MAP + lists every Geonosis map", |w| {
            let (_, items) = campaign_list(w)?;
            if items.len() > 10 {
                Ok(format!("{} maps: {items:?}", items.len()))
            } else {
                Err(format!("{items:?}"))
            }
        }),
        Wait(0.3),
        Shot("campaign_every_map"),
        Check("EVERY MAP + fits inside its button", |w| {
            let mut m = w.resource_mut::<OriginalMenus>();
            let p = m.pages.get("campaigns").ok_or("no campaign page")?;
            let b = match &p
                .screen
                .layout
                .widget("EveryMap")
                .ok_or("no EveryMap")?
                .kind
            {
                rc_menus::layout::Kind::Button(b) => b.clone(),
                _ => return Err("not a button".into()),
            };
            let sizes = m.assets.as_mut().ok_or("no assets")?;
            let t = rc_menus::quads::text_box(&b.blurred, sizes).ok_or("no text")?;
            let bg =
                rc_menus::quads::sprite_box(&b.background_blurred, sizes).ok_or("no background")?;
            let got = format!(
                "text {:.3}..{:.3}, button {:.3}..{:.3}",
                t[0], t[2], bg[0], bg[2]
            );
            if t[0] >= bg[0] && t[2] <= bg[2] {
                Ok(got)
            } else {
                Err(got)
            }
        }),
        // Click and hold the scroll bar's handle (at the top of its track)
        // and drag it to the bottom: the list follows, as in a browser.
        Pointer(|w| bar_point(w, 0.03).map(|p| (p, false))),
        Wait(0.1),
        Pointer(|w| bar_point(w, 0.03).map(|p| (p, true))),
        Wait(0.2),
        Pointer(|w| bar_point(w, 0.5).map(|p| (p, true))),
        Wait(0.2),
        Pointer(|w| bar_point(w, 1.2).map(|p| (p, true))),
        Wait(0.3),
        Check(
            "dragging the scroll bar down scrolls to the last maps",
            |w| {
                let m = w.resource::<OriginalMenus>();
                let p = m.pages.get("campaigns").ok_or("no campaign page")?;
                let l = p.screen.layout.lists.lists.first().ok_or("no list")?;
                let got = format!(
                    "first shown {} of {} ({} rows)",
                    l.position,
                    l.items.len(),
                    l.rows
                );
                if l.items.len() > l.rows && l.position == l.items.len() - l.rows {
                    Ok(got)
                } else {
                    Err(got)
                }
            },
        ),
        Shot("campaign_dragged"),
        Pointer(|w| bar_point(w, 1.2).map(|p| (p, false))),
        Wait(0.1),
        Pointer(|_| None),
        // Up comes back to the list's first map; the third is geo_01a,
        // the level the rest of the route walks.
        FocusAfter(Action::MenuUp, "LevelList[0]"),
        Press(Action::MenuDown),
        FocusAfter(Action::MenuDown, "LevelList[2]"),
        Check("the third map is geo_01a", |w| {
            let m = w.resource::<OriginalMenus>();
            let c = m.campaign.as_ref().ok_or("no campaign")?;
            match c.map(2) {
                Some(map) if map.eq_ignore_ascii_case("geo_01a") => Ok(map.to_string()),
                other => Err(format!("{other:?}")),
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
        // geo_01a opens with a short scene: the player is held in the
        // gunship, looking out, while the squad rappels down (the map's
        // ACTION_Briefing), then let go.
        Until(
            "the opening scene holds the player",
            |w| w.resource::<rc_game_core::Cutscene>().held.is_some(),
            20.0,
        ),
        Note(|w| {
            let f = feet(w);
            w.resource_mut::<Noted>().feet = f;
        }),
        Hold(Action::MoveForward, 1.0),
        Check("the opening scene holds the player still", |w| {
            let (Some(a), Some(b)) = (w.resource::<Noted>().feet, feet(w)) else {
                return Err("no player".into());
            };
            let held = w.resource::<rc_game_core::Cutscene>().held;
            let d = (b - a).length_2d();
            match held {
                Some(h) if d < 5.0 && h.yaw.is_some() => Ok(format!(
                    "moved {d:.0} uu while walking; looks {:.0} to {:.0} deg",
                    h.yaw.unwrap().0.to_degrees(),
                    h.yaw.unwrap().1.to_degrees()
                )),
                _ => Err(format!("moved {d:.0} uu; held {held:?}")),
            }
        }),
        Until(
            "the opening scene lets the player go",
            |w| w.resource::<rc_game_core::Cutscene>().held.is_none(),
            40.0,
        ),
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
        Check("the original HUD is drawn in its own colours", |w| {
            let quads = crate::original_hud::layout_drawn(w);
            if quads.is_empty() {
                return Err("the original HUD isn't drawn (placeholder shown)".into());
            }
            let pic = w
                .resource::<Shots>()
                .pictures
                .get("in_game")
                .cloned()
                .ok_or("no screenshot")?;
            // The health meters are tinted in the original's health
            // colour (green, yellow, orange or red): count pixels inside
            // the tinted pieces that clearly have that colour (a missing
            // picture shows black, white or magenta).
            let (pw, ph) = (pic.width as f32, pic.height as f32);
            let (mut inside, mut green) = (0u32, 0u32);
            for q in quads
                .iter()
                .filter(|q| q.widget == "vitals" && q.color[..3] != [255, 255, 255])
            {
                let x0 = (q.dst[0] * pw) as usize;
                let x1 = ((q.dst[2] * pw) as usize).min(pic.width);
                let y0 = (q.dst[1] * ph) as usize;
                let y1 = ((q.dst[3] * ph) as usize).min(pic.height);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let i = (y * pic.width + x) * 4;
                        let (r, g, b) = (
                            pic.pixels[i] as i32,
                            pic.pixels[i + 1] as i32,
                            pic.pixels[i + 2] as i32,
                        );
                        inside += 1;
                        let hi = r.max(g);
                        if hi - b > 60 && g > b + 15 {
                            green += 1;
                        }
                    }
                }
            }
            let groups: std::collections::BTreeSet<_> =
                quads.iter().map(|q| q.widget.as_str()).collect();
            let got = format!(
                "{} pictures in {groups:?}; {green} of {inside} health meter pixels are in a health colour",
                quads.len()
            );
            if inside > 0 && green * 5 >= inside {
                Ok(got)
            } else {
                Err(got)
            }
        }),
        Check("the first-person arms hold the weapon in hand", |w| {
            let arms = w
                .get_resource::<rc_combat::ArmsReadout>()
                .cloned()
                .ok_or("no arms readout")?;
            let mut q = w.query_filtered::<&rc_combat::Arsenal, With<Player>>();
            let held = q
                .iter(w)
                .next()
                .and_then(|a| a.current())
                .map(|c| c.0.clone());
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
        Check(
            "the visor starts in tactical mode and the squad glows",
            |w| {
                let mode = w.resource::<rc_visor::VisorState>().mode;
                let r = w.resource::<rc_visor::VisorReadout>().clone();
                let mut q =
                    w.query_filtered::<&rc_visor::TacticalGlow, With<rc_squad_plugin::SquadMate>>();
                let mates: Vec<String> = q.iter(w).map(|g| g.0.clone()).collect();
                let got = format!(
                "{} mode, {} glow colours from the game, {} glowing copies ({} shown), squad {mates:?}",
                mode.name(),
                r.colours,
                r.shells,
                r.shown
            );
                if mode == rc_visor::VisorMode::Tactical
                    && r.colours >= 4
                    && r.shown > 0
                    && !mates.is_empty()
                {
                    Ok(got)
                } else {
                    Err(got)
                }
            },
        ),
        Press(Action::Flashlight),
        Wait(0.3),
        Check(
            "CYCLE VISOR MODE (X) switches to low light: night vision, lamp and fog on",
            |w| {
                let mode = w.resource::<rc_visor::VisorState>().mode;
                let shown = w.resource::<rc_visor::VisorReadout>().shown;
                let lamps = count::<rc_visor::Headlamp>(w);
                let (night, fog) = night_and_fog(w);
                let got = format!(
                    "{} mode, {lamps} lamp, {shown} glowing copies shown, night vision {night}, fog {fog}",
                    mode.name()
                );
                if mode == rc_visor::VisorMode::LowLight && lamps == 1 && shown == 0 && night && fog
                {
                    Ok(got)
                } else {
                    Err(got)
                }
            },
        ),
        Press(Action::Flashlight),
        Wait(0.3),
        Press(Action::Flashlight),
        Wait(0.3),
        Check(
            "two more presses go through normal and back to tactical",
            |w| {
                let mode = w.resource::<rc_visor::VisorState>().mode;
                let r = w.resource::<rc_visor::VisorReadout>().clone();
                let lamps = count::<rc_visor::Headlamp>(w);
                let (night, fog) = night_and_fog(w);
                let got = format!(
                    "{} mode after {} changes, {lamps} lamp, {} shown, night vision {night}, fog {fog}",
                    mode.name(),
                    r.changes,
                    r.shown
                );
                if mode == rc_visor::VisorMode::Tactical
                    && lamps == 0
                    && r.shown > 0
                    && r.changes >= 3
                    && !night
                    && !fog
                {
                    Ok(got)
                } else {
                    Err(got)
                }
            },
        ),
        Check(
            "the level part loaded walls, props, doors and invisible walls",
            |w| {
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
        // The helmet frame, on the same picture (drawn a few seconds after
        // the level starts, once its shaders are ready), then the GAME
        // page's HELMET row switched off and on again.
        Check("the original helmet frame is drawn round the view", |w| {
            helmet_drawn(w, "look_up")
        }),
        Note(|w| w.resource_mut::<Settings>().game.helmet = false),
        Wait(0.5),
        Shot("no_helmet"),
        Check("HELMET OFF takes the frame away", |w| {
            let shown = w.resource::<crate::helmet::HelmetReadout>().shown;
            // Compared with the same view with the frame on: dark things in
            // the scene (the gunship overhead in geo_01a) can match the
            // frame's colours by chance, so a fixed share isn't enough.
            let with = helmet_match(w, "look_up")?.map_or(1.0, |(s, _)| s);
            match helmet_match(w, "no_helmet")? {
                None => Ok("no model in the install, nothing to take away".into()),
                Some((share, _)) if !shown && share < 0.3f32.max(with * 0.5) => Ok(format!(
                    "hidden; {:.0}% of its pixels still match (with the frame: {:.0}%)",
                    share * 100.0,
                    with * 100.0
                )),
                Some((share, _)) => Err(format!(
                    "shown {shown}; {:.0}% of its pixels still match (with the frame: {:.0}%)",
                    share * 100.0,
                    with * 100.0
                )),
            }
        }),
        Note(|w| w.resource_mut::<Settings>().game.helmet = true),
        // The level's effect spawners (`rc_scripts_plugin::fx_factories`):
        // the opening script sends OverheadVehicles, which switches on
        // geo_01a's five overhead spawners; the game's own gunship and
        // Geonosian fighter models then fly over, high above the arena.
        Until(
            "the opening script switches on the overhead flyovers",
            fx_overhead_on,
            60.0,
        ),
        Until(
            "gunships and fighters fly over, high above the arena",
            fx_overhead_flying,
            30.0,
        ),
        Shot("flyover"),
        // Projectile spawners: the events that set geo_01a's bunker
        // missiles and the bombardment going come later in the level than
        // this walk reaches, so the check sends them to the spawners
        // itself (as the level's ScriptedTrigger12 and Trigger5 would),
        // then again to switch them off.
        Note(|w| spawner_event(w, "BunkerProjectileEFX")),
        Until(
            "the bunkers' missiles fly with their trail and blow up with the game's effect",
            bunker_missiles_blow_up,
            20.0,
        ),
        Note(|w| spawner_event(w, "BunkerProjectileEFX")),
        Note(|w| spawner_event(w, "ArialBombardment01")),
        Until(
            "the bombardment's missiles fly in from high above and blow up",
            bombardment_blows_up,
            20.0,
        ),
        Note(|w| spawner_event(w, "ArialBombardment01")),
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
            "the scripts play the game's own lines, as long as each one lasts, some from their place in the level",
            level_script_lines,
        ),
        Check("the start barricade blocks the way", |w| {
            level_script_way(w, true)
        }),
        Until(
            "the scripts switch the bacta dispenser on",
            level_script_dispenser_on,
            300.0,
        ),
        // The dispenser is drawn with its animated model (it used to be
        // invisible, so a player couldn't find it).
        Check("the level's animated props are drawn", level_anim_props),
        Until(
            "the current objective is drawn at the top, as in the original",
            |w| objective_line(w).is_ok(),
            60.0,
        ),
        Shot("objective"),
        Check(
            "the current objective is drawn at the top, as in the original",
            objective_line,
        ),
        // The landing gunship hovers over the landing zone, as in the
        // user's screenshot of the original: its class turns
        // `bAlignBottom` off, so it is drawn from its map place, not
        // stood on the ground.
        Check("the landing gunship hovers where the map puts it", gunship_hovers),
        Check("the clone troopers hold their rifles", troopers_hold_rifles),
        Note(view_gunship),
        Wait(0.5),
        Shot("gunship"),
        Note(level_script_view_dispenser),
        Wait(0.5),
        Shot("bacta_dispenser"),
        Check("the bacta dispenser can be seen", |w| {
            let s = w
                .resource::<Shots>()
                .stats
                .get("bacta_dispenser")
                .copied()
                .ok_or("no picture")?;
            if s.lit > 0.5 {
                Ok(format!("{:.0}% of the picture lit", s.lit * 100.0))
            } else {
                Err(format!("only {:.0}% of the picture lit", s.lit * 100.0))
            }
        }),
        Check(
            "the hologram shows only if a squad mate is hurt, and aiming at the dispenser plays its sound",
            |w| {
                let hurt = w
                    .resource::<rc_game_core::SquadReadout>()
                    .members
                    .iter()
                    .any(|m| m.health < 0.999);
                bacta_hologram_shown(w, hurt)
            },
        ),
        // A squad mate gets hurt: now aiming at the squad spot fades the
        // hologram in.
        Note(bacta_hurt_a_squad_mate),
        Wait(0.5),
        Check(
            "with a squad mate hurt, aiming at the squad spot shows the hologram",
            |w| bacta_hologram_shown(w, true),
        ),
        Note(level_script_stand_at_dispenser),
        Wait(0.5),
        Check("the game's own prompt shows at the dispenser", |w| {
            let st = w
                .get_resource::<rc_scripts_plugin::bacta::BactaStats>()
                .cloned()
                .ok_or("no bacta stats")?;
            match &st.prompt {
                Some(p) if p.starts_with("PRESS AND HOLD") && p.ends_with("TO HEAL") => {
                    Ok(format!("\"{p}\""))
                }
                other => Err(format!("{other:?}")),
            }
        }),
        Shot("bacta_use_panel"),
        Check("the original's use panel (icon and ring) shows at the crosshair", |w| {
            let parts = crate::original_hud::use_panel_showing(w);
            if parts.iter().any(|p| p == "Ring") && parts.iter().any(|p| p == "Icon") {
                Ok(format!("showing {parts:?}"))
            } else {
                Err(format!("showing {parts:?}"))
            }
        }),
        // Hold Use through the whole heal, with a picture of the hands
        // plugged in on the way.
        ShotHolding("bacta_hands", Action::Use, 0.5),
        Hold(Action::Use, 4.5),
        Check(
            "healing pulled the player in front of the dispenser and held them, the use panel filled, and the heal sounds played",
            bacta_pull_and_panel,
        ),
        Check(
            "the game's zap effect hung on the dispenser's Eye bone while healing, with its hum, and was taken away after",
            bacta_zap,
        ),
        Until("the barricade opens", level_script_barricade_gone, 60.0),
        Wait(3.0),
        Check(
            "the barricade is gone: not solid, not drawn, the way through is open",
            |w| level_script_way(w, false),
        ),
        Check("the dispenser played its Load and Holster animations and their sounds", |w| {
            let mut q = w.query::<&rc_level_plugin::anim_props::AnimProp>();
            let p = q
                .iter(w)
                .find(|p| p.name.eq_ignore_ascii_case("BactaDispenserGEO0"))
                .ok_or("no BactaDispenserGEO0")?;
            let msg = format!(
                "played {:?}, now {:?}, holds {:?}; sounds {:?}",
                p.played,
                p.playing.as_ref().map(|x| &x.0),
                p.last,
                p.heard
            );
            let ok = p.played.first().is_some_and(|a| a == "Load")
                && p.played.iter().any(|a| a == "Holster")
                && p.playing.is_none()
                && !p.heard.is_empty()
                && p.heard.iter().all(|h| !h.ends_with("(not loaded)"));
            if ok { Ok(msg) } else { Err(msg) }
        }),
        Check(
            "the gun went down, the hands plugged into the dispenser (BactaLoad, BactaLoop) and came away (BactaHolster), then the gun came back up",
            bacta_hands_played,
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
        // Props weapons can break (`rc_scripts_plugin::breakables`):
        // shoot one of geo_01a's explosive barrels from just above it,
        // with a stand-in character above that; three DC-17m shots (20
        // each) take its 60 health, it explodes a moment later, and the
        // explosion reaches the stand-in.
        Check("the level's breakable props are read", props_read),
        Note(props_shoot_barrel),
        Until("shots break an explosive barrel", props_barrel_broken, 5.0),
        Until(
            "the barrel explodes a moment later",
            props_barrel_exploded,
            3.0,
        ),
        Wait(0.2),
        Check("its explosion hurts what stands near it", props_blast_hurt),
        // Its effect (`rc_effects_plugin`): the game's Explosion_Barrel_GEO
        // with its sound, the barrel's own sound, particles drawn, and all
        // gone once they burn out.
        Check("the barrel's fireball and sounds start", props_effect_shown),
        Until("the fireball and smoke burn out", props_effect_over, 40.0),
        // Simple collision shapes (`rc_level::collision`): props with
        // collision off on every section collide through their mesh's
        // simple shape, and walk-through props that the game lets you
        // shoot stop a line but not a body.
        Check(
            "props collide through their simple shapes",
            props_simple_shapes,
        ),
        Check(
            "walk-through props stop shots, not people",
            props_shots_only,
        ),
        // Script actions on characters: geo_01a's "LoneRanger", a
        // Geonosian warrior whose spawner gives him the sequence
        // LoneRangerAI. On LoneRangerJump he walks to a point, turns to
        // the player, plays EmoteSeeEnemy, walks on, and is removed; the
        // sequence waits for each walk and the animation.
        Note(cast_spawn),
        Until(
            "the lone Geonosian appears with his script",
            cast_appeared,
            10.0,
        ),
        Check(
            "he answers to his tag and follows LoneRangerAI",
            cast_tagged,
        ),
        Note(cast_go),
        Until("his sequence finishes", cast_finished, 90.0),
        Check(
            "he walked, turned, played his animation and was removed, each when the script said",
            cast_summary,
        ),
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
        // The map's ammo and grenade boxes (`rc_combat::ammo_box`).
        Check("geo_01a's ammo and grenade boxes are placed with their models", ammo_boxes_placed),
        Note(|w| {
            // Nearly empty rifle, then a DC-17m box at the player's feet.
            set_rifle(w, 20.0, 0);
            let name = move_box_to_player(w, "DC17mBlasterBox", 0);
            w.resource_mut::<Noted>().ammo_box = name;
        }),
        Wait(0.3),
        Check("walking into a DC-17m box fills the rifle to 300", ammo_box_taken),
        Check(
            "taking it shows its pickup message and plays its pickup sound",
            pickup_feedback,
        ),
        Note(|w| {
            // Full now: another box is left lying.
            move_box_to_player(w, "DC17mBlasterBox", 1);
        }),
        Wait(0.3),
        Check("a full rifle leaves the next box lying", ammo_box_left),
        Note(|w| {
            set_grenades(w, "thermal_detonator", 0);
            move_box_to_player(w, "ThermalDetonatorBox", 0);
        }),
        Wait(0.5),
        Check("the grenade crate by the start gives 5 thermal detonators and comes back", grenade_crate),
        // A weapon on the floor (`rc_combat::pickup`): walking over one you
        // don't carry isn't enough; Use takes it and puts it in hand.
        Note(|w| {
            let mut q = w.query_filtered::<&Transform, With<Player>>();
            if let Some(feet) = q.iter(w).next().map(|t| t.translation) {
                let mut commands = w.commands();
                rc_combat::spawn_pickup(&mut commands, "acp_array_gun", feet);
                w.flush();
            }
        }),
        Wait(0.3),
        Check("walking onto a weapon you don't carry leaves it lying", |w| {
            let held = carries(w, "acp_array_gun");
            let lying = weapons_lying(w, "acp_array_gun");
            let got = format!("carried {held}, lying {lying}");
            if !held && lying == 1 { Ok(got) } else { Err(got) }
        }),
        Press(Action::Use),
        Wait(0.3),
        Check("pressing Use picks the weapon up into your hands", |w| {
            let mut q = w.query_filtered::<&rc_combat::Arsenal, With<Player>>();
            let a = q.iter(w).next().ok_or("no arsenal")?;
            let current = a.current().map(|c| c.0.clone()).unwrap_or_default();
            let lying = weapons_lying(w, "acp_array_gun");
            let got = format!("in hand {current}, lying {lying}");
            if current == "acp_array_gun" && lying == 0 { Ok(got) } else { Err(got) }
        }),
        Press(Action::WeaponBlaster),
        Wait(0.2),
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
        Check("quickload keeps the taken ammo box taken", ammo_box_after_load),
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
                // Effect spawners are switched on and off by those same
                // looping scripts (geo_01a's BattleExplosions toggle).
                if k == "effect_spawners" {
                    notes.push(format!("[{k}] skipped (switched by looping scripts)"));
                    continue;
                }
                if matches!(k.as_str(), "movers" | "scripts") {
                    notes.push(format!("[{k}] checked above"));
                    continue;
                }
                // Fighters in a fight (clone troopers against droids)
                // turn, walk, pick targets and change mode from frame to
                // frame, so for them only who is there and how hurt is
                // compared.
                let (v, b) = if k == "enemies" {
                    (fight_settled(v), fight_settled(b))
                } else {
                    (v.clone(), b.clone())
                };
                let (v, b) = (&v, &b);
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
        // Squad marker jobs reach the level: geo_01a's bunker door charge
        // (DemolitionCharge2) going off sends its map events, the script
        // waiting for BunkerDoorDestroyed carries on, and the door prop
        // (Prop12) is blown away.
        Note(|w| marker_job(w, DEMO_ANCHOR)),
        Until(
            "the bunker door charge's events reach the scripts",
            |w| marker_heard(w, DEMO_DONE_EVENT),
            5.0,
        ),
        Check(
            "the charge blew the bunker door away and the waiting script went on",
            marker_demo_summary,
        ),
        // The level exit: geo_01a's arrows lead to geo_01b.
        Check(
            "the level has an open exit to the next map",
            level_exit_found,
        ),
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
        Check(
            "the next map has the player's health and ammo",
            level_exit_carried,
        ),
        Check(
            "the player arrives at the next map's start",
            level_exit_arrived,
        ),
        Check(
            "the next map's start is saved as a checkpoint, with the health",
            level_exit_checkpoint,
        ),
        // geo_01b's hangar door: slicing its lock (HackDoorGeo6) sends
        // HangarDoorHacked and opens the locked double door.
        Check("the hangar door starts locked shut", |w| {
            match door_closed(w, HANGAR_DOOR) {
                Some(true) => Ok(format!("{HANGAR_DOOR} closed")),
                other => Err(format!("{HANGAR_DOOR}: {other:?}")),
            }
        }),
        Note(|w| marker_job(w, HANGAR_LOCK)),
        Until(
            "slicing the lock opens the hangar door",
            |w| marker_heard(w, HANGAR_EVENT) && door_closed(w, HANGAR_DOOR) == Some(false),
            5.0,
        ),
        // The end-of-campaign debriefing, asked for the way geo_05c's
        // last script asks (sent here directly; the real one waits for
        // the end of that level). CONTINUE loads geo_01b again.
        Note(debriefing_ask),
        Wait(0.5),
        Check(
            "the debriefing pauses play and shows the original screen with the level's words and picture",
            debriefing_shown,
        ),
        Shot("debriefing"),
        Check("the debriefing matches the original's pictures", |w| {
            compare(w, "debriefing", 0.05)
        }),
        Press(Action::MenuBack),
        Wait(0.3),
        Check("back does nothing on the debriefing", |w| {
            match (shown_page(w), page(w)) {
                (Some(k), _) if k == "debriefing" => Ok("still showing".into()),
                (None, Page::Debriefing) => Ok("placeholder still showing".into()),
                other => Err(format!("{other:?}")),
            }
        }),
        Press(Action::MenuAccept), // CONTINUE
        Until(
            "CONTINUE goes on to the next map",
            |w| {
                in_state(w, GameState::Loading)
                    || (in_state(w, GameState::InGame)
                        && w.resource::<crate::debriefing::Debriefing>().showing.is_none()
                        && w.get_resource::<State<Play>>().map(|p| *p.get()) == Some(Play::Running))
            },
            10.0,
        ),
        Until(
            "the next map loads after the debriefing",
            |w| {
                in_state(w, GameState::InGame)
                    && w.resource::<CurrentLevel>().map.as_deref() == Some("geo_01b")
                    && count::<Player>(w) == 1
            },
            LOAD_TIMEOUT,
        ),
        Wait(1.0),
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
        Check(
            "the pause menu shows the original's rows for a living player, centred",
            |w| {
                let m = w.resource::<OriginalMenus>();
                let Some(p) = m.pages.get("paused") else {
                    return Ok("no original pause menu".into());
                };
                let l = &p.screen.layout;
                if !rc_menus::pause::is_pause_menu(l) {
                    return Ok("not the original's eleven rows".into());
                }
                let shown: Vec<usize> = (0..rc_menus::pause::NUM_OPTIONS)
                    .filter(|&i| {
                        rc_menus::pause::row(l, i).is_some_and(|x| !l.widgets[x].hidden)
                    })
                    .collect();
                // Alive: never LOAD LAST SAVE; RESUME and QUIT always.
                for (i, want) in [(0, false), (3, true), (10, true)] {
                    if shown.contains(&i) != want {
                        return Err(format!("row {i} shown {}, want {want}", !want));
                    }
                }
                let ys: Vec<f32> = shown
                    .iter()
                    .filter_map(|&i| rc_menus::pause::row(l, i))
                    .filter_map(|x| match &l.widgets[x].kind {
                        rc_menus::layout::Kind::Button(b) => Some(b.blurred.pos[1]),
                        _ => None,
                    })
                    .collect();
                let steps: Vec<f32> = ys.windows(2).map(|p| p[1] - p[0]).collect();
                if steps.iter().any(|d| (d - steps[0]).abs() > 1e-4) {
                    return Err(format!("rows not evenly spaced: {ys:?}"));
                }
                let focus = p.screen.focused_name().unwrap_or("none").to_string();
                if focus != "MenuOptions[3]" {
                    return Err(format!("focus on {focus}, want RESUME GAME"));
                }
                Ok(format!("rows {shown:?} from {:.4}, focus on RESUME GAME", ys[0]))
            },
        ),
        Shot("paused"),
        // The original save and load pages, from the pause menu.
        FocusPauseItem("SaveGameSelected", 0),
        Press(Action::MenuAccept),
        Wait(0.3),
        Check(
            "SAVE GAME opens the original save page, listing the autosave and quicksave",
            |w| {
                let items = save_items(w, "save_game")?;
                let got = format!("{:?} {items:?}", page(w));
                match items.as_slice() {
                    [a, q]
                        if page(w) == Page::SaveGame
                            && a.starts_with("AUTOSAVE")
                            && q.starts_with("QUICKSAVE") =>
                    {
                        Ok(got)
                    }
                    _ => Err(got),
                }
            },
        ),
        Shot("save_game"),
        FocusOn("YButton"),
        Press(Action::MenuAccept), // NEW
        Wait(0.5),
        Check("NEW saves a new save named after the campaign", |w| {
            let file = save_file(w, &SaveSlot::Named("Geonosis_1".into()))?;
            let items = save_items(w, "save_game")?;
            let got = format!("map {}, list {items:?}", file.header.map);
            if items.len() == 3 && items[2].starts_with("GEONOSIS 1") {
                Ok(got)
            } else {
                Err(got)
            }
        }),
        // Up from the buttons comes back to the first save; Enter on the
        // quicksave tries to overwrite it, which the original refuses.
        Press(Action::MenuUp),
        Press(Action::MenuDown),
        Press(Action::MenuAccept),
        Wait(0.2),
        Check(
            "the quicksave can't be overwritten: the original's message",
            |w| {
                let asking = w.resource::<OriginalMenus>().asking.clone();
                match (&asking, question_text(w)) {
                    (Some(original_menu::Asking::Notice), Some(t)) if t.contains("QUICKSAVE") => {
                        Ok(t)
                    }
                    other => Err(format!("{other:?}")),
                }
            },
        ),
        Shot("save_notice"),
        Press(Action::MenuBack),
        Wait(0.2),
        // Down to the new save, then down to the buttons (the save stays
        // chosen) and round to DELETE.
        Press(Action::MenuDown),
        Press(Action::MenuDown),
        FocusOn("XButton"),
        Press(Action::MenuAccept),
        Wait(0.2),
        Check(
            "DELETE asks first, with the save's name",
            |w| match question_text(w) {
                Some(t) if t.contains("DELETE GEONOSIS 1") => Ok(t),
                other => Err(format!("{other:?}")),
            },
        ),
        FocusOn("AButton"), // YES
        Press(Action::MenuAccept),
        Wait(0.5),
        Check("YES deletes the save and takes it off the list", |w| {
            let gone = save_file(w, &SaveSlot::Named("Geonosis_1".into())).is_err();
            let items = save_items(w, "save_game")?;
            let got = format!("file gone {gone}, list {items:?}");
            if gone && items.len() == 2 {
                Ok(got)
            } else {
                Err(got)
            }
        }),
        Press(Action::MenuBack),
        Wait(0.3),
        FocusPauseItem("LoadGameSelected", 0),
        Press(Action::MenuAccept),
        Wait(0.3),
        Check("LOAD GAME opens the original load page", |w| {
            let items = save_items(w, "load_game")?;
            let got = format!("{:?} {items:?}", page(w));
            if page(w) == Page::LoadGame && items.len() == 2 {
                Ok(got)
            } else {
                Err(got)
            }
        }),
        Shot("load_game"),
        Press(Action::MenuDown), // the quicksave, saved in geo_01a
        Press(Action::MenuAccept),
        Wait(0.2),
        Check(
            "loading in a level asks first (the original's question)",
            |w| match question_text(w) {
                Some(t) if t.contains("UNSAVED PROGRESS") => Ok(t),
                other => Err(format!("{other:?}")),
            },
        ),
        FocusOn("AButton"), // YES
        Press(Action::MenuAccept),
        Until(
            "the load page's YES loads the save",
            |w| !in_state(w, GameState::InGame),
            5.0,
        ),
        Until(
            "the save's level is back",
            |w| in_state(w, GameState::InGame) && !w.contains_resource::<RestoringSave>(),
            LOAD_TIMEOUT,
        ),
        Wait(0.5),
        Check("the quicksave's map is loaded", |w| {
            match w.resource::<CurrentLevel>().map.clone() {
                Some(m) if m == "geo_01a" => Ok(m),
                other => Err(format!("{other:?}")),
            }
        }),
        Press(Action::Pause),
        Wait(0.2),
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
            // The same for the visor rows: start from the original's
            // defaults (CYCLE, 2).
            let mut s = w.resource_mut::<Settings>();
            s.game.tactical_visor = crate::settings::TacticalVisor::Cycle;
            s.game.tactical_intensity = 0.2;
        }),
        // EXTRAS: the original page; its five videos are greyed out (no
        // MPEG-1 player yet), VIEW CREDITS rolls Credits.int.
        FocusOn("MenuOptions[6]"),
        Press(Action::MenuAccept),
        Wait(0.3),
        Check(
            "EXTRAS opens the original EXTRAS page, videos greyed, focus on VIEW CREDITS",
            |w| {
                let shown = original_page_shown(w);
                let greyed = (0..rc_menus::extras::NUM_EXTRAS)
                    .filter(|&i| {
                        let m = w.resource::<OriginalMenus>();
                        m.pages.get(original_credits::EXTRAS).is_some_and(|p| {
                            matches!(
                                p.screen.layout.widget(&rc_menus::extras::row_name(i))
                                    .map(|w| &w.kind),
                                Some(rc_menus::layout::Kind::Button(b)) if b.disabled
                            )
                        })
                    })
                    .count();
                let got = format!("{shown:?}, {greyed} videos greyed, focus {:?}", focus(w));
                match (shown.as_deref(), greyed, focus(w).as_deref()) {
                    (Some(original_credits::EXTRAS), 5, Some("Credits")) => Ok(got),
                    _ => Err(got),
                }
            },
        ),
        Shot("extras"),
        Press(Action::MenuAccept), // VIEW CREDITS
        Wait(1.5),
        Check("VIEW CREDITS rolls the names from Credits.int", |w| {
            let shown = original_page_shown(w);
            let m = w.resource::<OriginalMenus>();
            let lines: Vec<String> = m
                .pages
                .get(original_credits::CREDITS)
                .map(|p| {
                    p.screen
                        .layout
                        .widgets
                        .iter()
                        .filter(|w| w.name.starts_with("MenuCreditLineBuffer") && !w.hidden)
                        .filter_map(|w| match &w.kind {
                            rc_menus::layout::Kind::Text(t) => {
                                Some(format!("{} at {:.3}", t.text, t.pos[1]))
                            }
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            let got = format!("{shown:?}: {}", lines.join(" | "));
            match shown.as_deref() {
                Some(original_credits::CREDITS)
                    if lines.first().is_some_and(|l| l.starts_with("GAME DESIGN at 0.")) =>
                {
                    Ok(got)
                }
                _ => Err(got),
            }
        }),
        Shot("credits"),
        Press(Action::MenuBack),
        Wait(0.3),
        Check("BACK leaves the credits for EXTRAS", |w| {
            match original_page_shown(w).as_deref() {
                Some(original_credits::EXTRAS) => Ok("extras".into()),
                other => Err(format!("{other:?}")),
            }
        }),
        Press(Action::MenuBack),
        Wait(0.3),
        FocusOn("MenuOptions[0]"),
        // MULTIPLAYER is greyed out and skipped.
        FocusAfter(Action::MenuDown, "MenuOptions[1]"), // LOAD GAME
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
        FocusOn("Options[7]"),   // TACTICAL VISOR MODE
        Press(Action::MenuLeft), // CYCLE -> OFF
        Wait(0.2),
        Check("TACTICAL VISOR MODE works: left turns it OFF", |w| {
            let now = w.resource::<Settings>().game.tactical_visor;
            let visor = w.resource::<rc_visor::VisorSettings>().choice;
            let shown = button_text(w, "Options[7]").unwrap_or_default();
            let got = format!("setting {now:?}, visor {visor:?}, shows {shown}");
            if now == crate::settings::TacticalVisor::Off && visor == rc_visor::VisorChoice::Off {
                Ok(got)
            } else {
                Err(got)
            }
        }),
        Press(Action::MenuRight), // back to CYCLE
        FocusOn("Options[8]"),    // TACTICAL MODE INTENSITY
        Press(Action::MenuRight), // 2 -> 3
        Wait(0.2),
        Check(
            "TACTICAL MODE INTENSITY works: right raises it a step",
            |w| {
                let shown = button_text(w, "Options[8]").unwrap_or_default();
                let s = &w.resource::<Settings>().game;
                let got = format!(
                    "visor {:?}, intensity {:.1}, shows {shown}",
                    s.tactical_visor, s.tactical_intensity
                );
                if s.tactical_visor == crate::settings::TacticalVisor::Cycle
                    && (s.tactical_intensity - 0.3).abs() < 1e-4
                    && shown == "3"
                {
                    Ok(got)
                } else {
                    Err(got)
                }
            },
        ),
        Press(Action::MenuLeft), // back to 2
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
        Check("the greyed-out EAX row can't be reached, VOICES can", |w| {
            let focusable = original_focusable(w);
            if focusable.iter().any(|n| n == "Options[5]")
                || !focusable.iter().any(|n| n == "Options[0]")
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
        FocusOn("MenuOptions[5]"),
        Press(Action::MenuAccept), // PROFILES
        Wait(0.3),
        Check("PROFILES lists the profile, focus on it, BACK and DELETE shown", |w| {
            let items = save_items(w, "profiles")?;
            let back = button_hidden(w, "profiles", "BButton");
            let delete = button_hidden(w, "profiles", "XButton");
            let got = format!("{items:?} focus {:?} BACK hidden {back:?} DELETE hidden {delete:?}", focus(w));
            match (items.as_slice(), focus(w).as_deref(), back, delete) {
                ([n, p], Some("ProfileList[1]"), Some(false), Some(false))
                    if n == "< NEW >" && p == "SCRIPT_2" => Ok(got),
                _ => Err(got),
            }
        }),
        Shot("profiles"),
        FocusOn("XButton"),
        Press(Action::MenuAccept), // DELETE
        Wait(0.2),
        Check("the profile in use can't be deleted (the original's warning)", |w| {
            match warning_text(w).as_deref() {
                Some("YOU CANNOT DELETE THE ACTIVE PROFILE.") => Ok("warning".into()),
                other => Err(format!("{other:?}")),
            }
        }),
        Shot("profile_warning"),
        Press(Action::MenuBack),
        Wait(0.2),
        Check("back doesn't close the warning", |w| match warning_text(w) {
            Some(_) => Ok("still up".into()),
            None => Err("closed".into()),
        }),
        Press(Action::MenuAccept), // CONTINUE
        Wait(0.2),
        Check("CONTINUE closes the warning", |w| match warning_text(w) {
            None => Ok("closed".into()),
            Some(t) => Err(t),
        }),
        // A second profile, loaded, then the first one deleted.
        Press(Action::MenuUp),
        Wait(0.1),
        Check("up from the buttons goes to < NEW >", |w| match focus(w).as_deref() {
            Some("ProfileList[0]") => Ok("ProfileList[0]".into()),
            other => Err(format!("{other:?}")),
        }),
        Press(Action::MenuAccept), // < NEW >
        Wait(0.3),
        Type("script 2"),
        Type("\n"),
        Press(Action::MenuAccept), // OK
        Wait(0.2),
        Check("a name already taken (capitals don't count) gets the original's warning", |w| {
            match warning_text(w).as_deref() {
                Some("THERE IS ALREADY A PROFILE NAMED script_2") => Ok("warning".into()),
                other => Err(format!("{other:?}")),
            }
        }),
        Press(Action::MenuAccept), // CONTINUE
        Wait(0.2),
        Press(Action::MenuBack),
        Wait(0.3),
        Check("BACK on the name page goes back to the list", |w| {
            match (page(w), shown_page(w)) {
                (Page::Profiles, Some(k)) if k == "profiles" => Ok("profiles".into()),
                other => Err(format!("{other:?}")),
            }
        }),
        Press(Action::MenuUp),
        Wait(0.1),
        Check("up from SCRIPT_2 goes to < NEW >", |w| match focus(w).as_deref() {
            Some("ProfileList[0]") => Ok("ProfileList[0]".into()),
            other => Err(format!("{other:?}")),
        }),
        Press(Action::MenuAccept), // < NEW >
        Wait(0.3),
        Type("Alpha"),
        Type("\n"),
        Press(Action::MenuAccept), // OK
        Until(
            "the main menu after making Alpha",
            |w| page(w) == Page::Main && w.resource::<OriginalMenus>().question.is_none(),
            5.0,
        ),
        Press(Action::MenuAccept), // PROFILES (focus stayed on it)
        Wait(0.3),
        Check("the list has both, focus on ALPHA (in use)", |w| {
            let items = save_items(w, "profiles")?;
            match (items.len(), focus(w).as_deref()) {
                (3, Some("ProfileList[1]")) if items[1] == "ALPHA" && items[2] == "SCRIPT_2" => {
                    Ok(format!("{items:?}"))
                }
                other => Err(format!("{items:?} {other:?}")),
            }
        }),
        Press(Action::MenuDown), // SCRIPT_2
        FocusOn("XButton"),
        Press(Action::MenuAccept), // DELETE
        Wait(0.2),
        Check("DELETE asks the original's question", |w| match question_text(w) {
            Some(q) if q == "DELETE SCRIPT_2 PROFILE, AND ALL ASSOCIATED SAVED GAMES?" => Ok(q),
            other => Err(format!("{other:?}")),
        }),
        Press(Action::MenuRight), // YES
        Press(Action::MenuAccept),
        Wait(0.3),
        Check("YES deletes the profile and its folder", |w| {
            let items = save_items(w, "profiles")?;
            let gone = !std::path::Path::new("logs/script_saves/Profile_Script_2").exists();
            match (items.as_slice(), gone) {
                ([_, a], true) if a == "ALPHA" => Ok(format!("{items:?}, folder gone")),
                other => Err(format!("{other:?}")),
            }
        }),
        Press(Action::MenuBack),
        Wait(0.3),
        Check("BACK closes the list", |w| match page(w) {
            Page::Main => Ok("main".into()),
            p => Err(format!("{p:?}")),
        }),
        FocusOn("MenuOptions[7]"),
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
/// The original page showing, by its name in `original_menus.toml`.
fn shown_page(w: &mut World) -> Option<String> {
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

/// A point on the first scroll bar's track on screen, in window pixels:
/// `along` 0 at its top, 1 at its bottom (more goes past it).
fn bar_point(w: &mut World, along: f32) -> Option<Vec2> {
    let mut q = w.query::<(
        &original_menu::BarHit,
        &bevy::ui::ComputedNode,
        &bevy::ui::UiGlobalTransform,
    )>();
    let (_, node, at) = q.iter(w).find(|(b, _, _)| b.bar == 0)?;
    let size = node.size();
    let centre = at.translation;
    Some(Vec2::new(
        centre.x,
        centre.y - size.y / 2.0 + size.y * along,
    ))
}

/// What an original load or save page lists.
fn save_items(w: &mut World, k: &str) -> Result<Vec<String>, String> {
    let m = w.resource::<OriginalMenus>();
    let p = m.pages.get(k).ok_or(format!("no original {k} page"))?;
    Ok(p.screen
        .layout
        .lists
        .lists
        .first()
        .map(|l| l.items.clone())
        .unwrap_or_default())
}

/// Whether a widget on an original page is hidden.
fn button_hidden(w: &mut World, k: &str, name: &str) -> Option<bool> {
    let m = w.resource::<OriginalMenus>();
    m.pages.get(k)?.screen.layout.widget(name).map(|w| w.hidden)
}

/// The warning box's text, when it shows.
fn warning_text(w: &mut World) -> Option<String> {
    let m = w.resource::<OriginalMenus>();
    if m.asking != Some(original_menu::Asking::Warning) {
        return None;
    }
    match &m.question.as_ref()?.layout.widget("Message")?.kind {
        rc_menus::layout::Kind::Text(t) => Some(t.text.clone()),
        _ => None,
    }
}

/// The question box's question, when it shows.
fn question_text(w: &mut World) -> Option<String> {
    let q = w.resource::<OriginalMenus>().question.as_ref()?;
    match &q.layout.widget("Question")?.kind {
        rc_menus::layout::Kind::Text(t) => Some(t.text.clone()),
        _ => None,
    }
}

/// The original campaign page's planet name and list.
fn campaign_list(w: &mut World) -> Result<(String, Vec<String>), String> {
    let m = w.resource::<OriginalMenus>();
    let p = m
        .pages
        .get("campaigns")
        .ok_or("no original campaign page")?;
    let title = match p.screen.layout.widget("Campaign").map(|w| &w.kind) {
        Some(rc_menus::layout::Kind::Text(t)) => t.text.clone(),
        _ => String::new(),
    };
    let items = p
        .screen
        .layout
        .lists
        .lists
        .first()
        .map(|l| l.items.clone())
        .unwrap_or_default();
    Ok((title, items))
}

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
    let across = w.resource::<OriginalMenus>().across;
    let area = original_menu::area_for(Vec2::new(shot.width as f32, shot.height as f32), across);
    let ours = shrink(&shot, area, W, H);
    let video = {
        let v = w.resource::<crate::menu_video::MenuVideo>();
        if v.wanted {
            v.software_frame(W, H, area.width() / area.height().max(1.0))
        } else {
            None
        }
    };
    let mut menus = w.resource_mut::<OriginalMenus>();
    let reference =
        original_menu::software_picture(&mut menus, W, H, video).ok_or("no original menu")?;
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

/// How much of the helmet frame's software painting matches screenshot
/// `name`: the share of its pixels within a small colour difference, and
/// how many pixels it covers. `None` if the install has no model.
fn helmet_match(w: &mut World, name: &str) -> Result<Option<(f32, usize)>, String> {
    let helmet = w.resource::<crate::helmet::Helmet>();
    let model = match &helmet.model {
        Some(Ok(m)) => m.clone(),
        _ => return Ok(None),
    };
    let plan = helmet.plan.clone();
    // Drawn at its own light plus the level's where the player is (see
    // rc_game_core::place_light): paint it at that brightness.
    let mut lit = w.query_filtered::<(
        &rc_game_core::LitByPlace,
        &rc_level_plugin::place_light::PlaceLit,
    ), With<crate::helmet::HelmetPiece>>();
    let brightness = lit
        .iter(w)
        .find_map(|(l, p)| p.light().map(|now| l.brightness(now)))
        .map_or(plan.brightness, |b| (b.x + b.y + b.z) / 3.0);
    let shots = w.resource::<Shots>();
    let pic = shots.pictures.get(name).ok_or("no screenshot")?;
    let (pw, ph) = (pic.width, pic.height);
    let points = rc_menus::helmet::fit(&model, &plan, pw as f32 / ph as f32);
    let want = rc_menus::helmet::paint(&model, &points, brightness, pw, ph);
    let (mut close, mut total) = (0usize, 0usize);
    for i in 0..pw * ph {
        if want.pixels[i * 4 + 3] == 0 {
            continue;
        }
        total += 1;
        let d: i32 = (0..3)
            .map(|k| (want.pixels[i * 4 + k] as i32 - pic.pixels[i * 4 + k] as i32).abs())
            .sum();
        if d <= 90 {
            close += 1;
        }
    }
    Ok(Some((close as f32 / total.max(1) as f32, total)))
}

/// The helmet frame is on screen in its own colours: most of the pixels
/// that the software painting of it covers match screenshot `name` (the
/// display drawn over it covers a few). Without the model in the install
/// it is enough that the log says why.
fn helmet_drawn(w: &mut World, name: &str) -> Result<String, String> {
    let readout = w
        .get_resource::<crate::helmet::HelmetReadout>()
        .cloned()
        .ok_or("no helmet readout")?;
    match &w.resource::<crate::helmet::Helmet>().model {
        Some(Err(e)) => return Ok(format!("not drawn, as the log says: {e}")),
        None => return Err("the model was never read".into()),
        Some(Ok(_)) => {}
    }
    if !readout.shown || readout.triangles == 0 {
        return Err(format!("{readout:?}"));
    }
    let (share, total) = helmet_match(w, name)?.unwrap_or_default();
    let got = format!(
        "{} triangles fitted to shape {:.3}; {:.0}% of its {} pixels match the painting",
        readout.triangles,
        readout.fitted_to,
        share * 100.0,
        total
    );
    if total > 0 && share > 0.7 {
        Ok(got)
    } else {
        Err(got)
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
        // The reticle is tinted (200,240,255) at opacity 110 (#101), so
        // over dark rock it shows as a pale blue-grey rather than white;
        // count both. The arena's rock and sand are never bluish.
        let whitish = min > 170 && pr.max(pg).max(pb) - min < 40;
        let tinted = min > 80 && pb > pr + 20 && pg > pr + 10;
        if (whitish || tinted)
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
            Step::Pointer(f) => {
                let at = f(world);
                log.line(format!("script: pointer {at:?}"));
                world.resource_mut::<ScriptPointer>().0 = at;
            }
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
            Step::Type(text) => {
                use bevy::input::keyboard::{Key, KeyboardInput};
                use bevy::input::ButtonState;
                let window = world
                    .query_filtered::<Entity, With<bevy::window::PrimaryWindow>>()
                    .iter(world)
                    .next();
                // One key a frame: pressed on even frames, released on odd.
                let chars: Vec<char> = text.chars().collect();
                let at = (frames / 2) as usize;
                if let (Some(window), Some(&c)) = (window, chars.get(at)) {
                    let (key_code, logical_key, typed) = match c {
                        '\u{8}' => (KeyCode::Backspace, Key::Backspace, None),
                        '\n' => (KeyCode::Enter, Key::Enter, None),
                        ' ' => (KeyCode::Space, Key::Space, Some(" ".into())),
                        c => (
                            KeyCode::KeyA,
                            Key::Character(c.to_string().into()),
                            Some(c.to_string().into()),
                        ),
                    };
                    world.write_message(KeyboardInput {
                        key_code,
                        logical_key,
                        state: if frames % 2 == 0 {
                            ButtonState::Pressed
                        } else {
                            ButtonState::Released
                        },
                        text: if frames % 2 == 0 { typed } else { None },
                        repeat: false,
                        window,
                    });
                }
                done = at >= chars.len();
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
            Step::ShotHolding(name, a, secs) => {
                held.insert(a);
                if t < secs {
                    done = false;
                } else {
                    let mut shots = world.resource_mut::<Shots>();
                    if shots.stats.contains_key(name) {
                        shots.attempts = 0;
                    } else {
                        done = false;
                        if shots.pending.is_none() && (shots.attempts == 0 || frames % 10 == 0) {
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
                    | Step::Type(_)
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
        "{read} sounds read from {packages}, {measured} measured, {missing} missing; played {:?}, {} from a place ({:?}), {} started on the sound card, {} not loaded; first line of speech {line} lasts {:.1} s",
        st.sounds_played,
        st.sounds_placed.len(),
        st.sounds_placed,
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
        && !st.sounds_placed.is_empty()
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

/// geo_01a's five bacta dispensers draw with their animated model.
fn level_anim_props(w: &mut World) -> Result<String, String> {
    let info = w
        .get_resource::<rc_level_plugin::LevelInfo>()
        .ok_or("no LevelInfo")?;
    let (drawn, failed) = (info.anim_props, info.anim_prop_failures);
    let mut q = w.query::<&rc_level_plugin::anim_props::AnimProp>();
    let dispensers = q
        .iter(w)
        .filter(|p| p.class.to_ascii_lowercase().starts_with("bactadispenser"))
        .count();
    let msg = format!("{drawn} drawn, {failed} not drawn, {dispensers} bacta dispensers");
    if dispensers >= 5 && failed == 0 {
        Ok(msg)
    } else {
        Err(msg)
    }
}

/// Stand a few steps back from the landing zone's dispenser, looking at
/// it, for a picture.
/// geo_01a's clone troopers hold the game's DC-15 rifle model (their
/// class's `RequiredEquipment` as `CloneTrooperDefaults` in
/// Properties/properties.u sets it), hung on their weapon bone.
fn troopers_hold_rifles(w: &mut World) -> Result<String, String> {
    let mut q = w.query::<&rc_cast::Cast>();
    let troopers: Vec<_> = q
        .iter(w)
        .filter(|c| c.class.eq_ignore_ascii_case("CloneTrooper"))
        .collect();
    let armed = troopers
        .iter()
        .filter(|c| c.held_weapon() == Some("DC15BlasterClone"))
        .count();
    let msg = format!("{armed} of {} clone troopers hold a DC15BlasterClone", troopers.len());
    if !troopers.is_empty() && armed == troopers.len() {
        Ok(msg)
    } else {
        Err(msg)
    }
}

/// geo_01a's landing gunship (`GunshipHigh1`, the cast part draws it).
fn gunship_hovers(w: &mut World) -> Result<String, String> {
    let mut q = w.query::<(&rc_cast::Cast, &GlobalTransform)>();
    let (c, t) = q
        .iter(w)
        .find(|(c, _)| c.class.eq_ignore_ascii_case("GunshipHigh"))
        .ok_or("no gunship in the cast")?;
    let at = rc_game_core::units::to_unreal(t.translation());
    let msg = format!(
        "{} drawn from ({:.0}, {:.0}, {:.0}), map place ({:.0}, {:.0}, {:.0})",
        c.name, at.x, at.y, at.z, c.location[0], c.location[1], c.location[2]
    );
    if (at.z - c.location[2]).abs() < 1.0 {
        Ok(msg)
    } else {
        Err(msg)
    }
}

/// Stand the player on the ground 1500 uu from the gunship, at the first
/// of 16 directions with a clear view of it, and look at it, for
/// logs/gunship.png.
fn view_gunship(w: &mut World) {
    use rc_movement::Vec3 as V;
    let gunship = {
        let mut q = w.query::<&rc_cast::Cast>();
        q.iter(w)
            .find(|c| c.class.eq_ignore_ascii_case("GunshipHigh"))
            .map(|c| c.location)
    };
    let (Some(g), Some(world)) = (
        gunship,
        w.get_resource::<rc_game_core::LevelCollision>()
            .map(|c| c.0.clone()),
    ) else {
        return;
    };
    let clear = |a: V, b: V| world.sweep(V::ZERO, a, b).is_none_or(|h| h.time >= 1.0);
    let spot = (0..16).find_map(|i| {
        let turn = i as f32 / 16.0 * std::f32::consts::TAU;
        let (x, y) = (g[0] + 1500.0 * turn.cos(), g[1] + 1500.0 * turn.sin());
        // The floor under that point.
        let top = V::new(x, y, g[2]);
        let h = world.sweep(V::ZERO, top, V::new(x, y, g[2] - 2000.0))?;
        if h.time >= 1.0 {
            return None;
        }
        let eye = V::new(x, y, g[2] - 2000.0 * h.time + 100.0);
        clear(eye, V::new(g[0], g[1], g[2])).then_some(eye)
    });
    let log = w.resource::<GameLog>().clone();
    let Some(at) = spot else {
        log.line("script: no clear view of the gunship found");
        return;
    };
    let (dx, dy, dz) = (g[0] - at.x, g[1] - at.y, g[2] - at.z);
    let mut q = w.query_filtered::<&mut Body, With<Player>>();
    for mut b in q.iter_mut(w) {
        b.pawn.location = at;
        b.yaw = dy.atan2(dx);
        b.pitch = dz.atan2((dx * dx + dy * dy).sqrt());
        log.line(format!(
            "script: player looks at the gunship from ({:.0}, {:.0}, {:.0})",
            at.x, at.y, at.z
        ));
    }
}

fn level_script_view_dispenser(w: &mut World) {
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
    let (dx, dy) = (spot[0] - d.location[0], spot[1] - d.location[1]);
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let back = 260.0;
    let at = [
        d.location[0] + dx / len * back,
        d.location[1] + dy / len * back,
        spot[2] + 10.0,
    ];
    let log = w.resource::<GameLog>().clone();
    let mut q = w.query_filtered::<&mut Body, With<Player>>();
    for mut b in q.iter_mut(w) {
        b.pawn.location = rc_movement::Vec3::new(at[0], at[1], at[2]);
        b.yaw = (-dy).atan2(-dx);
        b.pitch = 0.0;
        log.line(format!(
            "script: player looks at {} from ({:.0}, {:.0}, {:.0})",
            d.name, at[0], at[1], at[2]
        ));
    }
}

/// The landing zone dispenser's hologram is placed; it shows only when
/// `want` (a squad mate is hurt and the crosshair is on the squad spot);
/// the crosshair on the dispenser played the aim sound.
fn bacta_hologram_shown(w: &mut World, want: bool) -> Result<String, String> {
    let st = w
        .get_resource::<rc_scripts_plugin::bacta::BactaStats>()
        .cloned()
        .ok_or("no bacta stats")?;
    let msg = format!(
        "{} placed, {} showing, sounds {:?}",
        st.holograms, st.holograms_shown, st.sounds
    );
    let showing = st.holograms_shown >= 1;
    if st.holograms >= 1 && showing == want && st.sounds.iter().any(|s| s == "aim") {
        Ok(msg)
    } else {
        Err(msg)
    }
}

/// Take some health off the first squad mate (the bacta marker is only
/// for a hurt squad).
fn bacta_hurt_a_squad_mate(w: &mut World) {
    let log = w.resource::<GameLog>().clone();
    let mut q = w.query_filtered::<&mut rc_combat::Health, With<rc_squad_plugin::SquadMate>>();
    if let Some(mut h) = q.iter_mut(w).next() {
        h.current = (h.current - h.max * 0.4).max(1.0);
        log.line(format!(
            "script: a squad mate is hurt, health {:.0} of {:.0}",
            h.current, h.max
        ));
    }
}

/// After holding Use at the dispenser: the player was pulled to the spot
/// in front of it and is standing there, the use panel reached full, and
/// the healing and "health full" sounds played.
fn bacta_pull_and_panel(w: &mut World) -> Result<String, String> {
    let st = w
        .get_resource::<rc_scripts_plugin::bacta::BactaStats>()
        .cloned()
        .ok_or("no bacta stats")?;
    let stand = st.last_stand.ok_or("never pulled")?;
    if st.holograms_while_healing > 0 {
        return Err(format!(
            "a hologram showed {} time(s) while the player healed",
            st.holograms_while_healing
        ));
    }
    let at = feet(w).ok_or("no player")?;
    let off = ((at.x - stand[0]).powi(2) + (at.y - stand[1]).powi(2)).sqrt();
    let msg = format!(
        "pulled {} time(s) to ({:.0}, {:.0}), player now {off:.1} uu from it; panel reached {:.0}%; sounds {:?}",
        st.pulls,
        stand[0],
        stand[1],
        st.panel_best * 100.0,
        st.sounds
    );
    let heard = |k: &str| st.sounds.iter().any(|s| s == k);
    if st.pulls >= 1 && off < 2.0 && st.panel_best > 0.99 && heard("healing") && heard("full") {
        Ok(msg)
    } else {
        Err(msg)
    }
}

/// After healing at the dispenser: the game's `DispenserEffectClass`
/// (`BactaZap_Dispenser`) was hung on the `Eye` bone, its hum
/// (`AmbientSound`) started, and both were taken away when healing ended.
fn bacta_zap(w: &mut World) -> Result<String, String> {
    let st = w
        .get_resource::<rc_scripts_plugin::bacta::BactaStats>()
        .cloned()
        .ok_or("no bacta stats")?;
    let fx = w
        .get_resource::<rc_effects_plugin::EffectStats>()
        .cloned()
        .ok_or("no effect stats")?;
    let still: Vec<String> = w
        .query::<&rc_effects_plugin::RunningEffect>()
        .iter(w)
        .filter(|r| r.name().eq_ignore_ascii_case("BactaZap_Dispenser"))
        .map(|r| r.name().to_string())
        .collect();
    let carriers = w
        .query::<&rc_scripts_plugin::bacta::BactaZap>()
        .iter(w)
        .count();
    let hum = fx.sounds.iter().any(|s| s.eq_ignore_ascii_case("ele_GEO_genEnrgy_lp_01"));
    let msg = format!(
        "zaps {:?}, removed {}; effect sounds {:?}; still running {:?}, carriers {carriers}",
        st.zaps, st.zaps_removed, fx.sounds, still
    );
    let ok = st
        .zaps
        .iter()
        .any(|z| z.0.eq_ignore_ascii_case("BactaZap_Dispenser"))
        && hum
        && st.zaps_removed >= 1
        && still.is_empty()
        && carriers == 0;
    if ok { Ok(msg) } else { Err(msg) }
}

/// The arms' moves while healing at the dispenser, in the game's order
/// (`HandSignal` with `SetupBactaHealer`): the gun's `Holster`, the hands'
/// `BactaLoad`, `BactaLoop` while Use is held, `BactaHolster`, the gun's
/// `Load`.
fn bacta_hands_played(w: &mut World) -> Result<String, String> {
    let seen = w
        .get_resource::<rc_combat::ArmsHistory>()
        .map(|h| h.0.clone())
        .ok_or("no arms history")?;
    let from = seen
        .iter()
        .position(|s| s.starts_with("hands:bacta"))
        .ok_or_else(|| format!("the hands never showed: {seen:?}"))?;
    let gun = |s: &String| !s.starts_with("hands:") && !s.starts_with("nothing");
    let before = from.checked_sub(1).map(|i| &seen[i]);
    let ours: Vec<&str> = seen[from..]
        .iter()
        .take_while(|s| s.starts_with("hands:"))
        .map(|s| s.as_str())
        .collect();
    let after = seen.get(from + ours.len());
    let msg = format!("before {before:?}, hands {ours:?}, after {after:?}");
    let ok = before.is_some_and(|b| gun(b) && b.ends_with(" Holster"))
        && ours.first() == Some(&"hands:bacta BactaLoad")
        && ours.get(1) == Some(&"hands:bacta BactaLoop")
        && ours.last() == Some(&"hands:bacta BactaHolster")
        && after.is_some_and(|a| gun(a) && a.ends_with(" Load"));
    if ok {
        Ok(msg)
    } else {
        Err(msg)
    }
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
    // Hurt, so Use at the dispenser does something (at full health it
    // doesn't, as in the game).
    let mut q = w.query_filtered::<&mut rc_combat::Health, With<Player>>();
    for mut h in q.iter_mut(w) {
        h.current = h.current.min(h.max - 60.0).max(1.0);
    }
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
const DEMO_ANCHOR: &str = "DemolitionCharge2";
const DEMO_DONE_EVENT: &str = "BunkerDoorDestroyed";
const HANGAR_LOCK: &str = "HackDoorGeo6";
const HANGAR_EVENT: &str = "HangarDoorHacked";
const HANGAR_DOOR: &str = "MoverDB_GEO6";

/// Report a squad marker job done, as the squad part does when a squad
/// mate finishes it.
fn marker_job(w: &mut World, anchor: &str) {
    w.write_message(rc_game_core::MarkerJob {
        anchor: anchor.into(),
        step: rc_game_core::MarkerJobStep::Done,
        by: "self-test".into(),
    });
}

/// Whether the level's scripts have had this event.
fn marker_heard(w: &mut World, event: &str) -> bool {
    w.get_resource::<rc_scripts_plugin::LevelScript>()
        .is_some_and(|s| {
            s.director
                .history
                .iter()
                .any(|(_, e)| e.eq_ignore_ascii_case(event))
        })
}

/// Whether the door with this actor name is shut (`None`: no such door).
fn door_closed(w: &mut World, name: &str) -> Option<bool> {
    let m = w.get_resource::<rc_movers_plugin::LevelMovers>()?;
    let set = m.read();
    set.movers
        .iter()
        .find(|d| d.info.place.name.eq_ignore_ascii_case(name))
        .map(|d| d.is_closed())
}

fn marker_demo_summary(w: &mut World) -> Result<String, String> {
    let Some(s) = w.get_resource::<rc_scripts_plugin::LevelScript>() else {
        return Err("no level scripts".into());
    };
    let sent: Vec<&str> = s
        .director
        .history
        .iter()
        .map(|(_, e)| e.as_str())
        .filter(|e| {
            [
                "DemoUnderway",
                "SpawnDemoDroids",
                "DemoChargeArmed",
                DEMO_DONE_EVENT,
            ]
            .iter()
            .any(|x| x.eq_ignore_ascii_case(e))
        })
        .collect();
    // ScriptedTrigger4 waits for BunkerDoorDestroyed at its action 1.
    let waiting = s
        .director
        .script_states()
        .into_iter()
        .find(|(n, ..)| n.eq_ignore_ascii_case("ScriptedTrigger4"))
        .map(|(_, next, what)| format!("{next} {what}"))
        .unwrap_or_default();
    let got = format!(
        "sent {sent:?}; steps {:?}; props destroyed {:?}; ScriptedTrigger4 at {waiting}",
        s.stats.marker_steps, s.stats.props_destroyed
    );
    let gone = !s.stats.props_destroyed.is_empty();
    if sent.len() == 4 && gone && !waiting.contains("bunkerdoordestroyed") {
        Ok(got)
    } else {
        Err(got)
    }
}

const CAST_SPAWN: &str = "SpawnTrench02";
const CAST_GO: &str = "LoneRangerJump";
const CAST_TAG: &str = "LoneRanger";
const CAST_SCRIPT: &str = "LoneRangerAI";
const CAST_SEQUENCE: &str = "ScriptedSequence21";

fn cast_spawn(w: &mut World) {
    if let Some(mut s) = w.get_resource_mut::<rc_scripts_plugin::LevelScript>() {
        s.director.fire(CAST_SPAWN, "self-test");
    }
}

fn cast_go(w: &mut World) {
    if let Some(mut s) = w.get_resource_mut::<rc_scripts_plugin::LevelScript>() {
        s.director.fire(CAST_GO, "self-test");
    }
}

/// The living characters tagged LoneRanger: (entity, what scripts call
/// them).
fn cast_members(w: &mut World) -> Vec<(Entity, rc_game_core::ScriptTags)> {
    let mut q = w.query_filtered::<(Entity, &rc_game_core::ScriptTags), Without<rc_combat::Dead>>();
    q.iter(w)
        .filter(|(_, t)| {
            t.tag
                .as_deref()
                .is_some_and(|x| x.eq_ignore_ascii_case(CAST_TAG))
        })
        .map(|(e, t)| (e, t.clone()))
        .collect()
}

fn cast_appeared(w: &mut World) -> bool {
    !cast_members(w).is_empty()
}

fn cast_state(w: &mut World) -> String {
    w.get_resource::<rc_scripts_plugin::LevelScript>()
        .and_then(|s| {
            s.director
                .script_states()
                .into_iter()
                .find(|(n, ..)| n.eq_ignore_ascii_case(CAST_SEQUENCE))
        })
        .map_or("no such sequence".into(), |(_, next, what)| {
            format!("at {next}, {what}")
        })
}

fn cast_tagged(w: &mut World) -> Result<String, String> {
    let m = cast_members(w);
    let state = cast_state(w);
    let got = format!(
        "{} tagged {CAST_TAG}, scripts {:?}, follow orders {:?}; {CAST_SEQUENCE} {state}",
        m.len(),
        m.iter().map(|(_, t)| t.script.clone()).collect::<Vec<_>>(),
        m.iter().map(|(_, t)| t.follows_orders).collect::<Vec<_>>()
    );
    let ok = m.len() == 1
        && m[0]
            .1
            .script
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case(CAST_SCRIPT))
        && m[0].1.follows_orders
        && state.contains(&CAST_GO.to_ascii_lowercase());
    if ok {
        Ok(got)
    } else {
        Err(got)
    }
}

/// His sequence is over and its last order (removing him) has landed.
fn cast_finished(w: &mut World) -> bool {
    cast_state(w).contains("finished") && cast_members(w).is_empty()
}

fn cast_summary(w: &mut World) -> Result<String, String> {
    let left = cast_members(w).len();
    let Some(s) = w.get_resource::<rc_scripts_plugin::LevelScript>() else {
        return Err("no level scripts".into());
    };
    let place = |t: &str| s.places.find(t).map(|p| p.location);
    let (jump, kill) = (
        place("JumpDestForLoneRanger"),
        place("KillPointForLoneRanger"),
    );
    let done: Vec<String> = s
        .stats
        .character_done
        .iter()
        .filter(|d| d.starts_with(CAST_SEQUENCE))
        .cloned()
        .collect();
    let orders: Vec<String> = s
        .stats
        .character_orders
        .iter()
        .filter(|o| o.contains(CAST_SCRIPT) || o.contains(CAST_TAG))
        .cloned()
        .collect();
    let fight = w.resource::<rc_enemies::Fight>();
    let log: Vec<String> = fight
        .stats
        .scripted
        .iter()
        .filter(|l| l.contains(CAST_SEQUENCE))
        .cloned()
        .collect();
    // Where he was when each walk was reported done, against the map's
    // points.
    let at = |index: usize| -> Option<rc_movement::Vec3> {
        let l = log
            .iter()
            .find(|l| l.contains(&format!("{CAST_SEQUENCE}#{index} at")))?;
        let n: Vec<f32> = l
            .rsplit(" at ")
            .next()?
            .split(' ')
            .filter_map(|x| x.parse().ok())
            .collect();
        (n.len() == 3).then(|| rc_movement::Vec3::new(n[0], n[1], n[2]))
    };
    let off = |p: Option<rc_movement::Vec3>, q: Option<[f32; 3]>| match (p, q) {
        (Some(p), Some(q)) => (p - rc_movement::Vec3::new(q[0], q[1], q[2])).length_2d(),
        _ => f32::INFINITY,
    };
    let (d1, d4) = (off(at(1), jump), off(at(4), kill));
    let got = format!(
        "done by the character {done:?}; walks ended {d1:.0} and {d4:.0} uu from their points; orders {orders:?}; enemies part {log:?}; {left} still in the level"
    );
    let has = |i: usize| done.iter().any(|d| d.ends_with(&format!("#{i}")));
    // #1 and #4 walks, #2 turn, #3 animation: each finished by him, not by
    // the time-out; then #5 removed him.
    let anim_ok = log.iter().any(|l| l.starts_with("anim EmoteSeeEnemy"));
    if has(1) && has(2) && has(3) && has(4) && d1 < 100.0 && d4 < 100.0 && anim_ok && left == 0 {
        Ok(got)
    } else {
        Err(got)
    }
}

/// The `[enemies]` save part without what a running fight changes from
/// one frame to the next: each fighter's facing, mode and where it last
/// saw its enemy, where a fighting one stands (it may be walking to
/// cover), and when each spawner tries next. Who is there, how hurt, and
/// what each spawner has made are still compared.
fn fight_settled(v: &toml::Value) -> toml::Value {
    let mut v = v.clone();
    if let Some(t) = v.as_table_mut() {
        for (list, keys) in [
            ("droids", &["facing", "mode", "last_seen_at"][..]),
            ("spawners", &["next_spawn"][..]),
        ] {
            if let Some(items) = t.get_mut(list).and_then(|x| x.as_array_mut()) {
                for item in items.iter_mut().filter_map(|x| x.as_table_mut()) {
                    // One that is fighting may be walking to cover.
                    let fighting = item
                        .get("mode")
                        .and_then(|m| m.as_str())
                        .is_some_and(|m| m != "idle");
                    if fighting {
                        item.remove("feet");
                    }
                    for k in keys {
                        item.remove(*k);
                    }
                }
            }
        }
    }
    v
}

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

/// The barrel the props checks shoot, and the stand-in above it.
#[derive(Resource)]
struct BarrelTest {
    export: usize,
    name: String,
    dummy: Entity,
}

const BARREL_DUMMY_HEALTH: f32 = 5000.0;

fn props_read(w: &mut World) -> Result<String, String> {
    let s = w
        .get_resource::<rc_scripts_plugin::LevelScript>()
        .ok_or("no level scripts")?;
    let all: Vec<_> = s.breaking.iter().collect();
    let barrels = all.iter().filter(|b| b.explosion.is_some()).count();
    let msg = format!(
        "{} breakable props, {} that explode, director knows {}",
        all.len(),
        barrels,
        s.director.stats.breakables
    );
    if barrels >= 2 && s.director.stats.breakables == all.len() {
        Ok(msg)
    } else {
        Err(msg)
    }
}

fn props_shoot_barrel(w: &mut World) {
    let Some(s) = w.get_resource::<rc_scripts_plugin::LevelScript>() else {
        return;
    };
    let mut barrels: Vec<_> = s
        .breaking
        .iter()
        .filter(|b| b.explosion.is_some() && !b.hidden)
        .map(|b| (b.export, b.name.clone()))
        .collect();
    barrels.sort();
    let Some((export, name)) = barrels.first().cloned() else {
        return;
    };
    let Some((lo, hi)) = w
        .get_resource::<rc_level_plugin::LevelParts>()
        .and_then(|p| p.bounds_of(export))
    else {
        return;
    };
    let centre = rc_movement::Vec3::new(
        (lo.x + hi.x) * 0.5,
        (lo.y + hi.y) * 0.5,
        (lo.z + hi.z) * 0.5,
    );
    let top = hi.z;
    // The stand-in's feet 120 uu over the barrel's top: its middle is
    // about 250 uu from the barrel's middle.
    let feet = rc_movement::Vec3::new(centre.x, centre.y, top + 120.0);
    let dummy = w
        .spawn((
            Transform::from_translation(rc_game_core::units::to_bevy(feet)),
            rc_combat::Hittable::COMMANDO,
            rc_combat::Health::new(BARREL_DUMMY_HEALTH),
            DespawnOnExit(GameState::InGame),
        ))
        .id();
    let shooter = {
        let mut q = w.query_filtered::<Entity, With<Player>>();
        q.iter(w).next()
    };
    let Some(shooter) = shooter else { return };
    let origin =
        rc_game_core::units::to_bevy(rc_movement::Vec3::new(centre.x, centre.y, top + 30.0));
    for _ in 0..3 {
        w.write_message(rc_combat::FireShot {
            shooter,
            origin,
            direction: Vec3::NEG_Y,
            weapon: "dc17m_blaster".into(),
        });
    }
    w.resource::<GameLog>().line(format!(
        "script: shooting {name} (export {export}) three times from 30 uu above it; stand-in {dummy:?} above it"
    ));
    w.insert_resource(BarrelTest {
        export,
        name,
        dummy,
    });
}

fn props_barrel_broken(w: &mut World) -> bool {
    let Some(t) = w.get_resource::<BarrelTest>() else {
        return false;
    };
    let (name, export) = (t.name.clone(), t.export);
    w.get_resource::<rc_scripts_plugin::LevelScript>()
        .is_some_and(|s| {
            s.props.broken.contains(&name)
                && s.director.prop_health(export).is_some_and(|h| h <= 0.0)
        })
}

fn props_barrel_exploded(w: &mut World) -> bool {
    let Some(t) = w.get_resource::<BarrelTest>() else {
        return false;
    };
    let name = t.name.clone();
    w.get_resource::<rc_scripts_plugin::LevelScript>()
        .is_some_and(|s| s.props.exploded.contains(&name))
}

fn props_blast_hurt(w: &mut World) -> Result<String, String> {
    let t = w.get_resource::<BarrelTest>().ok_or("no barrel test")?;
    let (export, dummy) = (t.export, t.dummy);
    let health = w
        .get::<rc_combat::Health>(dummy)
        .map(|h| h.current)
        .ok_or("the stand-in is gone")?;
    let solid = w
        .get_resource::<rc_level_plugin::LevelParts>()
        .and_then(|p| p.list().into_iter().find(|x| x.0 == export))
        .map(|x| x.3);
    let hits = w
        .get_resource::<rc_scripts_plugin::LevelScript>()
        .map_or(0, |s| s.props.hits);
    // It must also be gone from view: every drawn piece of it hidden.
    let (mut drawn, mut hidden) = (0, 0);
    let mut q = w.query::<(&rc_level_viewer::draw::MoverPart, &Visibility)>();
    for (p, v) in q.iter(w) {
        if p.0 == export {
            drawn += 1;
            hidden += (*v == Visibility::Hidden) as usize;
        }
    }
    let msg = format!(
        "stand-in health {health:.0} of {BARREL_DUMMY_HEALTH}; barrel solid now {solid:?}, {hidden} of {drawn} drawn pieces hidden; {hits} hits on props"
    );
    if health < BARREL_DUMMY_HEALTH && solid == Some(false) && drawn > 0 && hidden == drawn {
        Ok(msg)
    } else {
        Err(msg)
    }
}

/// geo_01a's five OverheadVehicles spawners are on.
fn fx_overhead_on(w: &mut World) -> bool {
    w.get_resource::<rc_scripts_plugin::fx_factories::LevelFxFactories>()
        .is_some_and(|fx| {
            fx.spawners
                .list
                .iter()
                .zip(&fx.spawners.states)
                .filter(|(f, s)| f.tag.eq_ignore_ascii_case("OverheadVehicles") && s.online)
                .count()
                == 5
        })
}

/// Send a level event straight to the effect and projectile spawners (not
/// through the level's scripts, so nothing else waiting for it goes on).
fn spawner_event(w: &mut World, name: &str) {
    w.write_message(rc_game_core::ScriptEvent {
        name: name.to_string(),
        by: "the self-test".to_string(),
    });
}

/// Projectiles of `class` made and blown up, with their trail and blast
/// effects started and none missing.
fn projectiles_blew_up(w: &mut World, class: &str, trail: &str, high: Option<f32>) -> bool {
    let Some(p) = w.get_resource::<rc_scripts_plugin::projectiles::Projectiles>() else {
        return false;
    };
    let made = p.made.iter().filter(|(_, c)| c == class).count();
    let blasts: Vec<[f32; 3]> = p
        .blasts
        .iter()
        .filter(|(c, _, world)| c == class && *world)
        .map(|b| b.1)
        .collect();
    let highest = p.highest;
    let Some(st) = w.get_resource::<rc_effects_plugin::EffectStats>() else {
        return false;
    };
    let has = |n: &str| st.spawned.iter().any(|e| e == n);
    let missing: Vec<&String> = st
        .missing
        .iter()
        .filter(|m| *m == trail || *m == "Explosion_Barrel_AlongX")
        .collect();
    let ok = made > 0
        && !blasts.is_empty()
        && has(trail)
        && has("Explosion_Barrel_AlongX")
        && high.is_none_or(|h| highest.is_some_and(|z| z > h));
    if ok || !missing.is_empty() {
        w.resource::<GameLog>().line(format!(
            "script: projectiles {class}: {made} made, {} blew up on the level (first at {:?}), highest {highest:?} uu; trail {trail} started {}, blast effect started {}; missing {missing:?}",
            blasts.len(),
            blasts.first().map(|b| b.map(|v| v.round())),
            has(trail),
            has("Explosion_Barrel_AlongX"),
        ));
    }
    ok && missing.is_empty()
}

fn bunker_missiles_blow_up(w: &mut World) -> bool {
    projectiles_blew_up(w, "GEO_01A_BunkerMissile", "GEO_BunkerMissile_Proj", None)
}

fn bombardment_blows_up(w: &mut World) -> bool {
    projectiles_blew_up(w, "GEO_01AProj", "GEO_01AProjectile", Some(15000.0))
}

/// Gunships and fighters made, their models drawn far above the arena's
/// floor (about -2300 uu), and nothing missing to draw them.
fn fx_overhead_flying(w: &mut World) -> bool {
    let made: Vec<String> = w
        .get_resource::<rc_scripts_plugin::fx_factories::LevelFxFactories>()
        .map(|fx| fx.made.iter().map(|(_, e)| e.clone()).collect())
        .unwrap_or_default();
    let Some(st) = w.get_resource::<rc_effects_plugin::EffectStats>() else {
        return false;
    };
    let gunships = made.iter().any(|e| e == "cFX_Gunships");
    let fighters = made.iter().any(|e| e == "cFX_GEOfighters");
    let high = st.highest_mesh.is_some_and(|z| z > 5000.0);
    let ok = gunships && fighters && high && st.meshes_drawn > 0;
    let missing: Vec<&String> = st
        .missing
        .iter()
        .filter(|m| m.starts_with("cFX_"))
        .collect();
    if ok || !missing.is_empty() {
        w.resource::<GameLog>().line(format!(
            "script: flyovers: made {made:?}; {} models drawn now, highest at {:?} uu; missing {missing:?}",
            st.meshes_drawn, st.highest_mesh
        ));
    }
    ok && missing.is_empty()
}

fn props_effect_shown(w: &mut World) -> Result<String, String> {
    let st = w
        .get_resource::<rc_effects_plugin::EffectStats>()
        .ok_or("no effect stats")?;
    let msg = format!(
        "effects {:?}, up to {} squares and {} debris models drawn, sounds {:?}, missing {:?}",
        st.spawned, st.peak_sprites, st.peak_meshes, st.sounds, st.missing
    );
    let has = |n: &str| st.sounds.iter().any(|s| s.eq_ignore_ascii_case(n));
    if st.spawned.iter().any(|e| e == "Explosion_Barrel_GEO")
        && st.peak_sprites > 0
        && st.peak_meshes > 0
        && has("exp_GEN_barrel_01")
        && has("exp_barrels")
        // Only the barrel's own parts count here: the missiles' trails
        // have a ribbon part that isn't drawn yet, and that is reported
        // by the spawner checks instead.
        && !st.missing.iter().any(|m| m.starts_with("Explosion_Barrel"))
    {
        Ok(msg)
    } else {
        Err(msg)
    }
}

fn props_effect_over(w: &mut World) -> bool {
    // The barrel's own effect (the overhead flyovers run all level long).
    let running = {
        let mut q = w.query::<&rc_effects_plugin::RunningEffect>();
        q.iter(w)
            .filter(|r| r.name().eq_ignore_ascii_case("Explosion_Barrel_GEO"))
            .count()
    };
    running == 0
        && w.get_resource::<rc_effects_plugin::EffectStats>()
            .is_some_and(|s| s.finished >= 1)
}

fn props_simple_shapes(w: &mut World) -> Result<String, String> {
    let info = w
        .get_resource::<rc_level_plugin::LevelInfo>()
        .ok_or("no level info")?;
    let msg = format!(
        "{}: {} props collide through their simple shape, {} stop shots but not people",
        info.map, info.simple_shape_props, info.shot_only_props
    );
    if info.simple_shape_props > 0 && info.shot_only_props > 0 {
        Ok(msg)
    } else {
        Err(msg)
    }
}

/// For each walk-through prop that stops shots, sweep a line and a small
/// box from just outside its box to its middle, along each level axis
/// both ways. Passes when some way has the line stopping at the prop
/// while the box goes on (other walls may be in the way of some ways).
fn props_shots_only(w: &mut World) -> Result<String, String> {
    let parts = w
        .get_resource::<rc_level_plugin::LevelParts>()
        .cloned()
        .ok_or("no LevelParts")?;
    let world = w.resource::<LevelCollision>().0.clone();
    let exports = parts.shots_only();
    let body = rc_movement::Vec3::new(10.0, 10.0, 10.0);
    let mut good = 0;
    for &e in &exports {
        let Some((lo, hi)) = parts.bounds_of(e) else {
            continue;
        };
        let c = (lo + hi) * 0.5;
        let half = (hi - lo) * 0.5;
        let ways = [
            rc_movement::Vec3::new(1.0, 0.0, 0.0),
            rc_movement::Vec3::new(-1.0, 0.0, 0.0),
            rc_movement::Vec3::new(0.0, 1.0, 0.0),
            rc_movement::Vec3::new(0.0, -1.0, 0.0),
        ];
        if ways.iter().any(|&d| {
            let reach = (half.x * d.x).abs() + (half.y * d.y).abs() + 30.0;
            let a = c - d * reach;
            let line = world.sweep(rc_movement::Vec3::ZERO, a, c);
            let boxed = world.sweep(body, a, c);
            line.is_some_and(|h| !h.start_solid) && boxed.is_none()
        }) {
            good += 1;
        }
    }
    let msg = format!(
        "{} walk-through props that stop shots; at {good} a line stops at it while a body walks on",
        exports.len()
    );
    if good > 0 {
        Ok(msg)
    } else {
        Err(msg)
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
    let got = format!(
        "{name} at ({:.0}, {:.0}, {:.0}) to {url}",
        at[0], at[1], at[2]
    );
    match rc_scripts::level_url(&url) {
        Some((m, _)) if m == "geo_01b" => Ok(got),
        _ => Err(got),
    }
}

/// Note the health and magazine, take a little health off (so carrying it
/// over shows), and put the player in the exit.
/// Words for the debriefing check: this script's own, not the game's.
const DEBRIEF_TITLE: &str = "SCRIPT DEBRIEFING";
const DEBRIEF_TEXT: &str = "A test of the debriefing screen: this text is the self-test's own, and long enough to wrap onto a second line.";

fn debriefing_ask(w: &mut World) {
    w.write_message(rc_game_core::ShowDebriefing {
        menu_class: "XInterfaceCTMenus.CTDebriefingInfoGEO".into(),
        title: DEBRIEF_TITLE.into(),
        text: DEBRIEF_TEXT.into(),
        // Named without its group, as geo_05c names it.
        picture: "GUIContent.GEO_Debrief".into(),
        then: rc_game_core::ChangeLevel {
            map: "geo_01b".into(),
            portal: None,
            by: "the self-test".into(),
        },
    });
    w.resource::<GameLog>()
        .line("script: asked for the debriefing, then on to geo_01b");
}

fn debriefing_shown(w: &mut World) -> Result<String, String> {
    let play = w.get_resource::<State<Play>>().map(|p| *p.get());
    if play != Some(Play::Paused) {
        return Err(format!("play is {play:?}, not paused"));
    }
    if page(w) != Page::Debriefing {
        return Err(format!("page {:?}", page(w)));
    }
    if !w.resource::<OriginalMenus>().active() {
        return Ok("no original menus: placeholder page".into());
    }
    let k = shown_page(w).ok_or("placeholder page, see the log for why")?;
    let m = w.resource::<OriginalMenus>();
    let screen = &m.pages[&k].screen;
    let text = |name: &str| match screen.layout.widget(name).map(|w| &w.kind) {
        Some(rc_menus::layout::Kind::Text(t)) => t.text.clone(),
        _ => String::new(),
    };
    let picture = match screen.layout.widget("InfoPic").map(|w| &w.kind) {
        Some(rc_menus::layout::Kind::Sprite(s)) => s.texture.clone().unwrap_or_default(),
        _ => String::new(),
    };
    let got = format!(
        "title {:?}, picture {picture}, focus on {:?}, buttons {:?}",
        text("InfoTitle"),
        screen.focused_name(),
        screen.reachable_names()
    );
    if text("InfoTitle") == DEBRIEF_TITLE
        && text("InfoText") == DEBRIEF_TEXT
        && picture.eq_ignore_ascii_case("GUIContent.LoadInfoScreens.GEO_Debrief")
        && screen.focused_name() == Some("AButton")
    {
        Ok(got)
    } else {
        Err(got)
    }
}

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
    let start = *w
        .get_resource::<rc_game_core::PlayerStart>()
        .ok_or("no PlayerStart")?;
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
    let (health, _, _) = w
        .resource::<Noted>()
        .leaving
        .clone()
        .ok_or("didn't leave")?;
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

// Ammo and grenade boxes (`rc_combat::ammo_box`), checked on geo_01a: 21
// DC-17m boxes and the grenade crate by the start, which comes back.

fn ammo_boxes_placed(w: &mut World) -> Result<String, String> {
    use rc_combat::ammo_box::AmmoBox;
    let mut q = w.query::<&AmmoBox>();
    let boxes: Vec<AmmoBox> = q.iter(w).cloned().collect();
    let mut parts = w.query::<&rc_level_viewer::draw::MoverPart>();
    let drawn: HashSet<usize> = parts.iter(w).map(|p| p.0).collect();
    let with_model = boxes.iter().filter(|b| drawn.contains(&b.export)).count();
    let rifle = boxes
        .iter()
        .filter(|b| b.class == "DC17mBlasterBox")
        .count();
    let usable = boxes.iter().filter(|b| b.gives.is_some()).count();
    let amounts: HashSet<u32> = boxes
        .iter()
        .filter(|b| b.class == "DC17mBlasterBox")
        .map(|b| b.amount)
        .collect();
    let got = format!(
        "{} boxes ({rifle} DC-17m, giving {amounts:?}), {with_model} with a drawn model, {usable} usable",
        boxes.len()
    );
    if boxes.len() == 22
        && rifle == 21
        && with_model == 22
        && usable == 22
        && amounts == HashSet::from([300])
    {
        Ok(got)
    } else {
        Err(got)
    }
}

fn set_rifle(w: &mut World, clip: f32, reserve: u32) {
    let mut q = w.query_filtered::<&mut rc_combat::Arsenal, With<Player>>();
    if let Some(mut a) = q.iter_mut(w).next() {
        if let Some((_, s)) = a.weapons.iter_mut().find(|(id, _)| id == "dc17m_blaster") {
            s.clip = clip;
            s.reserve = reserve;
            s.reloading = None;
        }
    }
}

fn set_grenades(w: &mut World, id: &str, n: u32) {
    let mut q = w.query_filtered::<&mut rc_combat::Grenades, With<Player>>();
    if let Some(mut g) = q.iter_mut(w).next() {
        if let Some((_, c)) = g.carried.iter_mut().find(|(i, _)| i == id) {
            *c = n;
        }
    }
}

/// Put the `nth` untaken box of `class` at the player's feet (its centre
/// one half-height up), as if they walked into it. Returns its name.
fn move_box_to_player(w: &mut World, class: &str, nth: usize) -> Option<String> {
    let mut q = w.query_filtered::<&Transform, With<Player>>();
    let feet = q.iter(w).next()?.translation;
    let mut boxes = w.query::<(&mut Transform, &rc_combat::ammo_box::AmmoBox)>();
    let mut list: Vec<_> = boxes
        .iter_mut(w)
        .filter(|(_, b)| b.class == class && !b.taken)
        .collect();
    list.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    let (t, b) = list.get_mut(nth)?;
    t.translation = feet + Vec3::Y * (b.height / rc_game_core::units::UU_PER_UNIT);
    Some(b.name.clone())
}

fn carries(w: &mut World, id: &str) -> bool {
    let mut q = w.query_filtered::<&rc_combat::Arsenal, With<Player>>();
    q.iter(w)
        .next()
        .is_some_and(|a| a.weapons.iter().any(|(w, _)| w == id))
}

fn weapons_lying(w: &mut World, id: &str) -> usize {
    let mut q = w.query::<&rc_combat::WeaponPickup>();
    q.iter(w).filter(|p| p.weapon == id).count()
}

fn rifle_total(w: &mut World) -> Option<u32> {
    let mut q = w.query_filtered::<&rc_combat::Arsenal, With<Player>>();
    let a = q.iter(w).next()?;
    let (_, s) = a.weapons.iter().find(|(id, _)| id == "dc17m_blaster")?;
    Some(s.shots_in_clip() + s.reserve)
}

/// The box's taken flag and whether its model is hidden.
fn box_state(w: &mut World, name: &str) -> Option<(bool, bool)> {
    let mut q = w.query::<&rc_combat::ammo_box::AmmoBox>();
    let b = q.iter(w).find(|b| b.name == name)?.clone();
    let mut parts = w.query::<(&rc_level_viewer::draw::MoverPart, &Visibility)>();
    let hidden = parts
        .iter(w)
        .filter(|(p, _)| p.0 == b.export)
        .all(|(_, v)| *v == Visibility::Hidden);
    Some((b.taken, hidden))
}

fn ammo_box_taken(w: &mut World) -> Result<String, String> {
    let name = w
        .resource::<Noted>()
        .ammo_box
        .clone()
        .ok_or("no box moved")?;
    let total = rifle_total(w).ok_or("no rifle")?;
    let (taken, hidden) = box_state(w, &name).ok_or("box gone")?;
    let got =
        format!("{name}: rifle has {total} (was 20), box taken {taken}, model hidden {hidden}");
    if total == 300 && taken && hidden {
        Ok(got)
    } else {
        Err(got)
    }
}

/// The map's current objective on the original HUD: its letters in the
/// original's colour (164, 200, 208), centred across the screen and
/// moved through the helmet's glass to about 1/7 down (the original
/// draws it 45 down its 600-line HUD).
fn objective_line(w: &mut World) -> Result<String, String> {
    let shown = w
        .get_resource::<rc_scripts_plugin::Objectives>()
        .and_then(|o| o.shown())
        .unwrap_or_default();
    let quads = crate::original_hud::layout_drawn(w);
    let letters: Vec<_> = quads
        .iter()
        .filter(|q| q.widget == "messages" && q.color[..3] == [164, 200, 208])
        .collect();
    let n = letters.len().max(1) as f32;
    let top = letters.iter().map(|q| q.dst[1]).fold(1.0f32, f32::min);
    let middle = letters
        .iter()
        .map(|q| (q.dst[0] + q.dst[2]) / 2.0)
        .sum::<f32>()
        / n;
    let got = format!(
        "objective {shown:?}: {} letters, top {:.1}% down, centred {:.1}% across",
        letters.len(),
        top * 100.0,
        middle * 100.0
    );
    let wanted = shown.chars().filter(|c| !c.is_whitespace()).count();
    if wanted > 0
        && letters.len() >= wanted
        && (0.10..0.20).contains(&top)
        && (0.40..0.60).contains(&middle)
    {
        Ok(got)
    } else {
        Err(got)
    }
}

/// The box just taken: its words on the original HUD (the "pickup" group,
/// above the middle) and its sound played.
fn pickup_feedback(w: &mut World) -> Result<String, String> {
    let quads = crate::original_hud::layout_drawn(w);
    let words: Vec<_> = quads.iter().filter(|q| q.widget == "pickup").collect();
    let middle = words
        .iter()
        .map(|q| (q.dst[1] + q.dst[3]) / 2.0)
        .sum::<f32>()
        / words.len().max(1) as f32;
    let sounds = w.resource::<rc_combat::pickup_sound::PickupSounds>();
    let played = sounds.played.clone();
    let got = format!(
        "{} letters drawn in the pickup line, centred {:.0}% down; sounds played {played:?}",
        words.len(),
        middle * 100.0
    );
    // "DC-17m Blaster Clips" is 18 letters without the spaces.
    if words.len() >= 15
        && (0.30..0.40).contains(&middle)
        && played
            .iter()
            .any(|s| s.eq_ignore_ascii_case("GEN_Sound.GenericWeaponPickup"))
    {
        Ok(got)
    } else {
        Err(got)
    }
}

fn ammo_box_left(w: &mut World) -> Result<String, String> {
    let mut q = w.query::<&rc_combat::ammo_box::AmmoBox>();
    let taken = q.iter(w).filter(|b| b.taken).count();
    let total = rifle_total(w).ok_or("no rifle")?;
    let got = format!("rifle {total}, {taken} box taken in all");
    if total == 300 && taken == 1 {
        Ok(got)
    } else {
        Err(got)
    }
}

fn grenade_crate(w: &mut World) -> Result<String, String> {
    let mut q = w.query_filtered::<&rc_combat::Grenades, With<Player>>();
    let n = q
        .iter(w)
        .next()
        .and_then(|g| {
            g.carried
                .iter()
                .find(|(i, _)| i == "thermal_detonator")
                .map(|c| c.1)
        })
        .ok_or("no thermal detonators carried")?;
    let mut b = w.query::<&rc_combat::ammo_box::AmmoBox>();
    let c = b
        .iter(w)
        .find(|b| b.class == "ThermalDetonatorBox")
        .cloned()
        .ok_or("no crate")?;
    let got = format!(
        "{n} thermal detonators (was 0); {} back: {}",
        c.name, !c.taken
    );
    if n == 5 && !c.taken && c.respawn.is_some() {
        Ok(got)
    } else {
        Err(got)
    }
}

fn ammo_box_after_load(w: &mut World) -> Result<String, String> {
    let name = w
        .resource::<Noted>()
        .ammo_box
        .clone()
        .ok_or("no box moved")?;
    let (taken, hidden) = box_state(w, &name).ok_or("box gone")?;
    let got = format!("{name}: taken {taken}, model hidden {hidden}");
    if taken && hidden {
        Ok(got)
    } else {
        Err(got)
    }
}

/// Whether the player's camera has the low light mode's night-vision
/// picture and its own distance fog (not the level zone's).
fn night_and_fog(w: &mut World) -> (bool, bool) {
    let mut q = w.query_filtered::<
        (Has<rc_visor::NightVisionView>, Has<rc_game_core::OwnFog>),
        With<PlayerCamera>,
    >();
    q.iter(w).next().unwrap_or((false, false))
}
