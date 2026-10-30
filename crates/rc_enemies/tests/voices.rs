//! With the real game (RC_GAME_DIR set to the install, or a copy of its
//! GameData/Sounds folder): every line each enemy file names is found in
//! its sound package, and each group holds at least one sound.

use bevy::audio::AudioSource;
use bevy::prelude::*;
use rc_ai::AiParams;
use rc_game_core::GamePaths;

#[test]
fn real_game_enemy_voices() {
    let Some(game) = std::env::var_os("RC_GAME_DIR") else {
        eprintln!("RC_GAME_DIR not set: skipped (not tested)");
        return;
    };
    let paths = GamePaths::new(game, vec![]);
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/enemies");
    let mut audio = Assets::<AudioSource>::default();
    let mut files = 0;
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_none_or(|x| x != "toml") {
            continue;
        }
        let params = AiParams::battle_droid()
            .with_overrides(&std::fs::read_to_string(&p).unwrap())
            .unwrap();
        let s = &params.sounds;
        let mut names: Vec<String> = s
            .spotted
            .iter()
            .chain(&s.hurt)
            .chain(&s.death)
            .chain(s.by_kind.values().flatten())
            .cloned()
            .collect();
        names.sort();
        names.dedup();
        assert!(!names.is_empty(), "{}: no lines", p.display());
        let (found, notes) = rc_combat::read_sounds(&paths, &s.package, &names, &mut audio);
        assert!(notes.is_empty(), "{}: {notes:?}", p.display());
        assert_eq!(found.len(), names.len(), "{}", p.display());
        // Every line belongs to this character (same prefix, e.g. BD1_).
        let prefix = &s.spotted[0][..4];
        assert!(names.iter().all(|n| n.starts_with(prefix)), "{names:?}");
        let lines: usize = found.values().map(|g| g.variants.len()).sum();
        eprintln!(
            "{}: {} groups, {lines} lines from {}",
            params.kind.id,
            found.len(),
            s.package
        );
        files += 1;
    }
    assert!(files >= 4);
}
