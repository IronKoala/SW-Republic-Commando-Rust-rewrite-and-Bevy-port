//! The end-of-campaign debriefing screen (`CTDebriefingInfo` and its
//! GEO, RAS and YYY versions in `XInterfaceCTMenus.u`). The screen's
//! pictures, frames, fonts and fixed texts ("YOU HAVE UNLOCKED", the
//! CONTINUE button) are the class's own default values; the level script
//! that shows it (`ACTION_ShowDebriefingScreen`) gives the title, the text
//! and the picture. This fills those in the way the original's
//! `SetInfoOptions` and `Init` do (what they do was read from the class's
//! script, not copied):
//!
//! - the title goes in `InfoTitle`, the text in `InfoText`;
//! - the picture is loaded by name into `InfoPic`, and the whole picture
//!   is used;
//! - on a PC the CONTINUE button (`AButton`) shows; the gamepad's "A"
//!   icon and its label (`AButtonIcon`, `ALabel`) stay hidden;
//! - "YOU HAVE UNLOCKED / EXTRAS CONTENT" shows in the English version
//!   (other languages hide it and move CONTINUE up; this port only reads
//!   the English texts so far).

use crate::layout::{Kind, MenuLayout};
use crate::Library;

/// The plain debriefing class (the campaign versions add only music).
pub const DEBRIEFING_MENU: &str = "XInterfaceCTMenus.CTDebriefingInfo";

/// What the CONTINUE button calls in the original.
pub const CONTINUE_FUNCTION: &str = "OnAButton";

/// What happened while filling the screen in, for the log.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Filled {
    /// The picture's full path, if it was found (it may name its package
    /// and picture without the group in between).
    pub picture: Option<String>,
    /// Anything that didn't work.
    pub problems: Vec<String>,
}

/// Put the title, text and picture on a debriefing screen built from the
/// class (`crate::build`).
pub fn setup(
    layout: &mut MenuLayout,
    library: &mut Library,
    title: &str,
    text: &str,
    picture: &str,
) -> Filled {
    let mut out = Filled::default();
    for (name, value) in [("InfoTitle", title), ("InfoText", text)] {
        match layout.widget_mut(name).map(|w| &mut w.kind) {
            Some(Kind::Text(t)) => t.text = value.to_string(),
            _ => out.problems.push(format!("no {name} text on the screen")),
        }
    }
    let found = if picture.trim().is_empty() {
        None
    } else {
        library.resolve_loose(picture.trim())
    };
    match layout.widget_mut("InfoPic").map(|w| &mut w.kind) {
        Some(Kind::Sprite(s)) => match &found {
            Some(p) => {
                s.texture = Some(p.clone());
                // The whole picture, as the original sets it.
                s.coords = [0; 4];
            }
            None => out.problems.push(format!(
                "picture {picture:?} not found; the class's own picture shows"
            )),
        },
        _ => out.problems.push("no InfoPic picture on the screen".into()),
    }
    out.picture = found;
    if !show_continue(layout) {
        out.problems
            .push("no AButton (CONTINUE) on the screen".into());
    }
    out
}

/// On a PC the CONTINUE button (`AButton`) shows, not the gamepad's icon
/// and label, as the original's `Init` sets it. False if there is no such
/// button.
pub fn show_continue(layout: &mut MenuLayout) -> bool {
    for name in ["AButtonIcon", "ALabel"] {
        if let Some(w) = layout.widget_mut(name) {
            w.hidden = true;
        }
    }
    match layout.widget_mut("AButton") {
        Some(w) => {
            w.hidden = false;
            true
        }
        None => false,
    }
}

/// Whether a class is one of the debriefing screens (by its family, as
/// `crate::library::ClassData::family` lists it).
pub fn is_debriefing(family: &[String]) -> bool {
    family
        .iter()
        .any(|c| c.eq_ignore_ascii_case(DEBRIEFING_MENU))
}
