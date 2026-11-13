//! Lists the sounds that loop where a map puts them (`rc_level::ambient`):
//! each actor, its spot and its sound.
//!
//!   cargo run -p rc_level --example ambient_sounds -- <game folder> <map> [map ...]
//!
//! Reads the install only; writes nothing.

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(game) = args.next() else {
        eprintln!("usage: ambient_sounds <game folder> <map> [map ...]");
        std::process::exit(2);
    };
    let data = PathBuf::from(game).join("GameData");
    for map in args {
        let path = data.join("Maps").join(format!("{map}.ctm"));
        match rc_level::map_collision::load(&path, &data, 84.0) {
            Ok(l) => {
                println!("{map}: {} looping sounds", l.ambient_sounds.len());
                for a in &l.ambient_sounds {
                    println!(
                        "  {:<16} {:<14} ({:.0}, {:.0}, {:.0})  {}{}",
                        a.name,
                        a.class,
                        a.location[0],
                        a.location[1],
                        a.location[2],
                        a.sound.join("."),
                        if a.from_map { "" } else { "  (from its class)" }
                    );
                }
            }
            Err(e) => println!("{map}: not read: {e}"),
        }
    }
}
