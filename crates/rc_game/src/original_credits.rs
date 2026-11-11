//! The EXTRAS page and the credits it opens, from the original menus
//! (`rc_menus::extras` and `rc_menus::credits` have the original's rules).
//! This part feeds the rolling credits each frame and closes the page when
//! the last name has gone by.

use crate::menu::Menu;
use crate::original_menu::{OriginalMenus, Shown};
use bevy::prelude::*;
use rc_game_core::{GameLog, GamePaths, GameState, Play};
use rc_menus::credits::{self, Roll};
use rc_menus::value::V;

/// The pages' names in `original_menus.toml`.
pub const EXTRAS: &str = "extras";
pub const CREDITS: &str = "credits";

/// The original's `HasReachedLevel`, for the EXTRAS page's locked rows.
/// This port doesn't keep campaign progress yet (the campaign page lists
/// every mission for the same reason), so every map counts as reached.
pub fn reached(_map: &str) -> bool {
    true
}

/// Where the credits' lines come from.
const CREDITS_INT: &str = "GameData/System/Credits.int";

/// Roll the credits while their page is on screen (the original's `Tick`).
/// The roll starts again each time the page opens, as the original makes
/// a new page each time.
#[allow(clippy::too_many_arguments)]
pub fn roll_credits(
    time: Res<Time>,
    state: Res<State<GameState>>,
    play: Option<Res<State<Play>>>,
    mut menu: ResMut<Menu>,
    mut menus: ResMut<OriginalMenus>,
    mut roll: Local<Option<Roll>>,
    paths: Res<GamePaths>,
    log: Res<GameLog>,
) {
    let shown = menus.shown(state.get(), play.as_deref().map(|p| p.get()), &menu);
    if shown != Some(Shown::Page(CREDITS.into())) {
        *roll = None;
        return;
    }
    let r = roll.get_or_insert_with(|| start(&mut menus, &paths, &log));
    let ended = r.step(time.delta_secs());
    if let Some(p) = menus.pages.get_mut(CREDITS) {
        r.fill(&mut p.screen.layout);
    }
    menus.mark_dirty();
    if ended {
        // Opened from EXTRAS (the only way in so far), so back to it, as
        // the original's OnBButton does with FROMEXTRAS.
        log.line("credits: the last line has gone by; back to EXTRAS");
        menu.back();
    }
}

/// Read the lines and the class's speed and line gap.
fn start(menus: &mut OriginalMenus, paths: &GamePaths, log: &GameLog) -> Roll {
    let (mut speed, mut gap) = (None, None);
    if let Some(a) = menus.assets.as_mut() {
        match a.library.class(credits::CREDITS_MENU) {
            Ok(c) => {
                if let Some(V::Float(f)) = c.get("ScrollSpeed") {
                    speed = Some(*f);
                }
                if let Some(V::Float(f)) = c.get("YLineOffset") {
                    gap = Some(*f);
                }
            }
            Err(e) => log.line(format!(
                "credits: can't read {}: {e}",
                credits::CREDITS_MENU
            )),
        }
    }
    // Without the class's values nothing would move; 0 keeps the page
    // still and BACK still works.
    let speed = speed.unwrap_or_else(|| {
        log.line("credits: the class has no ScrollSpeed; the names won't move");
        0.0
    });
    let gap = gap.unwrap_or(0.0);
    let path = paths.game_file(CREDITS_INT);
    let section = match std::fs::read(&path) {
        Ok(b) => credits::read_section(&crate::levels::decode_int(&b)),
        Err(e) => {
            log.line(format!("credits: can't read {} ({e})", path.display()));
            vec![]
        }
    };
    let r = Roll::new(&section, speed, gap);
    log.line(format!(
        "credits: {} lines from {}, moving {speed} screens a second, {gap} apart; about {:.0} s",
        r.lines.len(),
        path.display(),
        r.lines
            .last()
            .map_or(0.0, |l| (l.y + 0.05) / speed.max(f32::EPSILON))
    ));
    r
}
