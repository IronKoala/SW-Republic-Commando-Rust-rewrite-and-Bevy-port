//! Cutscenes on screen: the player held still, the HUD hidden and the
//! whole-screen fade, as the level scripts ask (`rc_game_core::Cutscene`,
//! filled in by `rc_scripts_plugin::cutscene`).
//!
//! - While a cutscene holds the player, `ActionState` keeps only looking,
//!   menus, saving and screenshots (and weapon changes if the scene
//!   allows), so nothing that reads it moves, shoots or orders the squad.
//!   The look limits and the frozen body are the player's own
//!   (`player.rs`).
//! - The HUD (the placeholder and the original pictures) is hidden while
//!   the scene asks.
//! - A full-screen colour layer over the game and the HUD, under the menus,
//!   shows fades to black and the visor's flashes.

use bevy::prelude::*;
use rc_game_core::{ActionState, Cutscene, GameLog, GameSet, GameState};

pub struct CutscenePlugin;

/// The top of a HUD drawing, hidden while a cutscene hides the HUD.
#[derive(Component)]
pub(crate) struct HudRoot;

/// The whole-screen colour layer.
#[derive(Component)]
struct FadeLayer;

impl Plugin for CutscenePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::InGame), spawn_layer)
            .add_systems(
                Update,
                hold_player
                    .after(GameSet::Input)
                    .before(GameSet::Simulate)
                    .run_if(in_state(GameState::InGame)),
            )
            .add_systems(
                Update,
                (show_tint, hide_hud, log_changes)
                    .in_set(GameSet::Hud)
                    .run_if(in_state(GameState::InGame)),
            );
    }
}

fn hold_player(cut: Res<Cutscene>, mut actions: ResMut<ActionState>) {
    if let Some(h) = &cut.held {
        actions.hold_still(h.weapon_change);
    }
}

fn spawn_layer(mut commands: Commands) {
    commands.spawn((
        FadeLayer,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::NONE),
        // Over the HUD (1-2), under the menus (10).
        GlobalZIndex(9),
        DespawnOnExit(GameState::InGame),
    ));
}

fn show_tint(cut: Res<Cutscene>, mut layer: Query<&mut BackgroundColor, With<FadeLayer>>) {
    let [r, g, b, a] = cut.tint;
    let want = if a <= 0.0 {
        Color::NONE
    } else {
        Color::srgba(r, g, b, a.min(1.0))
    };
    for mut c in &mut layer {
        if c.0 != want {
            c.0 = want;
        }
    }
}

fn hide_hud(cut: Res<Cutscene>, mut roots: Query<&mut Visibility, With<HudRoot>>) {
    let want = if cut.hide_hud {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for mut v in &mut roots {
        if *v != want {
            *v = want;
        }
    }
}

/// Log when the hold, the HUD or the screen cover change (once each).
fn log_changes(
    cut: Res<Cutscene>,
    log: Res<GameLog>,
    mut last: Local<Option<(bool, bool, bool, bool, bool)>>,
) {
    let now = (
        cut.held.is_some(),
        cut.hide_hud,
        cut.tint[3] >= 0.99,
        cut.frozen,
        cut.briefing_weapon.is_some(),
    );
    if *last != Some(now) {
        if last.is_some() || now != (false, false, false, false, false) {
            log.line(format!(
                "cutscene: player {}, HUD {}, screen {}, body {}, arms {}",
                if now.0 { "held" } else { "free" },
                if now.1 { "hidden" } else { "shown" },
                if now.2 { "covered" } else { "visible" },
                if now.3 { "frozen" } else { "moves" },
                if now.4 {
                    "hidden (other hands)"
                } else {
                    "shown"
                }
            ));
        }
        *last = Some(now);
    }
}
