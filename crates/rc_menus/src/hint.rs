//! The hint screens (`CTPauseHintText` in `XInterfaceCTMenus.u`): the
//! tutorial pop-ups that pause the game ("Crouch is useful for ducking
//! behind cover and crawling into vents."). The page's frames, fonts and
//! button words ("CONTINUE WITH HINTS", "CONTINUE WITHOUT HINTS") are the
//! class's own default values; the level script that shows it
//! (`ACTION_ShowDebriefingScreen` with `bIsForHintText`) gives only the
//! text. This fills the page in the way the class's own script does (read
//! from the script in the install, not copied):
//!
//! - `Init`: with the helmet off (`bVisor` false) the text frame, the text
//!   and the picture frame move up (`InfoTextBorder.PosY = 0.48`,
//!   `InfoText.PosY = 0.3`, `InfoPicBorder.PosY = 0.40333333`); on a PC
//!   the A button (`AButton`, CONTINUE WITH HINTS) shows (`HideAButton(0)`)
//!   and has the focus. Languages other than English also move the buttons
//!   and frames down and squeeze two labels; this port reads only the
//!   English texts so far, so that part isn't done.
//! - `SetInfoOptions`: the text goes in `InfoText`; the title (`InfoTitle`)
//!   and the picture (`InfoPic`) are hidden and the picture's frame
//!   (`InfoPicBorder`) shows.
//!
//! What the buttons do (in `rc_game`): A unpauses and closes the page
//! (`OnAButton`: `SetPause(false)`, then `MenuClose`); B switches hint
//! screens off (`bKeepHintMenusAwfulHack=false`) and then does what A
//! does. Back does nothing (`CTInfoBase.HandleInputBack` is empty).

use crate::layout::{Kind, MenuLayout};

/// The hint screen's class.
pub const HINT_MENU: &str = "XInterfaceCTMenus.CTPauseHintText";

/// CONTINUE WITH HINTS.
pub const A_FUNCTION: &str = "OnAButton";
/// CONTINUE WITHOUT HINTS.
pub const B_FUNCTION: &str = "OnBButton";

/// Whether a class is the hint screen (by its family, as
/// `crate::library::ClassData::family` lists it).
pub fn is_hint(family: &[String]) -> bool {
    family.iter().any(|c| c.eq_ignore_ascii_case(HINT_MENU))
}

/// The class's `Init` on a PC in English. `visor` is the helmet setting
/// (the original's `bVisor`). Returns anything that didn't work.
pub fn init(layout: &mut MenuLayout, visor: bool) -> Vec<String> {
    let mut problems = Vec::new();
    if !visor {
        // "Shift everything up a bit".
        for (name, y) in [
            ("InfoTextBorder", 0.48),
            ("InfoText", 0.3),
            ("InfoPicBorder", 0.403_333_33),
        ] {
            match layout.widget_mut(name).map(|w| &mut w.kind) {
                Some(Kind::Sprite(s)) => s.pos[1] = y,
                Some(Kind::Text(t)) => t.pos[1] = y,
                _ => problems.push(format!("no {name} on the screen")),
            }
        }
    }
    // HideAButton(0) on a PC: the A button shows; its gamepad icon and
    // label are left as the class has them.
    match layout.widget_mut("AButton") {
        Some(w) => w.hidden = false,
        None => problems.push("no AButton (CONTINUE WITH HINTS) on the screen".into()),
    }
    if layout.widget("BButton").is_none() {
        problems.push("no BButton (CONTINUE WITHOUT HINTS) on the screen".into());
    }
    problems
}

/// The class's `SetInfoOptions`: the text in, the title and picture
/// hidden, the picture's frame shown. Returns anything that didn't work.
pub fn set_text(layout: &mut MenuLayout, text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    match layout.widget_mut("InfoText").map(|w| &mut w.kind) {
        Some(Kind::Text(t)) => t.text = text.to_string(),
        _ => problems.push("no InfoText text on the screen".into()),
    }
    for (name, hidden) in [
        ("InfoTitle", true),
        ("InfoPic", true),
        ("InfoPicBorder", false),
    ] {
        match layout.widget_mut(name) {
            Some(w) => w.hidden = hidden,
            None => problems.push(format!("no {name} on the screen")),
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Sprite, Text, Widget};

    fn text(name: &str) -> Widget {
        Widget {
            name: name.into(),
            pass: 2,
            hidden: false,
            platform: 0,
            kind: Kind::Text(Text {
                pos: [0.5, 0.5],
                ..Default::default()
            }),
        }
    }

    fn sprite(name: &str, hidden: bool) -> Widget {
        Widget {
            name: name.into(),
            pass: 0,
            hidden,
            platform: 0,
            kind: Kind::Sprite(Sprite {
                pos: [0.5, 0.5],
                ..Default::default()
            }),
        }
    }

    fn page() -> MenuLayout {
        MenuLayout {
            class: "Test".into(),
            skipped: vec![],
            sounds: Default::default(),
            background_movie: String::new(),
            movie_widget: None,
            music: None,
            lists: Default::default(),
            white_texture: None,
            widgets: vec![
                sprite("InfoTextBorder", false),
                sprite("InfoPicBorder", true),
                sprite("InfoPic", false),
                text("InfoTitle"),
                text("InfoText"),
                sprite("AButton", true),
                sprite("BButton", false),
            ],
        }
    }

    fn y(l: &MenuLayout, name: &str) -> f32 {
        match &l.widget(name).unwrap().kind {
            Kind::Sprite(s) => s.pos[1],
            Kind::Text(t) => t.pos[1],
            _ => f32::NAN,
        }
    }

    #[test]
    fn helmet_on_keeps_places_and_shows_a() {
        let mut l = page();
        assert!(init(&mut l, true).is_empty());
        assert_eq!(y(&l, "InfoText"), 0.5);
        assert!(!l.widget("AButton").unwrap().hidden);
    }

    #[test]
    fn helmet_off_moves_up() {
        let mut l = page();
        assert!(init(&mut l, false).is_empty());
        assert_eq!(y(&l, "InfoTextBorder"), 0.48);
        assert_eq!(y(&l, "InfoText"), 0.3);
        assert!((y(&l, "InfoPicBorder") - 0.403_333_33).abs() < 1e-6);
    }

    #[test]
    fn text_in_title_and_picture_hidden() {
        let mut l = page();
        assert!(set_text(&mut l, "Crouch!").is_empty());
        match &l.widget("InfoText").unwrap().kind {
            Kind::Text(t) => assert_eq!(t.text, "Crouch!"),
            _ => panic!(),
        }
        assert!(l.widget("InfoTitle").unwrap().hidden);
        assert!(l.widget("InfoPic").unwrap().hidden);
        assert!(!l.widget("InfoPicBorder").unwrap().hidden);
    }
}
