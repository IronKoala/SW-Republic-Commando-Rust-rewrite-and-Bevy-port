//! The original PROFILES pages, as their scripts do them on a PC, rewritten
//! by hand from reading them (never copied):
//!
//! - `XInterfaceCommon.MenuSelectProfile` (page `profiles`): the list
//!   "< NEW >" then every profile, in capitals. SELECT (or Enter, or a
//!   double click) on "< NEW >" opens the name page; on a profile, loads
//!   it. DELETE asks first, and refuses the profile in use. BACK closes the
//!   page, except when no profile is loaded yet: then BACK is hidden and
//!   does nothing (`bMustSelectProfile`), and the main menu opens this page
//!   by itself, as the original's `CTMenuMain.Init` does.
//! - `XInterfaceCommon.MenuPCNewProfileName` (page `new_profile`): type a
//!   name in its box (`rc_menus::edit`), OK makes the profile. Spaces
//!   become "_"; the name may not be "Profile" or start with "Profile_",
//!   must have a letter or digit, and may not be taken. BACK goes back to
//!   the list.
//! - Problems show in `XInterfaceCommon.MenuWarning` (CONTINUE closes it).
//! - Loading or making a profile shows `XInterfaceCommon.MenuLoadingProfile`
//!   ("LOADING PROFILE: MATT") for its `HoldTime` (1 second), then goes back
//!   to the main menu.
//!
//! Every text shown comes from those classes. A profile is a folder in the
//! save folder holding its saves (`rc_save::profiles`), so each profile has
//! its own saves, as in the original.
//!
//! Not done: the original's profile also holds the player's settings and
//! campaign progress (its `User.ini`); here settings stay shared by every
//! profile, and the campaign page lists every mission.

use crate::menu::{Menu, Page};
use crate::original_menu::{Asking, OriginalMenus, Screen};
use crate::settings::{Settings, SettingsPath};
use bevy::prelude::*;
use rc_game_core::{GameLog, GameState};
use rc_menus::layout::{Kind, MenuLayout};
use rc_menus::library::ClassData;
use rc_menus::lists;
use rc_menus::pages::PageCmd;
use rc_menus::value::V;
use rc_save::profiles;
use std::path::{Path, PathBuf};

/// The list page's name in `original_menus.toml`.
pub const PROFILES: &str = "profiles";
/// The name page's name in `original_menus.toml`.
pub const NEW_PROFILE: &str = "new_profile";
/// The warning box's class (the original's `OverlayMenuClass` calls).
pub const WARNING: &str = "XInterfaceCommon.MenuWarning";
/// The "LOADING PROFILE" message's class.
pub const LOADING: &str = "XInterfaceCommon.MenuLoadingProfile";
/// The name page's box.
const NAME_BOX: &str = "NameBox";

/// The profile in use, and the save folder its folder is in.
#[derive(Resource, Debug, Clone)]
pub struct Profile {
    /// The save folder from the settings (profiles are folders in it).
    pub root: PathBuf,
    /// `None` until one is chosen.
    pub name: Option<String>,
}

impl Profile {
    /// The profile named in the settings, if its folder is there.
    pub fn at_start(root: &Path, wanted: &str) -> Profile {
        let names = profiles::list(root);
        Profile {
            root: root.to_path_buf(),
            name: profiles::find(&names, wanted.trim()).cloned(),
        }
    }

    /// Where this profile's saves go: its folder, or (none chosen) the
    /// save folder itself.
    pub fn saves(&self) -> PathBuf {
        match &self.name {
            Some(n) => profiles::folder(&self.root, n),
            None => self.root.clone(),
        }
    }
}

/// The texts of the profile pages, from their classes.
#[derive(Debug, Clone)]
pub struct Texts {
    pub new_profile: String,
    pub delete_confirm: (String, String),
    pub active_delete: String,
    pub prefix: String,
    pub in_use: String,
    pub min_len: String,
    pub invalid_name: String,
    pub invalid_start: String,
    pub loading: String,
    pub creating: String,
    /// "%action PROFILE: %pName".
    pub message: String,
    /// Seconds the message shows.
    pub hold: f32,
}

impl Default for Texts {
    fn default() -> Self {
        // Used only when a class doesn't have a text: `read` puts the
        // classes' own texts in.
        Texts {
            new_profile: "< NEW >".into(),
            delete_confirm: ("Delete ".into(), " ?".into()),
            active_delete: "This profile is in use.".into(),
            prefix: profiles::PREFIX.into(),
            in_use: "%s is taken.".into(),
            min_len: "Type a name.".into(),
            invalid_name: "%s can't be used.".into(),
            invalid_start: "Names can't start with %s.".into(),
            loading: "LOADING".into(),
            creating: "CREATING".into(),
            message: "%action PROFILE: %pName".into(),
            hold: 1.0,
        }
    }
}

/// What a button asked for, done by `apply_profiles`.
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileAsk {
    /// SELECT on the list.
    Select,
    /// DELETE on the list.
    Delete,
    /// The question was answered YES: delete this profile.
    DoDelete(String),
    /// OK on the name page.
    Create,
}

/// The profile pages' state.
#[derive(Debug, Default)]
pub struct ProfilePages {
    pub texts: Texts,
    pub warning_layout: Option<MenuLayout>,
    pub loading_layout: Option<MenuLayout>,
    /// The profiles listed after "< NEW >", as their folders spell them.
    pub names: Vec<String>,
    /// The profile in use (kept from [`Profile`] for the page code).
    pub current: Option<String>,
    /// The item focused when the list was last left (the original's
    /// `LastFocus`).
    pub last_focus: usize,
    pub asks: Vec<ProfileAsk>,
    /// The list page opened (or the profiles changed): fill it again.
    pub stale: bool,
    /// The "LOADING PROFILE" message is showing: seconds left.
    pub holding: Option<f32>,
}

fn text(class: &ClassData, name: &str, into: &mut String) {
    if let Some(V::Str(s)) = class.get(name) {
        *into = s.clone();
    }
}

/// Read the texts from the classes (any may be missing).
pub fn read(
    select: Option<&ClassData>,
    new_name: Option<&ClassData>,
    loading: Option<&ClassData>,
) -> Texts {
    let mut t = Texts::default();
    if let Some(c) = select {
        text(c, "NewProfileText", &mut t.new_profile);
        text(c, "StringDeleteConfirm", &mut t.delete_confirm.0);
        text(c, "StringDeleteConfirmEnd", &mut t.delete_confirm.1);
        text(c, "StringActiveProfileDelete", &mut t.active_delete);
    }
    if let Some(c) = new_name {
        text(c, "ProfilePrefix", &mut t.prefix);
        text(c, "InUseText", &mut t.in_use);
        text(c, "MinLenText", &mut t.min_len);
        text(c, "InvalidName", &mut t.invalid_name);
        text(c, "InvalidNameStart", &mut t.invalid_start);
    }
    if let Some(c) = loading {
        text(c, "LoadingText", &mut t.loading);
        text(c, "CreatingText", &mut t.creating);
        if let Some(m) = c.get("Message") {
            let m = m.text("Text");
            if !m.is_empty() {
                t.message = m;
            }
        }
        if let Some(V::Float(h)) = c.get("HoldTime") {
            t.hold = *h;
        }
    }
    t
}

/// The name typed, as `MenuPCNewProfileName.OnAButton` checks it: spaces
/// become "_" (`FixupName`), a trailing space is cut (`TrimTrailingSpace`,
/// which finds none after that), then the checks, in its order. `Ok` is
/// the name to make; `Err` the warning to show.
pub fn check_new_name(typed: &str, names: &[String], t: &Texts) -> Result<String, String> {
    let mut name = typed.replace(' ', "_");
    if name.ends_with(' ') {
        name.pop();
    }
    let caps = name.to_uppercase();
    if caps == t.prefix.to_uppercase() {
        return Err(t.invalid_name.replacen("%s", &t.prefix, 1));
    }
    let prohibited = format!("{}_", t.prefix);
    if caps.starts_with(&prohibited.to_uppercase()) {
        return Err(t.invalid_start.replacen("%s", &prohibited, 1));
    }
    if name.chars().count() < 1 {
        return Err(t.min_len.clone());
    }
    // The original asks its profile code (`NAME_EXISTS`); here capitals
    // don't count, as Windows folder names ignore them.
    if profiles::find(names, &name).is_some() {
        return Err(t.in_use.replacen("%s", &name, 1));
    }
    Ok(name)
}

/// The message while a profile loads or is made: `%action` is LOADING or
/// CREATING, `%pName` the name.
pub fn loading_message(t: &Texts, new: bool, name: &str) -> String {
    let action = if new { &t.creating } else { &t.loading };
    t.message
        .replacen("%action", action, 1)
        .replacen("%pName", name, 1)
}

/// Set the pages up once they have passed the page check (the parts of
/// their `Init` that are the same every time): the split-screen widgets
/// are hidden (`HideSplitWidgets`), and the name page's box starts empty.
pub fn setup(menus: &mut OriginalMenus) {
    if let Some(p) = menus.pages.get_mut(PROFILES) {
        for name in ["SplitTitle", "SplitALabel", "SplitAButtonIcon"] {
            if let Some(w) = p.screen.layout.widget_mut(name) {
                w.hidden = true;
            }
        }
    }
}

/// When page `k` comes on screen.
pub fn opened(menus: &mut OriginalMenus, k: &str) {
    match k {
        PROFILES => menus.profiles.stale = true,
        NEW_PROFILE => {
            let Some(p) = menus.pages.get_mut(NEW_PROFILE) else {
                return;
            };
            // A fresh page each time (the original makes a new menu), with
            // focus in the box (`FocusOnWidget(NameBox)`).
            rc_menus::screens::set_widget_text(&mut p.screen.layout, NAME_BOX, "");
            let at = p
                .screen
                .layout
                .widgets
                .iter()
                .position(|w| w.name == NAME_BOX);
            p.screen.focus = at;
            if let Some(Kind::Button(b)) = at.map(|i| &mut p.screen.layout.widgets[i].kind) {
                if let Some(e) = &mut b.edit {
                    e.show_cursor = true;
                    e.since_blink = 0.0;
                }
            }
        }
        _ => {}
    }
}

/// The item chosen on the list page, if any.
fn selected(menus: &OriginalMenus) -> Option<usize> {
    menus
        .pages
        .get(PROFILES)?
        .screen
        .layout
        .lists
        .lists
        .first()?
        .selected
}

/// What the pages' own commands do: they become work for
/// `apply_profiles`. Returns the command for the ordinary handling when it
/// isn't one of theirs.
pub fn command(menus: &mut OriginalMenus, k: &str, cmd: PageCmd, log: &GameLog) -> Option<PageCmd> {
    if k != PROFILES && k != NEW_PROFILE {
        return Some(cmd);
    }
    let PageCmd::Act(a) = &cmd else {
        return Some(cmd);
    };
    match a.as_str() {
        // BACK, and the back key: nothing while a profile must be chosen.
        "profiles_back" => {
            if menus.profiles.current.is_none() {
                log.line("original menus: profiles: a profile must be chosen first");
                return None;
            }
            return Some(PageCmd::Back);
        }
        "select_profile" => menus.profiles.asks.push(ProfileAsk::Select),
        "delete_profile" => menus.profiles.asks.push(ProfileAsk::Delete),
        "create_profile" => menus.profiles.asks.push(ProfileAsk::Create),
        _ => return Some(cmd),
    }
    None
}

/// Enter in the name box: the original moves focus to the nearest widget
/// below it (it has no `OnSelect`). OK and BACK are both below and equally
/// far to each side; which one the original's search picks wasn't worked
/// out, so this port goes to OK.
pub fn enter_in_box(screen: &mut Screen) {
    if let Some(ok) = screen
        .layout
        .widgets
        .iter()
        .position(|w| w.name == "AButton")
    {
        screen.focus = Some(ok);
    }
}

/// Show `text` in the original's warning box (CONTINUE closes it).
fn warn(menus: &mut OriginalMenus, text: &str, under: &str, log: &GameLog) {
    log.line(format!("original menus: warning {text:?}"));
    let Some(mut layout) = menus.profiles.warning_layout.clone() else {
        return;
    };
    let under = menus.pages.get(under).map(|p| &p.screen.layout);
    let focus = rc_menus::screens::setup_question(&mut layout, "", under);
    let _ = focus;
    rc_menus::screens::set_widget_text(&mut layout, "Message", text);
    let focus = layout.widgets.iter().position(|w| w.name == "AButton");
    menus.show_box(Screen { layout, focus }, Asking::Warning);
}

/// Show the "LOADING PROFILE" message; when it has shown long enough, the
/// game goes back to the main menu.
fn hold_message(menus: &mut OriginalMenus, text: &str, under: &str, log: &GameLog) {
    log.line(format!("original menus: {text:?}"));
    let hold = menus.profiles.texts.hold;
    let Some(mut layout) = menus.profiles.loading_layout.clone() else {
        menus.profiles.holding = Some(0.0);
        return;
    };
    let under = menus.pages.get(under).map(|p| &p.screen.layout);
    rc_menus::screens::setup_question(&mut layout, "", under);
    rc_menus::screens::set_widget_text(&mut layout, "Message", text);
    menus.show_box(
        Screen {
            layout,
            focus: None,
        },
        Asking::Wait,
    );
    menus.profiles.holding = Some(hold);
}

/// Fill the list page from the profile folders: "< NEW >", then each
/// profile in capitals; focus on the profile in use, else where the list
/// was left (`SetupProfileList`).
fn fill(menus: &mut OriginalMenus, root: &Path) {
    let names = profiles::list(root);
    let pp = &mut menus.profiles;
    pp.names = names.clone();
    let mut items = vec![pp.texts.new_profile.clone()];
    items.extend(names.iter().map(|n| n.to_uppercase()));
    let focus = pp
        .current
        .as_ref()
        .and_then(|c| names.iter().position(|n| n.eq_ignore_ascii_case(c)))
        .map(|i| i + 1)
        .unwrap_or_else(|| pp.last_focus.min(items.len() - 1));
    let must_choose = pp.current.is_none();
    let Some(page) = menus.pages.get_mut(PROFILES) else {
        return;
    };
    let screen = &mut page.screen;
    lists::set_items(&mut screen.layout, 0, items);
    if let Some(w) = lists::show_item(&mut screen.layout, 0, focus) {
        screen.layout.lists.lists[0].selected = Some(focus);
        screen.focus = Some(w);
    }
    // No profile yet: BACK is hidden (`HideBButton(1)`).
    if let Some(w) = screen.layout.widget_mut("BButton") {
        w.hidden = must_choose;
    }
}

/// DELETE shows only while a profile (not "< NEW >") is chosen
/// (`ShowProfileDetails`, and the scroll arrows' `HideXButton(1)`).
fn show_delete(menus: &mut OriginalMenus) -> bool {
    let sel = selected(menus);
    let Some(page) = menus.pages.get_mut(PROFILES) else {
        return false;
    };
    let Some(w) = page.screen.layout.widget_mut("XButton") else {
        return false;
    };
    let hide = !matches!(sel, Some(s) if s > 0);
    let changed = w.hidden != hide;
    w.hidden = hide;
    if hide
        && page
            .screen
            .focus
            .is_some_and(|f| page.screen.layout.widgets[f].name == "XButton")
    {
        page.screen.focus = None;
    }
    changed
}

/// Use profile `name`: its saves from now on, and remembered in the
/// settings for the next start.
fn load_profile(
    name: &str,
    profile: &mut Profile,
    folder: &mut rc_save::SaveFolder,
    settings: &mut Settings,
    path: &SettingsPath,
    log: &GameLog,
) {
    profile.name = Some(name.to_string());
    folder.0 = profile.saves();
    settings.save.profile = name.to_string();
    match settings.save(&path.0) {
        Ok(()) => log.line(format!(
            "profiles: now {name:?}, saves in {}; remembered in {}",
            folder.0.display(),
            path.0.display()
        )),
        Err(e) => log.line(format!(
            "profiles: now {name:?}, but the settings can't be saved: {e}"
        )),
    }
}

/// Open the list page by itself while no profile is loaded, and do the
/// work the pages' buttons and the question box asked for.
#[allow(clippy::too_many_arguments)]
pub fn apply_profiles(
    mut menus: ResMut<OriginalMenus>,
    mut profile: ResMut<Profile>,
    mut folder: ResMut<rc_save::SaveFolder>,
    mut settings: ResMut<Settings>,
    path: Res<SettingsPath>,
    mut menu: ResMut<Menu>,
    state: Res<State<GameState>>,
    time: Res<Time>,
    log: Res<GameLog>,
) {
    if !menus.pages.contains_key(PROFILES) {
        menus.profiles.asks.clear();
        return;
    }
    if menus.profiles.current != profile.name {
        menus.profiles.current = profile.name.clone();
    }
    // The original's main menu opens the list when no profile is loaded.
    if *state.get() == GameState::MainMenu
        && menu.page == Page::Main
        && profile.name.is_none()
        && menus.active()
        && menus.question.is_none()
    {
        log.line("original menus: no profile loaded; opening the profiles page");
        menu.go(Page::Profiles);
    }
    // The message after loading or making a profile, then the main menu.
    if let Some(left) = menus.profiles.holding {
        let left = left - time.delta_secs();
        if left > 0.0 {
            menus.profiles.holding = Some(left);
            return;
        }
        menus.profiles.holding = None;
        menus.close_box();
        log.line("original menus: profile loaded; back to the main menu");
        while menu.page != Page::Main && menu.back() {}
        return;
    }
    if menus.profiles.stale {
        menus.profiles.stale = false;
        fill(&mut menus, &profile.root);
        menus.mark_dirty();
    }
    if show_delete(&mut menus) {
        menus.mark_dirty();
    }
    // Blink the name box's cursor.
    if let Some(p) = menus.pages.get_mut(NEW_PROFILE) {
        if let Some(f) = p.screen.focus {
            if let Kind::Button(b) = &mut p.screen.layout.widgets[f].kind {
                if let Some(e) = &mut b.edit {
                    if e.tick(time.delta_secs()) {
                        menus.mark_dirty();
                    }
                }
            }
        }
    }
    let asks = std::mem::take(&mut menus.profiles.asks);
    for ask in asks {
        match ask {
            ProfileAsk::Select => {
                let Some(item) = selected(&menus) else {
                    log.line("original menus: profiles: SELECT with nothing chosen");
                    continue;
                };
                menus.profiles.last_focus = item;
                if item == 0 {
                    // "< NEW >": the name page in this one's place.
                    log.line("original menus: profiles: < NEW >");
                    menu.switch(Page::NewProfile);
                    continue;
                }
                let Some(name) = menus.profiles.names.get(item - 1).cloned() else {
                    continue;
                };
                load_profile(&name, &mut profile, &mut folder, &mut settings, &path, &log);
                menus.profiles.current = Some(name.clone());
                let text = loading_message(&menus.profiles.texts, false, &name.to_uppercase());
                hold_message(&mut menus, &text, PROFILES, &log);
            }
            ProfileAsk::Delete => {
                let Some(item) = selected(&menus).filter(|i| *i > 0) else {
                    continue;
                };
                let Some(name) = menus.profiles.names.get(item - 1).cloned() else {
                    continue;
                };
                menus.profiles.last_focus = item;
                let in_use = profile
                    .name
                    .as_ref()
                    .is_some_and(|c| c.eq_ignore_ascii_case(&name));
                if in_use {
                    let t = menus.profiles.texts.active_delete.clone();
                    warn(&mut menus, &t, PROFILES, &log);
                } else {
                    let (a, b) = &menus.profiles.texts.delete_confirm;
                    let q = format!("{a}{}{b}", name.to_uppercase());
                    log.line(format!("original menus: asking {q:?}"));
                    menus.ask(&q, Asking::DeleteProfile(name));
                }
            }
            ProfileAsk::DoDelete(name) => {
                match profiles::delete(&profile.root, &name) {
                    Ok(()) => log.line(format!("profiles: deleted {name:?} and its saves")),
                    Err(e) => log.line(format!("profiles: ERROR {e}")),
                }
                menus.profiles.stale = true;
            }
            ProfileAsk::Create => {
                let typed = menus
                    .pages
                    .get(NEW_PROFILE)
                    .and_then(|p| p.screen.layout.widget(NAME_BOX))
                    .and_then(|w| match &w.kind {
                        Kind::Button(b) => Some(b.blurred.text.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                let names = profiles::list(&profile.root);
                match check_new_name(&typed, &names, &menus.profiles.texts) {
                    Err(w) => warn(&mut menus, &w, NEW_PROFILE, &log),
                    Ok(name) => match profiles::create(&profile.root, &name) {
                        Ok(dir) => {
                            log.line(format!("profiles: made {name:?} ({})", dir.display()));
                            load_profile(
                                &name,
                                &mut profile,
                                &mut folder,
                                &mut settings,
                                &path,
                                &log,
                            );
                            menus.profiles.current = Some(name.clone());
                            let text = loading_message(&menus.profiles.texts, true, &name);
                            hold_message(&mut menus, &text, NEW_PROFILE, &log);
                        }
                        Err(e) => log.line(format!("profiles: ERROR {e}")),
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_names_are_checked_as_the_original_does() {
        let t = Texts {
            prefix: "Profile".into(),
            in_use: "THERE IS ALREADY A PROFILE NAMED %s".into(),
            invalid_name: "SORRY, YOU CANNOT CREATE A PROFILE NAMED %s".into(),
            invalid_start: "SORRY, YOU CANNOT CREATE A PROFILE BEGINNING WITH %s".into(),
            ..Default::default()
        };
        let names = vec!["Matt".to_string()];
        assert_eq!(check_new_name("Boss 2", &names, &t), Ok("Boss_2".into()));
        assert_eq!(
            check_new_name("matt", &names, &t),
            Err("THERE IS ALREADY A PROFILE NAMED matt".into())
        );
        assert_eq!(
            check_new_name("PROFILE", &names, &t),
            Err("SORRY, YOU CANNOT CREATE A PROFILE NAMED Profile".into())
        );
        assert_eq!(
            check_new_name("profile x", &names, &t),
            Err("SORRY, YOU CANNOT CREATE A PROFILE BEGINNING WITH Profile_".into())
        );
        assert_eq!(check_new_name("", &names, &t), Err(t.min_len.clone()));
    }

    #[test]
    fn the_loading_message_names_the_profile() {
        let t = Texts::default();
        assert_eq!(loading_message(&t, false, "MATT"), "LOADING PROFILE: MATT");
        assert_eq!(
            loading_message(&t, true, "Boss_2"),
            "CREATING PROFILE: Boss_2"
        );
    }
}
