//! The original's loading screens, drawn from the player's install
//! (`rc_menus::loading` has the rules and where each was read from).
//!
//! When a level starts loading, the map's row in the install's
//! `LevelLoadingInfo.int` names the screen: the briefing screen
//! (`CTLoadingInfo`: picture, mission name, mission text) or the plain one
//! (`CTLoading`: black, mission name); a map with no row gets the plain
//! screen with no name, as in the original. The bar fills by the original
//! engine's rules (see below). When a briefing screen's level has
//! loaded, play is paused and the same screen shows START; START
//! un-pauses (as the original's `LoadingFinished` and
//! `CTLoadingInfo.OnAButton` do). The plain screen just goes away.
//!
//! The two screens are pages `loading` and `loading_info` in
//! `data/ui/original_menus.toml`. If a map names a class that neither page
//! uses, or the page can't be drawn, the placeholder loading screen
//! (`menu.rs`) shows instead and the log says why.
//!
//! The bar: the original fills it from its own loader's progress
//! (`GameEngine.LoadProgressFraction`, which `UGameEngine::UpdateProgress`
//! passes to `UpdateLoadingProgress`). The port keeps that value by the
//! same rules and stages as the original's `LoadMap` (`rules::Progress`),
//! moved at the matching points of this port's loading (`progress`). It
//! is drawn exactly as the original draws a bar (standard alpha blending,
//! as `D3DDrv.dll` sets for a `bAlphaTexture` texture).

use crate::menu::{Menu, Page, PlaceholderLoadingScreen};
use crate::original_menu::{self, OriginalMenus};
use bevy::prelude::*;
use rc_game_core::{CurrentLevel, GameLog, GamePaths, GameState, LoadingGate, Play, StartHeld};
use rc_menus::loading::{self as rules, EngineClasses, LoadingInfo};

/// The pages' names in `data/ui/original_menus.toml`.
pub const LOADING_PAGE: &str = "loading";
pub const LOADING_INFO_PAGE: &str = "loading_info";

/// A briefing screen's level has loaded and play waits for START.
#[derive(Resource, Default)]
pub struct LoadingDone {
    pub waiting: bool,
}

/// What was read from the install, once.
#[derive(Resource, Default)]
struct LoadingRules {
    read: bool,
    rows: Vec<LoadingInfo>,
    classes: Option<EngineClasses>,
}

/// The screen showing now and how loading is going.
#[derive(Resource, Default)]
struct Showing {
    /// The page, when it pauses when done (`CTLoadingInfo`).
    pauses: bool,
    /// The original's bar (`rules::Progress`) and the next of
    /// `LoadMap`'s stages to start.
    bar: Option<rules::Progress>,
    stage: usize,
    /// The map itself (`rc_level_plugin`'s part) has been seen loading.
    saw_level: bool,
    /// The most other parts seen waiting at once.
    most_others: usize,
    /// The bar's value last drawn.
    shown_fill: f32,
    /// A level was showing when loading started (not the front end).
    from_level: bool,
}

pub struct LoadingScreenPlugin;

impl Plugin for LoadingScreenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LoadingDone>()
            .init_resource::<LoadingRules>()
            .init_resource::<Showing>()
            .add_systems(OnExit(GameState::InGame), |mut s: ResMut<Showing>| {
                s.from_level = true;
            })
            .add_systems(OnExit(GameState::MainMenu), |mut s: ResMut<Showing>| {
                s.from_level = false;
            })
            .add_systems(
                OnEnter(GameState::Loading),
                (original_menu::load, start)
                    .chain()
                    .before(PlaceholderLoadingScreen),
            )
            .add_systems(
                Update,
                progress
                    .run_if(in_state(GameState::Loading))
                    .before(crate::menu::MenuSystems),
            )
            .add_systems(OnExit(GameState::Loading), loaded)
            .add_systems(OnEnter(Play::Running), pause_for_start)
            .add_systems(OnExit(Play::Paused), started)
            .add_systems(OnEnter(GameState::MainMenu), forget);
    }
}

/// Read `LevelLoadingInfo.int` and the two fallback classes from
/// `System.ini` (the player's own copy in `GameData/Save` first, as the
/// original reads it, then the shipped one).
fn read_rules(paths: &GamePaths, log: &GameLog) -> LoadingRules {
    let mut out = LoadingRules {
        read: true,
        ..Default::default()
    };
    let info = paths.game_file("GameData/System/LevelLoadingInfo.int");
    match std::fs::read(&info) {
        Ok(b) => {
            out.rows = rules::parse_level_loading_info(&crate::levels::decode_int(&b));
            log.line(format!(
                "loading screen: {} maps listed in {}",
                out.rows.len(),
                info.display()
            ));
        }
        Err(e) => log.line(format!(
            "loading screen: can't read {} ({e}); every map gets the plain screen",
            info.display()
        )),
    }
    let (mut loading, mut loading_info) = (None, None);
    for rel in ["GameData/Save/System.ini", "GameData/System/System.ini"] {
        let p = paths.game_file(rel);
        let Ok(text) = std::fs::read_to_string(&p)
            .or_else(|_| std::fs::read(&p).map(|b| b.iter().map(|&c| c as char).collect()))
        else {
            continue;
        };
        let (a, b) = rules::engine_classes(&text);
        if loading.is_none() {
            loading = a;
        }
        if loading_info.is_none() {
            loading_info = b;
        }
    }
    match (loading, loading_info) {
        (Some(loading), Some(loading_info)) => {
            log.line(format!(
                "loading screen: System.ini: LoadingMenuClass={loading}, LoadingInfoMenuClass={loading_info}"
            ));
            out.classes = Some(EngineClasses {
                loading,
                loading_info,
            });
        }
        _ => log.line(
            "loading screen: System.ini has no LoadingMenuClass or LoadingInfoMenuClass; maps without their own class get the placeholder screen",
        ),
    }
    out
}

/// Loading starts: choose the screen, as the original's `LoadMap` does,
/// and set it up as its `LoadingStarted` does.
#[allow(clippy::too_many_arguments)]
fn start(
    mut menus: ResMut<OriginalMenus>,
    mut rules_read: ResMut<LoadingRules>,
    mut showing: ResMut<Showing>,
    mut done: ResMut<LoadingDone>,
    current: Res<CurrentLevel>,
    paths: Res<GamePaths>,
    log: Res<GameLog>,
    time: Res<Time<Real>>,
) {
    menus.loading_page = None;
    done.waiting = false;
    // `LoadMap`: BeginSlowTask, then the first stage (the map's package).
    let now = time.elapsed_secs_f64();
    let mut bar = rules::Progress::begin(now);
    let (a, b) = rules::LOAD_MAP_STAGES[0];
    bar.set_range(a, b, now);
    *showing = Showing {
        from_level: showing.from_level,
        bar: Some(bar),
        stage: 1,
        ..Default::default()
    };
    if !rules_read.read {
        *rules_read = read_rules(&paths, &log);
    }
    let map = current.map.clone().unwrap_or_default();
    let fallback = EngineClasses {
        loading: String::new(),
        loading_info: String::new(),
    };
    let info = rules::choose(
        &map,
        &rules_read.rows,
        rules_read.classes.as_ref().unwrap_or(&fallback),
    );
    if info.menu_class.is_empty() {
        log.line(format!(
            "loading screen: {map}: no class to show (System.ini not read); placeholder screen"
        ));
        return;
    }
    let menus = &mut *menus;
    let key = [LOADING_PAGE, LOADING_INFO_PAGE].into_iter().find(|k| {
        menus
            .pages
            .get(*k)
            .is_some_and(|p| p.plan.class.eq_ignore_ascii_case(&info.menu_class))
    });
    let Some(key) = key else {
        log.line(format!(
            "loading screen: {map}: its class {} isn't a page that can be drawn here; placeholder screen",
            info.menu_class
        ));
        return;
    };
    let Some(assets) = menus.assets.as_mut() else {
        return;
    };
    let family = match assets.library.class(&info.menu_class) {
        Ok(c) => c.family.clone(),
        Err(e) => {
            log.line(format!(
                "loading screen: {map}: can't read {} ({e}); placeholder screen",
                info.menu_class
            ));
            return;
        }
    };
    let page = menus.pages.get_mut(key).unwrap();
    // A fresh screen each time, as the original spawns a new menu.
    page.screen.layout = page.pristine.clone();
    page.screen.focus = None;
    let set = rules::setup(
        &mut page.screen.layout,
        &family,
        &mut assets.library,
        &info,
        showing.from_level,
    );
    if let Some(p) = &set.picture {
        if assets.texture(p).is_none() {
            log.line(format!("loading screen: PROBLEM picture {p} can't be read"));
        }
    }
    for p in &set.problems {
        log.line(format!("loading screen: PROBLEM {p}"));
    }
    showing.pauses = rules::pauses_when_done(&family);
    log.line(format!(
        "loading screen: {map}: original {} (page {key}), title {:?}, picture {:?}, hints {} (never on a PC), START when loaded: {}",
        info.menu_class, info.title, set.picture, info.show_hints, showing.pauses
    ));
    menus.loading_page = Some(key.to_string());
    menus.mark_dirty();
}

/// The bar, moved by the rules the original's engine uses
/// (`rules::Progress`) at the points of the port's loading that match
/// `LoadMap`'s: the map is read in the first stage; the next stage starts
/// when it's done; the port's other loading parts (scripts, cast,
/// enemies, movers and so on) count through that stage as they finish;
/// when all are done the remaining stages start and the bar fills, as at
/// `EndSlowTask`. Ours, not the original's: the original's first stage
/// moves as the package's objects load (`EndLoad`), but the port reads
/// the map in one step with nothing to count, and which of the port's
/// parts count in which of the original's later stages is the port's
/// choice (they don't match one to one).
fn progress(
    gate: Res<LoadingGate>,
    time: Res<Time<Real>>,
    mut showing: ResMut<Showing>,
    mut menus: ResMut<OriginalMenus>,
) {
    let Some(key) = menus.loading_page.clone() else {
        return;
    };
    let Some(mut bar) = showing.bar else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let level = rc_level_plugin::PART;
    let level_waiting = gate.waiting().any(|p| p == level);
    showing.saw_level |= level_waiting;
    let others = gate.waiting().filter(|p| *p != level).count();
    showing.most_others = showing.most_others.max(others);
    if showing.saw_level && !level_waiting {
        if showing.stage == 1 {
            let (a, b) = rules::LOAD_MAP_STAGES[1];
            bar.set_range(a, b, now);
            showing.stage = 2;
        }
        if gate.is_clear() {
            while let Some(&(a, b)) = rules::LOAD_MAP_STAGES.get(showing.stage) {
                bar.set_range(a, b, now);
                showing.stage += 1;
            }
            bar.end();
        } else if showing.most_others > 0 {
            let most = showing.most_others as u32;
            bar.status(most - others as u32, most, now);
        }
    }
    showing.bar = Some(bar);
    let fill = bar.fraction;
    if (fill - showing.shown_fill).abs() < 1e-4 {
        return;
    }
    showing.shown_fill = fill;
    if let Some(p) = menus.pages.get_mut(&key) {
        rules::set_progress(&mut p.screen.layout, fill);
        menus.mark_dirty();
    }
}

/// Loading is over: a briefing screen stays, now with START (play
/// pauses as soon as it starts, `pause_for_start`); the plain one goes.
fn loaded(
    mut menus: ResMut<OriginalMenus>,
    showing: Res<Showing>,
    mut done: ResMut<LoadingDone>,
    mut held: ResMut<StartHeld>,
    log: Res<GameLog>,
) {
    let Some(key) = menus.loading_page.clone() else {
        return;
    };
    let started = showing.pauses
        && key == LOADING_INFO_PAGE
        && menus.pages.get_mut(&key).is_some_and(|p| {
            let shown = rules::finish(&mut p.screen.layout);
            // The finished screen opens anew (`MenuOpenExisting`), so
            // START gets the focus as any opening page's first button.
            p.screen.focus = p.screen.buttons().first().copied();
            shown
        });
    if !started {
        menus.loading_page = None;
        log.line(if showing.pauses {
            "loading screen: done; no START button, so play goes on"
        } else {
            "loading screen: done; the plain screen closes"
        });
        return;
    }
    // Kept showing until START (`OriginalMenus::shown`). Nothing in the
    // level runs until then, not even the first frame before the pause
    // takes hold (`StartHeld`): orders a level script sent in that frame
    // were lost while paused (the intro Geonosian on geo_01a then missed
    // its whole opening).
    done.waiting = true;
    held.0 = true;
    menus.mark_dirty();
    log.line("loading screen: done; paused until START (LOADINGDONE)");
}

/// The level has started: pause it at once when a briefing screen waits
/// for START (the original pauses before the level runs).
fn pause_for_start(
    state: Res<State<GameState>>,
    done: ResMut<LoadingDone>,
    mut next: Option<ResMut<NextState<Play>>>,
    mut menu: ResMut<Menu>,
    log: Res<GameLog>,
) {
    if !done.waiting {
        return;
    }
    if *state.get() != GameState::InGame {
        return;
    }
    if let Some(n) = next.as_mut() {
        menu.reset(Page::LoadingDone);
        n.set(Play::Paused);
        log.line("loading screen: play paused for START");
    }
}

/// START (or anything else that un-pauses) ends the wait.
fn started(
    mut done: ResMut<LoadingDone>,
    mut held: ResMut<StartHeld>,
    mut menus: ResMut<OriginalMenus>,
    log: Res<GameLog>,
) {
    held.0 = false;
    if done.waiting {
        done.waiting = false;
        menus.loading_page = None;
        menus.mark_dirty();
        log.line("loading screen: START chosen; play goes on");
    }
}

fn forget(
    mut done: ResMut<LoadingDone>,
    mut held: ResMut<StartHeld>,
    mut menus: ResMut<OriginalMenus>,
) {
    done.waiting = false;
    held.0 = false;
    menus.loading_page = None;
}
