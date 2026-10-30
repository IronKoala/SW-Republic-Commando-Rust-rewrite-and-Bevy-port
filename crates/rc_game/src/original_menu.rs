//! The original game's menus, drawn from the player's install (see
//! `rc_menus` and `docs/ORIGINAL_MENUS.md`). The main menu replaces the
//! placeholder main page in `menu.rs`. The other pages (pause, campaign,
//! the four options pages) use the original menu named for them in
//! `data/ui/original_menus.toml` when every widget and button on it can
//! work here (`rc_menus::pages::check`); otherwise the placeholder page
//! stays and the log says why.
//!
//! The original lays menus out for a 4:3 screen. We draw them in a centred
//! 4:3 area, as large as the window allows, with black on either side
//! (or above and below on a tall phone screen), so nothing is stretched.
//!
//! Every picture and letter is a Bevy UI image node placed in percent of
//! that area, so the menu follows the window when it is resized. Menu
//! moves come from the same actions as the rest of the game (`MenuUp`,
//! `MenuAccept`...), so keyboard, gamepad and touch all work; the mouse
//! picks a button by pointing at it.
#![allow(deprecated)]

use crate::input::{BaseBindings, Bindings};
use crate::menu::{GamePathsFound, Menu, Page};
use crate::option_settings;
use crate::rebind::{RebindDone, StartRebind, UserBindingsPath};
use crate::settings::Settings;
use crate::settings::SettingsPath;
use bevy::asset::RenderAssetUsages;
use bevy::audio::{AudioSource, PlaybackSettings};
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use rc_game_core::{
    Action, ActionState, CurrentLevel, GameLog, GamePaths, GameSet, GameState, InputDevice,
    InputPrompts, LoadingGate, Play,
};
use rc_menus::gpu::{self, Variant};
use rc_menus::layout::{Kind, MenuLayout};
use rc_menus::pages::{self, PageButtons, PageCmd, PagePlan};
use rc_menus::quads::{self, Quad, Sizes};
use rc_menus::screens::{self, MainAction};
use rc_menus::value::V;
use std::collections::HashMap;

/// The original's menu shape.
const MENU_ASPECT: f32 = 4.0 / 3.0;

/// Main menu buttons this port can't offer yet; they are greyed out the way
/// the original greys out buttons in its demo versions. Multiplayer stays
/// off for good: this port is single-player and offline only.
const UNAVAILABLE: &[MainAction] = &[
    MainAction::Multiplayer,
    MainAction::LoadGame,
    MainAction::Profiles,
    MainAction::Extras,
];

/// One original screen ready to draw.
pub struct Screen {
    pub layout: MenuLayout,
    /// Index into `layout.widgets` of the button with focus.
    pub focus: Option<usize>,
}

impl Screen {
    /// The buttons keys and controllers move between.
    fn buttons(&self) -> Vec<usize> {
        self.layout
            .focusable_on_pc()
            .into_iter()
            .filter(|&i| match &self.layout.widgets[i].kind {
                Kind::Button(b) => !b.disabled,
                _ => false,
            })
            .collect()
    }

    /// The buttons the mouse can use (also the arrows beside choosers).
    fn clickable(&self) -> Vec<usize> {
        self.layout
            .buttons_on_pc()
            .into_iter()
            .filter(|&i| match &self.layout.widgets[i].kind {
                Kind::Button(b) => !b.disabled,
                _ => false,
            })
            .collect()
    }

    /// The focused widget is a chooser (left and right change its value).
    fn focus_is_chooser(&self) -> bool {
        self.focus.is_some_and(
            |f| matches!(&self.layout.widgets[f].kind, Kind::Button(b) if b.is_chooser()),
        )
    }

    /// The names of the buttons keys and controllers can reach.
    pub fn reachable_names(&self) -> Vec<String> {
        self.buttons()
            .into_iter()
            .map(|i| self.layout.widgets[i].name.clone())
            .collect()
    }

    pub fn focused_name(&self) -> Option<&str> {
        self.focus.map(|i| self.layout.widgets[i].name.as_str())
    }

    pub(crate) fn on_select(&self, i: usize) -> String {
        match &self.layout.widgets[i].kind {
            Kind::Button(b) => b.on_select.clone(),
            _ => String::new(),
        }
    }

    /// Move focus `step` buttons along (skipping disabled ones).
    fn step(&mut self, step: i32) -> bool {
        let list = self.buttons();
        if list.is_empty() {
            return false;
        }
        let at = self
            .focus
            .and_then(|f| list.iter().position(|&b| b == f))
            .unwrap_or(0) as i32;
        let next = list[(at + step).rem_euclid(list.len() as i32) as usize];
        let changed = self.focus != Some(next);
        self.focus = Some(next);
        changed
    }
}

/// A page behind the main menu drawn from its original menu.
pub struct PageScreen {
    pub screen: Screen,
    pub plan: PagePlan,
    pub buttons: PageButtons,
}

/// Which original screen is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shown {
    Main,
    /// A page by its name in `original_menus.toml` (`paused`...).
    Page(String),
}

/// The page's name in `original_menus.toml`.
pub fn page_key(page: Page) -> Option<&'static str> {
    match page {
        Page::Campaigns => Some("campaigns"),
        Page::Options => Some("options"),
        Page::OptionsSound => Some("options_sound"),
        Page::OptionsGraphics => Some("options_graphics"),
        Page::OptionsControls => Some("options_controls"),
        Page::Paused => Some("paused"),
        // The port's own settings, drawn in the original GAME page's look.
        Page::MoreSettings => Some("more_settings"),
        Page::Main | Page::Levels(_) | Page::NoGame => None,
    }
}

fn page_from_key(key: &str) -> Option<Page> {
    match key {
        "campaigns" => Some(Page::Campaigns),
        "options" => Some(Page::Options),
        "options_sound" => Some(Page::OptionsSound),
        "options_graphics" => Some(Page::OptionsGraphics),
        "options_controls" => Some(Page::OptionsControls),
        "more_settings" => Some(Page::MoreSettings),
        "paused" => Some(Page::Paused),
        _ => None,
    }
}

/// The original "reassign button" box over the controls page.
pub struct RebindBox {
    pub screen: Screen,
    pub action: Action,
    pub device: InputDevice,
    /// The bindings before, for CANCEL.
    before: Bindings,
    /// A key has been pressed; ACCEPT, CANCEL and CLEAR show.
    pub chosen: bool,
}

/// A drop-down list open under a chooser (the screen size).
pub struct ListBox {
    pub screen: Screen,
    /// The chooser it belongs to (widget index on the page).
    pub chooser: usize,
    pub items: Vec<String>,
    /// The first value shown (the list scrolls).
    pub first: usize,
}

impl ListBox {
    /// The value with focus, as an index into `items`.
    pub fn focused_item(&self) -> Option<usize> {
        let name = self.screen.focused_name()?;
        let n: usize = name
            .strip_prefix("ListItems[")?
            .strip_suffix(']')?
            .parse()
            .ok()?;
        Some(self.first + n)
    }
}

/// What the options pages asked for this frame, done by `apply_options`
/// (which has the settings and bindings).
#[derive(Debug, Clone, PartialEq)]
enum OptionWork {
    /// Step a chooser (widget index) left or right.
    Step(usize, i32),
    RestoreDefaults,
    /// Open the "reassign button" box for a key button (widget index).
    Rebind(usize),
    /// Set a chooser (widget index) to one of its values, from its list.
    Pick(usize, usize),
    RebindAccept,
    RebindCancel,
    /// CLEAR: forget the key pressed and wait for another.
    RebindAgain,
}

/// The original menus, when the install has them.
#[derive(Resource, Default)]
pub struct OriginalMenus {
    /// Tried to load them (only once).
    pub tried: bool,
    pub main: Option<Screen>,
    /// Pages behind the main menu that use their original menu, by name.
    pub pages: HashMap<String, PageScreen>,
    /// The "are you sure?" box over the screen showing.
    pub question: Option<Screen>,
    /// The question box's layout, kept for when QUIT is chosen again.
    question_layout: Option<MenuLayout>,
    /// The "reassign button" box while it shows, and its layout.
    pub rebind: Option<RebindBox>,
    map_key_layout: Option<MenuLayout>,
    /// The drop-down list while it shows.
    pub list: Option<ListBox>,
    /// Options work for `apply_options`.
    work: Vec<OptionWork>,
    quit_text: String,
    pub assets: Option<rc_menus::Assets>,
    /// Bevy images by texture path (lower case), render style and tint.
    images: HashMap<(String, Variant, [u8; 4]), Handle<Image>>,
    sounds: HashMap<String, Option<Handle<AudioSource>>>,
    focus_sound: Option<String>,
    select_sound: Option<String>,
    /// True when the drawing needs redoing.
    dirty: bool,
    /// The quads last drawn, for the script's colour check.
    pub drawn: Vec<Quad>,
    /// This frame's button press was used here (for example to open the
    /// campaign page), so the placeholder pages must not act on it too.
    pub used_input: bool,
}

impl OriginalMenus {
    /// The main menu is ours to draw and handle.
    pub fn active(&self) -> bool {
        self.main.is_some()
    }

    /// The original screen to show now, if any (else the placeholder
    /// page shows).
    pub fn shown(&self, state: &GameState, play: Option<&Play>, menu: &Menu) -> Option<Shown> {
        if *state == GameState::MainMenu && menu.page == Page::Main && self.active() {
            return Some(Shown::Main);
        }
        if *state != GameState::MainMenu && play != Some(&Play::Paused) {
            return None;
        }
        let key = page_key(menu.page)?;
        self.pages
            .contains_key(key)
            .then(|| Shown::Page(key.to_string()))
    }

    fn screen(&self, shown: &Shown) -> Option<&Screen> {
        match shown {
            Shown::Main => self.main.as_ref(),
            Shown::Page(k) => self.pages.get(k).map(|p| &p.screen),
        }
    }

    fn screen_mut(&mut self, shown: &Shown) -> Option<&mut Screen> {
        match shown {
            Shown::Main => self.main.as_mut(),
            Shown::Page(k) => self.pages.get_mut(k).map(|p| &mut p.screen),
        }
    }
}

/// Read the page list: the first copy found in the data folders (so a mod
/// wins), else the one built into the game.
fn page_plans(paths: &GamePaths, log: &GameLog) -> Vec<PagePlan> {
    const REL: &str = "ui/original_menus.toml";
    let text = match paths.data_file(REL) {
        Some(p) => match std::fs::read_to_string(&p) {
            Ok(t) => t,
            Err(e) => {
                log.line(format!("original menus: can't read {}: {e}", p.display()));
                include_str!("../../../data/ui/original_menus.toml").to_string()
            }
        },
        None => include_str!("../../../data/ui/original_menus.toml").to_string(),
    };
    pages::parse_plans(&text).unwrap_or_else(|e| {
        log.line(format!("original menus: {REL}: {e}; no original pages"));
        Vec::new()
    })
}

pub struct OriginalMenuPlugin;

impl Plugin for OriginalMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OriginalMenus>()
            .add_systems(OnEnter(GameState::MainMenu), (load, reset).chain())
            .add_systems(OnEnter(Play::Paused), load)
            .add_systems(
                Update,
                (focus_new_page, navigate, apply_options, draw, fit_area)
                    .chain()
                    .in_set(GameSet::Hud)
                    // Before the placeholder menu reads the same actions.
                    .before(crate::menu::MenuSystems),
            );
    }
}

fn load(
    mut menus: ResMut<OriginalMenus>,
    paths: Res<GamePaths>,
    found: Res<GamePathsFound>,
    log: Res<GameLog>,
) {
    if menus.tried {
        return;
    }
    menus.tried = true;
    if !found.0 {
        log.line("original menus: no game folder, using the placeholder menus");
        return;
    }
    let t = std::time::Instant::now();
    let mut assets = rc_menus::Assets::new(rc_menus::Library::new(&paths.game_data()));
    let main = match assets.library.class(screens::MAIN_MENU) {
        Ok(c) => c,
        Err(e) => {
            log.line(format!(
                "original menus: can't read {}: {e}; using the placeholder menus",
                screens::MAIN_MENU
            ));
            return;
        }
    };
    let mut paths_needed = Vec::new();
    for plan in page_plans(&paths, &log) {
        let class = match assets.library.class(&plan.class) {
            Ok(c) => c,
            Err(e) => {
                log.line(format!(
                    "original menus: page {} kept the placeholder: can't read {}: {e}",
                    plan.page, plan.class
                ));
                continue;
            }
        };
        let mut layout = rc_menus::build(&class);
        for w in &layout.widgets {
            if let Kind::Button(b) = &w.kind {
                if w.shown_on_pc() {
                    log.line(format!(
                        "original menus:   {} button {} calls {:?}",
                        plan.class, w.name, b.on_select
                    ));
                }
            }
        }
        match pages::check(&mut layout, &plan) {
            Ok(mut buttons) => {
                check_names(&mut layout, &mut buttons, &plan.page, &log);
                log.line(format!(
                    "original menus: page {} uses {} ({} buttons)",
                    plan.page,
                    plan.class,
                    buttons.commands.len()
                ));
                paths_needed.extend(assets.textures_of(&layout));
                menus.pages.insert(
                    plan.page.clone(),
                    PageScreen {
                        screen: Screen {
                            layout,
                            focus: None,
                        },
                        plan,
                        buttons,
                    },
                );
            }
            Err(why) => {
                for w in why {
                    log.line(format!(
                        "original menus: page {} kept the placeholder: {w}",
                        plan.page
                    ));
                }
            }
        }
    }
    let quit_text = match main.get("QuitGameConfirm") {
        Some(V::Str(s)) => s.clone(),
        _ => "Quit?".into(),
    };
    let mut layout = rc_menus::build(&main);
    for l in screens::setup_main_menu(&mut layout, UNAVAILABLE) {
        log.line(format!("original menus: {l}"));
    }
    let map_key_layout = if menus.pages.values().any(|p| !p.buttons.keys.is_empty()) {
        match assets.library.class(screens::MAP_KEY) {
            Ok(c) => Some(rc_menus::build(&c)),
            Err(e) => {
                log.line(format!(
                    "original menus: no reassign button box ({e}); keys can't be changed here"
                ));
                None
            }
        }
    } else {
        None
    };
    let question_layout = match assets.library.class(screens::QUESTION) {
        Ok(c) => Some(rc_menus::build(&c)),
        Err(e) => {
            log.line(format!(
                "original menus: no question box ({e}); QUIT quits at once"
            ));
            None
        }
    };
    // Load every picture up front so the menu never waits.
    paths_needed.extend(assets.textures_of(&layout));
    if let Some(q) = &question_layout {
        paths_needed.extend(assets.textures_of(q));
    }
    if let Some(m) = &map_key_layout {
        paths_needed.extend(assets.textures_of(m));
    }
    let mut pictures = 0;
    for p in paths_needed {
        if assets.texture(&p).is_some() {
            pictures += 1;
        }
    }
    for p in &assets.problems {
        log.line(format!("original menus: PROBLEM {p}"));
    }
    let shown = layout.widgets.iter().filter(|w| w.shown_on_pc()).count();
    log.line(format!(
        "original menus: {} loaded in {:.0} ms: {} widgets ({shown} shown), {} pictures, background video {:?} not played (not supported yet)",
        screens::MAIN_MENU,
        t.elapsed().as_secs_f64() * 1000.0,
        layout.widgets.len(),
        pictures,
        layout.background_movie
    ));
    for w in &layout.widgets {
        if w.shown_on_pc() {
            log.line(format!("original menus:   {} (pass {})", w.name, w.pass));
        }
    }
    menus.focus_sound = layout.sounds.focus.clone();
    menus.select_sound = layout.sounds.select.clone();
    menus.main = Some(Screen {
        layout,
        focus: None,
    });
    menus.question_layout = question_layout;
    menus.map_key_layout = map_key_layout;
    menus.quit_text = quit_text;
    menus.assets = Some(assets);
}

/// Grey out choosers whose setting, and key buttons whose action, this
/// game doesn't know (a typing mistake in `original_menus.toml`), and say
/// so in the log.
fn check_names(layout: &mut MenuLayout, buttons: &mut PageButtons, page: &str, log: &GameLog) {
    let mut bad = Vec::new();
    buttons.choosers.retain(|(i, name)| {
        let ok = option_settings::known(name);
        if !ok {
            bad.push((*i, format!("chooser setting {name:?} isn't a setting")));
        }
        ok
    });
    buttons.keys.retain(|(i, name)| {
        let ok = Action::from_name(name).is_some();
        if !ok {
            bad.push((*i, format!("key button action {name:?} isn't an action")));
        }
        ok
    });
    for (i, why) in bad {
        let name = layout.widgets[i].name.clone();
        log.line(format!(
            "original menus: page {page}: {name} greyed out: {why} (see original_menus.toml)"
        ));
        screens::disable_row(layout, &name);
    }
}

/// Show each chooser's setting and each key button's keys (for the device
/// used last).
fn refresh_page(p: &mut PageScreen, settings: &Settings, prompts: &InputPrompts) {
    for (i, name) in &p.buttons.choosers {
        let Kind::Button(b) = &mut p.screen.layout.widgets[*i].kind else {
            continue;
        };
        if let Some(items) = option_settings::items(name, settings) {
            b.items = items;
        }
        if let Some(at) = option_settings::read(name, settings, &b.items) {
            b.choose(at);
        }
    }
    for (i, name) in &p.buttons.keys {
        let Some(action) = Action::from_name(name) else {
            continue;
        };
        let Kind::Button(b) = &mut p.screen.layout.widgets[*i].kind else {
            continue;
        };
        let text = key_text(prompts, action);
        b.blurred.text = text.clone();
        b.focused.text = text;
    }
}

/// An action's keys as the controls page shows them: "W, UP", or "???" for
/// none (as the original does).
fn key_text(prompts: &InputPrompts, action: Action) -> String {
    let labels = prompts.labels(action);
    if labels.is_empty() {
        "???".into()
    } else {
        labels.join(", ").to_uppercase()
    }
}

/// Each time the main menu opens: focus on the first button, no question.
fn reset(mut menus: ResMut<OriginalMenus>, log: Res<GameLog>) {
    let Some(main) = menus.main.as_mut() else {
        return;
    };
    main.focus = main.buttons().first().copied();
    let name = main.focused_name().unwrap_or("none").to_string();
    menus.question = None;
    menus.dirty = true;
    log.line(format!("original menus: main menu opened, focus on {name}"));
}

/// When an original page comes on screen, focus its first button (the
/// main menu does this in `reset`).
#[allow(clippy::too_many_arguments)]
fn focus_new_page(
    state: Res<State<GameState>>,
    play: Option<Res<State<Play>>>,
    menu: Res<Menu>,
    mut menus: ResMut<OriginalMenus>,
    mut last: Local<Option<Shown>>,
    settings: Res<Settings>,
    prompts: Res<InputPrompts>,
    log: Res<GameLog>,
) {
    let shown = menus.shown(state.get(), play.as_deref().map(|p| p.get()), &menu);
    if shown == *last {
        // The controls page follows the device used last.
        if prompts.is_changed() {
            if let Some(Shown::Page(k)) = &shown {
                let p = menus.pages.get_mut(k).unwrap();
                if !p.buttons.keys.is_empty() {
                    refresh_page(p, &settings, &prompts);
                    menus.dirty = true;
                }
            }
        }
        return;
    }
    *last = shown.clone();
    menus.question = None;
    menus.rebind = None;
    menus.list = None;
    menus.dirty = true;
    if let Some(Shown::Page(k)) = &shown {
        let p = menus.pages.get_mut(k).unwrap();
        refresh_page(p, &settings, &prompts);
        let screen = &mut p.screen;
        screen.focus = screen.buttons().first().copied();
        log.line(format!(
            "original menus: page {k} opened, focus on {}",
            screen.focused_name().unwrap_or("none")
        ));
    }
}

#[derive(Component)]
struct MenuArea;

#[derive(Component)]
struct OriginalRoot;

/// A mouse target for one button: (is on a box over the page, such as the
/// question box, widget index).
#[derive(Component)]
struct HitBox(bool, usize);

fn play_sound(
    commands: &mut Commands,
    menus: &mut OriginalMenus,
    sources: &mut Assets<AudioSource>,
    path: Option<String>,
    log: &GameLog,
) {
    let Some(path) = path else {
        return;
    };
    let key = path.to_ascii_lowercase();
    if !menus.sounds.contains_key(&key) {
        let handle = match menus.assets.as_mut().map(|a| a.library.sound(&path)) {
            Some(Ok(bytes)) => Some(sources.add(AudioSource {
                bytes: bytes.into(),
            })),
            Some(Err(e)) => {
                log.line(format!("original menus: sound {path}: {e}"));
                None
            }
            None => None,
        };
        menus.sounds.insert(key.clone(), handle);
    }
    if let Some(Some(h)) = menus.sounds.get(&key) {
        // Turned down by the SOUND FX volume in music.rs, as every sound is.
        commands.spawn((AudioPlayer(h.clone()), PlaybackSettings::DESPAWN));
    }
}

/// What the drop-down list did with this frame's input.
enum ListInput {
    Nothing,
    Moved,
    Close,
    /// A value was chosen (index into the list's items).
    Pick(usize),
}

/// Open the drop-down list under chooser `i` of page `k`, focused on the
/// value chosen now.
fn open_list(menus: &mut OriginalMenus, k: &str, i: usize, log: &GameLog) {
    let page = &menus.pages[k];
    let Kind::Button(b) = &page.screen.layout.widgets[i].kind else {
        return;
    };
    let (items, current) = (b.items.clone(), b.current);
    let mut l = ListBox {
        screen: Screen {
            layout: page.screen.layout.clone(),
            focus: None,
        },
        chooser: i,
        items,
        first: 0,
    };
    if !show_list(&page.screen.layout, &mut l, current) {
        log.line(format!(
            "original menus: page {k}: no drop-down list for {}",
            page.screen.layout.widgets[i].name
        ));
        return;
    }
    log.line(format!(
        "original menus: page {k}: drop-down list of {} values opened on {}",
        l.items.len(),
        l.items[current.min(l.items.len() - 1)]
    ));
    menus.list = Some(l);
    menus.dirty = true;
}

/// Lay the list out with value `focus` shown and focused, scrolling only
/// as far as needed. False if the page can't make a list.
fn show_list(page: &MenuLayout, l: &mut ListBox, focus: usize) -> bool {
    let focus = focus.min(l.items.len().saturating_sub(1));
    let rows = screens::LIST_ROWS;
    if focus < l.first {
        l.first = focus;
    } else if focus >= l.first + rows {
        l.first = focus + 1 - rows;
    }
    l.first = l.first.min(l.items.len().saturating_sub(rows));
    let name = &page.widgets[l.chooser].name;
    let Some(layout) = screens::drop_down(page, name, &l.items, l.first) else {
        return false;
    };
    let row = format!("ListItems[{}]", focus - l.first);
    l.screen.focus = layout.widgets.iter().position(|w| w.name == row);
    l.screen.layout = layout;
    true
}

/// Keys, controller, mouse and wheel in the drop-down list.
fn list_input(
    menus: &mut OriginalMenus,
    k: &str,
    actions: &ActionState,
    hits: &Query<(&Interaction, &HitBox), Changed<Interaction>>,
    mouse_moved: bool,
    wheel_y: f32,
) -> ListInput {
    let page = &menus.pages[k].screen.layout;
    let l = menus.list.as_mut().unwrap();
    let Some(now) = l.focused_item() else {
        return ListInput::Close;
    };
    if actions.just_pressed(Action::MenuBack) {
        return ListInput::Close;
    }
    if actions.just_pressed(Action::MenuAccept) {
        return ListInput::Pick(now);
    }
    let last = l.items.len().saturating_sub(1);
    let mut to = None;
    if actions.just_pressed(Action::MenuDown) {
        to = Some((now + 1).min(last));
    }
    if actions.just_pressed(Action::MenuUp) {
        to = Some(now.saturating_sub(1));
    }
    if wheel_y != 0.0 {
        // The wheel scrolls the list and the focus with it.
        let by = if wheel_y > 0.0 { -1 } else { 1 };
        let first = (l.first as i32 + by)
            .clamp(0, l.items.len().saturating_sub(screens::LIST_ROWS) as i32)
            as usize;
        let at = now as i32 - l.first as i32;
        l.first = first;
        to = Some((first as i32 + at).clamp(0, last as i32) as usize);
    }
    // Every mouse area under the pointer gets the press (the clear one over
    // the whole screen too), so a row wins over "outside".
    let mut outside = false;
    for (i, hit) in hits {
        if !hit.0 {
            continue;
        }
        let name = l
            .screen
            .layout
            .widgets
            .get(hit.1)
            .map(|w| w.name.clone())
            .unwrap_or_default();
        let item = name
            .strip_prefix("ListItems[")
            .and_then(|n| n.strip_suffix(']'))
            .and_then(|n| n.parse::<usize>().ok())
            .map(|n| l.first + n);
        match (i, item) {
            (Interaction::Pressed, Some(item)) => return ListInput::Pick(item),
            (Interaction::Pressed, None) if name == "ListOutside" => outside = true,
            (Interaction::Hovered, Some(item)) if mouse_moved && item != now => to = Some(item),
            _ => {}
        }
    }
    if outside {
        return ListInput::Close;
    }
    match to {
        Some(to) if to != now || wheel_y != 0.0 => {
            show_list(page, l, to);
            ListInput::Moved
        }
        _ => ListInput::Nothing,
    }
}

#[allow(clippy::too_many_arguments)]
fn navigate(
    mut commands: Commands,
    actions: Res<ActionState>,
    state: Res<State<GameState>>,
    play: Option<Res<State<Play>>>,
    mut menu: ResMut<Menu>,
    mut menus: ResMut<OriginalMenus>,
    hits: Query<(&Interaction, &HitBox), Changed<Interaction>>,
    (mut current, mut gate): (ResMut<CurrentLevel>, ResMut<LoadingGate>),
    mut next_state: ResMut<NextState<GameState>>,
    mut next_play: Option<ResMut<NextState<Play>>>,
    mut exit: MessageWriter<AppExit>,
    mut sources: ResMut<Assets<AudioSource>>,
    (mut cursor, mut wheel): (MessageReader<CursorMoved>, MessageReader<MouseWheel>),
    log: Res<GameLog>,
) {
    // Pointing at a button moves focus only when the mouse itself moves,
    // not when the menu is redrawn under a still pointer.
    let mouse_moved = cursor.read().count() > 0;
    let wheel_y: f32 = wheel.read().map(|w| w.y).sum();
    menus.used_input = false;
    let Some(shown) = menus.shown(state.get(), play.as_deref().map(|p| p.get()), &menu) else {
        return;
    };
    let menus = &mut *menus;
    // While the "reassign button" box waits for a key, the key goes to the
    // rebinding, not to the menu.
    if menus.rebind.as_ref().is_some_and(|r| !r.chosen) {
        menus.used_input = true;
        return;
    }
    // The drop-down list takes all input while it's open.
    if menus.list.is_some() {
        let Shown::Page(k) = &shown else {
            menus.list = None;
            return;
        };
        menus.used_input = true;
        let pick = list_input(menus, k, &actions, &hits, mouse_moved, wheel_y);
        match pick {
            ListInput::Nothing => {}
            ListInput::Moved => {
                menus.dirty = true;
                let s = menus.focus_sound.clone();
                play_sound(&mut commands, menus, &mut sources, s, &log);
            }
            ListInput::Close => {
                menus.list = None;
                menus.dirty = true;
            }
            ListInput::Pick(item) => {
                let l = menus.list.take().unwrap();
                menus.work.push(OptionWork::Pick(l.chooser, item));
                menus.dirty = true;
                let s = menus.select_sound.clone();
                play_sound(&mut commands, menus, &mut sources, s, &log);
            }
        }
        return;
    }
    let in_question = menus.question.is_some();
    let in_rebind = menus.rebind.is_some();
    let in_overlay = in_question || in_rebind;
    let mut chosen = None;
    let mut moved = false;
    // A chooser stepped by left or right: (widget, step).
    let mut stepped = None;
    {
        let screen = if in_question {
            menus.question.as_mut().unwrap()
        } else if in_rebind {
            &mut menus.rebind.as_mut().unwrap().screen
        } else {
            menus.screen_mut(&shown).unwrap()
        };
        let chooser = screen.focus_is_chooser();
        if actions.just_pressed(Action::MenuDown) {
            moved |= screen.step(1);
        }
        if actions.just_pressed(Action::MenuUp) {
            moved |= screen.step(-1);
        }
        if actions.just_pressed(Action::MenuRight) {
            if chooser {
                stepped = screen.focus.map(|f| (f, 1));
            } else {
                moved |= screen.step(1);
            }
        }
        if actions.just_pressed(Action::MenuLeft) {
            if chooser {
                stepped = screen.focus.map(|f| (f, -1));
            } else {
                moved |= screen.step(-1);
            }
        }
        for (i, hit) in &hits {
            if hit.0 != in_overlay {
                continue;
            }
            match i {
                Interaction::Hovered | Interaction::Pressed => {
                    let pointing = mouse_moved || *i == Interaction::Pressed;
                    if pointing && screen.focus != Some(hit.1) && screen.buttons().contains(&hit.1)
                    {
                        screen.focus = Some(hit.1);
                        moved = true;
                    }
                    if *i == Interaction::Pressed && screen.clickable().contains(&hit.1) {
                        chosen = Some(hit.1);
                    }
                }
                Interaction::None => {}
            }
        }
        if actions.just_pressed(Action::MenuAccept) {
            chosen = chosen.or(screen.focus);
        }
        if moved {
            log.line(format!(
                "original menus: focus on {}",
                screen.focused_name().unwrap_or("none")
            ));
        }
    }
    if moved {
        menus.dirty = true;
        let s = menus.focus_sound.clone();
        play_sound(&mut commands, menus, &mut sources, s, &log);
    }
    let back = actions.just_pressed(Action::MenuBack);
    if in_question {
        let q = menus.question.as_ref().unwrap();
        let answer = match chosen {
            Some(i) => Some(q.on_select(i)),
            None if back => Some("OnBButton".into()),
            None => None,
        };
        let Some(answer) = answer else {
            return;
        };
        menus.used_input = true;
        let s = menus.select_sound.clone();
        play_sound(&mut commands, menus, &mut sources, s, &log);
        // In the question box, A is YES and B is NO (MenuQuestionYesNo).
        if answer == "OnAButton" {
            log.line("original menus: quit question answered YES; quitting");
            exit.write(AppExit::Success);
        } else {
            log.line("original menus: quit question answered NO");
            menus.question = None;
            menus.dirty = true;
        }
        return;
    }
    if in_rebind {
        // The key has been pressed: ACCEPT (A) keeps it, CANCEL (B, or
        // back) puts the old keys back, CLEAR (X) waits for another key.
        let r = menus.rebind.as_ref().unwrap();
        let answer = match chosen {
            Some(i) => Some(r.screen.on_select(i)),
            None if back => Some("OnBButton".into()),
            None => None,
        };
        let Some(answer) = answer else {
            return;
        };
        menus.used_input = true;
        let s = menus.select_sound.clone();
        play_sound(&mut commands, menus, &mut sources, s, &log);
        menus.work.push(match answer.as_str() {
            "OnAButton" => OptionWork::RebindAccept,
            "OnXButton" => OptionWork::RebindAgain,
            _ => OptionWork::RebindCancel,
        });
        return;
    }
    if let Some((w, step)) = stepped {
        menus.used_input = true;
        menus.work.push(OptionWork::Step(w, step));
        return;
    }
    let cmd = match &shown {
        Shown::Main => {
            let Some(i) = chosen else {
                return;
            };
            let main = menus.main.as_ref().unwrap();
            let name = main.layout.widgets[i].name.clone();
            let action = screens::main_action(&main.on_select(i));
            log.line(format!("original menus: chose {name}: {action:?}"));
            match action {
                // The original opens the prologue map, PRO.
                MainAction::NewGame => PageCmd::Play("pro".into()),
                MainAction::CampaignMap => PageCmd::Open("campaigns".into()),
                MainAction::Options => PageCmd::Open("options".into()),
                MainAction::Quit => PageCmd::QuitGame,
                other => {
                    log.line(format!("original menus: {other:?} isn't available yet"));
                    return;
                }
            }
        }
        Shown::Page(k) => {
            let p = &menus.pages[k];
            // Only the arrows (or left and right) change a chooser;
            // choosing the screen size's value opens its drop-down list.
            if let Some(i) = chosen.filter(|i| p.buttons.chooser(*i).is_some()) {
                menus.used_input = true;
                if p.buttons.chooser(i) == Some(option_settings::LIST_SETTING) {
                    let s = menus.select_sound.clone();
                    play_sound(&mut commands, menus, &mut sources, s, &log);
                    open_list(menus, k, i, &log);
                }
                return;
            }
            if let Some(i) = chosen.filter(|i| p.buttons.key(*i).is_some()) {
                menus.used_input = true;
                let s = menus.select_sound.clone();
                play_sound(&mut commands, menus, &mut sources, s, &log);
                menus.work.push(OptionWork::Rebind(i));
                return;
            }
            let cmd = match chosen {
                Some(i) => p.buttons.command(i).cloned(),
                None if back => Some(p.plan.back.clone().unwrap_or(PageCmd::Back)),
                None => None,
            };
            let Some(cmd) = cmd else {
                return;
            };
            let name = chosen.map_or("back".to_string(), |i| {
                p.screen.layout.widgets[i].name.clone()
            });
            log.line(format!("original menus: page {k}: chose {name}: {cmd:?}"));
            cmd
        }
    };
    menus.used_input = true;
    let s = menus.select_sound.clone();
    play_sound(&mut commands, menus, &mut sources, s, &log);
    let resume = |next_play: &mut Option<ResMut<NextState<Play>>>| {
        if let Some(n) = next_play.as_mut() {
            n.set(Play::Running);
        }
    };
    match cmd {
        PageCmd::Play(map) => {
            log.line(format!("menu: loading map {map}"));
            menu.notice = None;
            current.map = Some(map);
            gate.reset();
            next_state.set(GameState::Loading);
        }
        PageCmd::Restart => match current.map.clone() {
            Some(map) => {
                log.line(format!("menu: restarting map {map}"));
                gate.reset();
                next_state.set(GameState::Loading);
            }
            None => log.line("original menus: no level to restart"),
        },
        PageCmd::Open(key) => match page_from_key(&key) {
            Some(p) => menu.go(p),
            None => log.line(format!("original menus: no page called {key:?}")),
        },
        PageCmd::Switch(key) => match page_from_key(&key) {
            Some(p) => menu.switch(p),
            None => log.line(format!("original menus: no page called {key:?}")),
        },
        PageCmd::Back => {
            if !menu.back() && menu.page == Page::Paused {
                resume(&mut next_play);
            }
        }
        PageCmd::Resume => resume(&mut next_play),
        PageCmd::QuitToMenu => next_state.set(GameState::MainMenu),
        PageCmd::QuitGame => match &menus.question_layout {
            Some(l) => {
                let mut layout = l.clone();
                let under = menus.screen(&shown).map(|s| &s.layout);
                let focus = screens::setup_question(&mut layout, &menus.quit_text, under);
                menus.question = Some(Screen { layout, focus });
                menus.dirty = true;
                log.line(format!(
                    "original menus: question box: {:?}",
                    menus.quit_text
                ));
            }
            None => {
                exit.write(AppExit::Success);
            }
        },
        PageCmd::OptionLeft | PageCmd::OptionRight => {
            let step = if cmd == PageCmd::OptionLeft { -1 } else { 1 };
            if let (Some(arrow), Some(screen)) = (chosen, menus.screen_mut(&shown)) {
                if let Some(c) = screens::chooser_for(&screen.layout, arrow) {
                    screen.focus = Some(c);
                    menus.work.push(OptionWork::Step(c, step));
                }
            }
        }
        PageCmd::RestoreDefaults => menus.work.push(OptionWork::RestoreDefaults),
        PageCmd::PrevSet | PageCmd::NextSet => {
            if let Some(screen) = menus.screen_mut(&shown) {
                let set = screens::current_set(&screen.layout);
                let set = if cmd == PageCmd::PrevSet {
                    set.saturating_sub(1)
                } else {
                    set + 1
                };
                screens::show_set(&mut screen.layout, set);
                // Stay on the set buttons where possible, else the first.
                let list = screen.buttons();
                let keep = screen.focus.filter(|f| list.contains(f));
                screen.focus = keep.or_else(|| {
                    let other = if cmd == PageCmd::PrevSet {
                        "NextSet"
                    } else {
                        "PrevSet"
                    };
                    list.iter()
                        .copied()
                        .find(|&i| screen.layout.widgets[i].name == other)
                        .or(list.first().copied())
                });
                log.line(format!(
                    "original menus: controls set {} of {}",
                    screens::current_set(&screen.layout) + 1,
                    screens::set_count(&screen.layout)
                ));
                menus.dirty = true;
            }
        }
        // Handled above: they belong to the row.
        PageCmd::Rebind => {}
        PageCmd::Unavailable => {}
    }
}

/// Do what the options pages asked for: change a setting (and save it,
/// as the original saves each change), put defaults back, and run the
/// "reassign button" box.
#[allow(clippy::too_many_arguments)]
fn apply_options(
    state: Res<State<GameState>>,
    play: Option<Res<State<Play>>>,
    menu: Res<Menu>,
    mut menus: ResMut<OriginalMenus>,
    mut settings: ResMut<Settings>,
    settings_path: Res<SettingsPath>,
    (mut bindings, base, user_path): (
        ResMut<Bindings>,
        Option<Res<BaseBindings>>,
        Option<Res<UserBindingsPath>>,
    ),
    (mut start, mut done): (MessageWriter<StartRebind>, MessageReader<RebindDone>),
    prompts: Res<InputPrompts>,
    log: Res<GameLog>,
) {
    let work = std::mem::take(&mut menus.work);
    let results: Vec<RebindDone> = done.read().cloned().collect();
    let Some(Shown::Page(k)) = menus.shown(state.get(), play.as_deref().map(|p| p.get()), &menu)
    else {
        return;
    };
    let menus = &mut *menus;
    let Some(page) = menus.pages.get_mut(&k) else {
        return;
    };
    let save_bindings = |b: &Bindings| {
        if let (Some(base), Some(path)) = (base.as_deref(), user_path.as_deref()) {
            crate::rebind::save_user_bindings(b, base, path, &log);
        }
    };
    let mut settings_changed = false;
    for w in work {
        match w {
            OptionWork::Step(i, step) => {
                let Some(name) = page.buttons.chooser(i).map(str::to_string) else {
                    continue;
                };
                let Kind::Button(b) = &mut page.screen.layout.widgets[i].kind else {
                    continue;
                };
                if !b.step(step) {
                    continue;
                }
                let (at, text, items) = (b.current, b.items[b.current].clone(), b.items.clone());
                if option_settings::write(&name, &mut settings, at, &items) {
                    log.line(format!("original menus: page {k}: {name} is now {text}"));
                    settings_changed = true;
                }
                menus.dirty = true;
            }
            OptionWork::Pick(i, at) => {
                let Some(name) = page.buttons.chooser(i).map(str::to_string) else {
                    continue;
                };
                let Kind::Button(b) = &mut page.screen.layout.widgets[i].kind else {
                    continue;
                };
                if at >= b.items.len() {
                    continue;
                }
                b.choose(at);
                let (text, items) = (b.items[at].clone(), b.items.clone());
                if option_settings::write(&name, &mut settings, at, &items) {
                    log.line(format!(
                        "original menus: page {k}: {name} is now {text} (from its list)"
                    ));
                    settings_changed = true;
                }
                menus.dirty = true;
            }
            OptionWork::RestoreDefaults => {
                for (_, name) in &page.buttons.choosers {
                    option_settings::restore(name, &mut settings);
                    settings_changed = true;
                }
                if !page.buttons.keys.is_empty() {
                    if let Some(base) = base.as_deref() {
                        *bindings = base.0.clone();
                        save_bindings(&bindings);
                    }
                }
                log.line(format!("original menus: page {k}: defaults restored"));
                refresh_page(page, &settings, &prompts);
                menus.dirty = true;
            }
            OptionWork::Rebind(i) => {
                let Some(action) = page.buttons.key(i).and_then(Action::from_name) else {
                    continue;
                };
                let Some(layout) = menus.map_key_layout.clone() else {
                    log.line("original menus: no reassign button box in the install");
                    continue;
                };
                let row = &page.screen.layout;
                let label = screens::row_widgets(row, &row.widgets[i].name)
                    .into_iter()
                    .find_map(|r| match &row.widgets[r].kind {
                        Kind::Text(t) => Some(t.text.to_uppercase()),
                        _ => None,
                    })
                    .unwrap_or_else(|| action.name().to_uppercase());
                let device = if prompts.device == InputDevice::Gamepad {
                    InputDevice::Gamepad
                } else {
                    InputDevice::KeyboardMouse
                };
                let mut layout = layout;
                screens::setup_map_key(
                    &mut layout,
                    &label,
                    &key_text(&prompts, action),
                    device == InputDevice::Gamepad,
                );
                log.line(format!(
                    "original menus: reassign {label} ({}) on {device:?}",
                    action.name()
                ));
                menus.rebind = Some(RebindBox {
                    screen: Screen {
                        layout,
                        focus: None,
                    },
                    action,
                    device,
                    before: bindings.clone(),
                    chosen: false,
                });
                start.write(StartRebind { action, device });
                menus.dirty = true;
            }
            OptionWork::RebindAccept => {
                if let Some(r) = menus.rebind.take() {
                    log.line(format!(
                        "original menus: kept the new key for {}",
                        r.action.name()
                    ));
                }
                menus.dirty = true;
            }
            OptionWork::RebindCancel => {
                if let Some(r) = menus.rebind.take() {
                    *bindings = r.before;
                    save_bindings(&bindings);
                    log.line(format!(
                        "original menus: {} keeps its old keys",
                        r.action.name()
                    ));
                }
                menus.dirty = true;
            }
            OptionWork::RebindAgain => {
                if let Some(r) = menus.rebind.as_mut() {
                    *bindings = r.before.clone();
                    save_bindings(&bindings);
                    r.chosen = false;
                    r.screen.focus = None;
                    screens::map_key_waiting(&mut r.screen.layout, true);
                    start.write(StartRebind {
                        action: r.action,
                        device: r.device,
                    });
                    log.line(format!(
                        "original menus: choose another key for {}",
                        r.action.name()
                    ));
                }
                menus.dirty = true;
            }
        }
    }
    for d in results {
        let Some(r) = menus.rebind.as_mut() else {
            continue;
        };
        if d.action != r.action || d.device != r.device {
            continue;
        }
        match &d.button {
            Some(name) => {
                screens::map_key_chosen(&mut r.screen.layout, &name.to_uppercase());
                r.chosen = true;
                // Focus ACCEPT, as the original's A button.
                r.screen.focus = r
                    .screen
                    .buttons()
                    .into_iter()
                    .find(|&i| r.screen.layout.widgets[i].name == "AButton");
                if !d.also_used_by.is_empty() {
                    log.line(format!(
                        "original menus: {name} is also used by {}",
                        d.also_used_by
                            .iter()
                            .map(|a| a.name())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            None => {
                // Escape, or nothing pressed in time: as CANCEL.
                log.line(format!(
                    "original menus: no new key for {}",
                    r.action.name()
                ));
                menus.rebind = None;
            }
        }
        menus.dirty = true;
    }
    // The box shows the key by its friendly name once the prompts follow
    // the new bindings, and the page's rows follow too.
    if prompts.is_changed() {
        if let Some(r) = menus.rebind.as_mut().filter(|r| r.chosen) {
            screens::map_key_chosen(&mut r.screen.layout, &key_text(&prompts, r.action));
        }
        if !page.buttons.keys.is_empty() {
            refresh_page(page, &settings, &prompts);
        }
        menus.dirty = true;
    }
    if settings_changed {
        // Re-read, so a chooser shows what the setting really became.
        refresh_page(page, &settings, &prompts);
        crate::menu::save_settings(&settings, &settings_path, &log);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    mut commands: Commands,
    state: Res<State<GameState>>,
    play: Option<Res<State<Play>>>,
    menu: Res<Menu>,
    mut menus: ResMut<OriginalMenus>,
    roots: Query<Entity, With<OriginalRoot>>,
    mut image_assets: ResMut<Assets<Image>>,
) {
    let shown = menus.shown(state.get(), play.as_deref().map(|p| p.get()), &menu);
    let Some(shown) = shown else {
        for r in &roots {
            commands.entity(r).despawn();
        }
        menus.dirty = true;
        return;
    };
    if !menus.dirty && !roots.is_empty() {
        return;
    }
    menus.dirty = false;
    for r in &roots {
        commands.entity(r).despawn();
    }
    let menus = &mut *menus;
    // The pause menu draws over the game; the rest over black.
    let backdrop = match &shown {
        Shown::Page(k) if menus.pages[k].plan.over_game => Color::NONE,
        _ => Color::BLACK,
    };
    let Some(assets) = menus.assets.as_mut() else {
        return;
    };
    // The question box replaces the screen under it (it borrows only
    // its background picture), as in the original. The "reassign button"
    // box is drawn over the controls page.
    let mut screens: Vec<(bool, &Screen)> = match &menus.question {
        Some(q) => vec![(true, q)],
        None => match &shown {
            Shown::Main => vec![(false, menus.main.as_ref().unwrap())],
            Shown::Page(k) => vec![(false, &menus.pages[k].screen)],
        },
    };
    if let (None, Some(r)) = (&menus.question, &menus.rebind) {
        screens.push((true, &r.screen));
    }
    if let (None, Some(l)) = (&menus.question, &menus.list) {
        screens.push((true, &l.screen));
    }
    let overlay = screens.len() > 1;
    // (on top?, pictures, mouse areas by widget index)
    type Drawn = (bool, Vec<Quad>, Vec<(usize, [f32; 4])>);
    let mut list: Vec<Drawn> = Vec::new();
    for (on_top, screen) in screens {
        // The page under a box shows no focus and takes no clicks.
        let under = overlay && !on_top;
        let focus = if under { None } else { screen.focus };
        let q = quads::menu_quads(&screen.layout, focus, assets);
        let boxes = if under {
            Vec::new()
        } else {
            screen
                .layout
                .buttons_on_pc()
                .into_iter()
                .filter_map(|i| match &screen.layout.widgets[i].kind {
                    Kind::Button(b) => quads::sprite_box(&b.background_blurred, assets)
                        .or_else(|| quads::text_box(&b.blurred, assets))
                        .map(|bx| (i, bx)),
                    _ => None,
                })
                .collect()
        };
        list.push((on_top, q, boxes));
    }
    menus.drawn = list.iter().flat_map(|l| l.1.clone()).collect();
    for q in &menus.drawn {
        let key = (
            q.texture.to_ascii_lowercase(),
            gpu::variant(q.render_style),
            q.color,
        );
        if menus.images.contains_key(&key) {
            continue;
        }
        if let Some(rgba) = assets.texture(&q.texture) {
            let mut pixels = rgba.pixels.clone();
            gpu::prepare(&mut pixels, key.1, key.2);
            let image = Image::new(
                Extent3d {
                    width: rgba.width as u32,
                    height: rgba.height as u32,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                pixels,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            menus.images.insert(key, image_assets.add(image));
        }
    }
    let images = &menus.images;
    commands
        .spawn((
            OriginalRoot,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(backdrop),
            GlobalZIndex(10),
        ))
        .with_children(|root| {
            root.spawn((
                MenuArea,
                Node {
                    position_type: PositionType::Absolute,
                    overflow: Overflow::clip(),
                    ..default()
                },
            ))
            .with_children(|area| {
                for (is_question, quads, boxes) in &list {
                    for q in quads {
                        let Some(image) = images.get(&(
                            q.texture.to_ascii_lowercase(),
                            gpu::variant(q.render_style),
                            q.color,
                        )) else {
                            continue;
                        };
                        area.spawn((
                            ImageNode {
                                image: image.clone(),
                                rect: Some(Rect::new(q.src[0], q.src[1], q.src[2], q.src[3])),
                                // The tint is baked into the picture (gpu.rs).
                                color: Color::WHITE,
                                // Fill the box exactly; Bevy otherwise keeps
                                // the picture's shape, which squashes the
                                // stretched middle strips to a line.
                                image_mode: NodeImageMode::Stretch,
                                ..default()
                            },
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Percent(q.dst[0] * 100.0),
                                top: Val::Percent(q.dst[1] * 100.0),
                                width: Val::Percent((q.dst[2] - q.dst[0]) * 100.0),
                                height: Val::Percent((q.dst[3] - q.dst[1]) * 100.0),
                                ..default()
                            },
                        ));
                    }
                    for (i, b) in boxes {
                        area.spawn((
                            HitBox(*is_question, *i),
                            Interaction::None,
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Percent(b[0] * 100.0),
                                top: Val::Percent(b[1] * 100.0),
                                width: Val::Percent((b[2] - b[0]) * 100.0),
                                height: Val::Percent((b[3] - b[1]) * 100.0),
                                ..default()
                            },
                        ));
                    }
                }
            });
        });
}

/// The menu area: the largest 4:3 box that fits the window, centred.
pub fn area_for(window: Vec2) -> Rect {
    let (w, h) = if window.x / window.y.max(1.0) > MENU_ASPECT {
        (window.y * MENU_ASPECT, window.y)
    } else {
        (window.x, window.x / MENU_ASPECT)
    };
    let min = (window - Vec2::new(w, h)) / 2.0;
    Rect::from_corners(min, min + Vec2::new(w, h))
}

fn fit_area(
    windows: Query<&Window, With<PrimaryWindow>>,
    mut areas: Query<&mut Node, With<MenuArea>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let r = area_for(Vec2::new(window.width(), window.height()));
    for mut n in &mut areas {
        let want = (
            Val::Px(r.min.x),
            Val::Px(r.min.y),
            Val::Px(r.width()),
            Val::Px(r.height()),
        );
        if (n.left, n.top, n.width, n.height) != want {
            n.left = want.0;
            n.top = want.1;
            n.width = want.2;
            n.height = want.3;
        }
    }
}

/// Paint what the menu shows without the GPU, at `width` x `height`, from
/// the original's own pictures (for the script's colour check).
pub fn software_picture(
    menus: &mut OriginalMenus,
    width: usize,
    height: usize,
) -> Option<rc_image::Rgba> {
    let quads = menus.drawn.clone();
    let assets = menus.assets.as_mut()?;
    // Make sure every texture's size is known (they are cached already).
    for q in &quads {
        assets.texture_size(&q.texture);
    }
    Some(rc_menus::render::render(
        &quads,
        assets,
        width,
        height,
        [0, 0, 0, 255],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_page_list_reads() {
        let plans =
            pages::parse_plans(include_str!("../../../data/ui/original_menus.toml")).unwrap();
        let paused = plans.iter().find(|p| p.page == "paused").expect("paused");
        assert!(paused.over_game);
        assert_eq!(paused.back, Some(PageCmd::Resume));
        assert_eq!(
            paused.command("restartmissionselected"),
            Some(&PageCmd::Restart)
        );
        for p in &plans {
            assert!(page_from_key(&p.page).is_some(), "unknown page {}", p.page);
        }
    }

    #[test]
    fn a_bad_command_is_reported() {
        let e =
            pages::parse_plans("[pages.paused]\nclass = \"A.B\"\nback = \"fly\"\n").unwrap_err();
        assert!(e.contains("fly"), "{e}");
    }
}
