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

/// The row a widget belongs to on an options page: its name without the
/// array index and a trailing "s", and the index. `Options[3]` is row 3 of
/// `Option` (with `OptionLabels[3]` and `OptionLeftArrows[3]`);
/// `OptionSet1[2]` is row 2 of `OptionSet1` (with `OptionSet1Labels[2]`).
fn row_of(name: &str) -> Option<(&str, &str)> {
    let open = name.find('[')?;
    let base = &name[..open];
    let index = &name[open..];
    Some((base.strip_suffix('s').unwrap_or(base), index))
}

/// The other widgets in the same row as `name` (its label and arrows).
pub fn row_widgets(layout: &MenuLayout, name: &str) -> Vec<usize> {
    let Some((family, index)) = row_of(name) else {
        return vec![];
    };
    (0..layout.widgets.len())
        .filter(|&i| {
            let n = &layout.widgets[i].name;
            n.starts_with(family)
                && n.ends_with(index)
                && row_of(n).is_some_and(|(_, ix)| ix == index)
        })
        .collect()
}

/// Grey out a whole options row (chooser or key button, label, arrows),
/// the way the original's options pages do (`DisableOption`).
pub fn disable_row(layout: &mut MenuLayout, name: &str) {
    for i in row_widgets(layout, name) {
        match &mut layout.widgets[i].kind {
            Kind::Button(b) => {
                b.disabled = true;
                for c in [&mut b.blurred.color, &mut b.background_blurred.color] {
                    c[0] = DISABLED_GREY;
                    c[1] = DISABLED_GREY;
                    c[2] = DISABLED_GREY;
                }
            }
            Kind::Text(t) => {
                t.color[0] = DISABLED_GREY;
                t.color[1] = DISABLED_GREY;
                t.color[2] = DISABLED_GREY;
            }
            Kind::Sprite(_) => {}
        }
    }
}

/// The chooser an arrow belongs to (the chooser in the same row).
pub fn chooser_for(layout: &MenuLayout, widget: usize) -> Option<usize> {
    (0..layout.widgets.len()).find(|&c| {
        matches!(&layout.widgets[c].kind, Kind::Button(b) if b.is_chooser())
            && row_widgets(layout, &layout.widgets[c].name).contains(&widget)
    })
}

/// How many sets of rows the page has (`OptionSet0`, `OptionSet1`...; the
/// controls page has three, flipped with PREVIOUS SET and NEXT SET).
pub fn set_count(layout: &MenuLayout) -> usize {
    (0..10)
        .take_while(|k| {
            let p = format!("OptionSet{k}[");
            layout.widgets.iter().any(|w| w.name.starts_with(&p))
        })
        .count()
}

/// Show one set of rows and hide the others, and show PREVIOUS SET and
/// NEXT SET only where there is a set to go to (the controls page's
/// `SwitchToSet`). Does nothing on a page without sets.
pub fn show_set(layout: &mut MenuLayout, set: usize) {
    let n = set_count(layout);
    if n == 0 {
        return;
    }
    let set = set.min(n - 1);
    for w in layout.widgets.iter_mut() {
        if let Some(rest) = w.name.strip_prefix("OptionSet") {
            let k: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(k) = k.parse::<usize>() {
                w.hidden = k != set;
            }
        }
    }
    if let Some(w) = layout.widget_mut("PrevSet") {
        w.hidden = set == 0;
    }
    if let Some(w) = layout.widget_mut("NextSet") {
        w.hidden = set + 1 >= n;
    }
}

/// The set of rows showing (0 on a page without sets).
pub fn current_set(layout: &MenuLayout) -> usize {
    (0..set_count(layout))
        .find(|k| {
            let p = format!("OptionSet{k}[");
            layout
                .widgets
                .iter()
                .any(|w| w.name.starts_with(&p) && !w.hidden)
        })
        .unwrap_or(0)
}

/// Add a button the port needs to an original page (see
/// [`ExtraButton`]): a copy of `copy_of`, moved down, with new text and a
/// function name of its own. Returns false if `copy_of` isn't a button.
pub fn add_button_copy(layout: &mut MenuLayout, e: &crate::pages::ExtraButton) -> bool {
    let Some(at) = layout
        .widgets
        .iter()
        .position(|w| w.name.eq_ignore_ascii_case(&e.copy_of))
    else {
        return false;
    };
    let mut w = layout.widgets[at].clone();
    let Kind::Button(b) = &mut w.kind else {
        return false;
    };
    b.blurred.text = e.text.clone();
    b.focused.text = e.text.clone();
    b.on_select = e.name.clone();
    b.disabled = false;
    for p in [
        &mut b.blurred.pos,
        &mut b.focused.pos,
        &mut b.background_blurred.pos,
        &mut b.background_focused.pos,
    ] {
        p[1] += e.move_down;
    }
    w.name = e.name.clone();
    w.hidden = false;
    layout.widgets.insert(at + 1, w);
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

/// The class of the original "reassign button" box, shown over the
/// controls page while a new key is chosen.
pub const MAP_KEY: &str = "XInterfaceCTMenus.CTMapKeyMenu";

/// Fill in the "reassign button" box: the action's name, its current
/// key, and the note about keys that can't be changed (the keyboard note,
/// or the controller one when `pad`). The A, B and X buttons (ACCEPT,
/// CANCEL, CLEAR) stay hidden until a key has been pressed, as in the
/// original.
pub fn setup_map_key(layout: &mut MenuLayout, action: &str, current: &str, pad: bool) {
    setup_titled(layout);
    let set_text = |layout: &mut MenuLayout, name: &str, text: &str| {
        if let Some(w) = layout.widget_mut(name) {
            if let Kind::Text(t) = &mut w.kind {
                t.text = text.to_string();
            }
        }
    };
    set_text(layout, "ActionLabel", action);
    set_text(layout, "ButtonLabel", current);
    if let Some(w) = layout.widget_mut("LabelNoXboxSpecial") {
        w.hidden = !pad;
    }
    if let Some(w) = layout.widget_mut("LabelNoPCSpecial") {
        w.hidden = pad;
    }
    map_key_waiting(layout, true);
}

/// Switch the "reassign button" box between waiting for a key (the prompt
/// shows, the buttons hide) and showing the key chosen (`waiting` false:
/// ACCEPT, CANCEL and CLEAR show).
pub fn map_key_waiting(layout: &mut MenuLayout, waiting: bool) {
    if let Some(w) = layout.widget_mut("Prompt") {
        w.hidden = !waiting;
    }
    for name in ["AButton", "BButton", "XButton"] {
        if let Some(w) = layout.widget_mut(name) {
            w.hidden = waiting;
        }
    }
}

/// Show the key just pressed in the "reassign button" box.
pub fn map_key_chosen(layout: &mut MenuLayout, key: &str) {
    if let Some(w) = layout.widget_mut("ButtonLabel") {
        if let Kind::Text(t) = &mut w.kind {
            t.text = key.to_string();
        }
    }
    map_key_waiting(layout, false);
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
