//! Which enemy file makes each map's spawners. For every map given (or
//! every map in `GameData/Maps` when none is), reads its spawners (the
//! game's pawn factories) and matches each one's class against
//! data/enemies/*.toml `[type] classes`, the same way the game does. No
//! window. Needs the game files:
//!
//! `RC_GAME_DIR=<game folder> cargo run -p rc_enemies --example rc_enemy_spawners [map ...]`
//!
//! Prints, per map, the spawners each enemy type claims and the classes no
//! file claims yet (squad mates and clone troopers belong to other parts),
//! then a total per class. Writes `logs/enemy_spawners.log`.

use rc_ai::params::AiParams;
use rc_package::Package;
use std::collections::BTreeMap;
use std::io::Write;

fn main() {
    let Ok(game) = std::env::var("RC_GAME_DIR") else {
        eprintln!("set RC_GAME_DIR to the game folder");
        std::process::exit(2);
    };
    let maps_dir = std::path::Path::new(&game).join("GameData/Maps");
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
            types.push(t.kind);
        }
    }
    types.sort_by(|a, b| a.id.cmp(&b.id));
    std::fs::create_dir_all("logs").expect("can't create logs");
    let mut log = std::fs::File::create("logs/enemy_spawners.log").expect("can't create the log");
    let mut line = |s: String| {
        println!("{s}");
        writeln!(log, "{s}").ok();
    };
    // class -> (claimed by, spawners, maps)
    let mut total: BTreeMap<String, (String, u32, Vec<String>)> = BTreeMap::new();
    for map in &maps {
        let path = {
            let p = std::path::Path::new(map);
            if p.exists() {
                p.to_path_buf()
            } else {
                let with_ext = if map.to_ascii_lowercase().ends_with(".ctm") {
                    map.clone()
                } else {
                    format!("{map}.ctm")
                };
                maps_dir.join(with_ext)
            }
        };
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let read = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|data| {
                let pkg = Package::parse(&data).map_err(|e| e.to_string())?;
                rc_ai::read_factories(&pkg, &data)
            });
        let factories = match read {
            Ok(f) => f,
            Err(e) => {
                line(format!("{name}: can't read: {e}"));
                continue;
            }
        };
        let mut per: BTreeMap<String, u32> = BTreeMap::new();
        let mut unclaimed: BTreeMap<String, u32> = BTreeMap::new();
        for f in &factories {
            let by = types.iter().find(|t| t.matches(&f.prototype));
            let entry = total
                .entry(f.prototype.clone())
                .or_insert_with(|| (by.map(|t| t.id.clone()).unwrap_or_default(), 0, vec![]));
            entry.1 += 1;
            if !entry.2.contains(&name) {
                entry.2.push(name.clone());
            }
            match by {
                Some(t) => *per.entry(t.id.clone()).or_default() += 1,
                None => *unclaimed.entry(f.prototype.clone()).or_default() += 1,
            }
        }
        line(format!(
            "{name}: {} spawners; by enemy type {per:?}; no enemy file {unclaimed:?}",
            factories.len()
        ));
    }
    line("--- every spawner class ---".into());
    for (class, (by, n, maps)) in &total {
        line(format!(
            "{class}: {n} spawners on {} maps ({}) -> {}",
            maps.len(),
            maps.join(", "),
            if by.is_empty() { "NO ENEMY FILE" } else { by }
        ));
    }
}
