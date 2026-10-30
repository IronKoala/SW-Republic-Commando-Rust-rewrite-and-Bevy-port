//! The optional Panini view (`video.panini` in the settings file): bends
//! the player's 3D view so very wide screens aren't stretched at the sides.
//! The bend itself is in `rc_panini`; this sets it up for the player's
//! camera after the camera has its normal field of view.

use crate::player::place_camera;
use crate::settings::Settings;
use bevy::prelude::*;
use rc_game_core::{GameLog, GameSet, GameState, PlayerCamera};
use rc_panini::{Fit, PaniniView};

pub struct PaniniViewPlugin;

impl Plugin for PaniniViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(rc_panini::PaniniPlugin).add_systems(
            Update,
            bend_view
                .after(place_camera)
                .in_set(GameSet::View)
                .run_if(in_state(GameState::InGame)),
        );
    }
}

fn bend_view(
    mut commands: Commands,
    settings: Res<Settings>,
    window: Query<&Window>,
    mut view: Query<(Entity, &mut Projection, Option<&PaniniView>), With<PlayerCamera>>,
    log: Res<GameLog>,
    mut logged: Local<Option<(f32, f32)>>,
) {
    let Ok((e, mut proj, has)) = view.single_mut() else {
        return;
    };
    let strength = settings.video.panini.clamp(0.0, 1.0);
    let Projection::Perspective(p) = &mut *proj else {
        return;
    };
    let aspect = window
        .iter()
        .next()
        .map(|w| w.width() / w.height().max(1.0))
        .unwrap_or(16.0 / 9.0);
    if strength <= 0.0 {
        if has.is_some() {
            commands.entity(e).remove::<PaniniView>();
        }
        return;
    }
    // `place_camera` set the normal up-and-down angle this frame.
    let fit = Fit::new(p.fov, aspect, strength);
    p.fov = fit.source_vertical_fov;
    if has != Some(&fit.view) {
        commands.entity(e).insert(fit.view);
    }
    if *logged != Some((aspect, strength)) {
        *logged = Some((aspect, strength));
        log.line(format!(
            "panini: strength {strength:.2} at aspect {aspect:.3}: drawn {:.1} degrees up and down, {:.1} seen across, middle keeps {:.0}% of its detail",
            fit.source_vertical_fov.to_degrees(),
            fit.seen_across_degrees,
            fit.middle_detail * 100.0
        ));
    }
}
