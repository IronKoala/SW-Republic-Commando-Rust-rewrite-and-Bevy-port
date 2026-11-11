//! Which pause menu rows show, and where, as the original decides.
//!
//! The original pause menu (`XInterfaceCTMenus.CTPausePCMenu`) shows only
//! some of its eleven rows each time it opens, then moves the rows that
//! show so they sit centred on the screen. Its script (read from the
//! install's `XInterfaceCTMenus.u` with `strings -n1`; rewritten here by
//! hand, nothing copied) does this:
//!
//! - LOAD LAST SAVE (row 0) shows only while the player is down
//!   (incapacitated) or dead.
//! - QUICK LOAD (row 2) shows only when there is a quicksave.
//! - QUICK SAVE (row 1) and SAVE GAME (row 5) hide while the player has no
//!   health left, during a briefing, and in the prologue (map `PRO`).
//! - ENABLE HINTS (row 8) shows only when the player can switch hints back
//!   on: hint screens are off and the level has hints.
//! - Dead for good (no health and not down waiting for a squad mate): the
//!   menu's background picture and RESUME GAME hide, LOAD LAST SAVE gets
//!   the focus, and the rows' plain text turns a darker blue
//!   (45, 100, 255), because the screen behind is white.
//! - Otherwise RESUME GAME gets the focus.
//! - Then the rows that show are stacked from the top of the first row,
//!   one row step apart (the step between rows 0 and 1), the stack centred
//!   on the screen: the first row's top is
//!   `0.5 - 0.5 * ((shown - 1) * step + row background height)`.
//!
//! When the load or save page closes, the original hides LOAD LAST SAVE if
//! no saves are left and QUICK LOAD if the quicksave is gone, without
//! moving the other rows ([`after_save_pages`]).
//!
//! Not done: the original narrows the row backgrounds (to 0.65 of their
//! width) in French, Spanish and Italian. This port reads the English
//! (`.int`) texts only. The marketing-demo rules aren't needed either.

use crate::layout::{Kind, MenuLayout};

/// How many rows the original pause menu has.
pub const NUM_OPTIONS: usize = 11;
/// The rows the rules name, by their place in `MenuOptions`.
pub const LOAD_LAST_SAVE: usize = 0;
pub const QUICK_SAVE: usize = 1;
pub const QUICK_LOAD: usize = 2;
pub const RESUME_GAME: usize = 3;
pub const SAVE_GAME: usize = 5;
pub const ENABLE_HINTS: usize = 8;

/// The darker plain-text colour over the white screen of death.
pub const DEAD_TEXT: [u8; 3] = [45, 100, 255];

/// What the pause menu needs to know about the game when it opens.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PauseState {
    /// The player has no health left (down or dead).
    pub no_health: bool,
    /// The player is down, waiting for a squad mate (the original's
    /// `bIncapacitated` or `bIncapacitatedOnTurret`).
    pub incapacitated: bool,
    /// A briefing holds the player (the original's `bBriefing`).
    pub briefing: bool,
    /// The level is the prologue (map `PRO`).
    pub prologue: bool,
    pub has_quick_save: bool,
    /// Hint screens can be switched back on (the original's
    /// `CanEnableHints`).
    pub can_enable_hints: bool,
}

/// The widget index of row `i` (`MenuOptions[i]`).
pub fn row(layout: &MenuLayout, i: usize) -> Option<usize> {
    let name = format!("MenuOptions[{i}]");
    layout
        .widgets
        .iter()
        .position(|w| w.name.eq_ignore_ascii_case(&name))
}

fn set_hidden(layout: &mut MenuLayout, i: usize, hidden: bool) {
    if let Some(w) = row(layout, i) {
        layout.widgets[w].hidden = hidden;
    }
}

fn is_hidden(layout: &MenuLayout, i: usize) -> bool {
    row(layout, i).is_none_or(|w| layout.widgets[w].hidden)
}

/// Is this the pause menu (it has the eleven rows the rules use)?
pub fn is_pause_menu(layout: &MenuLayout) -> bool {
    (0..NUM_OPTIONS).all(|i| row(layout, i).is_some())
}

/// What [`init`] did, for the log.
#[derive(Debug, Clone, PartialEq)]
pub struct PauseRows {
    /// The rows that show, top to bottom.
    pub shown: Vec<usize>,
    /// The row with the focus.
    pub focus: usize,
    /// The top of the first row that shows, as a fraction of the screen.
    pub top: f32,
}

/// Set up the pause menu as the original does when it opens. `layout`
/// must be the menu as read from the game (each opening starts afresh, as
/// the original makes a new menu each time). Returns `None` when this
/// isn't the pause menu.
pub fn init(layout: &mut MenuLayout, s: &PauseState) -> Option<PauseRows> {
    if !is_pause_menu(layout) {
        return None;
    }
    set_hidden(layout, LOAD_LAST_SAVE, true);
    if !s.has_quick_save {
        set_hidden(layout, QUICK_LOAD, true);
    }
    if s.no_health || s.briefing || s.prologue {
        set_hidden(layout, QUICK_SAVE, true);
        set_hidden(layout, SAVE_GAME, true);
    }
    let mut focus = RESUME_GAME;
    if s.no_health || s.incapacitated {
        set_hidden(layout, LOAD_LAST_SAVE, false);
        if s.no_health && !s.incapacitated {
            // Dead for good.
            if let Some(b) = layout
                .widgets
                .iter()
                .position(|w| w.name.eq_ignore_ascii_case("Background"))
            {
                layout.widgets[b].hidden = true;
            }
            set_hidden(layout, RESUME_GAME, true);
            focus = LOAD_LAST_SAVE;
            for i in 0..NUM_OPTIONS {
                if let Some(w) = row(layout, i) {
                    if let Kind::Button(b) = &mut layout.widgets[w].kind {
                        b.blurred.color[..3].copy_from_slice(&DEAD_TEXT);
                    }
                }
            }
        }
    }
    set_hidden(layout, ENABLE_HINTS, !s.can_enable_hints);
    let top = adjust_buttons(layout);
    let shown = (0..NUM_OPTIONS)
        .filter(|&i| !is_hidden(layout, i))
        .collect();
    Some(PauseRows { shown, focus, top })
}

/// Stack the rows that show, centred on the screen (the original's
/// `AdjustButtons`). Moves each row's text (plain and focused) and
/// background (plain and focused). Returns the first row's top.
pub fn adjust_buttons(layout: &mut MenuLayout) -> f32 {
    let button = |layout: &MenuLayout, i: usize| {
        row(layout, i).and_then(|w| match &layout.widgets[w].kind {
            Kind::Button(b) => Some(b.clone()),
            _ => None,
        })
    };
    let (Some(first), Some(second)) = (button(layout, 0), button(layout, 1)) else {
        return 0.0;
    };
    let shown = (0..NUM_OPTIONS).filter(|&i| !is_hidden(layout, i)).count() as f32;
    let step = second.blurred.pos[1] - first.blurred.pos[1];
    let top = 0.5 - 0.5 * ((shown - 1.0) * step + first.background_blurred.scale[1]);
    let mut at = top;
    for i in 0..NUM_OPTIONS {
        if is_hidden(layout, i) {
            continue;
        }
        let Some(w) = row(layout, i) else { continue };
        if let Kind::Button(b) = &mut layout.widgets[w].kind {
            b.blurred.pos[1] = at;
            b.focused.pos[1] = at;
            b.background_blurred.pos[1] = at;
            b.background_focused.pos[1] = at;
        }
        at += step;
    }
    top
}

/// When the load or save page closes back to the pause menu: hide LOAD
/// LAST SAVE if no saves are left, and QUICK LOAD if the quicksave is gone
/// (the original's `MenuClosed`). The other rows stay where they are.
/// Returns true when a row was hidden (the original then focuses nothing).
pub fn after_save_pages(layout: &mut MenuLayout, any_saves: bool, has_quick_save: bool) -> bool {
    let mut hid = false;
    if !is_hidden(layout, LOAD_LAST_SAVE) && !any_saves {
        set_hidden(layout, LOAD_LAST_SAVE, true);
        hid = true;
    }
    if !is_hidden(layout, QUICK_LOAD) && !has_quick_save {
        set_hidden(layout, QUICK_LOAD, true);
        hid = true;
    }
    hid
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Button, Sprite, Text, Widget};

    /// A stand-in pause menu: eleven rows 0.08 apart from 0.1, 0.06 high,
    /// and a background picture.
    fn menu() -> MenuLayout {
        let mut widgets = vec![Widget {
            name: "Background".into(),
            pass: 0,
            hidden: false,
            platform: 0,
            kind: Kind::Sprite(Sprite::default()),
        }];
        for i in 0..NUM_OPTIONS {
            let y = 0.1 + 0.08 * i as f32;
            let text = Text {
                pos: [0.5, y],
                color: [255, 255, 255, 255],
                ..Default::default()
            };
            let back = Sprite {
                pos: [0.5, y],
                scale: [0.5, 0.06],
                ..Default::default()
            };
            widgets.push(Widget {
                name: format!("MenuOptions[{i}]"),
                pass: 1,
                hidden: false,
                platform: 0,
                kind: Kind::Button(Button {
                    blurred: text.clone(),
                    focused: text,
                    background_blurred: back.clone(),
                    background_focused: back,
                    ..Default::default()
                }),
            });
        }
        MenuLayout { widgets, ..blank() }
    }

    fn blank() -> MenuLayout {
        MenuLayout {
            class: "Test".into(),
            widgets: vec![],
            skipped: vec![],
            sounds: Default::default(),
            background_movie: String::new(),
            music: None,
            lists: Default::default(),
        }
    }

    fn tops(l: &MenuLayout, rows: &[usize]) -> Vec<f32> {
        rows.iter()
            .map(|&i| match &l.widgets[row(l, i).unwrap()].kind {
                Kind::Button(b) => {
                    assert_eq!(b.blurred.pos[1], b.background_focused.pos[1]);
                    b.blurred.pos[1]
                }
                _ => unreachable!(),
            })
            .collect()
    }

    #[test]
    fn playing_with_a_quicksave_shows_nine_rows_centred() {
        let mut l = menu();
        let s = PauseState {
            has_quick_save: true,
            ..Default::default()
        };
        let r = init(&mut l, &s).unwrap();
        // No LOAD LAST SAVE, no ENABLE HINTS.
        assert_eq!(r.shown, vec![1, 2, 3, 4, 5, 6, 7, 9, 10]);
        assert_eq!(r.focus, RESUME_GAME);
        // 0.5 - 0.5 * (8 * 0.08 + 0.06) = 0.15
        assert!((r.top - 0.15).abs() < 1e-5, "{}", r.top);
        let t = tops(&l, &r.shown);
        for (n, y) in t.iter().enumerate() {
            assert!((y - (0.15 + 0.08 * n as f32)).abs() < 1e-5);
        }
    }

    #[test]
    fn no_quicksave_hides_quick_load() {
        let mut l = menu();
        let r = init(&mut l, &PauseState::default()).unwrap();
        assert_eq!(r.shown, vec![1, 3, 4, 5, 6, 7, 9, 10]);
    }

    #[test]
    fn hints_row_follows_can_enable_hints() {
        let mut l = menu();
        let s = PauseState {
            can_enable_hints: true,
            ..Default::default()
        };
        assert!(init(&mut l, &s).unwrap().shown.contains(&ENABLE_HINTS));
    }

    #[test]
    fn briefing_and_prologue_hide_both_saves() {
        for s in [
            PauseState {
                briefing: true,
                ..Default::default()
            },
            PauseState {
                prologue: true,
                ..Default::default()
            },
        ] {
            let mut l = menu();
            let r = init(&mut l, &s).unwrap();
            assert!(!r.shown.contains(&QUICK_SAVE));
            assert!(!r.shown.contains(&SAVE_GAME));
        }
    }

    #[test]
    fn down_shows_load_last_and_keeps_resume() {
        let mut l = menu();
        let s = PauseState {
            no_health: true,
            incapacitated: true,
            ..Default::default()
        };
        let r = init(&mut l, &s).unwrap();
        assert_eq!(r.shown, vec![0, 3, 4, 6, 7, 9, 10]);
        assert_eq!(r.focus, RESUME_GAME);
        assert!(!l.widgets[0].hidden, "background stays");
    }

    #[test]
    fn dead_hides_resume_and_background_and_darkens_text() {
        let mut l = menu();
        let s = PauseState {
            no_health: true,
            ..Default::default()
        };
        let r = init(&mut l, &s).unwrap();
        assert_eq!(r.shown, vec![0, 4, 6, 7, 9, 10]);
        assert_eq!(r.focus, LOAD_LAST_SAVE);
        assert!(l.widgets[0].hidden);
        match &l.widgets[row(&l, 4).unwrap()].kind {
            Kind::Button(b) => assert_eq!(b.blurred.color, [45, 100, 255, 255]),
            _ => unreachable!(),
        }
    }

    #[test]
    fn closing_the_save_pages_hides_without_moving() {
        let mut l = menu();
        let s = PauseState {
            no_health: true,
            incapacitated: true,
            has_quick_save: true,
            ..Default::default()
        };
        let r = init(&mut l, &s).unwrap();
        let before = tops(&l, &[3]);
        assert!(after_save_pages(&mut l, false, false));
        assert!(is_hidden(&l, LOAD_LAST_SAVE) && is_hidden(&l, QUICK_LOAD));
        assert_eq!(tops(&l, &[3]), before);
        assert!(r.shown.contains(&QUICK_LOAD));
    }

    #[test]
    fn other_menus_are_left_alone() {
        let mut l = blank();
        assert!(init(&mut l, &PauseState::default()).is_none());
    }
}
