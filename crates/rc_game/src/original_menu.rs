//! The original game's menus, drawn from the player's install (see
//! `rc_menus` and `docs/ORIGINAL_MENUS.md`). The main menu replaces the
//! placeholder main page in `menu.rs`. The other pages (pause, campaign,
//! options) use the original menu named for them in
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

use crate::menu::{GamePathsFound, Menu, Page};
use crate::settings::Settings;
use bevy::asset::RenderAssetUsages;
use bevy::audio::{AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use rc_game_core::{
    Action, ActionState, CurrentLevel, GameLog, GamePaths, GameSet, GameState, LoadingGate, Play,
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
    fn buttons(&self) -> Vec<usize> {
        self.layout
            .buttons_on_pc()
            .into_iter()
            .filter(|&i| match &self.layout.widgets[i].kind {
                Kind::Button(b) => !b.disabled,
                _ => false,
            })
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
        Page::Paused => Some("paused"),
        Page::Main | Page::Levels(_) | Page::NoGame => None,
    }
}

fn page_from_key(key: &str) -> Option<Page> {
    match key {
        "campaigns" => Some(Page::Campaigns),
        "options" => Some(Page::Options),
        "paused" => Some(Page::Paused),
        _ => None,
    }
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
                (focus_new_page, navigate, draw, fit_area)
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
            Ok(buttons) => {
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
    menus.quit_text = quit_text;
    menus.assets = Some(assets);
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
fn focus_new_page(
    state: Res<State<GameState>>,
    play: Option<Res<State<Play>>>,
    menu: Res<Menu>,
    mut menus: ResMut<OriginalMenus>,
    mut last: Local<Option<Shown>>,
    log: Res<GameLog>,
) {
    let shown = menus.shown(state.get(), play.as_deref().map(|p| p.get()), &menu);
    if shown == *last {
        return;
    }
    *last = shown.clone();
    menus.question = None;
    menus.dirty = true;
    if let Some(Shown::Page(k)) = &shown {
        let screen = &mut menus.pages.get_mut(k).unwrap().screen;
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

/// A mouse target for one button: (is the question box, widget index).
#[derive(Component)]
struct HitBox(bool, usize);

fn play_sound(
    commands: &mut Commands,
    menus: &mut OriginalMenus,
    sources: &mut Assets<AudioSource>,
    settings: &Settings,
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
        let v = (settings.audio.master_volume * settings.audio.effects_volume).clamp(0.0, 1.0);
        commands.spawn((
            AudioPlayer(h.clone()),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(v)),
        ));
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
    mut current: ResMut<CurrentLevel>,
    mut gate: ResMut<LoadingGate>,
    mut next_state: ResMut<NextState<GameState>>,
    mut next_play: Option<ResMut<NextState<Play>>>,
    mut exit: MessageWriter<AppExit>,
    mut sources: ResMut<Assets<AudioSource>>,
    mut cursor: MessageReader<CursorMoved>,
    settings: Res<Settings>,
    log: Res<GameLog>,
) {
    // Pointing at a button moves focus only when the mouse itself moves,
    // not when the menu is redrawn under a still pointer.
    let mouse_moved = cursor.read().count() > 0;
    menus.used_input = false;
    let Some(shown) = menus.shown(state.get(), play.as_deref().map(|p| p.get()), &menu) else {
        return;
    };
    let menus = &mut *menus;
    let in_question = menus.question.is_some();
    let mut chosen = None;
    let mut moved = false;
    {
        let screen = if in_question {
            menus.question.as_mut().unwrap()
        } else {
            menus.screen_mut(&shown).unwrap()
        };
        if actions.just_pressed(Action::MenuDown) || actions.just_pressed(Action::MenuRight) {
            moved |= screen.step(1);
        }
        if actions.just_pressed(Action::MenuUp) || actions.just_pressed(Action::MenuLeft) {
            moved |= screen.step(-1);
        }
        for (i, hit) in &hits {
            if hit.0 != in_question {
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
                    if *i == Interaction::Pressed && screen.buttons().contains(&hit.1) {
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
        play_sound(&mut commands, menus, &mut sources, &settings, s, &log);
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
        play_sound(&mut commands, menus, &mut sources, &settings, s, &log);
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
    play_sound(&mut commands, menus, &mut sources, &settings, s, &log);
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
        PageCmd::Unavailable => {}
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
    // its background picture), as in the original.
    let (is_question, screen) = match &menus.question {
        Some(q) => (true, q),
        None => match &shown {
            Shown::Main => (false, menus.main.as_ref().unwrap()),
            Shown::Page(k) => (false, &menus.pages[k].screen),
        },
    };
    // (question box?, pictures, mouse areas by widget index)
    type Drawn = (bool, Vec<Quad>, Vec<(usize, [f32; 4])>);
    let mut list: Vec<Drawn> = Vec::new();
    {
        let focus = screen.focus;
        let q = quads::menu_quads(&screen.layout, focus, assets);
        let boxes = screen
            .layout
            .buttons_on_pc()
            .into_iter()
            .filter_map(|i| match &screen.layout.widgets[i].kind {
                Kind::Button(b) => quads::sprite_box(&b.background_blurred, assets)
                    .or_else(|| quads::text_box(&b.blurred, assets))
                    .map(|bx| (i, bx)),
                _ => None,
            })
            .collect();
        list.push((is_question, q, boxes));
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
