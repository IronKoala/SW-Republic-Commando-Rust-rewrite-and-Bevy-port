//! Lists the sounds the first-person arms animations make: for each arms
//! model, each animation's `AnimNotify_Sound` events, with the moment
//! (0 = start, 1 = end) and the sound's path. These are what
//! `rc_combat::arms_sound` plays in the game.
//!
//!   cargo run -p rc_combat --example rc_arms_sounds -- <game folder> [Mesh ...]
//!
//! With no mesh names it lists the arms the weapon files use (HudArmsDC17m,
//! HudArmsSniper, HudArmsAntiArmor, HudArmsDC15s, HudArmsUzi). Reads the
//! install only; writes nothing.

use rc_combat::arms::ArmsAnim;
use rc_game_core::GamePaths;
use rc_model::{Model, ModelSpec};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(game) = args.next() else {
        eprintln!("usage: rc_arms_sounds <game folder> [Mesh ...]");
        std::process::exit(2);
    };
    let paths = GamePaths::new(game, Vec::new());
    let mut meshes: Vec<String> = args.collect();
    if meshes.is_empty() {
        meshes = [
            "HudArmsDC17m",
            "HudArmsSniper",
            "HudArmsAntiArmor",
            "HudArmsDC15s",
            "HudArmsUzi",
        ]
        .map(String::from)
        .to_vec();
    }
    for mesh in meshes {
        let spec = ModelSpec {
            package: "GameData/Animations/hudarms.ukx".into(),
            mesh: mesh.clone(),
            sequences: ArmsAnim::ALL
                .iter()
                .map(|a| (a.sequence().to_string(), a.sequence().to_string()))
                .collect(),
            ..Default::default()
        };
        match Model::load(&|rel| paths.game_file(rel), &spec) {
            Ok((m, _)) => {
                println!("{mesh}");
                for a in ArmsAnim::ALL {
                    let Some(clip) = m.clip(a.sequence()) else {
                        continue;
                    };
                    let heard = m.sounds.get(a.sequence());
                    println!(
                        "  {:<14} {:5.2} s  {}",
                        a.sequence(),
                        clip.duration(),
                        heard.map_or(0, |h| h.len())
                    );
                    for (t, path) in heard.into_iter().flatten() {
                        println!("      at {t:.2}  {path}");
                    }
                }
            }
            Err(e) => println!("{mesh}: not read: {e}"),
        }
    }
}
