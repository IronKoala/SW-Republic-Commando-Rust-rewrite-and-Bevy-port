//! With the real game (RC_GAME_DIR set to the folder holding GameData):
//! the broken barrel model comes from the class unless the map sets the
//! whole broken step itself. geo_01b's barrels set it with no model;
//! geo_03a's keep the class's GEO_EplBarrel_Dest. Skipped without it.

use rc_level::class_look::{ClassLooks, Loaded};
use rc_package::Package;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn packages(game: &Path) -> HashMap<String, PathBuf> {
    let mut out = HashMap::new();
    for dir in ["System", "Properties"] {
        for e in std::fs::read_dir(game.join("GameData").join(dir))
            .unwrap()
            .flatten()
        {
            let p = e.path();
            if let Some(stem) = p.file_stem() {
                out.insert(stem.to_string_lossy().to_ascii_lowercase(), p);
            }
        }
    }
    out
}

fn barrels(game: &Path, map: &str) -> Vec<rc_level::breakables::Breakable> {
    all_breakables(game, map)
        .into_iter()
        .filter(|b| b.class == "ExplosiveBarrelGEO")
        .collect()
}

fn all_breakables(game: &Path, map: &str) -> Vec<rc_level::breakables::Breakable> {
    let files = packages(game);
    let mut cache: HashMap<String, Option<Loaded>> = HashMap::new();
    let mut open = |n: &str| -> Option<Loaded> {
        let key = n.to_ascii_lowercase();
        cache
            .entry(key.clone())
            .or_insert_with(|| {
                let data = std::fs::read(files.get(&key)?).ok()?;
                let pkg = Package::parse(&data).ok()?;
                Some(Rc::new((pkg, data)))
            })
            .clone()
    };
    let data = std::fs::read(
        game.join("GameData")
            .join("Maps")
            .join(format!("{map}.ctm")),
    )
    .unwrap();
    let pkg = Package::parse(&data).unwrap();
    let level = rc_level::actors::read_level(&pkg, &data).unwrap();
    let mut looks = ClassLooks::new();
    rc_level::breakables::read(&mut looks, &pkg, &data, &level, &mut open)
}

/// geo_04c's machine parts have no health, but a level script switches
/// them to `Damagable`, so they are read (not breakable at the start);
/// Geo4MachinePart1 sends `TurretDestroyed` when it breaks.
#[test]
fn script_switched_props_are_read() {
    let Some(game) = std::env::var_os("RC_GAME_DIR").map(PathBuf::from) else {
        eprintln!("RC_GAME_DIR not set: skipped (not tested)");
        return;
    };
    let all = all_breakables(&game, "geo_04c");
    let parts: Vec<_> = all
        .iter()
        .filter(|b| b.class.starts_with("Geo4MachinePart"))
        .collect();
    assert_eq!(
        parts.len(),
        6,
        "{:?}",
        parts.iter().map(|b| &b.name).collect::<Vec<_>>()
    );
    assert!(parts.iter().all(|b| !b.starts_damagable && b.health == 0.0));
    assert!(parts
        .iter()
        .any(|b| b.broken_event() == Some("TurretDestroyed")));
    // Props with health still start breakable.
    assert!(all.iter().any(|b| b.starts_damagable));
}

#[test]
fn broken_barrel_models_follow_the_map() {
    let Some(game) = std::env::var_os("RC_GAME_DIR").map(PathBuf::from) else {
        eprintln!("RC_GAME_DIR not set: skipped (not tested)");
        return;
    };
    let b01 = barrels(&game, "geo_01b");
    assert!(!b01.is_empty());
    for b in &b01 {
        assert_eq!(b.destroyed.mesh_path, None, "{}", b.name);
        assert_eq!(b.destroyed.effect.as_deref(), Some("Explosion_Barrel_GEO"));
        assert_eq!(
            b.destroyed
                .sound_path
                .as_ref()
                .and_then(|p| p.last())
                .map(|s| s.as_str()),
            Some("exp_GEN_barrel_01")
        );
    }
    let b03 = barrels(&game, "geo_03a");
    let with_model = b03
        .iter()
        .filter(|b| {
            b.destroyed.mesh_path.as_deref()
                == Some(
                    &[
                        "MarkerIcons".to_string(),
                        "SetTrap".into(),
                        "GEO_EplBarrel_Dest".into(),
                    ][..],
                )
        })
        .count();
    assert!(
        with_model >= 5,
        "{with_model} of {} geo_03a barrels keep the broken model",
        b03.len()
    );
}
