//! What each original menu does on a PC, rewritten by hand in Rust from
//! reading the game's own menu scripts (never copied). Each menu's script
//! adjusts a few widgets when it opens (`Init`), and names a function for
//! each button; here those become plain data the game acts on.

use crate::layout::{pivot_fraction, Kind, MenuLayout, Sprite, MSCM_FIT, MSCM_FIT_STRETCH};

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

/// Give a widget new text (a label, or a button's text when blurred and
/// focused). False if the page has no such widget.
pub fn set_widget_text(layout: &mut MenuLayout, name: &str, text: &str) -> bool {
    let Some(w) = layout.widget_mut(name) else {
        return false;
    };
    match &mut w.kind {
        Kind::Text(t) => t.text = text.to_string(),
        Kind::Button(b) => {
            b.blurred.text = text.to_string();
            b.focused.text = text.to_string();
        }
        Kind::Sprite(_) => return false,
    }
    true
}

/// Move a widget down (up when `by` is negative). False if the page has no
/// such widget.
pub fn move_widget(layout: &mut MenuLayout, name: &str, by: f32) -> bool {
    let Some(w) = layout.widget_mut(name) else {
        return false;
    };
    match &mut w.kind {
        Kind::Text(t) => t.pos[1] += by,
        Kind::Sprite(s) => s.pos[1] += by,
        Kind::Button(b) => {
            for p in [
                &mut b.blurred.pos,
                &mut b.focused.pos,
                &mut b.background_blurred.pos,
                &mut b.background_focused.pos,
            ] {
                p[1] += by;
            }
        }
    }
    true
}

/// Shrink each options row's label that would run into its own row's
/// arrow or value (long names such as AUTO PULL MANEUVERS), keeping a small
/// gap, so text never overlaps whatever the screen size: everything is
/// laid out in the same 4:3 area, so fitting once fits every resolution.
/// Returns the names of the labels shrunk.
pub fn fit_row_labels(layout: &mut MenuLayout, sizes: &mut dyn crate::quads::Sizes) -> Vec<String> {
    use crate::quads::{sprite_box, text_box};
    const GAP: f32 = 0.006;
    let mut shrunk = Vec::new();
    let rows: Vec<usize> = (0..layout.widgets.len())
        .filter(|&i| {
            let w = &layout.widgets[i];
            w.name.contains('[') && matches!(&w.kind, Kind::Button(b) if !b.mouse_only)
        })
        .collect();
    for r in rows {
        let row = row_widgets(layout, &layout.widgets[r].name);
        // Where each button in the row starts (the value bar, the arrows).
        let starts: Vec<f32> = row
            .iter()
            .filter_map(|&j| match &layout.widgets[j].kind {
                Kind::Button(o) if layout.widgets[j].shown_on_pc() => {
                    sprite_box(&o.background_blurred, sizes)
                        .map(|x| x[0])
                        .or_else(|| {
                            text_box(&o.blurred, sizes)
                                .filter(|_| !o.blurred.text.is_empty())
                                .map(|x| x[0])
                        })
                }
                _ => None,
            })
            .collect();
        for &i in &row {
            let w = &layout.widgets[i];
            let Kind::Text(t) = &w.kind else { continue };
            if t.text.is_empty() || !w.shown_on_pc() {
                continue;
            }
            let Some(b) = text_box(t, sizes) else {
                continue;
            };
            let limit = starts
                .iter()
                .filter(|&&x| x > b[0] + 0.01)
                .fold(f32::MAX, |m, &x| m.min(x - GAP));
            if limit == f32::MAX || b[2] <= limit {
                continue;
            }
            let factor = (limit - b[0]) / (b[2] - b[0]);
            if !(0.3..1.0).contains(&factor) {
                continue;
            }
            let name = w.name.clone();
            if let Kind::Text(t) = &mut layout.widgets[i].kind {
                // Shrink from the label's left edge, keeping its middle line.
                let (px, py) = pivot_fraction(t.pivot);
                let mid = b[1] + (b[3] - b[1]) / 2.0;
                t.scale[0] *= factor;
                t.scale[1] *= factor;
                let (nw, nh) = ((b[2] - b[0]) * factor, (b[3] - b[1]) * factor);
                t.pos[0] = b[0] + nw * px;
                t.pos[1] = mid - nh / 2.0 + nh * py;
            }
            shrunk.push(name);
        }
    }
    shrunk
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

/// How many values a drop-down list shows at once (more scroll).
pub const LIST_ROWS: usize = 10;
/// Choosing a row of a drop-down list.
pub const LIST_PICK: &str = "ListPick";
/// Clicking outside a drop-down list closes it.
pub const LIST_CLOSE: &str = "ListClose";

/// The box a sprite covers when its size is given directly (the "fit"
/// scale modes), as fractions of the menu area.
fn fitted_box(s: &Sprite) -> Option<[f32; 4]> {
    if s.scale_mode != MSCM_FIT && s.scale_mode != MSCM_FIT_STRETCH {
        return None;
    }
    let (px, py) = pivot_fraction(s.pivot);
    let x0 = s.pos[0] - s.scale[0] * px;
    let y0 = s.pos[1] - s.scale[1] * py;
    Some([x0, y0, x0 + s.scale[0], y0 + s.scale[1]])
}

fn place(s: &mut Sprite, b: [f32; 4]) {
    s.pivot = 0;
    s.scale_mode = MSCM_FIT_STRETCH;
    s.pos = [b[0], b[1]];
    s.scale = [b[2] - b[0], b[3] - b[1]];
}

/// A drop-down list of a chooser's values (the original has none; PC games
/// show one for screen sizes), opening under the chooser and drawn only
/// with pieces of the page itself: the page's display box for the frame
/// over a black backing, and copies of the chooser for the rows, so it
/// looks like the rest of the options page. Shows `items[first..]`, up to
/// [`LIST_ROWS`]; rows are buttons named `ListItems[n]` (n counted from
/// `first`) that call [`LIST_PICK`]. A clear button over the whole screen,
/// drawn first, calls [`LIST_CLOSE`] so a click outside the list closes it.
pub fn drop_down(
    page: &MenuLayout,
    chooser: &str,
    items: &[String],
    first: usize,
) -> Option<MenuLayout> {
    let row = page.widget(chooser)?.clone();
    let Kind::Button(b) = &row.kind else {
        return None;
    };
    let frame = page
        .widgets
        .iter()
        .find(|w| matches!(&w.kind, Kind::Sprite(s) if s.texture.as_deref().is_some_and(|t| t.ends_with("CT_DisplayBox"))))?
        .clone();
    // The row's own bar gives the list its width and row height.
    let bar = fitted_box(&b.background_blurred).unwrap_or([
        b.blurred.pos[0] - 0.13,
        b.blurred.pos[1] - 0.013,
        b.blurred.pos[0] + 0.13,
        b.blurred.pos[1] + 0.013,
    ]);
    let step = 0.04;
    let shown = items.len().saturating_sub(first).min(LIST_ROWS);
    let pad = 0.012;
    // Wide enough to cover the arrows of the rows underneath.
    let side = 0.04;
    let outer = [
        bar[0] - side,
        bar[3] + 0.006,
        bar[2] + side,
        bar[3] + 0.006 + shown as f32 * step + 2.0 * pad,
    ];
    let mut widgets = Vec::new();
    // Over the whole screen: catches clicks outside the list.
    let mut outside = row.clone();
    outside.name = "ListOutside".into();
    if let Kind::Button(o) = &mut outside.kind {
        o.items.clear();
        o.blurred.text.clear();
        o.focused.text.clear();
        o.on_select = LIST_CLOSE.into();
        o.mouse_only = true;
        o.disabled = false;
        for s in [&mut o.background_blurred, &mut o.background_focused] {
            place(s, [-1.0, -1.0, 2.0, 2.0]);
            s.color[3] = 0;
        }
    }
    widgets.push(outside);
    // A solid black square behind the frame hides the rows underneath
    // (the page's own `blacksquare` picture, which it draws over a level).
    if let Some(black) = page.widgets.iter().find(|w| {
        matches!(&w.kind, Kind::Sprite(s) if s.texture.as_deref().is_some_and(|t| t.ends_with("blacksquare")))
    }) {
        let mut backing = black.clone();
        backing.name = "ListBacking".into();
        backing.hidden = false;
        if let Kind::Sprite(s) = &mut backing.kind {
            place(s, [outer[0] + 0.004, outer[1] + 0.004, outer[2] - 0.004, outer[3] - 0.004]);
            s.color = [0, 0, 0, 255];
        }
        widgets.push(backing);
    }
    let mut border = frame;
    border.name = "ListBorder".into();
    if let Kind::Sprite(s) = &mut border.kind {
        place(s, outer);
    }
    border.hidden = false;
    widgets.push(border);
    for n in 0..shown {
        let mut w = row.clone();
        w.name = format!("ListItems[{n}]");
        w.hidden = false;
        let dy = bar[3] + 0.006 + pad - bar[1] + n as f32 * step;
        if let Kind::Button(r) = &mut w.kind {
            r.items.clear();
            r.current = 0;
            r.blurred.text = items[first + n].clone();
            r.focused.text = items[first + n].clone();
            r.on_select = LIST_PICK.into();
            r.disabled = false;
            r.mouse_only = false;
            for p in [
                &mut r.blurred.pos,
                &mut r.focused.pos,
                &mut r.background_blurred.pos,
                &mut r.background_focused.pos,
            ] {
                p[1] += dy;
            }
        }
        widgets.push(w);
    }
    Some(MenuLayout {
        class: format!("{} (drop-down list)", page.class),
        widgets,
        skipped: Vec::new(),
        sounds: page.sounds.clone(),
        background_movie: String::new(),
        music: None,
    })
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
