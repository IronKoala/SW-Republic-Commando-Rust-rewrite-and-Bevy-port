//! List the weapons lying in maps, for checking `rc_level::pickups`.
//!
//! `cargo run --release -p rc_level --example weapon_pickups -- <GameData folder> <map.ctm>...`

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((game, maps)) = args.split_first() else {
        eprintln!("usage: weapon_pickups <GameData folder> <map.ctm>...");
        std::process::exit(2);
    };
    let mut total = 0;
    for m in maps {
        match rc_level::map_collision::load(Path::new(m), Path::new(game), 64.0) {
            Ok(c) => {
                for w in &c.weapons {
                    total += 1;
                    println!(
                        "{}: {} ({}) gives {} with {} shots, \"{}\", sound {:?}, radius {} height {}, at {:?}, dropped {:.1}",
                        Path::new(m).file_name().unwrap().to_string_lossy(),
                        w.name,
                        w.class,
                        w.weapon,
                        w.ammo,
                        w.message,
                        w.sound,
                        w.radius,
                        w.height,
                        w.location,
                        w.dropped
                    );
                }
            }
            Err(e) => println!("{m}: FAILED {e}"),
        }
    }
    println!("{total} weapon pickups");
}
