//! `rc_effect_check`: reads every particle effect in the game and says how
//! many could be read, or prints one effect in full.
//!
//! ```text
//! rc_effect_check                          (every effect, a summary)
//! rc_effect_check Explosion_Barrel_GEO     (one effect, every value)
//! rc_effect_check --maps [map ...]         (the effects every map places,
//!                                           or the named ones: counts, how
//!                                           many edited in the map, any
//!                                           that can't be read)
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
    if std::env::args().nth(1).as_deref() == Some("--maps") {
        maps(&lib, &game.join("GameData"));
        return;
    }
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

/// The effects the maps place, read as the game does (the map's own
/// copies of each effect's emitters).
fn maps(lib: &Library, data: &std::path::Path) {
    let mut names: Vec<String> = std::env::args().skip(2).collect();
    if names.is_empty() {
        let mut all: Vec<String> = std::fs::read_dir(data.join("Maps"))
            .map(|d| {
                d.flatten()
                    .filter_map(|e| {
                        let p = e.path();
                        if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ctm")) {
                            return None;
                        }
                        Some(p.file_stem()?.to_string_lossy().into_owned())
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Single-player only.
        all.retain(|n| {
            let n = n.to_ascii_lowercase();
            !(n.starts_with("ctf_") || n.starts_with("dm_") || n == "mpcustom")
        });
        all.sort();
        names = all;
    }
    let (mut total, mut edited, mut own, mut failed, mut hidden, mut start_off, mut maps_with) =
        (0, 0, 0, 0, 0, 0, 0);
    let mut kinds = std::collections::BTreeSet::new();
    for name in &names {
        let path = data.join("Maps").join(format!("{name}.ctm"));
        let Ok(bytes) = std::fs::read(&path) else {
            println!("{name}: can't read {}", path.display());
            failed += 1;
            continue;
        };
        let Ok(pkg) = rc_package::Package::parse(&bytes) else {
            println!("{name}: not a package");
            failed += 1;
            continue;
        };
        let list = lib.placed_effects(&pkg, name, &bytes);
        if !list.is_empty() {
            maps_with += 1;
        }
        let mut map_edited = 0;
        for p in &list {
            total += 1;
            kinds.insert(p.class.to_ascii_lowercase());
            if p.class.eq_ignore_ascii_case("Emitter") {
                own += 1;
            }
            if p.changed_from_class.is_some_and(|n| n > 0) {
                edited += 1;
                map_edited += 1;
            }
            if p.hidden {
                hidden += 1;
            }
            match &p.def {
                Ok(d) => {
                    if d.emitters.iter().any(|e| e.disabled) {
                        start_off += 1;
                    }
                }
                Err(e) => {
                    failed += 1;
                    println!("FAILED {name} {} ({}): {e}", p.name, p.class);
                }
            }
        }
        println!(
            "{name}: {} placed effects, {map_edited} edited in the map",
            list.len()
        );
    }
    println!(
        "placed effects: {total} in {maps_with} maps, {} kinds; {edited} edited in the map, {own} built in the map alone, {start_off} with parts that start off, {hidden} hidden; failed: {failed}",
        kinds.len()
    );
    if failed > 0 {
        std::process::exit(1);
    }
}
