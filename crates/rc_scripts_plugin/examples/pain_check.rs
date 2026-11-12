//! Reads every single-player map's pain volumes (pits, crushers, fires)
//! with the game's own classes, the way the game does while a level loads,
//! and checks the rules that matter most against the game's files.
//!
//! `RC_GAME_DIR=<game folder> cargo run -p rc_scripts_plugin --example pain_check`
//!
//! Prints one line per volume, then `PAIN CHECK: N passed, M failed`.
//! Without `RC_GAME_DIR` it prints `skipped: no game folder`.

use rc_combat::DamageKind;
use rc_game_core::{GameLog, GamePaths};
use rc_scripts_plugin::pain;

fn main() {
    let Some(game) = std::env::var_os("RC_GAME_DIR") else {
        println!("skipped: no game folder (set RC_GAME_DIR)");
        return;
    };
    let paths = GamePaths::new(game, Vec::new());
    let log = GameLog::terminal();
    let mut cache = pain::PainClasses::default();
    let lib = pain::classes(&paths, &mut cache, &log);
    let mut maps: Vec<String> = std::fs::read_dir(paths.game_data().join("Maps"))
        .map(|r| {
            r.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.to_ascii_lowercase().ends_with(".ctm"))
                // Single-player maps only (no ctf_/dm_ multiplayer).
                .filter(|n| !n.starts_with("ctf_") && !n.starts_with("dm_"))
                .collect()
        })
        .unwrap_or_default();
    maps.sort();
    let (mut pass, mut fail) = (0, 0);
    let mut check = |ok: bool, what: String| {
        if ok {
            pass += 1;
        } else {
            fail += 1;
            println!("FAIL: {what}");
        }
    };
    let (mut total, mut with) = (0, 0);
    for m in &maps {
        let path = paths.map_path(&m[..m.len() - 4]);
        let read = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|d| {
                let pkg = rc_package::Package::parse(&d).map_err(|e| e.to_string())?;
                let level = rc_level::actors::read_level(&pkg, &d).map_err(|e| e.to_string())?;
                Ok((d, pkg, level))
            });
        let (data, pkg, level) = match read {
            Ok(r) => r,
            Err(e) => {
                check(false, format!("{m}: not read ({e})"));
                continue;
            }
        };
        let p = pain::read(&pkg, &data, &level, &lib, &log);
        total += p.volumes.len();
        with += (!p.volumes.is_empty()) as usize;
        for lv in &p.volumes {
            let v = &lv.volume;
            let c = p.damage_class(&v.damage_type);
            let ty = v.damage_type.to_ascii_lowercase();
            if ty == "damagefalltodeath" {
                check(
                    c.kind == DamageKind::Fall
                        && c.rules.bypass_shields
                        && c.rules.kills
                        && c.rules.kills_player,
                    format!("{m} {}: DamageFallToDeath should go past shields and kill everyone outright ({c:?})", v.name),
                );
            }
            if ty == "crushed" {
                check(
                    !c.rules.bypass_shields && !c.rules.kills && c.rules.kills_player,
                    format!(
                        "{m} {}: Crushed should kill only the player outright ({c:?})",
                        v.name
                    ),
                );
            }
            check(
                !v.shape.polygons.is_empty(),
                format!("{m} {}: has a shape", v.name),
            );
            if let Some(cls) = &v.proximity {
                check(
                    lib.has_class(cls),
                    format!("{m} {}: its class {cls} is one of the game's", v.name),
                );
            }
        }
    }
    // Who the class filter lets through, by the game's own class tree.
    let chain = |c: &str| {
        lib.chain(c)
            .iter()
            .map(|x| x.to_ascii_lowercase())
            .collect::<Vec<_>>()
    };
    check(
        chain("PlayerCommando").contains(&"clonecommando".into()),
        "the player is a CloneCommando (CloneCommando pits hurt the player)".into(),
    );
    check(
        chain("Commando62Delta").contains(&"clonecommando".into()),
        "Scorch is a CloneCommando (CloneCommando pits hurt the squad)".into(),
    );
    check(
        !chain("BattleDroidTM").contains(&"clonecommando".into())
            && chain("BattleDroidTM").contains(&"pawn".into()),
        "a battle droid is a Pawn but not a CloneCommando".into(),
    );
    check(
        total == 60,
        format!("60 pain volumes over the single-player maps (found {total})"),
    );
    println!("{total} pain volumes in {with} of {} maps", maps.len());
    println!("PAIN CHECK: {pass} passed, {fail} failed");
}
