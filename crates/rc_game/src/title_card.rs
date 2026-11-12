//! The title card before a campaign ("WAR HAS COME TO THE GALAXY..."
//! before Geonosis). A level script runs `ACTION_ShowTitleCardScreen`; the
//! level scripts part sends that as a [`ShowTitleCard`]. Here, as the
//! original's `ShowTitleCardMenu` does, play pauses and the card shows,
//! drawn from the original's own menu class (`[pages.title_card]` in
//! `data/ui/original_menus.toml`). Its lines and their times come from the
//! card's own file in the install (`GameData/System/geo_titlecard.int`);
//! `rc_menus::titlecard` has the class's rules. When the last line has
//! faded out the card closes and play goes on; the level script's next
//! action, which waited while play was paused, carries on from there.
//! Back (Escape, B) does nothing, as in the original.
//!
//! The card's music (`BackgroundMusic` in the card file) is played by
//! `music.rs` while the card shows.
//!
//! If the original page can't be drawn, a placeholder page (marked as
//! one) shows the lines on screen at each moment as its notice.

use crate::menu::{Menu, Page};
use crate::original_menu::OriginalMenus;
use bevy::prelude::*;
use rc_game_core::{GameLog, GamePaths, GameSet, GameState, Play, ShowTitleCard};
use rc_menus::titlecard::{self, Card};

/// The page's name in `original_menus.toml`.
pub const TITLE_CARD: &str = "title_card";

/// The title card showing now, if any.
#[derive(Resource, Default)]
pub struct TitleCards {
    pub showing: Option<Card>,
    /// The original page couldn't be drawn: the placeholder shows.
    placeholder: bool,
    /// What sent it, for the log.
    by: String,
}

pub struct TitleCardPlugin;

impl Plugin for TitleCardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TitleCards>()
            .add_systems(OnEnter(GameState::Loading), forget)
            .add_systems(OnEnter(GameState::MainMenu), forget)
            .add_systems(Update, (show, run).chain().in_set(GameSet::Hud));
    }
}

/// A new level or the main menu: nothing is showing any more.
fn forget(mut cards: ResMut<TitleCards>) {
    *cards = TitleCards::default();
}

/// A level script asked for the title card: read it, pause and show it.
#[allow(clippy::too_many_arguments)]
fn show(
    mut asked: MessageReader<ShowTitleCard>,
    state: Res<State<GameState>>,
    mut cards: ResMut<TitleCards>,
    mut menu: ResMut<Menu>,
    mut next_play: Option<ResMut<NextState<Play>>>,
    paths: Res<GamePaths>,
    log: Res<GameLog>,
) {
    for a in asked.read() {
        if *state.get() != GameState::InGame {
            log.line(format!(
                "title card: {:?} asked for by {} while not in a level; not shown",
                a.args, a.by
            ));
            continue;
        }
        if cards.showing.is_some() {
            log.line(format!(
                "title card: {:?} asked for by {} while one is showing; not shown",
                a.args, a.by
            ));
            continue;
        }
        if !a
            .menu_class
            .eq_ignore_ascii_case(titlecard::TITLE_CARD_MENU)
        {
            log.line(format!(
                "title card: {} names menu {}, which this port doesn't have; {} is used",
                a.by,
                a.menu_class,
                titlecard::TITLE_CARD_MENU
            ));
        }
        let file = titlecard::card_file(&a.args);
        let path = paths.game_file(&format!("GameData/System/{file}.int"));
        let section = match std::fs::read(&path) {
            Ok(b) => titlecard::read_section(&crate::levels::decode_int(&b)),
            Err(e) => {
                log.line(format!(
                    "title card: can't read {} ({e}); the card has no lines and closes at once",
                    path.display()
                ));
                vec![]
            }
        };
        let card = Card::new(&file, &section);
        log.line(format!(
            "title card: {file} from {} (asked for by {}): {} lines, music {}, closes after {:.1} s",
            path.display(),
            a.by,
            card.entries.len(),
            card.music.as_deref().unwrap_or("none"),
            card.length()
        ));
        for (i, e) in card.entries.iter().enumerate() {
            log.line(format!(
                "title card: line {i}: in {} to {} s, out {} to {} s: {:?}",
                e.fade_in_start, e.full_on_start, e.fade_out_start, e.fade_out_end, e.text
            ));
        }
        cards.placeholder = false;
        cards.showing = Some(card);
        cards.by = a.by.clone();
        // Paused: the level and its scripts stop, and the menu opens on
        // this page (see `menu.rs`, which opens the pause menu otherwise).
        menu.reset(Page::TitleCard);
        if let Some(n) = next_play.as_mut() {
            n.set(Play::Paused);
        }
    }
}

/// While the card shows: its clock runs (the original's `Tick`), its
/// lines fade, and when the last has gone it closes and play goes on.
#[allow(clippy::too_many_arguments)]
fn run(
    time: Res<Time>,
    play: Option<Res<State<Play>>>,
    mut cards: ResMut<TitleCards>,
    mut menu: ResMut<Menu>,
    mut menus: ResMut<OriginalMenus>,
    mut next_play: Option<ResMut<NextState<Play>>>,
    log: Res<GameLog>,
) {
    // Only once the pause has begun and the card is the page on screen
    // (the original's Tick runs only while the menu shows).
    if play.as_deref().map(|p| p.get()) != Some(&Play::Paused) || menu.page != Page::TitleCard {
        return;
    }
    let cards = &mut *cards;
    let Some(card) = cards.showing.as_mut() else {
        return;
    };
    let ended = card.step(time.delta_secs());
    // The original pages are built when the menu first opens, a frame
    // after the card asked for it: only once they have been tried is it
    // known whether this page can be drawn.
    if menus.tried && !cards.placeholder && !menus.pages.contains_key(TITLE_CARD) {
        cards.placeholder = true;
        log.line("title card: the original page can't be drawn; using the placeholder page");
    }
    if cards.placeholder {
        let shown: Vec<&str> = card
            .entries
            .iter()
            .zip(&card.looks)
            .filter(|(_, l)| !l.hidden)
            .map(|(e, _)| e.text.as_str())
            .collect();
        let notice = (!shown.is_empty()).then(|| shown.join("  "));
        if menu.notice != notice {
            menu.notice = notice;
        }
    } else if let Some(p) = menus.pages.get_mut(TITLE_CARD) {
        card.fill(&mut p.screen.layout);
        menus.mark_dirty();
    }
    if ended {
        log.line(format!(
            "title card: {} done after {:.1} s; play goes on",
            card.file, card.elapsed
        ));
        cards.showing = None;
        menu.notice = None;
        if let Some(n) = next_play.as_mut() {
            n.set(Play::Running);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_is_in_the_shipped_menus() {
        let plans =
            rc_menus::pages::parse_plans(include_str!("../../../data/ui/original_menus.toml"))
                .unwrap();
        let p = plans.iter().find(|p| p.page == TITLE_CARD).unwrap();
        assert_eq!(p.class, titlecard::TITLE_CARD_MENU);
        assert!(p.nothing_to_choose);
        assert_eq!(p.back, Some(rc_menus::pages::PageCmd::Unavailable));
    }
}
