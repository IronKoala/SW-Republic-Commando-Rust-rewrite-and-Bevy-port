//! Level scripts switch order markers on and off ([`MarkerSwitch`]):
//! `ACTION_GotoState` on a marker (`Enabled`, `Disabled`, `Auto`), a
//! marker's anchor changing state (a charge, console or door lock making
//! its marker obsolete, or enabling it again), and
//! `ACTION_SetCanCancelMarker`. The rules are in
//! [`rc_squad::Squad::switch_marker`]. Switches sent before the squad is
//! in the level (at the level's start) wait for it.

use crate::{log_line, SquadState};
use bevy::prelude::*;
use rc_game_core::{GameLog, MarkerChange, MarkerSwitch};

/// Switches waiting for the squad.
#[derive(Resource, Default)]
pub struct PendingSwitches(pub Vec<MarkerSwitch>);

pub(crate) fn switch_markers(
    mut state: ResMut<SquadState>,
    mut asks: MessageReader<MarkerSwitch>,
    mut pending: ResMut<PendingSwitches>,
    log: Option<Res<GameLog>>,
) {
    pending.0.extend(asks.read().cloned());
    let Some(squad) = state.squad.as_mut() else {
        return;
    };
    for s in std::mem::take(&mut pending.0) {
        let mut events = Vec::new();
        let busy = s.only_if_free && squad.order_marker_busy(&s.marker);
        let n = match s.change {
            MarkerChange::On => squad.switch_marker(&s.marker, true, s.only_if_free, &mut events),
            MarkerChange::Off => squad.switch_marker(&s.marker, false, s.only_if_free, &mut events),
            MarkerChange::CanCancel(can) => squad.set_marker_can_cancel(&s.marker, can),
        };
        let what = match s.change {
            MarkerChange::On => "switched on".to_string(),
            MarkerChange::Off => "switched off".to_string(),
            MarkerChange::CanCancel(true) => "can be cancelled once under way".to_string(),
            MarkerChange::CanCancel(false) => "can't be cancelled once under way".to_string(),
        };
        if n == 0 {
            log_line(
                &log,
                format!(
                    "{}: squad marker {} {what}: not a marker the squad has",
                    s.by, s.marker
                ),
            );
            continue;
        }
        if busy {
            log_line(
                &log,
                format!(
                    "{}: squad marker {} not {what}: a squad mate is on it",
                    s.by, s.marker
                ),
            );
            continue;
        }
        log_line(&log, format!("{}: squad marker {} {what}", s.by, s.marker));
        for e in events {
            log_line(&log, format!("squad: {e}"));
        }
    }
}

/// A new level: switches from the last one are dropped.
pub(crate) fn forget(mut pending: ResMut<PendingSwitches>) {
    pending.0.clear();
}
