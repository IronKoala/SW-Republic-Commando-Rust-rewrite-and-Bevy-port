//! Checks the droidekas and dwarf spider droids against the original's
//! maps, without a window: every Droideka and DwarfSpiderDroid spawner and
//! placed character on geo_04d, geo_05b and geo_05c is claimed by an enemy
//! file, makes its droid when its event comes, and killing that droid sends
//! the event the level's scripts wait for (`SpiderDead`, `IntroDead`,
//! `DroidekaDead00`, ...). It also checks that the intro droideka on
//! geo_05b can play its scripted `DroidekaIntro` and that the animation
//! ends in the `AnimNotify_GoRagdoll` that kills it (so `IntroDead` comes
//! at its end). Needs the game files:
//!
//! `RC_GAME_DIR=<game folder> cargo run --release -p rc_enemies --example rc_heavy_droid_check`
//!
//! Prints PASS/FAIL lines and ends `HEAVY DROID CHECK: N passed, M failed`.

use rc_ai::params::AiParams;
use rc_ai::{Battle, BattleEvent};
use rc_game_core::GamePaths;
use rc_movement::BrushWorld;
use rc_nav::NavGraph;
use rc_package::Package;
use std::sync::Arc;

fn main() {
    let Ok(game) = std::env::var("RC_GAME_DIR") else {
        eprintln!("set RC_GAME_DIR to the game folder");
        std::process::exit(2);
    };
    let game_data = std::path::Path::new(&game).join("GameData");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/enemies");
    let mut types = Vec::new();
    for e in std::fs::read_dir(&dir)
        .expect("can't read data/enemies")
        .flatten()
    {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "toml") {
            let text = std::fs::read_to_string(&p).expect("can't read an enemy file");
            let mut t = AiParams::battle_droid()
                .with_overrides(&text)
                .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            t.kind.id = p.file_stem().unwrap().to_string_lossy().to_string();
            types.push(Arc::new(t));
        }
    }
    types.sort_by(|a, b| {
        (a.kind.id != "battle_droid", &a.kind.id).cmp(&(b.kind.id != "battle_droid", &b.kind.id))
    });
    let (classes, _) = rc_enemies::read_class_library(&game_data);
    let (mut passed, mut failed) = (0, 0);
    let mut check = |ok: bool, what: String| {
        println!("{} {what}", if ok { "PASS" } else { "FAIL" });
        if ok {
            passed += 1
        } else {
            failed += 1
        }
    };
    let heavy = |class: &str| {
        let c = class.to_ascii_lowercase();
        c.starts_with("droideka") || c.starts_with("dwarfspiderdroid")
    };
    // The events the maps' scripts wait for, by map.
    let wanted: [(&str, &[&str]); 3] = [
        ("geo_04d", &["SpiderDead"]),
        ("geo_05b", &["SpiderDead", "IntroDead"]),
        ("geo_05c", &[]),
    ];
    for (map, events) in wanted {
        let path = game_data.join("Maps").join(format!("{map}.ctm"));
        let Ok(data) = std::fs::read(&path) else {
            check(false, format!("{map}: can't read {}", path.display()));
            continue;
        };
        let pkg = Package::parse(&data).expect("map");
        let nav = NavGraph::from_map(&pkg, &data).expect("nav");
        let factories = rc_ai::read_factories(&pkg, &data).expect("factories");
        let (placed, _) =
            rc_enemies::placed_enemies(&pkg, &data, &classes, &types).expect("placed");
        let mut list = Vec::new();
        for f in factories {
            let t = types.iter().find(|t| t.kind.matches(&f.prototype));
            if heavy(&f.prototype) {
                check(
                    t.is_some(),
                    format!(
                        "{map}: spawner {} ({}) is made by {}",
                        f.name,
                        f.prototype,
                        t.map_or("NO FILE", |t| t.kind.id.as_str())
                    ),
                );
            }
            if let Some(t) = t {
                list.push((f, t.clone()));
            }
        }
        for (f, t) in placed {
            if heavy(&f.prototype) {
                check(
                    true,
                    format!(
                        "{map}: placed {} ({}) is made by {}",
                        f.name, f.prototype, t.kind.id
                    ),
                );
            }
            list.push((f, t));
        }
        let mut battle = Battle::with_types(types[0].clone(), nav, list);
        let world = BrushWorld {
            brushes: Vec::new(),
        };
        let mut seen = Vec::new();
        battle.step(0.05, &world, None);
        // Switch on every heavy droid's spawner by its event.
        let tags: Vec<String> = battle
            .spawners
            .iter()
            .filter(|s| heavy(&s.factory.prototype) && !s.factory.placed)
            .map(|s| s.factory.tag.clone())
            .collect();
        for tag in &tags {
            battle.trigger(tag);
        }
        for _ in 0..200 {
            battle.step(0.05, &world, None);
        }
        let ids: Vec<(u32, String)> = battle
            .alive()
            .filter(|f| matches!(f.params.kind.id.as_str(), "droideka" | "dwarf_spider_droid"))
            .map(|f| (f.id, f.params.kind.id.clone()))
            .collect();
        let made = |id: &str| ids.iter().filter(|(_, k)| k == id).count();
        println!(
            "  {map}: made {} droidekas and {} dwarf spider droids",
            made("droideka"),
            made("dwarf_spider_droid")
        );
        check(
            !ids.is_empty() || events.is_empty(),
            format!("{map}: heavy droids were made"),
        );
        for (id, _) in &ids {
            for e in battle.killed(*id) {
                if let BattleEvent::Event { name, .. } = e {
                    seen.push(name);
                }
            }
        }
        println!("  {map}: their deaths sent {seen:?}");
        for e in events {
            check(
                seen.iter().any(|s| s.eq_ignore_ascii_case(e)),
                format!("{map}: a death sends {e}"),
            );
        }
    }
    // The intro droideka's scripted animation, and the notify that kills it.
    let paths = GamePaths::new(&game, Vec::new());
    let droideka = types
        .iter()
        .find(|t| t.kind.id == "droideka")
        .expect("droideka file");
    match rc_enemies::look::DroidLook::load(&paths, &droideka.look) {
        Ok((look, _)) => {
            let clip = look.named_clip("DroidekaIntro");
            check(
                clip.is_some(),
                "droideka: DroidekaIntro is in its animation sets".into(),
            );
            let at = look
                .ragdoll_at
                .get("droidekaintro")
                .cloned()
                .unwrap_or_default();
            check(
                at.iter().any(|&t| t > 0.95),
                format!("droideka: DroidekaIntro ends in AnimNotify_GoRagdoll (at {at:?})"),
            );
        }
        Err(e) => check(false, format!("droideka: model loads ({e})")),
    }
    println!("HEAVY DROID CHECK: {passed} passed, {failed} failed");
    if failed > 0 {
        std::process::exit(1);
    }
}
