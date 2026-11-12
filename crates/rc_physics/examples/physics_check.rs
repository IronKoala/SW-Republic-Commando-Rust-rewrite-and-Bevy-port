//! Runs a map's physics props without drawing anything and logs where they
//! end up: `cargo run --release -p rc_physics --example physics_check --
//! <GameData folder> geo_04a [seconds] [event ...]`. Events (MegaForce or
//! prop tags) are sent one second apart after the props have settled.

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use rc_game_core::units::UU_PER_UNIT;
use rc_game_core::{GameLog, ScriptEvent, ScriptSpawnActor};
use rc_level_plugin::LevelKarma;
use rc_physics::{PhysicsPropsPlugin, Prop};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let gd = std::path::PathBuf::from(&args[1]);
    let map = &args[2];
    let secs: f32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(10.0);
    let events: Vec<String> = args.iter().skip(4).cloned().collect();
    let lvl = rc_level::map_collision::load(&gd.join("Maps").join(format!("{map}.ctm")), &gd, 70.0)
        .expect("map loads");
    let own: std::collections::HashSet<usize> = lvl.karma.iter().filter_map(|b| b.export).collect();
    let karma = LevelKarma {
        bodies: lvl.karma.clone(),
        spawns: lvl.karma_spawns.clone(),
        megaforces: lvl.megaforces.clone(),
        ground: lvl.triangles.clone(),
        parts: lvl
            .part_triangles
            .iter()
            .filter(|(e, _)| !own.contains(e))
            .map(|(e, t)| (*e, t.clone()))
            .collect(),
        impulses: lvl.damage_impulses.clone(),
    };
    let spawns: Vec<usize> = lvl.karma_spawns.iter().map(|s| s.action).collect();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, AssetPlugin::default()))
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0 / 60.0),
        ))
        .insert_resource(GameLog::terminal())
        .add_plugins(PhysicsPropsPlugin)
        .insert_resource(karma);
    app.finish();
    app.cleanup();
    for _ in 0..(secs * 60.0) as usize {
        app.update();
    }
    for e in &events {
        if e == "spawn" {
            for &a in &spawns {
                app.world_mut().write_message(ScriptSpawnActor {
                    action: a,
                    class: String::new(),
                    by: "check".into(),
                });
            }
        } else {
            app.world_mut().write_message(ScriptEvent {
                name: e.clone(),
                by: "check".into(),
            });
        }
        for _ in 0..(secs * 60.0) as usize {
            app.update();
        }
    }
    let mut q = app.world_mut().query::<(&Prop, &Transform)>();
    for (p, t) in q.iter(app.world()) {
        if !p.karma {
            continue;
        }
        let at = t.translation * UU_PER_UNIT;
        println!(
            "{} {}: start ({:.0}, {:.0}, {:.0}) now ({:.0}, {:.0}, {:.0})",
            p.body.class,
            p.body.name,
            p.body.location[0],
            p.body.location[1],
            p.body.location[2],
            at.x,
            at.z,
            at.y
        );
    }
}
