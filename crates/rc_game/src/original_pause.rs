//! The pause menu's rows, set up the original's way each time it opens
//! (`rc_menus::pause` has the rules). This part gathers what those rules
//! ask about the game: health, a briefing, the prologue, the quicksave and
//! whether hint screens can be switched back on.

use crate::original_menu::PageScreen;
use crate::settings::Settings;
use bevy::prelude::*;
use rc_game_core::save::{SaveList, SaveSlot};
use rc_game_core::{CurrentLevel, GameLog, GamePaths, GameState, PlayerVitals};
use rc_menus::pause::{self, PauseState};
use rc_package::properties::Value;

/// The page's name in `original_menus.toml`.
pub const PAUSED: &str = "paused";

/// Whether the level being played has hint screens (its LevelInfo's
/// `bLevelContainsHints`). Read once when the level starts.
#[derive(Resource, Debug, Clone, Default)]
pub struct LevelHasHints {
    pub map: Option<String>,
    pub hints: bool,
}

pub struct OriginalPausePlugin;

impl Plugin for OriginalPausePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LevelHasHints>()
            .add_systems(OnEnter(GameState::InGame), read_level_hints);
    }
}

/// Read `bLevelContainsHints` from the map's LevelInfo.
fn read_level_hints(
    current: Res<CurrentLevel>,
    paths: Res<GamePaths>,
    mut out: ResMut<LevelHasHints>,
    log: Res<GameLog>,
) {
    let Some(map) = current.map.clone() else {
        return;
    };
    if out.map.as_deref() == Some(map.as_str()) {
        return;
    }
    let path = paths.map_path(&map);
    let read = std::fs::read(&path)
        .map_err(|e| e.to_string())
        .and_then(|d| {
            rc_package::Package::parse(&d)
                .map(|p| (p, d))
                .map_err(|e| e.to_string())
        })
        .and_then(|(p, d)| {
            let level = rc_level::actors::read_level(&p, &d).map_err(|e| e.to_string())?;
            Ok(level_contains_hints(&p, &d, &level))
        });
    let hints = match read {
        Ok(h) => h,
        Err(e) => {
            log.line(format!(
                "pause menu: can't read {} ({e}); taking the level to have no hints",
                path.display()
            ));
            false
        }
    };
    log.line(format!("pause menu: {map}: level has hints: {hints}"));
    *out = LevelHasHints {
        map: Some(map),
        hints,
    };
}

/// The map's LevelInfo `bLevelContainsHints` (false when not set).
pub fn level_contains_hints(
    pkg: &rc_package::Package,
    data: &[u8],
    level: &rc_level::actors::LevelInfo,
) -> bool {
    for &a in level.actors.iter().flatten() {
        if !pkg
            .export_class_name(&pkg.exports[a])
            .eq_ignore_ascii_case("LevelInfo")
        {
            continue;
        }
        let Ok(props) = rc_level::actors::read_actor_properties(pkg, data, a) else {
            continue;
        };
        return props.iter().any(|p| {
            pkg.name(p.name).eq_ignore_ascii_case("bLevelContainsHints")
                && matches!(p.value, Value::Bool(true))
        });
    }
    false
}

/// What the rules need to know, now.
pub fn state_now(
    vitals: Option<&PlayerVitals>,
    cutscene: Option<&rc_game_core::Cutscene>,
    current: &CurrentLevel,
    saves: &SaveList,
    settings: &Settings,
    hints: &LevelHasHints,
) -> PauseState {
    let map = current
        .map
        .as_deref()
        .unwrap_or_default()
        .trim_end_matches(".ctm")
        .trim_end_matches(".CTM");
    PauseState {
        no_health: vitals.is_some_and(|v| v.health <= 0.0),
        incapacitated: vitals.is_some_and(|v| v.incapacitated),
        briefing: cutscene.is_some_and(|c| c.held.is_some()),
        prologue: map.eq_ignore_ascii_case("pro"),
        has_quick_save: saves.get(&SaveSlot::Quick).is_some(),
        // The original's CanEnableHints: hint screens are off and the
        // level has some.
        can_enable_hints: !settings.game.hint_screens && hints.hints,
    }
}

/// Set the pause menu up as it opens, from the layout read from the game.
/// Returns the widget to focus and a line for the log.
pub fn open(page: &mut PageScreen, s: &PauseState) -> Option<(usize, String)> {
    let base = page
        .base
        .get_or_insert_with(|| page.screen.layout.clone())
        .clone();
    page.screen.layout = base;
    let rows = pause::init(&mut page.screen.layout, s)?;
    let focus = pause::row(&page.screen.layout, rows.focus)?;
    let names: Vec<String> = rows
        .shown
        .iter()
        .filter_map(|&i| pause::row(&page.screen.layout, i))
        .map(|w| page.screen.layout.widgets[w].name.clone())
        .collect();
    Some((
        focus,
        format!("{s:?}: rows {} from {:.4} down", names.join(", "), rows.top),
    ))
}
