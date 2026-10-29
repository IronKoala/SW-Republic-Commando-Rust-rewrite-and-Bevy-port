//! The squad fighting and reviving in the movement test room, with the real
//! squad and combat plugins and no game files (so it runs anywhere,
//! including the GitHub checks). Stand-in droids (the game's droid size and
//! health, standing still, not shooting) are put in front of the squad.
//!
//! Run: cargo test --release -p rc_squad_plugin --test fight_room -- --nocapture

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use rc_combat::{Damage, DamageKind, Dead, Health, Hittable, Team};
use rc_game_core::units::to_bevy;
use rc_game_core::{
    CorePlugin, GameLog, GamePaths, GameState, LevelCollision, Player, PlayerCamera,
};
use rc_movement::room::test_room;
use rc_movement::Vec3 as UVec3;
use rc_squad_plugin::{yaw_rotation, SquadFightStats, SquadMate, SquadPlugin, SquadState};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const DT: f32 = 1.0 / 60.0;

fn app() -> App {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let log_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../logs/fight_room.log");
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        TransformPlugin,
        CorePlugin,
        rc_combat::CombatPlugin,
        SquadPlugin,
    ))
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
        DT,
    )))
    // No game folder: the weapons' numbers come from data/, sounds are skipped.
    .insert_resource(GamePaths::new(PathBuf::from("no-game-here"), vec![data]))
    .insert_resource(GameLog::create(&log_path).expect("log file"))
    .insert_resource(LevelCollision(Arc::new(test_room())));
    app.world_mut()
        .resource_mut::<NextState<GameState>>()
        .set(GameState::InGame);
    // The player at the room's start, facing +X.
    app.world_mut().spawn((
        Player,
        Transform::from_translation(to_bevy(UVec3::ZERO)).with_rotation(yaw_rotation(0.0)),
    ));
    app.world_mut().spawn((
        PlayerCamera,
        Transform::from_translation(to_bevy(UVec3::new(0.0, 0.0, 144.0)))
            .with_rotation(yaw_rotation(0.0)),
    ));
    app
}

fn run(app: &mut App, secs: f32) {
    for _ in 0..(secs / DT).round() as usize {
        app.update();
    }
}

fn droid(app: &mut App, feet: UVec3) -> Entity {
    app.world_mut()
        .spawn((
            Name::new("stand-in droid"),
            Health::new(100.0),
            Team::Separatist,
            Hittable {
                radius: 35.0,
                half_height: 80.0,
                center_up: 80.0,
            },
            Transform::from_translation(to_bevy(feet)),
        ))
        .id()
}

fn knock_down(app: &mut App, target: Entity) {
    app.world_mut().write_message(Damage {
        target,
        amount: 10_000.0,
        kind: DamageKind::Other,
        instigator: None,
        point: Vec3::ZERO,
        direction: Vec3::ZERO,
    });
}

fn mate(app: &mut App, index: usize) -> Entity {
    let mut q = app.world_mut().query::<(Entity, &SquadMate)>();
    q.iter(app.world())
        .find(|(_, m)| m.index == index)
        .map(|(e, _)| e)
        .expect("squad mate")
}

fn player(app: &mut App) -> Entity {
    let mut q = app
        .world_mut()
        .query_filtered::<Entity, (With<Player>, Without<PlayerCamera>)>();
    q.single(app.world()).unwrap()
}

#[test]
fn the_squad_kills_droids_and_revives_each_other_and_the_player() {
    let mut app = app();
    run(&mut app, 1.0);
    assert!(app.world().resource::<SquadState>().squad.is_some());

    // Three droids ahead.
    let droids = [
        droid(&mut app, UVec3::new(1200.0, 500.0, 0.0)),
        droid(&mut app, UVec3::new(1300.0, -600.0, 0.0)),
        droid(&mut app, UVec3::new(1600.0, 0.0, 0.0)),
    ];
    run(&mut app, 15.0);
    let stats = app.world().resource::<SquadFightStats>().clone();
    println!("{stats:?}");
    for d in droids {
        assert!(app.world().get::<Dead>(d).is_some(), "a droid is still up");
    }
    assert_eq!(stats.kills.iter().sum::<u32>(), 3);
    // A droid takes 5 DC-17m hits (100 health, 20 a hit).
    assert!(stats.hits.iter().sum::<u32>() >= 15, "{stats:?}");
    let p = player(&mut app);
    assert_eq!(
        app.world().get::<Health>(p).unwrap().current,
        200.0,
        "the squad shot the player"
    );

    // Sev goes down; a squad mate revives him.
    let sev = mate(&mut app, 0);
    knock_down(&mut app, sev);
    run(&mut app, 0.5);
    assert!(app.world().get::<Dead>(sev).is_some());
    run(&mut app, 7.5);
    assert!(app.world().get::<Dead>(sev).is_none(), "Sev not revived");
    assert_eq!(app.world().get::<Health>(sev).unwrap().current, 200.0);

    // The player goes down; a squad mate revives them.
    knock_down(&mut app, p);
    run(&mut app, 0.5);
    assert!(app.world().get::<Dead>(p).is_some());
    run(&mut app, 7.5);
    assert!(app.world().get::<Dead>(p).is_none(), "player not revived");
    assert_eq!(app.world().get::<Health>(p).unwrap().current, 100.0);
    let stats = app.world().resource::<SquadFightStats>().clone();
    assert_eq!(stats.revives.iter().sum::<u32>(), 2, "{stats:?}");
    assert_eq!(stats.revive_waits.len(), 2);
    for w in &stats.revive_waits {
        assert!((3.0..8.0).contains(w), "down for {w} s");
    }
}
