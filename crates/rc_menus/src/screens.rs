//! What each original menu does on a PC, rewritten by hand in Rust from
//! reading the game's own menu scripts (never copied). Each menu's script
//! adjusts a few widgets when it opens (`Init`), and names a function for
//! each button; here those become plain data the game acts on.

use crate::layout::{Kind, MenuLayout};

/// What choosing a main menu button asks the game to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainAction {
    /// Start the campaign at its first map (the original opens `PRO`).
    NewGame,
    LoadGame,
    CampaignMap,
    Multiplayer,
    Options,
    Profiles,
    Extras,
    /// Ask "are you sure?", then quit.
    Quit,
    /// A function name we don't know.
    Other(String),
}

pub fn main_action(on_select: &str) -> MainAction {
    match on_select {
        "NewGameSelected" => MainAction::NewGame,
        "LoadGameSelected" => MainAction::LoadGame,
        "CampaignMapSelected" => MainAction::CampaignMap,
        "MultiplayerSelected" => MainAction::Multiplayer,
        "OptionsSelected" => MainAction::Options,
        "ProfilesSelected" => MainAction::Profiles,
        "ExtrasSelected" => MainAction::Extras,
        "QuitSelected" => MainAction::Quit,
        other => MainAction::Other(other.to_string()),
    }
}

/// The class of the original main menu.
pub const MAIN_MENU: &str = "XInterfaceCTMenus.CTMenuMain";

/// Grey the original uses for buttons that can't be chosen (its demo
/// versions set the button's colour to 128, 128, 128).
const DISABLED_GREY: u8 = 128;

/// Disable a button the way the original's demo builds do.
pub fn disable(layout: &mut MenuLayout, name: &str) -> bool {
    let Some(w) = layout.widget_mut(name) else {
        return false;
    };
    let Kind::Button(b) = &mut w.kind else {
        return false;
    };
    b.disabled = true;
    b.blurred.color[0] = DISABLED_GREY;
    b.blurred.color[1] = DISABLED_GREY;
    b.blurred.color[2] = DISABLED_GREY;
    true
}

/// Every menu with a title strip (`MenuTemplateTitled` and its children)
/// hides its Xbox Live name and controller number on a PC.
pub fn setup_titled(layout: &mut MenuLayout) {
    for name in ["HostText", "ControllerNumText", "ControllerIcon"] {
        if let Some(w) = layout.widget_mut(name) {
            w.hidden = true;
        }
    }
}

/// The class of the original yes/no question box.
pub const QUESTION: &str = "XInterface.MenuQuestionYesNo";

/// Set up the yes/no question box with its question (shown in capitals,
/// as the original does). On a PC it starts on NO.
///
/// The box replaces the menu that asked (`under`) on screen, but borrows
/// that menu's background picture.
pub fn setup_question(
    layout: &mut MenuLayout,
    question: &str,
    under: Option<&MenuLayout>,
) -> Option<usize> {
    setup_titled(layout);
    let background = under.and_then(|u| u.widget("Background")).cloned();
    if let (Some(bg), Some(w)) = (background, layout.widget_mut("Background")) {
        w.kind = bg.kind;
        w.hidden = bg.hidden;
    }
    if let Some(w) = layout.widget_mut("Question") {
        if let Kind::Text(t) = &mut w.kind {
            t.text = question.to_uppercase();
        }
    }
    layout
        .widgets
        .iter()
        .position(|w| w.name.eq_ignore_ascii_case("BButton"))
}

/// Set up the main menu as the PC version of the original does when it
/// opens:
/// - the border grows by one row to make room for QUIT (PC only);
/// - Xbox Live name, friends icon and controller number are hidden.
///
/// `unavailable` lists buttons (by their action) this port can't offer:
/// they are greyed out and can't be chosen.
pub fn setup_main_menu(layout: &mut MenuLayout, unavailable: &[MainAction]) -> Vec<String> {
    let mut log = Vec::new();
    if let Some(w) = layout.widget_mut("Border") {
        if let Kind::Sprite(s) = &mut w.kind {
            // One row of the button list is 0.05666 of the screen; the
            // border's centre moves down by half of that.
            s.pos[1] += 0.02833;
            s.scale[1] += 0.05666;
        }
    }
    setup_titled(layout);
    if let Some(w) = layout.widget_mut("FriendsIcon") {
        w.hidden = true;
    }
    let names: Vec<(String, MainAction)> = layout
        .widgets
        .iter()
        .filter_map(|w| match &w.kind {
            Kind::Button(b) => Some((w.name.clone(), main_action(&b.on_select))),
            _ => None,
        })
        .collect();
    for (name, action) in names {
        if unavailable.contains(&action) {
            disable(layout, &name);
            log.push(format!(
                "{name} ({action:?}) greyed out: not available in this port yet"
            ));
        }
    }
    log
}
