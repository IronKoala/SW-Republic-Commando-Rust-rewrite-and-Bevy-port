//! The menus: main menu, level select, options, pause, and the loading
//! screen. The original menus (`original_menu.rs`) draw the main menu,
//! pause menu and options pages when the install has them; the pages here
//! are then used only where an original can't be (and for MORE SETTINGS,
//! this port's own extra settings: field of view, controllers, gyro).
//!
//! PLACEHOLDER: these amber menus are our own stand-ins. The goal is to use
//! the original game's menus (read from the install at run time) as much as
//! possible; see `docs/ORIGINAL_MENUS.md`. Keep the page and command logic
//! separate from the drawing so the original's look can replace it.
//!
//! Every menu is a page: a title and a list of items. Arrow keys, a gamepad
//! d-pad or the mouse pick an item; Enter, the gamepad's A button or a click
//! choose it; Escape or B goes back. On the options page, left and right
//! change the value.
//!
//! Mouse clicks use Bevy's older `Button`/`Interaction`, marked deprecated in
//! Bevy 0.20 but still working; the newer picking-based widgets need more of
//! Bevy switched on. Swap when Bevy removes them.
#![allow(deprecated)]

use crate::input::LookSettings;
use crate::levels::LevelList;
use crate::original_menu::OriginalMenus;
use crate::player::FieldOfView;
use crate::settings::{Settings, SettingsPath};
use crate::theme::Theme;
use bevy::prelude::*;
use rc_game_core::{
    Action, ActionState, CurrentLevel, GameLog, GamePaths, GameSet, GameState, LoadingGate, Play,
};

/// Items shown at once; longer lists scroll.
const VISIBLE_ITEMS: usize = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Main,
    Campaigns,
    Levels(usize),
    /// The options pages: GAME (also the placeholder options page), SOUND,
    /// GRAPHICS and CONTROLS.
    Options,
    OptionsSound,
    OptionsGraphics,
    OptionsControls,
    /// This port's own settings that the original has no page for (field
    /// of view, wide-screen view, controllers, gyro). A placeholder page,
    /// marked as one.
    MoreSettings,
    /// The game folder wasn't found.
    NoGame,
    Paused,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Go(Page),
    Play(String),
    Back,
    Resume,
    QuitToMenu,
    Quit,
    Setting(Opt),
    Nothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opt {
    MusicVolume,
    EffectsVolume,
    MouseSensitivity,
    InvertY,
    FieldOfView,
    Difficulty,
    MasterVolume,
    DisplayMode,
    Panini,
    StickLookSpeed,
    LookStickDeadZone,
    GyroMode,
    GyroSensitivity,
}

pub struct Item {
    pub label: String,
    pub cmd: Cmd,
}

/// Where the menus are.
#[derive(Resource, Debug)]
pub struct Menu {
    pub page: Page,
    pub selected: usize,
    pub scroll: usize,
    /// Pages to go back to.
    pub history: Vec<(Page, usize)>,
    /// A message shown under the title (an error, like a level that failed
    /// to load).
    pub notice: Option<String>,
    /// True when the menu needs drawing again.
    dirty: bool,
}

impl Menu {
    pub fn new(page: Page) -> Self {
        Menu {
            page,
            selected: 0,
            scroll: 0,
            history: vec![],
            notice: None,
            dirty: true,
        }
    }

    pub(crate) fn go(&mut self, page: Page) {
        self.history.push((self.page, self.selected));
        self.page = page;
        self.selected = 0;
        self.scroll = 0;
        self.dirty = true;
    }

    /// Show another page in this one's place: going back skips it (the
    /// options pages' tabs).
    pub(crate) fn switch(&mut self, page: Page) {
        self.page = page;
        self.selected = 0;
        self.scroll = 0;
        self.dirty = true;
    }

    pub(crate) fn back(&mut self) -> bool {
        match self.history.pop() {
            Some((p, s)) => {
                self.page = p;
                self.selected = s;
                self.scroll = s.saturating_sub(VISIBLE_ITEMS - 1);
                self.dirty = true;
                true
            }
            None => false,
        }
    }

    fn reset(&mut self, page: Page) {
        let notice = self.notice.take();
        *self = Menu::new(page);
        self.notice = notice;
    }
}

pub struct MenuPlugin;

/// The placeholder menu's systems (the original main menu runs before
/// them).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct MenuSystems;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Menu::new(Page::Main))
            .add_systems(
                OnEnter(GameState::MainMenu),
                |mut m: ResMut<Menu>, paths: Res<GamePathsFound>| {
                    m.reset(if paths.0 { Page::Main } else { Page::NoGame });
                },
            )
            .add_systems(OnEnter(Play::Paused), |mut m: ResMut<Menu>| {
                m.reset(Page::Paused)
            })
            .add_systems(
                Update,
                (pause_key, navigate, draw_menu, highlight)
                    .chain()
                    .in_set(GameSet::Hud)
                    .in_set(MenuSystems),
            )
            .add_systems(OnEnter(GameState::Loading), spawn_loading_screen)
            .add_systems(
                Update,
                loading
                    .run_if(in_state(GameState::Loading))
                    .in_set(GameSet::Hud),
            );
    }
}

/// Whether the game folder was found at start-up.
#[derive(Resource)]
pub struct GamePathsFound(pub bool);

fn menu_visible(state: &GameState, play: Option<&Play>) -> bool {
    *state == GameState::MainMenu || play == Some(&Play::Paused)
}

/// The original menus (`original_menu.rs`) draw and handle the main page,
/// and any other page whose original menu works here; this placeholder
/// then stays out of the way.
fn original_has_it(
    state: &GameState,
    play: Option<&Play>,
    menu: &Menu,
    original: &OriginalMenus,
) -> bool {
    original.shown(state, play, menu).is_some()
}

pub fn items(
    page: Page,
    levels: &LevelList,
    settings: &Settings,
    paths: &GamePaths,
) -> (String, Vec<Item>) {
    let it = |label: &str, cmd: Cmd| Item {
        label: label.to_string(),
        cmd,
    };
    match page {
        Page::Main => (
            "REPUBLIC COMMANDO".into(),
            vec![
                it("Campaign", Cmd::Go(Page::Campaigns)),
                it("Options", Cmd::Go(Page::Options)),
                it("Quit", Cmd::Quit),
            ],
        ),
        Page::Campaigns => {
            let mut v: Vec<Item> = levels
                .campaigns
                .iter()
                .enumerate()
                .map(|(i, c)| it(&format!("{}  ({})", c.name, c.levels.len()), Cmd::Go(Page::Levels(i))))
                .collect();
            if v.is_empty() {
                v.push(it("No levels found", Cmd::Nothing));
            }
            v.push(it("Back", Cmd::Back));
            ("CAMPAIGN".into(), v)
        }
        Page::Levels(c) => {
            let camp = &levels.campaigns[c.min(levels.campaigns.len().saturating_sub(1))];
            let mut v: Vec<Item> = camp
                .levels
                .iter()
                .map(|l| it(&format!("{}   [{}]", l.title, l.map), Cmd::Play(l.map.clone())))
                .collect();
            v.push(it("Back", Cmd::Back));
            (camp.name.to_uppercase(), v)
        }
        Page::Options | Page::OptionsSound | Page::OptionsGraphics | Page::OptionsControls => {
            let a = &settings.audio;
            let c = &settings.controls;
            (
                "OPTIONS".into(),
                vec![
                    it(&format!("Music volume   < {:.0}% >", a.music_volume * 100.0), Cmd::Setting(Opt::MusicVolume)),
                    it(&format!("Effects volume   < {:.0}% >", a.effects_volume * 100.0), Cmd::Setting(Opt::EffectsVolume)),
                    it(&format!("Mouse sensitivity   < {:.2} >", c.mouse_sensitivity * 400.0), Cmd::Setting(Opt::MouseSensitivity)),
                    it(&format!("Invert look   < {} >", if c.invert_y { "on" } else { "off" }), Cmd::Setting(Opt::InvertY)),
                    it(&format!("Difficulty   < {:?} >", settings.game.difficulty), Cmd::Setting(Opt::Difficulty)),
                    it("More settings", Cmd::Go(Page::MoreSettings)),
                    it("Back (saves)", Cmd::Back),
                ],
            )
        }
        Page::MoreSettings => {
            let v = &settings.video;
            let c = &settings.controls;
            let g = &settings.gyro;
            (
                // Marked: this page is ours, not the original's.
                "MORE SETTINGS (PLACEHOLDER)".into(),
                vec![
                    it(&format!("Field of view   < {:.0} >", v.fov), Cmd::Setting(Opt::FieldOfView)),
                    it(&format!("Wide-screen correction (Panini)   < {:.0}% >", v.panini * 100.0), Cmd::Setting(Opt::Panini)),
                    it(&format!("Display mode   < {:?} >", v.display_mode), Cmd::Setting(Opt::DisplayMode)),
                    it(&format!("Master volume   < {:.0}% >", settings.audio.master_volume * 100.0), Cmd::Setting(Opt::MasterVolume)),
                    it(&format!("Controller look speed   < {:.1} >", c.stick_look_speed), Cmd::Setting(Opt::StickLookSpeed)),
                    it(&format!("Controller look dead zone   < {:.2} >", c.look_stick_dead_zone), Cmd::Setting(Opt::LookStickDeadZone)),
                    it(&format!("Gyro aiming   < {:?} >", g.mode), Cmd::Setting(Opt::GyroMode)),
                    it(&format!("Gyro sensitivity   < {:.1} >", g.sensitivity), Cmd::Setting(Opt::GyroSensitivity)),
                    it("Back (saves)", Cmd::Back),
                ],
            )
        }
        Page::NoGame => (
            "GAME FILES NOT FOUND".into(),
            vec![
                it(
                    &format!(
                        "Set [game] folder in settings.toml to the folder holding GameData. Looked in: {}",
                        paths.game_dir.display()
                    ),
                    Cmd::Nothing,
                ),
                it("Quit", Cmd::Quit),
            ],
        ),
        Page::Paused => (
            "PAUSED".into(),
            vec![
                it("Resume", Cmd::Resume),
                it("Options", Cmd::Go(Page::Options)),
                it("Quit to main menu", Cmd::QuitToMenu),
                it("Quit game", Cmd::Quit),
            ],
        ),
    }
}

/// Escape (or Start) pauses during play.
fn pause_key(
    actions: Res<ActionState>,
    play: Option<Res<State<Play>>>,
    mut next: Option<ResMut<NextState<Play>>>,
    log: Res<GameLog>,
) {
    let (Some(play), Some(next)) = (play, next.as_mut()) else {
        return;
    };
    if *play.get() == Play::Running && actions.just_pressed(Action::Pause) {
        next.set(Play::Paused);
        log.line("menu: paused");
    }
}

#[derive(Component)]
struct MenuRoot;

#[derive(Component)]
struct ItemButton(usize);

#[allow(clippy::too_many_arguments)]
fn navigate(
    actions: Res<ActionState>,
    state: Res<State<GameState>>,
    play: Option<Res<State<Play>>>,
    mut menu: ResMut<Menu>,
    levels: Res<LevelList>,
    mut settings: ResMut<Settings>,
    paths: Res<GamePaths>,
    clicks: Query<(&Interaction, &ItemButton), Changed<Interaction>>,
    mut current: ResMut<CurrentLevel>,
    mut gate: ResMut<LoadingGate>,
    mut next_state: ResMut<NextState<GameState>>,
    mut next_play: Option<ResMut<NextState<Play>>>,
    mut exit: MessageWriter<AppExit>,
    settings_path: Res<SettingsPath>,
    original: Res<OriginalMenus>,
    log: Res<GameLog>,
) {
    if !menu_visible(state.get(), play.as_deref().map(|p| p.get()))
        || original_has_it(
            state.get(),
            play.as_deref().map(|p| p.get()),
            &menu,
            &original,
        )
        || original.used_input
    {
        return;
    }
    let (_, list) = items(menu.page, &levels, &settings, &paths);
    let n = list.len();
    if n == 0 {
        return;
    }
    let mut chosen = None;
    let mut step: i32 = 0;
    if actions.just_pressed(Action::MenuDown) {
        menu.selected = (menu.selected + 1) % n;
    }
    if actions.just_pressed(Action::MenuUp) {
        menu.selected = (menu.selected + n - 1) % n;
    }
    if actions.just_pressed(Action::MenuRight) {
        step = 1;
    }
    if actions.just_pressed(Action::MenuLeft) {
        step = -1;
    }
    for (i, b) in &clicks {
        match i {
            Interaction::Hovered => menu.selected = b.0.min(n - 1),
            Interaction::Pressed => {
                menu.selected = b.0.min(n - 1);
                chosen = Some(b.0.min(n - 1));
            }
            Interaction::None => {}
        }
    }
    if actions.just_pressed(Action::MenuAccept) {
        chosen = Some(menu.selected.min(n - 1));
    }
    // Keep the selection on screen.
    if menu.selected < menu.scroll {
        menu.scroll = menu.selected;
        menu.dirty = true;
    } else if menu.selected >= menu.scroll + VISIBLE_ITEMS {
        menu.scroll = menu.selected + 1 - VISIBLE_ITEMS;
        menu.dirty = true;
    }
    let back = actions.just_pressed(Action::MenuBack);
    let cmd = if back {
        Some(Cmd::Back)
    } else if step != 0 {
        match &list[menu.selected.min(n - 1)].cmd {
            Cmd::Setting(o) => {
                change_setting(&mut settings, *o, step);
                menu.dirty = true;
                None
            }
            _ => None,
        }
    } else {
        chosen.map(|i| list[i].cmd.clone())
    };
    let Some(cmd) = cmd else {
        return;
    };
    log.line(format!("menu: {:?} on page {:?}", cmd, menu.page));
    match cmd {
        Cmd::Go(p) => menu.go(p),
        Cmd::Back => {
            if is_options(menu.page) {
                save_settings(&settings, &settings_path, &log);
            }
            if !menu.back() && menu.page == Page::Paused {
                if let Some(n) = next_play.as_mut() {
                    n.set(Play::Running);
                }
            }
        }
        Cmd::Play(map) => {
            log.line(format!("menu: loading map {map}"));
            menu.notice = None;
            current.map = Some(map);
            gate.reset();
            next_state.set(GameState::Loading);
        }
        Cmd::Resume => {
            if let Some(n) = next_play.as_mut() {
                n.set(Play::Running);
            }
        }
        Cmd::QuitToMenu => next_state.set(GameState::MainMenu),
        Cmd::Quit => {
            log.line("menu: quit");
            exit.write(AppExit::Success);
        }
        Cmd::Setting(o) => {
            change_setting(&mut settings, o, 1);
            menu.dirty = true;
        }
        Cmd::Nothing => {}
    }
}

fn change_setting(s: &mut Settings, o: Opt, step: i32) {
    let f = step as f32;
    match o {
        Opt::MusicVolume => s.audio.music_volume = (s.audio.music_volume + 0.1 * f).clamp(0.0, 1.0),
        Opt::EffectsVolume => {
            s.audio.effects_volume = (s.audio.effects_volume + 0.1 * f).clamp(0.0, 1.0)
        }
        Opt::MouseSensitivity => {
            s.controls.mouse_sensitivity =
                (s.controls.mouse_sensitivity + 0.00025 * f).clamp(0.00025, 0.02)
        }
        Opt::InvertY => s.controls.invert_y = !s.controls.invert_y,
        Opt::FieldOfView => s.video.fov = (s.video.fov + 5.0 * f).clamp(60.0, 120.0),
        Opt::Difficulty => {
            use crate::settings::Difficulty::*;
            let all = [Easy, Medium, Hard];
            let i = all
                .iter()
                .position(|d| *d == s.game.difficulty)
                .unwrap_or(1) as i32;
            s.game.difficulty = all[(i + step).rem_euclid(3) as usize];
        }
        Opt::MasterVolume => {
            s.audio.master_volume = (s.audio.master_volume + 0.1 * f).clamp(0.0, 1.0)
        }
        Opt::DisplayMode => {
            use crate::settings::DisplayMode::*;
            let all = [Windowed, Borderless, Fullscreen];
            let i = all.iter().position(|m| *m == s.video.display_mode).unwrap_or(0) as i32;
            s.video.display_mode = all[(i + step).rem_euclid(3) as usize];
        }
        Opt::Panini => s.video.panini = (s.video.panini + 0.1 * f).clamp(0.0, 1.0),
        Opt::StickLookSpeed => {
            s.controls.stick_look_speed = (s.controls.stick_look_speed + 0.25 * f).clamp(0.5, 8.0)
        }
        Opt::LookStickDeadZone => {
            s.controls.look_stick_dead_zone =
                (s.controls.look_stick_dead_zone + 0.01 * f).clamp(0.0, 0.5)
        }
        Opt::GyroMode => {
            use rc_pads::GyroMode::*;
            let all = [Off, Always, Aiming];
            let i = all.iter().position(|m| *m == s.gyro.mode).unwrap_or(0) as i32;
            s.gyro.mode = all[(i + step).rem_euclid(3) as usize];
        }
        Opt::GyroSensitivity => s.gyro.sensitivity = (s.gyro.sensitivity + 0.1 * f).clamp(0.1, 5.0),
    }
}

/// The options pages (each saves the settings on the way out).
pub fn is_options(page: Page) -> bool {
    matches!(
        page,
        Page::Options
            | Page::OptionsSound
            | Page::OptionsGraphics
            | Page::OptionsControls
            | Page::MoreSettings
    )
}

pub fn save_settings(settings: &Settings, path: &SettingsPath, log: &GameLog) {
    match settings.save(&path.0) {
        Ok(()) => log.line(format!("menu: options saved to {}", path.0.display())),
        Err(e) => log.line(format!(
            "menu: ERROR saving options to {}: {e}",
            path.0.display()
        )),
    }
}

/// Settings changed in the options menu take effect at once.
pub fn apply_settings(
    settings: Res<Settings>,
    mut fov: ResMut<FieldOfView>,
    mut look: ResMut<LookSettings>,
    mut gyro: ResMut<rc_pads::GyroSettings>,
) {
    if !settings.is_changed() {
        return;
    }
    *fov = FieldOfView::from_settings(&settings);
    *look = LookSettings::from_settings(&settings);
    *gyro = settings.gyro.clone();
}

#[allow(clippy::too_many_arguments)]
fn draw_menu(
    mut commands: Commands,
    state: Res<State<GameState>>,
    play: Option<Res<State<Play>>>,
    mut menu: ResMut<Menu>,
    levels: Res<LevelList>,
    settings: Res<Settings>,
    paths: Res<GamePaths>,
    theme: Res<Theme>,
    roots: Query<Entity, With<MenuRoot>>,
    original: Res<OriginalMenus>,
) {
    let visible = menu_visible(state.get(), play.as_deref().map(|p| p.get()))
        && !original_has_it(
            state.get(),
            play.as_deref().map(|p| p.get()),
            &menu,
            &original,
        );
    if !visible {
        for r in &roots {
            commands.entity(r).despawn();
        }
        menu.dirty = true;
        return;
    }
    if !menu.dirty && !roots.is_empty() {
        return;
    }
    menu.dirty = false;
    for r in &roots {
        commands.entity(r).despawn();
    }
    let (title, list) = items(menu.page, &levels, &settings, &paths);
    let paused = menu.page == Page::Paused || play.is_some();
    let background = if paused {
        theme.background.with_alpha(0.75)
    } else {
        theme.background
    };
    let scroll = menu.scroll;
    let notice = menu.notice.clone();
    commands
        .spawn((
            MenuRoot,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                justify_content: JustifyContent::Center,
                padding: UiRect::left(Val::Percent(10.0)),
                row_gap: Val::VMin(1.2),
                ..default()
            },
            BackgroundColor(background),
            GlobalZIndex(10),
        ))
        .with_children(|root| {
            root.spawn((
                Text::new(title),
                theme.font(theme.title),
                TextColor(theme.text),
            ));
            // A thin amber line under the title, like the visor's.
            root.spawn((
                Node {
                    width: Val::Percent(40.0),
                    height: Val::VMin(0.3),
                    margin: UiRect::bottom(Val::VMin(2.0)),
                    ..default()
                },
                BackgroundColor(theme.text_dim),
            ));
            if let Some(n) = notice {
                root.spawn((
                    Text::new(n),
                    theme.font(theme.small),
                    TextColor(theme.warning),
                ));
            }
            if scroll > 0 {
                root.spawn((
                    Text::new("  ..."),
                    theme.font(theme.small),
                    TextColor(theme.text_dim),
                ));
            }
            for (i, item) in list.iter().enumerate().skip(scroll).take(VISIBLE_ITEMS) {
                root.spawn((
                    Interaction::None,
                    ItemButton(i),
                    Node {
                        padding: UiRect::axes(Val::VMin(1.0), Val::VMin(0.4)),
                        max_width: Val::Percent(85.0),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new(item.label.clone()),
                        theme.font(theme.item),
                        TextColor(theme.text_dim),
                    ));
                });
            }
            if list.len() > scroll + VISIBLE_ITEMS {
                root.spawn((
                    Text::new("  ..."),
                    theme.font(theme.small),
                    TextColor(theme.text_dim),
                ));
            }
        });
}

/// Colour the selected item.
fn highlight(
    menu: Res<Menu>,
    theme: Res<Theme>,
    buttons: Query<(&ItemButton, &Children, &mut BackgroundColor)>,
    mut texts: Query<&mut TextColor>,
) {
    for (b, children, mut bg) in buttons {
        let on = b.0 == menu.selected;
        bg.0 = if on { theme.panel } else { Color::NONE };
        for c in children {
            if let Ok(mut t) = texts.get_mut(*c) {
                t.0 = if on { theme.highlight } else { theme.text_dim };
            }
        }
    }
}

#[derive(Component)]
struct LoadingText;

#[derive(Resource, Default)]
struct LoadingFrames(u32);

fn spawn_loading_screen(
    mut commands: Commands,
    current: Res<CurrentLevel>,
    levels: Res<LevelList>,
    theme: Res<Theme>,
    log: Res<GameLog>,
) {
    let map = current.map.clone().unwrap_or_default();
    let (title, text) = match levels.find(&map) {
        Some(l) => (l.title.to_uppercase(), l.text.clone()),
        None => (map.to_uppercase(), String::new()),
    };
    log.line(format!("loading: {map} ({title})"));
    commands.insert_resource(LoadingFrames(0));
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                padding: UiRect::horizontal(Val::Percent(12.0)),
                row_gap: Val::VMin(2.0),
                ..default()
            },
            BackgroundColor(theme.background.with_alpha(1.0)),
            GlobalZIndex(10),
            DespawnOnExit(GameState::Loading),
        ))
        .with_children(|root| {
            root.spawn((
                Text::new(title),
                theme.font(theme.heading),
                TextColor(theme.text),
            ));
            if !text.is_empty() {
                root.spawn((
                    Text::new(text),
                    theme.font(theme.small),
                    TextColor(theme.text_dim),
                ));
            }
            root.spawn((
                LoadingText,
                Text::new("LOADING..."),
                theme.font(theme.small),
                TextColor(theme.text),
            ));
        });
}

/// Move on once every part has finished loading, or go back to the menu
/// with the reason if one failed.
fn loading(
    mut frames: ResMut<LoadingFrames>,
    gate: Res<LoadingGate>,
    mut text: Query<&mut Text, With<LoadingText>>,
    mut next: ResMut<NextState<GameState>>,
    mut menu: ResMut<Menu>,
    log: Res<GameLog>,
) {
    frames.0 += 1;
    if let Some(f) = gate.failure() {
        log.line(format!("loading: FAILED: {f}"));
        menu.notice = Some(format!("Couldn't load the level: {f}"));
        next.set(GameState::MainMenu);
        return;
    }
    let waiting: Vec<&str> = gate.waiting().collect();
    if let Ok(mut t) = text.single_mut() {
        t.0 = if waiting.is_empty() {
            "LOADING...".into()
        } else {
            format!("LOADING...   {}", waiting.join(", "))
        };
    }
    if frames.0 >= 2 && waiting.is_empty() {
        log.line(format!("loading: done after {} frames", frames.0));
        next.set(GameState::InGame);
    }
}
