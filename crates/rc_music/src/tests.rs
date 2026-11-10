//! The director's rules on a small made-up library (no game files needed).

use super::*;
use std::sync::Arc;

fn piece(name: &str, looping: bool, fade_in: f32, fade_out: f32) -> Entry {
    Entry::Piece(Piece {
        name: name.into(),
        file: name.to_ascii_lowercase(),
        looping,
        fade_in: Fade {
            seconds: fade_in,
            ..Fade::NONE
        },
        fade_out: Fade {
            seconds: fade_out,
            ..Fade::NONE
        },
        volume: (1.0, 1.0),
        likelihood: 1.0,
    })
}

fn group(
    name: &str,
    members: &[&str],
    first: Option<&str>,
    won: Option<&str>,
    lost: Option<&str>,
) -> Entry {
    Entry::Group(Group {
        name: name.into(),
        members: members.iter().map(|s| s.to_string()).collect(),
        first: first.map(Into::into),
        won: won.map(Into::into),
        lost: lost.map(Into::into),
        likelihood: 1.0,
    })
}

fn library() -> Arc<MusicLibrary> {
    let mut lib = MusicLibrary::default();
    for e in [
        piece("suspense_lp", true, 5.0, 7.0),
        piece("battle_lp", true, 0.0, 1.5),
        piece("victory_ep", false, 1.5, 3.0),
        piece("incap_lp", true, 3.0, 5.0),
        piece("ol_a", false, 0.0, 0.0),
        piece("ol_b", false, 0.0, 0.0),
        piece("stinger_ep", false, 0.0, 0.0),
        group(
            "battlemulti",
            &["battle_lp"],
            None,
            Some("victory_ep"),
            Some("death_ep"),
        ),
        group("megamulti", &["battlemulti"], None, None, Some("incap_lp")),
        group("room_ols", &["ol_b"], Some("ol_a"), None, None),
    ] {
        let name = match &e {
            Entry::Piece(p) => p.name.clone(),
            Entry::Group(g) => g.name.clone(),
        };
        lib.entries.insert(name.to_ascii_lowercase(), e);
    }
    Arc::new(lib)
}

fn battle() -> BattleSettings {
    BattleSettings {
        music: Some("megamulti".into()),
        threshold: 1,
        inner: 1000.0,
        outer: 2000.0,
        criteria: Criteria::Proximity,
    }
}

fn plays(cmds: &[Command]) -> Vec<(Layer, String)> {
    cmds.iter()
        .filter_map(|c| match c {
            Command::Play { layer, piece, .. } => Some((*layer, piece.name.clone())),
            _ => None,
        })
        .collect()
}

const ME: Option<[f32; 3]> = Some([0.0, 0.0, 0.0]);

fn enemy(x: f32, engaged: bool) -> Threat {
    Threat {
        at: [x, 0.0, 0.0],
        engaged,
    }
}

#[test]
fn level_starts_silent() {
    let mut m = Imuse::new(library(), battle(), Tuning::default(), 1);
    m.update(1.0, &[], ME, false);
    assert!(m.take_commands().is_empty());
}

#[test]
fn script_state_fades_with_the_pieces_own_times() {
    let mut m = Imuse::new(library(), battle(), Tuning::default(), 1);
    m.set_state(Some("suspense_lp"), None);
    let c = m.take_commands();
    match &c[0] {
        Command::Play {
            layer,
            piece,
            fade_in,
            ..
        } => {
            assert_eq!(*layer, Layer::Main);
            assert_eq!(piece.name, "suspense_lp");
            assert_eq!(fade_in.seconds, 5.0);
        }
        other => panic!("{other:?}"),
    }
    // Silence: the state fades out over its own fade-out time.
    m.set_state(None, None);
    assert_eq!(
        m.take_commands(),
        vec![Command::Stop {
            layer: Layer::Main,
            fade: Fade {
                seconds: 7.0,
                ..Fade::NONE
            }
        }]
    );
}

#[test]
fn script_fade_override_wins() {
    let mut m = Imuse::new(library(), battle(), Tuning::default(), 1);
    m.set_state(Some("suspense_lp"), None);
    m.take_commands();
    let f = FadeOverride {
        fade_in: Fade::NONE,
        fade_out: Fade {
            seconds: 0.5,
            ..Fade::NONE
        },
    };
    m.set_state(Some("battle_lp"), Some(f));
    match &m.take_commands()[0] {
        Command::Play {
            fade_out_old,
            fade_in,
            ..
        } => {
            assert_eq!(fade_out_old.seconds, 0.5);
            assert_eq!(fade_in.seconds, 0.0);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn enemies_close_start_a_fight_and_its_end_plays_the_flourish() {
    let mut m = Imuse::new(library(), battle(), Tuning::default(), 1);
    m.set_state(Some("suspense_lp"), None);
    m.take_commands();
    // Far away: nothing.
    m.update(1.0, &[enemy(1500.0, true)], ME, false);
    assert!(!m.in_fight());
    // Inside the inner radius: battle loop on the main layer.
    m.update(1.0, &[enemy(900.0, false)], ME, false);
    assert!(m.in_fight());
    assert_eq!(
        plays(&m.take_commands()),
        vec![(Layer::Main, "battle_lp".into())]
    );
    // Between the radii: the fight goes on.
    for _ in 0..10 {
        m.update(1.0, &[enemy(1800.0, true)], ME, false);
    }
    assert!(m.in_fight());
    // Gone: after the grace time, flourish and the room music again.
    for _ in 0..10 {
        m.update(1.0, &[], ME, false);
    }
    assert!(!m.in_fight());
    assert_eq!(
        plays(&m.take_commands()),
        vec![
            (Layer::Episode, "victory_ep".into()),
            (Layer::Main, "suspense_lp".into())
        ]
    );
}

#[test]
fn criteria_attack_needs_engaged_enemies() {
    let mut b = battle();
    b.criteria = Criteria::AttackAndProximity;
    let mut m = Imuse::new(library(), b, Tuning::default(), 1);
    m.update(1.0, &[enemy(500.0, false)], ME, false);
    assert!(!m.in_fight());
    m.update(1.0, &[enemy(500.0, true)], ME, false);
    assert!(m.in_fight());
}

#[test]
fn threshold_counts_enemies() {
    let mut b = battle();
    b.threshold = 3;
    let mut m = Imuse::new(library(), b, Tuning::default(), 1);
    m.update(1.0, &[enemy(500.0, true), enemy(600.0, true)], ME, false);
    assert!(!m.in_fight());
    m.update(
        1.0,
        &[enemy(500.0, true), enemy(600.0, true), enemy(700.0, true)],
        ME,
        false,
    );
    assert!(m.in_fight());
}

#[test]
fn battle_engine_off_means_no_fight_music() {
    let mut m = Imuse::new(library(), battle(), Tuning::default(), 1);
    m.enable_battle(false);
    m.update(1.0, &[enemy(10.0, true)], ME, false);
    assert!(!m.in_fight());
    // Switching off mid-fight ends it without a flourish.
    m.enable_battle(true);
    m.update(1.0, &[enemy(10.0, true)], ME, false);
    m.take_commands();
    m.enable_battle(false);
    assert!(!plays(&m.take_commands())
        .iter()
        .any(|(l, _)| *l == Layer::Episode));
}

#[test]
fn forced_fight_lasts_until_the_flourish() {
    let mut m = Imuse::new(library(), battle(), Tuning::default(), 1);
    m.force_battle();
    for _ in 0..20 {
        m.update(1.0, &[], ME, false);
    }
    assert!(m.in_fight());
    m.force_victory();
    assert!(!m.in_fight());
    assert!(plays(&m.take_commands()).contains(&(Layer::Episode, "victory_ep".into())));
}

#[test]
fn player_down_plays_the_megamultis_lost_music() {
    let mut m = Imuse::new(library(), battle(), Tuning::default(), 1);
    m.update(1.0, &[enemy(10.0, true)], ME, false);
    m.take_commands();
    m.update(1.0, &[enemy(10.0, true)], ME, true);
    assert_eq!(
        plays(&m.take_commands()),
        vec![(Layer::Main, "incap_lp".into())]
    );
    m.update(1.0, &[enemy(10.0, true)], ME, false);
    assert_eq!(
        plays(&m.take_commands()),
        vec![(Layer::Main, "battle_lp".into())]
    );
}

#[test]
fn room_overlays_start_with_first_sound_then_wait_their_spacing() {
    let mut m = Imuse::new(library(), battle(), Tuning::default(), 1);
    let room = RoomMusic {
        class: "Room".into(),
        state: Some("suspense_lp".into()),
        overlays: Some("room_ols".into()),
        startup_delay: 5.0,
        min_spacing: 2.0,
        max_spacing: 4.0,
        once_only: true,
    };
    m.enter_room("PhysicsVolume1", &room);
    assert_eq!(
        plays(&m.take_commands()),
        vec![(Layer::Main, "suspense_lp".into())]
    );
    m.update(4.0, &[], ME, false);
    assert!(m.take_commands().is_empty());
    m.update(1.5, &[], ME, false);
    assert_eq!(
        plays(&m.take_commands()),
        vec![(Layer::Overlay, "ol_a".into())]
    );
    m.finished(Layer::Overlay);
    m.update(1.0, &[], ME, false);
    assert!(m.take_commands().is_empty());
    for _ in 0..4 {
        m.update(1.0, &[], ME, false);
    }
    assert_eq!(
        plays(&m.take_commands()),
        vec![(Layer::Overlay, "ol_b".into())]
    );
    // Once only: walking in again changes nothing.
    m.enter_room("PhysicsVolume1", &room);
    assert!(m.take_commands().is_empty());
}

#[test]
fn state_set_during_a_fight_waits_for_its_end() {
    let mut m = Imuse::new(library(), battle(), Tuning::default(), 1);
    m.force_battle();
    m.take_commands();
    m.set_state(Some("suspense_lp"), None);
    assert!(plays(&m.take_commands()).is_empty());
    m.force_victory();
    assert!(plays(&m.take_commands()).contains(&(Layer::Main, "suspense_lp".into())));
}

#[test]
fn one_shot_state_does_not_come_back_after_a_fight() {
    let mut m = Imuse::new(library(), battle(), Tuning::default(), 1);
    m.set_state(Some("stinger_ep"), None);
    m.finished(Layer::Main);
    m.force_battle();
    m.force_victory();
    assert!(!plays(&m.take_commands())
        .iter()
        .any(|(_, n)| n == "stinger_ep" && m.in_fight()));
    assert!(m.playing(Layer::Main).is_none());
}

#[test]
fn fade_shapes_run_from_zero_to_one() {
    for s in [
        FadeShape::Linear,
        FadeShape::Sine,
        FadeShape::Log,
        FadeShape::LogInverse,
    ] {
        assert!(s.gain(0.0).abs() < 1e-5, "{s:?}");
        assert!((s.gain(1.0) - 1.0).abs() < 1e-5, "{s:?}");
    }
    assert!(FadeShape::Log.gain(0.2) > 0.2);
    assert!(FadeShape::LogInverse.gain(0.2) < 0.2);
}
