//! The original game's main menu, drawn from the player's install (see
//! `rc_menus` and `docs/ORIGINAL_MENUS.md`). It replaces the placeholder
//! main menu page in `menu.rs`; the other pages (campaign list, options,
//! pause) are still the placeholders until their original screens are
//! built.
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
    Action, ActionState, CurrentLevel, GameLog, GamePaths, GameSet, GameState, LoadingGate,
};
use rc_menus::gpu::{self, Variant};
use rc_menus::layout::{Kind, MenuLayout};
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

    fn on_select(&self, i: usize) -> String {
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

/// The original menus, when the install has them.
#[derive(Resource, Default)]
pub struct OriginalMenus {
    /// Tried to load them (only once).
    pub tried: bool,
    pub main: Option<Screen>,
    /// The "are you sure?" box over the main menu.
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
}

pub struct OriginalMenuPlugin;

impl Plugin for OriginalMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OriginalMenus>()
            .add_systems(OnEnter(GameState::MainMenu), (load, reset).chain())
            .add_systems(
                Update,
                (navigate, draw, fit_area)
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
    let mut paths_needed = assets.textures_of(&layout);
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

fn showing(state: &GameState, menu: &Menu, menus: &OriginalMenus) -> bool {
    *state == GameState::MainMenu && menu.page == Page::Main && menus.active()
}

#[derive(Component)]
struct MenuArea;

#[derive(Component)]
struct OriginalRoot;

/// A mouse target for one button: (is the question box, widget index).
#[derive(Component)]
struct HitBox(bool, usize);

fn play(
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
    mut menu: ResMut<Menu>,
    mut menus: ResMut<OriginalMenus>,
    hits: Query<(&Interaction, &HitBox), Changed<Interaction>>,
    mut current: ResMut<CurrentLevel>,
    mut gate: ResMut<LoadingGate>,
    mut next_state: ResMut<NextState<GameState>>,
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
    if !showing(state.get(), &menu, &menus) {
        return;
    }
    let menus = &mut *menus;
    let in_question = menus.question.is_some();
    let mut chosen = None;
    let mut moved = false;
    {
        let screen = if in_question {
            menus.question.as_mut().unwrap()
        } else {
            menus.main.as_mut().unwrap()
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
        play(&mut commands, menus, &mut sources, &settings, s, &log);
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
        let s = menus.select_sound.clone();
        play(&mut commands, menus, &mut sources, &settings, s, &log);
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
    let Some(i) = chosen else {
        return;
    };
    menus.used_input = true;
    let main = menus.main.as_ref().unwrap();
    let name = main.layout.widgets[i].name.clone();
    let action = screens::main_action(&main.on_select(i));
    log.line(format!("original menus: chose {name}: {action:?}"));
    let s = menus.select_sound.clone();
    play(&mut commands, menus, &mut sources, &settings, s, &log);
    match action {
        MainAction::NewGame => {
            // The original opens the prologue map, PRO.
            log.line("menu: loading map pro");
            menu.notice = None;
            current.map = Some("pro".into());
            gate.reset();
            next_state.set(GameState::Loading);
        }
        MainAction::CampaignMap => menu.go(Page::Campaigns),
        MainAction::Options => menu.go(Page::Options),
        MainAction::Quit => match &menus.question_layout {
            Some(l) => {
                let mut layout = l.clone();
                let under = menus.main.as_ref().map(|m| &m.layout);
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
        other => log.line(format!("original menus: {other:?} isn't available yet")),
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    mut commands: Commands,
    state: Res<State<GameState>>,
    menu: Res<Menu>,
    mut menus: ResMut<OriginalMenus>,
    roots: Query<Entity, With<OriginalRoot>>,
    mut image_assets: ResMut<Assets<Image>>,
) {
    let show = showing(state.get(), &menu, &menus);
    if !show {
        for r in &roots {
            commands.entity(r).despawn();
        }
        menus.dirty = true;
        return;
    }
    if !menus.dirty && !roots.is_empty() {
        return;
    }
    menus.dirty = false;
    for r in &roots {
        commands.entity(r).despawn();
    }
    let menus = &mut *menus;
    let Some(assets) = menus.assets.as_mut() else {
        return;
    };
    // The question box replaces the main menu on screen (it borrows only
    // its background picture), as in the original.
    let (is_question, screen) = match &menus.question {
        Some(q) => (true, q),
        None => (false, menus.main.as_ref().unwrap()),
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
            BackgroundColor(Color::BLACK),
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
