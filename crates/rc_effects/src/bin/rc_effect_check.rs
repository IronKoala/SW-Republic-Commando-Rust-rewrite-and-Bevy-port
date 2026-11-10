//! `rc_effect_check`: reads every particle effect in the game and says how
//! many could be read, or prints one effect in full.
//!
//! ```text
//! rc_effect_check                          (every effect, a summary)
//! rc_effect_check Explosion_Barrel_GEO     (one effect, every value)
//! ```
//!
//! The game folder comes from `RC_GAME_DIR` (the folder holding
//! `GameData`). Read only; it writes nothing.

use rc_effects::def::EmitterKind;
use rc_effects::read::Library;
use std::path::PathBuf;

fn main() {
    let Some(game) = std::env::var_os("RC_GAME_DIR").map(PathBuf::from) else {
        eprintln!("Set RC_GAME_DIR to the game folder (the one holding GameData).");
        std::process::exit(2);
    };
    let lib = match Library::open(&game.join("GameData")) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    if let Some(name) = std::env::args().nth(1) {
        match lib.effect(&name) {
            Ok(e) => println!("{e:#?}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let (mut ok, mut failed, mut emitters, mut sprites, mut meshes, mut other) = (0, 0, 0, 0, 0, 0);
    let mut empty = 0;
    for p in &lib.packages {
        for c in p.classes() {
            match p.effect(&c) {
                Ok(e) => {
                    ok += 1;
                    if e.emitters.is_empty() {
                        empty += 1;
                    }
                    for m in &e.emitters {
                        emitters += 1;
                        match m.kind {
                            EmitterKind::Sprite => sprites += 1,
                            EmitterKind::Mesh(_) => meshes += 1,
                            _ => other += 1,
                        }
                    }
                }
                Err(e) => {
                    failed += 1;
                    println!("FAILED {}: {e}", p.name);
                }
            }
        }
    }
    println!(
        "effects read: {ok}, failed: {failed}, with no emitters: {empty}; emitters: {emitters} ({sprites} sprite, {meshes} mesh, {other} other)"
    );
    if failed > 0 {
        std::process::exit(1);
    }
}
