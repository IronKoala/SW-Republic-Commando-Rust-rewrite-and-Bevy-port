//! Lists how each room of a map sounds (`rc_level::zone_audio`): every
//! zone, its audio class, the class's echo, looping sound and random
//! one-shots, and the classes the map's scripts switch zones to.
//!
//!   cargo run -p rc_level --example zone_audio -- <game folder> <map> [map ...]
//!
//! With `--effects` after the game folder it also prints each echo's
//! values. Reads the install only; writes nothing.

use std::path::PathBuf;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let effects = args.iter().any(|a| a == "--effects");
    args.retain(|a| a != "--effects");
    let mut args = args.into_iter();
    let Some(game) = args.next() else {
        eprintln!("usage: zone_audio <game folder> [--effects] <map> [map ...]");
        std::process::exit(2);
    };
    let data = PathBuf::from(game).join("GameData");
    for map in args {
        let path = data.join("Maps").join(format!("{map}.ctm"));
        let l = match rc_level::map_collision::load(&path, &data, 84.0) {
            Ok(l) => l,
            Err(e) => {
                println!("{map}: not read: {e}");
                continue;
            }
        };
        let z = &l.zone_audio;
        let with = z.zones.iter().filter(|x| x.env.is_some()).count();
        println!(
            "{map}: {} zones, {with} with an audio class, {} classes",
            z.zones.len(),
            z.envs.len()
        );
        for (i, zone) in z.zones.iter().enumerate() {
            let env = zone.env.map(|e| z.envs[e].class.as_str()).unwrap_or("-");
            println!(
                "  zone {i:>2} {:<16} tag {:<16} {env}",
                zone.actor, zone.tag
            );
        }
        for e in &z.envs {
            let s = |p: &Option<Vec<String>>| p.as_ref().map(|p| p.join(".")).unwrap_or("-".into());
            println!(
                "  class {}.{}: echo {}  loop {}  one-shot {} every {}..{} s, radius {}..{}",
                e.package,
                e.class,
                e.reverb
                    .as_ref()
                    .map(|r| format!("{} ({})", r.name, r.class))
                    .unwrap_or("none".into()),
                s(&e.pre_ambient),
                s(&e.random_one_shot),
                e.ros_interval_min,
                e.ros_interval_max,
                e.ros_radius_min,
                e.ros_radius_max
            );
            if effects {
                if let Some(r) = &e.reverb {
                    println!("    {r:?}");
                }
            }
        }
        let set: Vec<_> = z
            .vision_echoes
            .iter()
            .filter_map(|(m, r)| r.as_ref().map(|r| format!("{m} = {}", r.name)))
            .collect();
        println!(
            "  vision modes: {}, with their own echo: {}{}",
            z.vision_echoes.len(),
            set.len(),
            if set.is_empty() {
                String::new()
            } else {
                format!(" ({})", set.join(", "))
            }
        );
        for p in &z.problems {
            println!("  PROBLEM {p}");
        }
    }
}
