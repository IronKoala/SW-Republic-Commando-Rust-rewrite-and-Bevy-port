//! Which characters are placed straight in each map (not made by a
//! spawner), and which enemy file makes each one. For every map given (or
//! every map in `GameData/Maps` when none is), reads the placed characters
//! the same way the game does when the map loads (`rc_enemies::
//! placed_enemies`). No window. Needs the game files:
//!
//! `RC_GAME_DIR=<game folder> cargo run --release -p rc_enemies --example rc_placed_enemies [map ...]`
//!
//! Prints each placed enemy the game makes (with its tag, script and
//! events), the ones left to other parts, and totals. Writes
//! `logs/placed_enemies.log`.

use rc_ai::params::AiParams;
use rc_package::Package;
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;

fn main() {
    let Ok(game) = std::env::var("RC_GAME_DIR") else {
        eprintln!("set RC_GAME_DIR to the game folder");
        std::process::exit(2);
    };
    let game_data = std::path::Path::new(&game).join("GameData");
    let maps_dir = game_data.join("Maps");
    let mut maps: Vec<String> = std::env::args().skip(1).collect();
    if maps.is_empty() {
        maps = std::fs::read_dir(&maps_dir)
            .expect("can't read GameData/Maps")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.to_ascii_lowercase().ends_with(".ctm"))
            .collect();
        maps.sort();
    }
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
    // The game puts battle_droid first; the rest by name.
    types.sort_by(|a, b| {
        (a.kind.id != "battle_droid", &a.kind.id).cmp(&(b.kind.id != "battle_droid", &b.kind.id))
    });
    let (classes, notes) = rc_enemies::read_class_library(&game_data);
    for n in notes {
        eprintln!("classes: {n}");
    }
    std::fs::create_dir_all("logs").expect("can't create logs");
    let mut log = std::fs::File::create("logs/placed_enemies.log").expect("can't create the log");
    let mut line = |s: String| {
        println!("{s}");
        writeln!(log, "{s}").ok();
    };
    let mut total: BTreeMap<String, (u32, Vec<String>)> = BTreeMap::new();
    let mut made = 0;
    let mut maps_with = 0;
    for map in &maps {
        let file = if map.to_ascii_lowercase().ends_with(".ctm") {
            map.clone()
        } else {
            format!("{map}.ctm")
        };
        let path = maps_dir.join(&file);
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let read = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|data| {
                let pkg = Package::parse(&data).map_err(|e| e.to_string())?;
                rc_enemies::placed_enemies(&pkg, &data, &classes, &types)
            });
        let (placed, notes) = match read {
            Ok(r) => r,
            Err(e) => {
                line(format!("{name}: can't read: {e}"));
                continue;
            }
        };
        if placed.is_empty() && notes.is_empty() {
            continue;
        }
        line(format!("{name}: {} placed enemies made", placed.len()));
        if !placed.is_empty() {
            maps_with += 1;
        }
        for (f, t) in &placed {
            made += 1;
            let o = &f.offspring;
            line(format!(
                "  {} ({}) -> {}: at ({:.0}, {:.0}, {:.0}), tag {}{}{}{}, starts {:?}",
                f.name,
                f.prototype,
                t.kind.id,
                f.location.x,
                f.location.y,
                f.location.z,
                o.tag.as_deref().unwrap_or("-"),
                o.script
                    .as_ref()
                    .map(|s| format!(", script {s}"))
                    .unwrap_or_default(),
                o.event
                    .as_ref()
                    .map(|e| format!(", death event {e}"))
                    .unwrap_or_default(),
                o.first_damaged_event
                    .as_ref()
                    .map(|e| format!(", first-hurt event {e}"))
                    .unwrap_or_default(),
                o.start_state,
            ));
            let e = total.entry(f.prototype.clone()).or_default();
            e.0 += 1;
            if !e.1.contains(&name) {
                e.1.push(name.clone());
            }
        }
        for n in notes {
            line(format!("  {n}"));
        }
    }
    line(format!(
        "total: {made} placed enemies made on {maps_with} maps"
    ));
    for (class, (n, maps)) in total {
        line(format!("  {class}: {n} on {}", maps.join(", ")));
    }
}
