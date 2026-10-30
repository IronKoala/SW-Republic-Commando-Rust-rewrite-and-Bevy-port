//! Original pages with a scrolling list: the campaign page (planets and
//! their missions). `original_menu.rs` draws them like any other original
//! page; this file does what is special about them: moving along the list
//! with the keys, the mouse wheel, picking a row, and the campaign page's
//! planets and SELECT.
//!
//! The list itself (rows, scrolling, the scroll bar's handle) is in
//! `rc_menus::lists`, and the campaign page's script, rewritten by hand, in
//! `rc_menus::campaign`.

use crate::levels::LevelList;
use crate::original_menu::{OriginalMenus, Screen};
use rc_game_core::{Action, ActionState, GameLog};
use rc_menus::campaign::{self, Campaign};
use rc_menus::layout::Kind;
use rc_menus::lists;
use rc_menus::pages::PageCmd;
use std::time::{Duration, Instant};

/// The campaign page's name in `original_menus.toml`.
pub const CAMPAIGNS: &str = "campaigns";

/// Two clicks on the same row within this time open it (the original's
/// `OnDoubleClick`).
const DOUBLE_CLICK: Duration = Duration::from_millis(500);

/// Set the campaign page up from its class once it has passed the page
/// check: read its planets and missions, name the every-map list's maps
/// from the game's own level names, and choose the first planet. False if
/// the class has no planets (the placeholder page is used then).
pub fn setup_campaign(
    menus: &mut OriginalMenus,
    class: &rc_menus::library::ClassData,
    levels: Option<&LevelList>,
    log: &GameLog,
) -> bool {
    let Some(page) = menus.pages.get_mut(CAMPAIGNS) else {
        return false;
    };
    let Some(mut c) = campaign::read(class) else {
        log.line("original menus: page campaigns has no planets; using the placeholder");
        return false;
    };
    if let Some(levels) = levels {
        for p in &mut c.planets {
            for (map, name) in &mut p.every_map {
                *name = match levels.find(map).filter(|e| !e.title.is_empty()) {
                    Some(e) => e.title.to_uppercase(),
                    None => name.to_uppercase(),
                };
            }
        }
    }
    for p in &c.planets {
        log.line(format!(
            "original menus: campaign {:?}: {} missions ({}), {} maps",
            p.title,
            p.missions.len(),
            p.missions
                .iter()
                .map(|(m, n)| format!("{n} = {m}"))
                .collect::<Vec<_>>()
                .join(", "),
            p.every_map.len()
        ));
    }
    let layout = &mut page.screen.layout;
    campaign::setup(layout, &c);
    campaign::choose(layout, &mut c, 0);
    menus.campaign = Some(c);
    true
}

/// Focus and choose item `item` of the page's list (scrolling to it), and
/// light its mission blip on the campaign page.
fn pick(screen: &mut Screen, campaign: Option<&Campaign>, item: usize) {
    let layout = &mut screen.layout;
    let Some(w) = lists::show_item(layout, 0, item) else {
        return;
    };
    layout.lists.lists[0].selected = Some(item);
    screen.focus = Some(w);
    if let Some(c) = campaign {
        campaign::highlight(layout, c, Some(item));
    }
}

/// Nothing chosen in the list (the keys left it for a button).
fn unpick(screen: &mut Screen, campaign: Option<&Campaign>) {
    if let Some(l) = screen.layout.lists.lists.first_mut() {
        l.selected = None;
    }
    if let Some(c) = campaign {
        campaign::highlight(&mut screen.layout, c, None);
    }
}

/// When a page with a list opens: its first item has focus.
pub fn focus_first(menus: &mut OriginalMenus, k: &str) {
    let campaign = if k == CAMPAIGNS {
        menus.campaign.as_ref()
    } else {
        None
    };
    let Some(page) = menus.pages.get_mut(k) else {
        return;
    };
    let has_items = page
        .screen
        .layout
        .lists
        .lists
        .first()
        .is_some_and(|l| !l.items.is_empty());
    if has_items {
        pick(&mut page.screen, campaign, 0);
    }
}

fn button(screen: &Screen, name: &str) -> Option<usize> {
    screen.layout.widgets.iter().position(|w| w.name == name)
}

/// The keys and the mouse wheel on a page with a list, as the original's
/// `HandleInputUp`, `HandleInputDown`, `HandleInputLeft` and
/// `HandleInputRight` do on a PC: up and down move along the list (down
/// from its last item goes to SELECT, up from the buttons comes back to
/// the first item), left and right change planet. `None` when the input is
/// for the ordinary button handling; otherwise whether focus moved.
pub fn keys(
    menus: &mut OriginalMenus,
    k: &str,
    actions: &ActionState,
    wheel_y: f32,
    log: &GameLog,
) -> Option<bool> {
    let is_campaign = k == CAMPAIGNS && menus.campaign.is_some();
    let page = menus.pages.get_mut(k)?;
    let screen = &mut page.screen;
    let list = screen.layout.lists.lists.first()?.clone();
    let campaign = if is_campaign {
        menus.campaign.as_mut()
    } else {
        None
    };
    let at = screen.focus.and_then(|f| lists::item_of(&screen.layout, f));
    if wheel_y != 0.0 {
        let by = if wheel_y > 0.0 { -1 } else { 1 };
        if lists::scroll_to(&mut screen.layout, 0, list.position as i32 + by) {
            // The chosen row scrolls with the list; focus follows it while
            // it's on screen.
            let sel = screen.layout.lists.lists[0].selected;
            let row = sel.and_then(|s| lists::widget_of(&screen.layout, 0, s));
            if screen
                .focus
                .is_some_and(|f| lists::row_of(&screen.layout, f).is_some())
            {
                screen.focus = row;
            }
            return Some(true);
        }
        return Some(false);
    }
    if actions.just_pressed(Action::MenuDown) {
        let c = campaign.as_deref();
        match at {
            Some(i) if i + 1 < list.items.len() => pick(screen, c, i + 1),
            Some(_) => {
                unpick(screen, c);
                screen.focus = button(screen, "AButton");
            }
            None if screen.focus.is_none() && !list.items.is_empty() => pick(screen, c, 0),
            // On the buttons under the list, down goes round them.
            None => {
                let others: Vec<usize> = screen
                    .buttons()
                    .into_iter()
                    .filter(|&b| lists::row_of(&screen.layout, b).is_none())
                    .collect();
                let now = screen
                    .focus
                    .and_then(|f| others.iter().position(|&b| b == f))?;
                screen.focus = Some(others[(now + 1) % others.len()]);
            }
        }
        return Some(true);
    }
    if actions.just_pressed(Action::MenuUp) {
        let c = campaign.as_deref();
        match at {
            Some(0) => return Some(false),
            Some(i) => pick(screen, c, i - 1),
            None if !list.items.is_empty() => pick(screen, c, 0),
            None => return None,
        }
        return Some(true);
    }
    let step = if actions.just_pressed(Action::MenuRight) {
        1
    } else if actions.just_pressed(Action::MenuLeft) {
        -1
    } else {
        return None;
    };
    let c = campaign?;
    let Some(p) = c.neighbour(step) else {
        return Some(false);
    };
    campaign::choose(&mut screen.layout, c, p);
    log.line(format!(
        "original menus: campaign planet {} ({})",
        c.planets[p].title, c.planets[p].button
    ));
    screen.focus = None;
    if !screen.layout.lists.lists[0].items.is_empty() {
        pick(screen, Some(c), 0);
    }
    Some(true)
}

/// What the list's own commands do. Returns the command for the ordinary
/// handling to finish (opening a map, for example), or `None` when done.
/// `widget` is the button chosen; `by_key` when it was chosen with the
/// accept key or button rather than the mouse.
pub fn command(
    menus: &mut OriginalMenus,
    k: &str,
    widget: Option<usize>,
    by_key: bool,
    cmd: PageCmd,
    log: &GameLog,
) -> Option<PageCmd> {
    let is_campaign = k == CAMPAIGNS && menus.campaign.is_some();
    let now = Instant::now();
    let last_click = menus.last_click.take();
    let page = menus.pages.get_mut(k)?;
    let screen = &mut page.screen;
    let campaign = if is_campaign {
        menus.campaign.as_mut()
    } else {
        None
    };
    match cmd {
        PageCmd::ScrollUp | PageCmd::ScrollDown => {
            let by = if cmd == PageCmd::ScrollUp { -1 } else { 1 };
            let pos = screen.layout.lists.lists.first()?.position as i32;
            lists::scroll_to(&mut screen.layout, 0, pos + by);
            let sel = screen.layout.lists.lists[0].selected;
            if screen
                .focus
                .is_some_and(|f| lists::row_of(&screen.layout, f).is_some())
            {
                screen.focus = sel.and_then(|s| lists::widget_of(&screen.layout, 0, s));
            }
            None
        }
        PageCmd::ListPick => {
            let w = widget?;
            let item = lists::item_of(&screen.layout, w)?;
            let double = last_click.is_some_and(|(lw, t)| lw == w && now - t < DOUBLE_CLICK);
            menus.last_click = Some((w, now));
            pick(screen, campaign.as_deref(), item);
            log.line(format!(
                "original menus: page {k}: list item {item} {:?}",
                screen.layout.lists.lists[0].items[item]
            ));
            // The accept key on a row, or a double click, opens it at once
            // (the original's `HandleInputSelect` and `OnDoubleClick`).
            if by_key || double {
                return command(
                    menus,
                    k,
                    None,
                    by_key,
                    PageCmd::Act("start_level".into()),
                    log,
                );
            }
            None
        }
        PageCmd::Act(a) if a == "planet" => {
            let c = campaign?;
            let on_select = match &screen.layout.widgets[widget?].kind {
                Kind::Button(b) => b.on_select.clone(),
                _ => return None,
            };
            let p = c.planet_for(&on_select)?;
            campaign::choose(&mut screen.layout, c, p);
            log.line(format!(
                "original menus: campaign planet {}",
                c.planets[p].title
            ));
            if !screen.layout.lists.lists[0].items.is_empty() {
                pick(screen, Some(c), 0);
            }
            None
        }
        PageCmd::Act(a) if a == "every_map" => {
            let c = campaign?;
            c.every_map = !c.every_map;
            let p = c.chosen;
            campaign::choose(&mut screen.layout, c, p);
            let text = if c.every_map {
                "MISSIONS +"
            } else {
                "EVERY MAP +"
            };
            rc_menus::screens::set_widget_text(&mut screen.layout, "EveryMap", text);
            log.line(format!(
                "original menus: campaign list shows {}",
                if c.every_map {
                    "every map"
                } else {
                    "the missions"
                }
            ));
            None
        }
        PageCmd::Act(a) if a == "start_level" => {
            let c = campaign?;
            let Some(item) = screen.layout.lists.lists.first().and_then(|l| l.selected) else {
                log.line("original menus: SELECT with no mission chosen");
                return None;
            };
            let map = c.map(item)?.to_ascii_lowercase();
            log.line(format!(
                "original menus: campaign {} item {item}: opening {map}",
                c.planets[c.chosen].title
            ));
            Some(PageCmd::Play(map))
        }
        PageCmd::Act(a) => {
            log.line(format!("original menus: page {k}: nothing does {a:?}"));
            None
        }
        other => Some(other),
    }
}
