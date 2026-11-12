//! The hint screens: the tutorial pop-ups in geo_01a, geo_01b, geo_01c,
//! geo_03a and geo_04d (29 in all). A level script runs
//! `ACTION_ShowDebriefingScreen` with `MenuClass=XInterfaceCTMenus.
//! CTPauseHintText`, a `Text` and `bIsForHintText`; the level scripts part
//! sends that as a [`ShowHint`]. What happens next is the original's own
//! script (read from `CTGame.u`, `Engine.u` and `XInterfaceCTMenus.u` in
//! the install, not copied):
//!
//! - `ACTION_ShowDebriefingScreen.InitActionFor`: shown only if it isn't
//!   for hint text or hints are on (`bKeepHintMenusAwfulHack`, our
//!   `hint_screens` setting; `true` in the install's `User.ini`). The script
//!   goes straight on to its next step.
//! - `PlayerController.ShowMenu`: play pauses and the page opens with the
//!   text (`SetInfoOptions`, see `rc_menus::hint`).
//! - CONTINUE WITH HINTS (`OnAButton`): play goes on and the page closes.
//! - CONTINUE WITHOUT HINTS (`OnBButton`): hints off, then as A. Like
//!   ENABLE HINTS on the pause menu, the setting is saved (the original
//!   keeps it in `User.ini`).
//! - Back does nothing (`CTInfoBase.HandleInputBack` is empty).
//!
//! If the original page can't be drawn, a placeholder page (marked as one)
//! shows the text with the same two choices.
//!
//! Not done: a second hint asked for while one shows is left out (none of
//! the 29 can do that: each waits on a timer first); what the original
//! would do then is decided in its compiled code and wasn't checked.
//! Gamepad B on the original calls CONTINUE WITHOUT HINTS
//! (`HandleInputGamePad`); here back does nothing on every device, because
//! whether the PC version sends the pad's B there is in its compiled code.

use crate::menu::{save_settings, Menu, Page};
use crate::original_menu::OriginalMenus;
use crate::settings::{Settings, SettingsPath};
use bevy::prelude::*;
use rc_game_core::{GameLog, GameSet, GameState, Play, ShowHint};

/// The hint showing now, if any.
#[derive(Resource, Default)]
pub struct Hints {
    pub showing: Option<ShowHint>,
    /// Hints shown since the program started (for the self-test).
    pub shown: usize,
    /// Hints left out because hint screens are off.
    pub skipped: usize,
}

pub struct HintsPlugin;

impl Plugin for HintsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Hints>()
            .add_message::<ShowHint>()
            .add_systems(OnEnter(GameState::Loading), forget)
            .add_systems(OnEnter(GameState::MainMenu), forget)
            .add_systems(Update, (show, answer).chain().in_set(GameSet::Hud));
    }
}

/// A new level or the main menu: nothing is showing any more.
fn forget(mut h: ResMut<Hints>, mut menu: ResMut<Menu>) {
    h.showing = None;
    menu.hint_answer = None;
}

/// A level script asked for a hint: pause and show it.
#[allow(clippy::too_many_arguments)]
fn show(
    mut asked: MessageReader<ShowHint>,
    state: Res<State<GameState>>,
    play: Option<Res<State<Play>>>,
    mut h: ResMut<Hints>,
    mut menu: ResMut<Menu>,
    mut menus: ResMut<OriginalMenus>,
    settings: Res<Settings>,
    mut next_play: Option<ResMut<NextState<Play>>>,
    log: Res<GameLog>,
) {
    for a in asked.read() {
        if *state.get() != GameState::InGame {
            log.line(format!(
                "hints: {:?} asked for while not in a level; left out",
                a.text
            ));
            continue;
        }
        if a.for_hint_text && !settings.game.hint_screens {
            h.skipped += 1;
            log.line(format!(
                "hints: {} {:?} not shown: hint screens are off",
                a.by, a.text
            ));
            continue;
        }
        if h.showing.is_some() {
            log.line(format!(
                "hints: {} {:?} left out: a hint is already showing",
                a.by, a.text
            ));
            continue;
        }
        if play.as_ref().is_some_and(|p| *p.get() != Play::Running) {
            log.line(format!(
                "hints: {} {:?} left out: play is already paused",
                a.by, a.text
            ));
            continue;
        }
        if !a.menu_class.eq_ignore_ascii_case(rc_menus::hint::HINT_MENU) {
            log.line(format!(
                "hints: {} names {} (only {} is known); shown on the hint page",
                a.by,
                a.menu_class,
                rc_menus::hint::HINT_MENU
            ));
        }
        log.line(format!("hints: {} showing {:?}", a.by, a.text));
        let (shown, lines) = menus.show_hint(&a.text, settings.game.helmet);
        for l in &lines {
            log.line(format!("hints: {l}"));
        }
        if !shown {
            log.line("hints: the original page can't be drawn; using the placeholder page");
            menu.notice = Some(a.text.clone());
        }
        h.showing = Some(a.clone());
        h.shown += 1;
        menu.hint_answer = None;
        // Paused: the level stops, and the menu opens on this page (see
        // `menu.rs`, which opens the pause menu otherwise).
        menu.reset(Page::Hint);
        if let Some(n) = next_play.as_mut() {
            n.set(Play::Paused);
        }
    }
}

/// One of the two buttons was chosen: hints off for CONTINUE WITHOUT
/// HINTS, then play goes on.
fn answer(
    mut h: ResMut<Hints>,
    mut menu: ResMut<Menu>,
    mut settings: ResMut<Settings>,
    settings_path: Res<SettingsPath>,
    mut next_play: Option<ResMut<NextState<Play>>>,
    log: Res<GameLog>,
) {
    let Some(with) = menu.hint_answer.take() else {
        return;
    };
    if h.showing.take().is_none() {
        return;
    }
    if !with && settings.game.hint_screens {
        settings.game.hint_screens = false;
        save_settings(&settings, &settings_path, &log);
    }
    log.line(format!(
        "hints: CONTINUE {} HINTS chosen; play goes on (hint screens {})",
        if with { "WITH" } else { "WITHOUT" },
        if settings.game.hint_screens {
            "on"
        } else {
            "off"
        }
    ));
    menu.notice = None;
    if let Some(n) = next_play.as_mut() {
        n.set(Play::Running);
    }
}
