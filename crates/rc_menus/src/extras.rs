//! The EXTRAS page (`XInterfaceCTMenus.CTExtrasMenu`), opened from the
//! main menu's EXTRAS button.
//!
//! Everything on the page (the planet pictures, the frame, the five video
//! rows, VIEW CREDITS, the "COMPLETE ... CAMPAIGN FOR" lines) is the
//! class's own default values, read from the install. What the class's
//! script does when the page opens (its `Init`, read from the install's
//! `XInterfaceCTMenus.u` with `strings -n1`; rewritten here by hand,
//! nothing copied):
//!
//! - focus goes to the first video row (`Extras[0]`);
//! - rows 0 and 1 (ASH VIDEO, CONCEPT ART MONTAGE) are always open;
//! - row 2 (FOLEY FEATURETTE) opens once the player has reached the map
//!   `RAS_01Briefing`, row 3 (SPEC OPS FEATURETTE) at `YYY_01Briefing` and
//!   row 4 (TEMUERA MORRISON INTERVIEW) at `Epilogue` (the original's
//!   `HasReachedLevel`);
//! - a row that isn't open yet: its two small bars beside it
//!   (`UnlockedLeftBars`, `UnlockedRightBars`) hide, the button can't be
//!   chosen, its plain text turns grey (128, 128, 128) and moves down by
//!   0.01, and its `UnlockText` line ("COMPLETE GEONOSIS CAMPAIGN FOR")
//!   shows above it;
//! - an open row hides its `UnlockText` line.
//!
//! Each video row plays a film from `GameData/Movies` (the script names
//! `ExtrasAsh.xmv` and so on; on a PC the files are `extrasash.mpg` and so
//! on, MPEG-1 video with MPEG audio). VIEW CREDITS opens the credits page
//! (`CTCreditsMenu`), which returns to EXTRAS when it ends.

use crate::layout::{Kind, MenuLayout};

/// The page's class.
pub const EXTRAS_MENU: &str = "XInterfaceCTMenus.CTExtrasMenu";

/// How many video rows the page has (`Extras[5]`).
pub const NUM_EXTRAS: usize = 5;

/// The map each video row waits for, by row (`None`: always open).
pub const UNLOCKED_AT: [Option<&str>; NUM_EXTRAS] = [
    None,
    None,
    Some("RAS_01Briefing"),
    Some("YYY_01Briefing"),
    Some("Epilogue"),
];

/// Grey of a row that isn't open yet.
pub const LOCKED_GREY: u8 = 128;

/// How far a locked row's text moves down, as a fraction of the screen.
pub const LOCKED_DROP: f32 = 0.01;

/// Is this class (or one it's built on) the EXTRAS page?
pub fn is_extras(family: &[String]) -> bool {
    family.iter().any(|c| c.eq_ignore_ascii_case(EXTRAS_MENU))
}

fn set_hidden(layout: &mut MenuLayout, name: &str, hidden: bool) {
    if let Some(w) = layout.widget_mut(name) {
        w.hidden = hidden;
    }
}

/// Set the page up the way the original's `Init` does. `reached` answers
/// the original's `HasReachedLevel` for a map name. Returns the rows left
/// locked.
pub fn init(layout: &mut MenuLayout, reached: impl Fn(&str) -> bool) -> Vec<usize> {
    let mut locked = Vec::new();
    for (i, map) in UNLOCKED_AT.iter().enumerate() {
        // The original's Init only looks at rows 2 to 4; rows 0 and 1
        // keep their (empty) UnlockText as stored.
        let Some(map) = map else { continue };
        if reached(map) {
            set_hidden(layout, &format!("UnlockText[{i}]"), true);
            continue;
        }
        locked.push(i);
        set_hidden(layout, &format!("UnlockedLeftBars[{i}]"), true);
        set_hidden(layout, &format!("UnlockedRightBars[{i}]"), true);
        set_hidden(layout, &format!("UnlockText[{i}]"), false);
        if let Some(w) = layout.widget_mut(&format!("Extras[{i}]")) {
            if let Kind::Button(b) = &mut w.kind {
                b.disabled = true;
                b.blurred.color[0] = LOCKED_GREY;
                b.blurred.color[1] = LOCKED_GREY;
                b.blurred.color[2] = LOCKED_GREY;
                b.blurred.pos[1] += LOCKED_DROP;
            }
        }
    }
    locked
}

/// The name of video row `i`'s button.
pub fn row_name(i: usize) -> String {
    format!("Extras[{i}]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Button, Text, Widget};

    fn widget(name: &str, kind: Kind) -> Widget {
        Widget {
            name: name.into(),
            pass: 0,
            hidden: false,
            platform: 0,
            kind,
        }
    }

    fn page() -> MenuLayout {
        let mut l = MenuLayout {
            class: "Test".into(),
            widgets: vec![],
            skipped: vec![],
            sounds: Default::default(),
            background_movie: String::new(),
            movie_widget: None,
            music: None,
            lists: Default::default(),
        };
        for i in 0..NUM_EXTRAS {
            let mut b = Button::default();
            b.blurred.color = [120, 200, 255, 255];
            b.blurred.pos = [0.5, 0.3];
            l.widgets
                .push(widget(&format!("Extras[{i}]"), Kind::Button(b)));
            l.widgets.push(widget(
                &format!("UnlockText[{i}]"),
                Kind::Text(Text::default()),
            ));
            for side in ["Left", "Right"] {
                l.widgets.push(widget(
                    &format!("Unlocked{side}Bars[{i}]"),
                    Kind::Text(Text::default()),
                ));
            }
        }
        l
    }

    fn button(l: &MenuLayout, i: usize) -> &Button {
        match &l.widget(&row_name(i)).unwrap().kind {
            Kind::Button(b) => b,
            _ => unreachable!(),
        }
    }

    fn hidden(l: &MenuLayout, name: &str) -> bool {
        l.widget(name).unwrap().hidden
    }

    #[test]
    fn everything_reached_opens_every_row() {
        let mut l = page();
        assert!(init(&mut l, |_| true).is_empty());
        for i in 0..NUM_EXTRAS {
            assert!(!button(&l, i).disabled);
            assert!(!hidden(&l, &format!("UnlockedLeftBars[{i}]")));
        }
        for i in 2..NUM_EXTRAS {
            assert!(hidden(&l, &format!("UnlockText[{i}]")));
        }
    }

    #[test]
    fn nothing_reached_locks_rows_two_to_four() {
        let mut l = page();
        assert_eq!(init(&mut l, |_| false), vec![2, 3, 4]);
        for i in 0..2 {
            assert!(!button(&l, i).disabled);
            assert_eq!(button(&l, i).blurred.color, [120, 200, 255, 255]);
        }
        for i in 2..NUM_EXTRAS {
            let b = button(&l, i);
            assert!(b.disabled);
            assert_eq!(b.blurred.color, [128, 128, 128, 255]);
            assert!((b.blurred.pos[1] - 0.31).abs() < 1e-6);
            assert!(hidden(&l, &format!("UnlockedLeftBars[{i}]")));
            assert!(hidden(&l, &format!("UnlockedRightBars[{i}]")));
            assert!(!hidden(&l, &format!("UnlockText[{i}]")));
        }
    }

    #[test]
    fn rows_open_at_their_maps() {
        let mut l = page();
        let locked = init(&mut l, |m| m.eq_ignore_ascii_case("ras_01briefing"));
        assert_eq!(locked, vec![3, 4]);
    }
}
