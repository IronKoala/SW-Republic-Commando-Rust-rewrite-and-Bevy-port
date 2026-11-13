//! The map's `TriggerLight`s switching on and off: a level event named by
//! a light's `Tag` triggers it, and each frame it fades toward on or off
//! by gameplay.u's rules (see `rc_level_viewer::lighting::SwitchedLight`,
//! from the class's script text). geo_01briefing's gunship cabin uses
//! them: the blue lights go out as the blackout box closes, the slit light
//! and the hot light by the door come on, the advisor's light follows the
//! hologram.

use crate::PART;
use bevy::prelude::*;
use rc_game_core::{GameLog, ScriptEvent};
use rc_level_viewer::lighting::SwitchedLight;

pub fn switch(
    time: Res<Time>,
    mut events: MessageReader<ScriptEvent>,
    mut lights: Query<(
        &mut SwitchedLight,
        Option<&mut PointLight>,
        Option<&mut SpotLight>,
    )>,
    log: Res<GameLog>,
) {
    let heard: Vec<String> = events.read().map(|e| e.name.clone()).collect();
    let dt = time.delta_secs();
    for (mut s, point, spot) in &mut lights {
        for name in &heard {
            if s.tag.eq_ignore_ascii_case(name) && s.trigger() {
                log.line(format!(
                    "{PART}: light {} triggered ({}, change time {} s)",
                    s.tag, s.rule.state, s.rule.change_time
                ));
            }
        }
        let before = s.factor;
        s.tick(dt);
        if s.factor == before && dt > 0.0 {
            continue;
        }
        let now = s.full * s.factor;
        if let Some(mut p) = point {
            p.intensity = now;
        }
        if let Some(mut p) = spot {
            p.intensity = now;
        }
    }
}
