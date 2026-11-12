//! With the real game (RC_GAME_DIR set to the folder holding GameData):
//! geo_05c's eight proximity mines are read with the class's values
//! (CTMarkers.u `ProximityMine`), each on its barrel. Skipped without it.

use rc_level::class_look::{ClassLooks, Loaded};
use rc_package::Package;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

#[test]
fn geo_05c_mines_have_the_class_values() {
    let Some(game) = std::env::var_os("RC_GAME_DIR").map(PathBuf::from) else {
        eprintln!("RC_GAME_DIR not set: skipped (not tested)");
        return;
    };
    let mut files = HashMap::new();
    for dir in ["System", "Properties"] {
        for e in std::fs::read_dir(game.join("GameData").join(dir))
            .unwrap()
            .flatten()
        {
            let p = e.path();
            if let Some(stem) = p.file_stem() {
                files.insert(stem.to_string_lossy().to_ascii_lowercase(), p);
            }
        }
    }
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
    let data = std::fs::read(game.join("GameData").join("Maps").join("geo_05c.ctm")).unwrap();
    let pkg = Package::parse(&data).unwrap();
    let level = rc_level::actors::read_level(&pkg, &data).unwrap();
    let mut looks = ClassLooks::new();
    let mines = rc_level::mines::read(&mut looks, &pkg, &data, &level, &mut open);
    assert_eq!(mines.len(), 8);
    for m in &mines {
        eprintln!("{m:?}");
        assert!(
            m.prop_to_destroy
                .as_deref()
                .is_some_and(|p| p.starts_with("ExplosiveBarrelCORE")),
            "{}",
            m.name
        );
        assert!(m
            .squad_marker
            .as_deref()
            .is_some_and(|p| p.starts_with("MarkerSetProximityMine")));
        assert_eq!(m.trigger_radius, 512.0);
        assert_eq!(m.alert_time_limit, 1.5);
        assert_eq!(m.placed.duration, 1.0);
        assert_eq!(m.armed.duration, 3.0);
        assert_eq!(m.explode.damage, 256.0);
        assert_eq!(m.explode.damage_radius, 256.0);
        assert_eq!(m.explode.effect.as_deref(), Some("Explosion_Sparks"));
        assert_eq!(m.explode.damage_type.as_deref(), Some("CTDamageExplosion"));
        let join = |p: &Option<Vec<String>>| p.as_ref().map(|p| p.join("."));
        assert_eq!(
            join(&m.alert_sound).as_deref(),
            Some("GEN_Sound.Interface.int_GEN_proxMineChirp_01")
        );
        assert_eq!(
            join(&m.activate_sound).as_deref(),
            Some("GEN_Sound.Interface.int_GEN_proxMineFull_01")
        );
        assert_eq!(
            join(&m.placed.sound_path).as_deref(),
            Some("GEN_Sound.Impacts_Explos.imp_GEN_proxMineSet_01")
        );
        assert_eq!(
            join(&m.armed.sound_path).as_deref(),
            Some("GEN_Sound.Interface.int_GEN_ProxMineArm_01")
        );
        assert_eq!(
            join(&m.explode.sound_path).as_deref(),
            Some("GEN_Sound.Impacts_Explos.exp_GEN_proxMine_01")
        );
        assert_eq!(
            join(&m.armed.material_path).as_deref(),
            Some("MarkerIconTextures.SetTrap.ProximityMine_Active")
        );
        assert_eq!(
            join(&m.mesh_path).as_deref(),
            Some("MarkerIcons.SetTrap.ProximityMine")
        );
    }
}
