//! The level scripts' music actions, turned into [`MusicCue`]s for the
//! music (`rc_game`'s `music.rs`), which follows the game's rules for them.
//!
//! Values a map leaves out keep their action class's default, read from the
//! game's `Gameplay.u`: `ACTION_EnableBattleEngine` `onOff` false;
//! `ACTION_SetBattleEngineParams` `criteria` 2 (attack and proximity),
//! `baddieCount` 1, `radiusInner` 1250, `radiusOuter` 2500.

use rc_game_core::{MusicCue, MusicFades};
use rc_level::scripts::ScriptObject;

/// The script's fade times, if it overrides the music's own.
fn fades(a: &ScriptObject) -> Option<MusicFades> {
    a.flag("OverrideFadeDefaults", false).then(|| MusicFades {
        fade_in: a.float("FadeInTime").unwrap_or(0.0),
        fade_in_shape: a.int("FadeInType").unwrap_or(0) as u8,
        fade_out: a.float("FadeOutTime").unwrap_or(0.0),
        fade_out_shape: a.int("FadeOutType").unwrap_or(0) as u8,
    })
}

/// The music cue for a script action, or `None` if it isn't a music action.
pub fn cue(a: &ScriptObject) -> Option<MusicCue> {
    let kind = a.class.strip_prefix("ACTION_").unwrap_or(&a.class);
    Some(match kind.to_ascii_lowercase().as_str() {
        "setimusestate" => MusicCue::SetState {
            music: a.text("Music").map(str::to_string),
            fades: fades(a),
        },
        "playmusic" => MusicCue::PlayEpisode {
            music: a.text("Music")?.to_string(),
            fades: fades(a),
        },
        "enablebattleengine" => MusicCue::EnableBattle(a.flag("onOff", false)),
        "setbattleengineparams" => MusicCue::BattleParams {
            criteria: a.int("criteria").unwrap_or(2).clamp(0, 2) as u8,
            threshold: a.int("baddieCount").unwrap_or(1).max(0) as u32,
            inner: a.float("radiusInner").unwrap_or(1250.0),
            outer: a.float("radiusOuter").unwrap_or(2500.0),
        },
        "setbattlemusicmulti" => MusicCue::BattleMusic(a.text("multiSound").map(str::to_string)),
        "forcebattlemusic" => MusicCue::ForceBattle,
        "forcevictoryflourish" => MusicCue::ForceVictory,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_level::scripts::Val;

    fn action(class: &str, values: &[(&str, Val)]) -> ScriptObject {
        ScriptObject {
            export: 0,
            class: class.into(),
            name: format!("{class}0"),
            values: values
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        }
    }

    #[test]
    fn music_actions_become_cues_with_the_class_defaults() {
        // From geo_01c: a state change with its own fade-out.
        let a = action(
            "ACTION_SetImuseState",
            &[
                ("Music", Val::Text("musYYY_jwbattle01_lp".into())),
                ("OverrideFadeDefaults", Val::Bool(true)),
                ("FadeOutTime", Val::Float(0.5)),
            ],
        );
        assert_eq!(
            cue(&a),
            Some(MusicCue::SetState {
                music: Some("musYYY_jwbattle01_lp".into()),
                fades: Some(MusicFades {
                    fade_out: 0.5,
                    ..MusicFades::default()
                })
            })
        );
        // No Music: silence.
        assert_eq!(
            cue(&action("ACTION_SetImuseState", &[])),
            Some(MusicCue::SetState {
                music: None,
                fades: None
            })
        );
        // `onOff` left out means off.
        assert_eq!(
            cue(&action("ACTION_EnableBattleEngine", &[])),
            Some(MusicCue::EnableBattle(false))
        );
        assert_eq!(
            cue(&action(
                "ACTION_SetBattleEngineParams",
                &[
                    ("criteria", Val::Byte(1)),
                    ("radiusInner", Val::Float(3000.0))
                ]
            )),
            Some(MusicCue::BattleParams {
                criteria: 1,
                threshold: 1,
                inner: 3000.0,
                outer: 2500.0
            })
        );
        assert_eq!(cue(&action("ACTION_PlaySound", &[])), None);
    }
}
