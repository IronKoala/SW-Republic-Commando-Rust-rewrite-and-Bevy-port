//! The original load and save pages (`XInterfaceCTMenus.CTLoadGameMenu` and
//! `CTSaveGameMenu`) and the pause menu's quick save, quick load and "load
//! last save", as their scripts do them on a PC, rewritten by hand from
//! reading them (never copied).
//!
//! Both pages list the saves in `SaveList` (kept up to date by `rc_save`):
//! the autosave (our checkpoint save) first, then the quicksave, then the
//! named saves, newest first. LOAD GAME: SELECT loads the chosen save
//! (asking first while in a level, with the class's `LoadConfirm`), DELETE
//! deletes it after asking. SAVE GAME: NEW saves under a new name (the
//! campaign's name and a number, at most `MaxSaveSlots` of them),
//! OVERWRITE saves over the chosen one after asking; the autosave and
//! quicksave can't be overwritten. Every text shown comes from the class.
//!
//! `original_lists.rs` moves along the list; this file does the pages'
//! buttons. The work that needs the save messages is done by
//! [`apply_saves`], which also keeps the lists filled.

use crate::levels::LevelList;
use crate::original_menu::{Asking, OriginalMenus};
use rc_game_core::save::{DeleteSave, LoadGame, SaveGame, SaveList, SaveSlot};
use rc_game_core::{CurrentLevel, GameLog, GameState, Play};
use rc_menus::library::ClassData;
use rc_menus::lists;
use rc_menus::pages::PageCmd;
use rc_menus::value::V;

use bevy::prelude::*;

/// The load page's name in `original_menus.toml`.
pub const LOAD: &str = "load_game";
/// The save page's name in `original_menus.toml`.
pub const SAVE: &str = "save_game";

/// What a save button asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum SaveAsk {
    /// SELECT on the load page.
    Load(SaveSlot),
    /// DELETE on either page.
    Delete(SaveSlot),
    /// OVERWRITE on the save page.
    Overwrite(SaveSlot),
    /// NEW on the save page.
    New,
    /// The pause menu's QUICK SAVE, QUICK LOAD and LOAD LAST SAVE.
    QuickSave,
    QuickLoad,
    LoadLast,
    /// Something answered YES (or needed no question): do it now.
    Do(SaveDo),
}

/// What is done in the end.
#[derive(Debug, Clone, PartialEq)]
pub enum SaveDo {
    Load(SaveSlot),
    Save(SaveSlot),
    Delete(SaveSlot),
}

/// The save pages' texts, from their classes, and what they list.
#[derive(Debug, Clone)]
pub struct SavePages {
    pub load_confirm: String,
    pub delete_confirm: (String, String),
    pub overwrite_confirm: String,
    pub too_many: String,
    pub no_auto_quick_overwrite: String,
    pub max_slots: usize,
    /// GEO, RAS and YYY campaign names, and the one for other maps.
    pub campaign_names: [String; 4],
    pub quick_name: String,
    pub auto_name: String,
    /// The slot behind each list item, per page.
    pub slots: Vec<(String, Vec<SaveSlot>)>,
    /// Work for `apply_saves`.
    pub asks: Vec<SaveAsk>,
    /// The lists need filling again.
    pub stale: bool,
}

impl Default for SavePages {
    fn default() -> Self {
        // Used only when a class doesn't have a text: the class's own
        // texts replace these in `read`.
        SavePages {
            load_confirm: "Load this save? Unsaved progress will be lost.".into(),
            delete_confirm: ("Delete ".into(), " ?".into()),
            overwrite_confirm: "Overwrite ".into(),
            too_many: "All save slots are used.".into(),
            no_auto_quick_overwrite: "This save can't be overwritten.".into(),
            max_slots: 5,
            campaign_names: [
                "Geonosis".into(),
                "AssaultShip".into(),
                "Kashyyyk".into(),
                "Unknown".into(),
            ],
            quick_name: "QuickSave".into(),
            auto_name: "AutoSave".into(),
            slots: Vec::new(),
            asks: Vec::new(),
            stale: true,
        }
    }
}

fn text(class: &ClassData, name: &str, into: &mut String) {
    if let Some(V::Str(s)) = class.get(name) {
        *into = s.clone();
    }
}

/// Read the pages' texts from their classes (either may be missing).
pub fn read(load: Option<&ClassData>, save: Option<&ClassData>) -> SavePages {
    let mut p = SavePages::default();
    for c in load.iter().chain(save.iter()) {
        text(c, "StringDeleteConfirm", &mut p.delete_confirm.0);
        text(c, "StringDeleteConfirmEnd", &mut p.delete_confirm.1);
        text(c, "QuickSaveName", &mut p.quick_name);
        text(c, "AutoSaveName", &mut p.auto_name);
    }
    if let Some(c) = load {
        text(c, "LoadConfirm", &mut p.load_confirm);
    }
    if let Some(c) = save {
        text(c, "StringOverwriteConfirm", &mut p.overwrite_confirm);
        text(c, "StringTooManySaves", &mut p.too_many);
        text(
            c,
            "StringNoAutoQuickOverwrite",
            &mut p.no_auto_quick_overwrite,
        );
        if let Some(V::Int(n)) = c.get("MaxSaveSlots") {
            p.max_slots = (*n).max(1) as usize;
        }
        for (i, name) in [
            "GEOCampaignName",
            "RASCampaignName",
            "YYYCampaignName",
            "UnknownCampaignName",
        ]
        .iter()
        .enumerate()
        {
            text(c, name, &mut p.campaign_names[i]);
        }
    }
    p
}

impl SavePages {
    /// A save's name as the lists and questions show it.
    pub fn name(&self, slot: &SaveSlot) -> String {
        match slot {
            SaveSlot::Checkpoint => self.auto_name.to_uppercase(),
            SaveSlot::Quick => self.quick_name.to_uppercase(),
            SaveSlot::Named(n) => n.replace('_', " ").to_uppercase(),
        }
    }

    /// The slot of item `item` on page `k`.
    fn slot(&self, k: &str, item: usize) -> Option<&SaveSlot> {
        self.slots
            .iter()
            .find(|(p, _)| p == k)
            .and_then(|(_, s)| s.get(item))
    }

    /// A new save's name: the campaign of `map` and the first number not
    /// used yet (`Geonosis_1`).
    pub fn new_name(&self, map: &str, list: &SaveList) -> String {
        let prefix = map.get(..3).unwrap_or("").to_ascii_lowercase();
        let campaign = match prefix.as_str() {
            "geo" | "pro" => &self.campaign_names[0],
            "ras" => &self.campaign_names[1],
            "yyy" => &self.campaign_names[2],
            _ => &self.campaign_names[3],
        };
        let campaign: String = campaign
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        (1..)
            .map(|n| format!("{campaign}_{n}"))
            .find(|name| list.get(&SaveSlot::Named(name.clone())).is_none())
            .unwrap()
    }
}

/// The order the pages list saves in: autosave, quicksave, then the named
/// saves newest first (the list is newest first already).
pub fn ordered(list: &SaveList) -> Vec<&rc_game_core::save::SaveInfo> {
    let rank = |s: &SaveSlot| match s {
        SaveSlot::Checkpoint => 0,
        SaveSlot::Quick => 1,
        SaveSlot::Named(_) => 2,
    };
    let mut v: Vec<_> = list.0.iter().collect();
    // A stable sort keeps the named saves newest first.
    v.sort_by_key(|s| rank(&s.slot));
    v
}

/// What the pages' own commands do: they become work for `apply_saves`.
/// Returns the command for the ordinary handling when it isn't one of
/// theirs.
pub fn command(menus: &mut OriginalMenus, k: &str, cmd: PageCmd, log: &GameLog) -> Option<PageCmd> {
    let PageCmd::Act(a) = &cmd else {
        return Some(cmd);
    };
    let chosen = menus
        .pages
        .get(k)
        .and_then(|p| p.screen.layout.lists.lists.first())
        .and_then(|l| l.selected)
        .and_then(|item| menus.saves.slot(k, item).cloned());
    let ask = match a.as_str() {
        "quick_save" => SaveAsk::QuickSave,
        "quick_load" => SaveAsk::QuickLoad,
        "load_last" => SaveAsk::LoadLast,
        "new_save" => SaveAsk::New,
        "load_save" | "delete_save" | "overwrite_save" => {
            let Some(slot) = chosen else {
                log.line(format!("original menus: page {k}: {a} with no save chosen"));
                return None;
            };
            match a.as_str() {
                "load_save" => SaveAsk::Load(slot),
                "delete_save" => SaveAsk::Delete(slot),
                _ => SaveAsk::Overwrite(slot),
            }
        }
        _ => {
            log.line(format!("original menus: page {k}: nothing does {a:?}"));
            return None;
        }
    };
    menus.saves.asks.push(ask);
    None
}

/// Fill both pages' lists from `SaveList`, keeping the chosen save chosen
/// where it's still there.
fn fill(menus: &mut OriginalMenus, list: &SaveList, levels: Option<&LevelList>) {
    let saves = ordered(list);
    let slots: Vec<SaveSlot> = saves.iter().map(|s| s.slot.clone()).collect();
    let items: Vec<String> = saves
        .iter()
        .map(|s| {
            let level = levels
                .and_then(|l| l.find(&s.map))
                .filter(|e| !e.title.is_empty())
                .map_or_else(|| s.map.to_uppercase(), |e| e.title.to_uppercase());
            let when = rc_save::file::time_text(s.saved_at);
            let when = when.trim_end_matches(" UTC");
            format!("{}   {level}   {when}", menus.saves.name(&s.slot))
        })
        .collect();
    menus.saves.slots.clear();
    for k in [LOAD, SAVE] {
        let Some(page) = menus.pages.get_mut(k) else {
            continue;
        };
        if page.screen.layout.lists.lists.is_empty() {
            continue;
        }
        let had_row_focus = page
            .screen
            .focus
            .is_some_and(|f| lists::row_of(&page.screen.layout, f).is_some());
        let old = page.screen.layout.lists.lists[0].selected;
        lists::set_items(&mut page.screen.layout, 0, items.clone());
        if let Some(at) = old.filter(|_| !items.is_empty()) {
            let at = at.min(items.len() - 1);
            if let Some(w) = lists::show_item(&mut page.screen.layout, 0, at) {
                page.screen.layout.lists.lists[0].selected = Some(at);
                if had_row_focus {
                    page.screen.focus = Some(w);
                }
            }
        } else if had_row_focus {
            page.screen.focus = None;
        }
        menus.saves.slots.push((k.to_string(), slots.clone()));
    }
}

/// Fill the lists when the saves change, and do the work the buttons and
/// the question box asked for.
#[allow(clippy::too_many_arguments)]
pub fn apply_saves(
    mut menus: ResMut<OriginalMenus>,
    list: Res<SaveList>,
    levels: Option<Res<LevelList>>,
    state: Res<State<GameState>>,
    current: Res<CurrentLevel>,
    mut next_play: Option<ResMut<NextState<Play>>>,
    (mut save, mut load, mut delete): (
        MessageWriter<SaveGame>,
        MessageWriter<LoadGame>,
        MessageWriter<DeleteSave>,
    ),
    log: Res<GameLog>,
) {
    if !menus.pages.contains_key(LOAD) && !menus.pages.contains_key(SAVE) {
        menus.saves.asks.clear();
        return;
    }
    if list.is_changed() || menus.saves.stale {
        menus.saves.stale = false;
        fill(&mut menus, &list, levels.as_deref());
        menus.mark_dirty();
    }
    let asks = std::mem::take(&mut menus.saves.asks);
    let in_level = *state.get() == GameState::InGame;
    for ask in asks {
        let s = &menus.saves;
        let question = match ask {
            SaveAsk::Load(slot) if in_level => {
                Some((s.load_confirm.clone(), Asking::Confirm(SaveDo::Load(slot))))
            }
            SaveAsk::Load(slot) => {
                menus.saves.asks.push(SaveAsk::Do(SaveDo::Load(slot)));
                None
            }
            SaveAsk::Delete(slot) => Some((
                format!(
                    "{}{}{}",
                    s.delete_confirm.0,
                    s.name(&slot),
                    s.delete_confirm.1
                ),
                Asking::Confirm(SaveDo::Delete(slot)),
            )),
            SaveAsk::Overwrite(SaveSlot::Named(n)) => Some((
                format!(
                    "{}{}{}",
                    s.overwrite_confirm,
                    s.name(&SaveSlot::Named(n.clone())),
                    s.delete_confirm.1
                ),
                Asking::Confirm(SaveDo::Save(SaveSlot::Named(n))),
            )),
            SaveAsk::Overwrite(_) => Some((s.no_auto_quick_overwrite.clone(), Asking::Notice)),
            SaveAsk::New => {
                let named = list
                    .0
                    .iter()
                    .filter(|i| matches!(i.slot, SaveSlot::Named(_)))
                    .count();
                if named >= s.max_slots {
                    Some((s.too_many.clone(), Asking::Notice))
                } else {
                    let map = current.map.clone().unwrap_or_default();
                    let name = s.new_name(&map, &list);
                    menus
                        .saves
                        .asks
                        .push(SaveAsk::Do(SaveDo::Save(SaveSlot::Named(name))));
                    None
                }
            }
            SaveAsk::QuickSave => {
                // The original saves and goes back to the game.
                menus
                    .saves
                    .asks
                    .push(SaveAsk::Do(SaveDo::Save(SaveSlot::Quick)));
                if let Some(n) = next_play.as_mut() {
                    n.set(Play::Running);
                }
                None
            }
            SaveAsk::QuickLoad => {
                menus
                    .saves
                    .asks
                    .push(SaveAsk::Do(SaveDo::Load(SaveSlot::Quick)));
                None
            }
            SaveAsk::LoadLast => {
                match list.newest() {
                    Some(n) => menus
                        .saves
                        .asks
                        .push(SaveAsk::Do(SaveDo::Load(n.slot.clone()))),
                    None => log.line("original menus: no save to load"),
                }
                None
            }
            SaveAsk::Do(d) => {
                log.line(format!("original menus: {d:?}"));
                match d {
                    SaveDo::Load(slot) => {
                        load.write(LoadGame { slot });
                    }
                    SaveDo::Save(slot) => {
                        save.write(SaveGame { slot });
                    }
                    SaveDo::Delete(slot) => {
                        delete.write(DeleteSave { slot });
                    }
                }
                None
            }
        };
        if let Some((text, asking)) = question {
            log.line(format!("original menus: asking {text:?}"));
            menus.ask(&text, asking);
        }
    }
    // Work added above (a YES needing no question) runs next frame.
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_game_core::save::SaveInfo;

    fn info(slot: SaveSlot, at: u64) -> SaveInfo {
        SaveInfo {
            slot,
            map: "geo_01a".into(),
            checkpoint: None,
            saved_at: at,
            play_seconds: 0.0,
        }
    }

    #[test]
    fn saves_are_listed_auto_quick_then_newest() {
        let list = SaveList(vec![
            info(SaveSlot::Named("b".into()), 50),
            info(SaveSlot::Quick, 40),
            info(SaveSlot::Named("a".into()), 30),
            info(SaveSlot::Checkpoint, 20),
        ]);
        let order: Vec<SaveSlot> = ordered(&list).iter().map(|s| s.slot.clone()).collect();
        assert_eq!(
            order,
            [
                SaveSlot::Checkpoint,
                SaveSlot::Quick,
                SaveSlot::Named("b".into()),
                SaveSlot::Named("a".into())
            ]
        );
    }

    #[test]
    fn new_saves_are_named_after_the_campaign() {
        let p = SavePages::default();
        let list = SaveList(vec![info(SaveSlot::Named("Geonosis_1".into()), 1)]);
        assert_eq!(p.new_name("geo_01a", &list), "Geonosis_2");
        assert_eq!(p.new_name("ras_02b", &list), "AssaultShip_1");
        assert_eq!(p.name(&SaveSlot::Named("Geonosis_2".into())), "GEONOSIS 2");
        assert_eq!(p.name(&SaveSlot::Quick), "QUICKSAVE");
    }
}
