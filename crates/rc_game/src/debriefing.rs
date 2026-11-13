//! The end-of-campaign debriefing screen. When a campaign's last level
//! ends, its script runs `ACTION_ShowDebriefingScreen` with a title, a
//! text, a picture and the next map (`NewLevel`). The level scripts part
//! sends that as a [`ShowDebriefing`]; here play pauses and the
//! debriefing page shows, drawn from the original's own menu class
//! (`[pages.debriefing]` in `data/ui/original_menus.toml`, filled in by
//! `rc_menus::debriefing`). CONTINUE (or the accept key or button) sends
//! the held [`ChangeLevel`], and the save part goes on to the next map as
//! for any other level exit. Back (Escape, B) does nothing here, as in the
//! original.
//!
//! If the original page can't be drawn, a placeholder page (marked as
//! one) shows the same title and text with a Continue button.
//!
//! Not done: the original also plays the campaign's music on this screen
//! (`CTDebriefingInfoGEO`/`RAS`/`YYY` differ only in that); the level's own
//! music carries on instead.

use crate::menu::{Menu, Page};
use crate::original_menu::OriginalMenus;
use bevy::prelude::*;
use rc_game_core::{ChangeLevel, GameLog, GameSet, GameState, Play, ShowDebriefing};

/// The debriefing showing now, if any.
#[derive(Resource, Default)]
pub struct Debriefing {
    pub showing: Option<ShowDebriefing>,
}

/// The debriefing's systems (a hint asked for in the same frame runs after
/// them and takes the debriefing's place, see `hints.rs`).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct DebriefingSet;

pub struct DebriefingPlugin;

impl Plugin for DebriefingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Debriefing>()
            .add_message::<ShowDebriefing>()
            .add_systems(OnEnter(GameState::Loading), forget)
            .add_systems(OnEnter(GameState::MainMenu), forget)
            .add_systems(
                Update,
                (show, go_on)
                    .chain()
                    .in_set(GameSet::Hud)
                    .in_set(DebriefingSet),
            );
    }
}

/// A new level or the main menu: nothing is showing any more.
fn forget(mut d: ResMut<Debriefing>, mut menu: ResMut<Menu>) {
    d.showing = None;
    menu.go_on = false;
}

/// A level script asked for the debriefing: pause and show it.
#[allow(clippy::too_many_arguments)]
fn show(
    mut asked: MessageReader<ShowDebriefing>,
    state: Res<State<GameState>>,
    mut d: ResMut<Debriefing>,
    mut menu: ResMut<Menu>,
    mut menus: ResMut<OriginalMenus>,
    mut next_play: Option<ResMut<NextState<Play>>>,
    mut change: MessageWriter<ChangeLevel>,
    log: Res<GameLog>,
) {
    for a in asked.read() {
        if *state.get() != GameState::InGame {
            log.line(format!(
                "debriefing: {:?} asked for while not in a level; going on to {}",
                a.title, a.then.map
            ));
            change.write(a.then.clone());
            continue;
        }
        if d.showing.is_some() {
            log.line("debriefing: one is already showing; this one is ignored");
            continue;
        }
        log.line(format!(
            "debriefing: showing {:?} ({}), picture {}; CONTINUE goes on to {}",
            a.title, a.menu_class, a.picture, a.then.map
        ));
        let original = menus.show_debriefing(&a.title, &a.text, &a.picture);
        for l in &original.lines {
            log.line(format!("debriefing: {l}"));
        }
        if !original.shown {
            log.line("debriefing: the original page can't be drawn; using the placeholder page");
            menu.notice = Some(format!("{}: {}", a.title, a.text));
        }
        d.showing = Some(a.clone());
        menu.go_on = false;
        // Paused: the level stops, and the menu opens on this page (see
        // `menu.rs`, which opens the pause menu otherwise).
        menu.reset(Page::Debriefing);
        if let Some(n) = next_play.as_mut() {
            n.set(Play::Paused);
        }
    }
}

/// CONTINUE was chosen: on to the next map.
fn go_on(
    mut d: ResMut<Debriefing>,
    mut menu: ResMut<Menu>,
    mut change: MessageWriter<ChangeLevel>,
    log: Res<GameLog>,
) {
    if !menu.go_on {
        return;
    }
    menu.go_on = false;
    let Some(shown) = d.showing.take() else {
        return;
    };
    log.line(format!(
        "debriefing: CONTINUE chosen; on to {}",
        shown.then.map
    ));
    menu.notice = None;
    change.write(shown.then);
}
