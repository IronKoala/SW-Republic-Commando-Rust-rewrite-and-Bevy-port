//! With the real game (RC_GAME_DIR set to the folder holding GameData):
//! every effect reads, the barrel blast reads as we found it, and the
//! broken barrel model is where each map says. Skipped without it.

use rc_effects::def::{DrawStyle, EmitterKind};
use rc_effects::read::Library;
use rc_effects::EffectState;
use std::path::PathBuf;

fn game() -> Option<PathBuf> {
    match std::env::var_os("RC_GAME_DIR") {
        Some(g) => Some(PathBuf::from(g)),
        None => {
            eprintln!("RC_GAME_DIR not set: skipped (not tested)");
            None
        }
    }
}

#[test]
fn every_effect_reads_and_the_barrel_blast_is_as_found() {
    let Some(game) = game() else { return };
    let lib = Library::open(&game.join("GameData")).unwrap();
    let mut n = 0;
    for p in &lib.packages {
        for c in p.classes() {
            p.effect(&c).unwrap_or_else(|e| panic!("{c}: {e}"));
            n += 1;
        }
    }
    assert!(n >= 500, "only {n} effects");
    let e = lib.effect("Explosion_Barrel_GEO").unwrap();
    assert_eq!(e.emitters.len(), 7);
    assert_eq!(
        e.sound.as_ref().and_then(|s| s.last()).map(|s| s.as_str()),
        Some("exp_barrels")
    );
    let fire = e
        .emitters
        .iter()
        .find(|m| m.name.contains("(Fire)"))
        .unwrap();
    assert_eq!(fire.draw_style, DrawStyle::Brighten);
    assert_eq!((fire.frames_u, fire.frames_v), (5, 5));
    assert_eq!(fire.initial_rate, 1200.0);
    assert!(fire
        .texture
        .as_ref()
        .unwrap()
        .last()
        .unwrap()
        .eq_ignore_ascii_case("explosion_1a"));
    let debris = e
        .emitters
        .iter()
        .find(|m| matches!(m.kind, EmitterKind::Mesh(_)))
        .unwrap();
    assert!(debris.collides && debris.acceleration[2] < 0.0);
    // It runs and ends.
    let mut s = EffectState::new(&e, [0.0; 3], 1);
    s.floor = Some(0.0);
    let mut peak = 0;
    for _ in 0..(60 * 20) {
        s.step(&e, 1.0 / 60.0);
        peak = peak.max(s.live());
    }
    assert!(peak >= 100, "peak {peak}");
    assert!(s.done(&e), "still {} particles after 20 s", s.live());
}
