//! Squad mates and the level's ammo boxes, for the optional squad ammo
//! (`[ammo] limited` in `data/squad/squad.toml`, off by default as in the
//! original; see `rc_squad::ammo`).
//!
//! `gather_boxes` tells the squad where the untaken boxes are, and
//! `take_boxes` lets a squad mate take one they touch, with the same touch
//! test as the player's (`rc_combat::ammo_box::touches`). A taken box is
//! hidden and saved as taken just like one the player takes. Nothing here
//! runs with `limited` off.

use crate::{log_line, SquadState};
use bevy::prelude::*;
use rc_combat::ammo_box::{touches, AmmoBox};
use rc_combat::Hittable;
use rc_game_core::units::{to_bevy, to_unreal};
use rc_game_core::GameLog;
use rc_movement::Vec3 as UVec3;
use rc_squad::SupplyBox;

fn limited(state: &SquadState) -> bool {
    state
        .squad
        .as_ref()
        .is_some_and(|s| s.settings.ammo.limited)
}

fn supply(e: Entity, t: &Transform, b: &AmmoBox) -> Option<SupplyBox> {
    let gives = b.gives.clone()?;
    Some(SupplyBox {
        id: e.to_bits(),
        name: b.name.clone(),
        // The box's transform is the middle of its touch cylinder.
        feet: to_unreal(t.translation) - UVec3::new(0.0, 0.0, b.height),
        gives,
        amount: b.amount,
    })
}

/// The boxes not taken, for the squad to pick from.
pub(crate) fn gather_boxes(
    mut state: ResMut<SquadState>,
    boxes: Query<(Entity, &Transform, &AmmoBox)>,
) {
    if !limited(&state) {
        return;
    }
    let Some(squad) = state.squad.as_mut() else {
        return;
    };
    squad.boxes = boxes
        .iter()
        .filter(|(_, _, b)| !b.taken)
        .filter_map(|(e, t, b)| supply(e, t, b))
        .collect();
}

/// Squad mates take the boxes they touch, if the box fills something they
/// aren't full of.
pub(crate) fn take_boxes(
    mut state: ResMut<SquadState>,
    mut boxes: Query<(Entity, &Transform, &mut AmmoBox)>,
    log: Option<Res<GameLog>>,
) {
    if !limited(&state) {
        return;
    }
    let Some(squad) = state.squad.as_mut() else {
        return;
    };
    for i in 0..squad.members.len() {
        if squad.members[i].down {
            continue;
        }
        let feet = to_bevy(squad.members[i].pawn.feet());
        for (e, t, mut b) in &mut boxes {
            if b.taken || !touches(feet, &Hittable::COMMANDO, t.translation, b.radius, b.height) {
                continue;
            }
            let Some(s) = supply(e, t, &b) else {
                continue;
            };
            let Some(ev) = squad.take_box(i, &s) else {
                continue;
            };
            b.taken = true;
            b.touching = false;
            b.back_in = b.respawn.unwrap_or(0.0);
            log_line(&log, format!("t={:.2} {ev} (box hidden)", squad.time));
        }
    }
}
