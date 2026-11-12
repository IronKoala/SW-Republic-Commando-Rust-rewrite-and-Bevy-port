//! The core ship turret on geo_05a in a Bevy app with no window: it stays
//! folded until its tag goes off, then shoots at the player, and when it is
//! shot to pieces it sends its events. Needs the game files
//! (`RC_GAME_DIR`); without them the test says so and passes.

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use rc_combat::{CombatPlugin, Damage, DamageKind, Team};
use rc_game_core::{
    CorePlugin, CurrentLevel, GameLog, GamePaths, GameState, LevelCollision, Player, RaiseEvent,
    ScriptEvent,
};
use rc_turrets_plugin::{LevelTurrets, TurretsPlugin};
use std::sync::Arc;
use std::time::Duration;

const STEP: f32 = 1.0 / 60.0;

/// Unreal position to Bevy (same rule as rc_game_core::units).
fn b(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, z, y) / 50.0
}

#[derive(Resource, Default)]
struct Seen {
    raised: Vec<String>,
    player_hurt: f32,
}

fn record(
    mut seen: ResMut<Seen>,
    mut r: MessageReader<RaiseEvent>,
    mut d: MessageReader<Damage>,
    player: Query<Entity, With<Player>>,
) {
    seen.raised.extend(r.read().map(|e| e.name.clone()));
    for d in d.read() {
        if player.contains(d.target) {
            seen.player_hurt += d.amount;
        }
    }
}

fn run(app: &mut App, seconds: f32) {
    for _ in 0..(seconds / STEP).round() as usize {
        app.update();
    }
}

fn hurt(app: &mut App, target: Entity, amount: f32) {
    app.world_mut().write_message(Damage {
        rules: Default::default(),
        target,
        amount,
        kind: DamageKind::Blaster,
        instigator: None,
        point: Vec3::ZERO,
        direction: Vec3::X,
    });
}

fn turret(app: &App, name: &str) -> usize {
    let lt = app.world().resource::<LevelTurrets>();
    lt.list
        .iter()
        .position(|t| t.place.name == name)
        .unwrap_or_else(|| panic!("{name} is not on the map"))
}

#[test]
fn core_ship_turret_switches_on_shoots_and_dies() {
    let Ok(game) = std::env::var("RC_GAME_DIR") else {
        eprintln!("RC_GAME_DIR not set: skipped");
        return;
    };
    if !std::path::Path::new(&game)
        .join("GameData/Maps/geo_05a.ctm")
        .exists()
    {
        eprintln!("geo_05a not in {game}: skipped");
        return;
    }
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    // Nothing in the way: one small triangle far off.
    let far = rc_movement::Vec3::new(-1.0e6, -1.0e6, -1.0e6);
    let world = rc_movement::TriangleWorld::new(vec![[
        far,
        far + rc_movement::Vec3::new(1.0, 0.0, 0.0),
        far + rc_movement::Vec3::new(0.0, 1.0, 0.0),
    ]]);
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, TransformPlugin))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            STEP,
        )))
        .add_plugins((CorePlugin, CombatPlugin, TurretsPlugin))
        .insert_resource(GamePaths::new(&game, vec![data]))
        .insert_resource(GameLog::terminal())
        .insert_resource(CurrentLevel {
            map: Some("geo_05a".into()),
        })
        .insert_resource(LevelCollision(Arc::new(world)))
        .init_resource::<Seen>()
        .add_systems(Last, record);
    app.world_mut()
        .resource_mut::<NextState<GameState>>()
        .set(GameState::Loading);
    app.update();
    app.world_mut()
        .resource_mut::<NextState<GameState>>()
        .set(GameState::InGame);
    app.update();

    let i = turret(&app, "CoreShipTurret1");
    let at = app.world().resource::<LevelTurrets>().list[i]
        .place
        .location;
    // The player 600 uu in front of it (it faces -X), feet below its middle.
    let player = app
        .world_mut()
        .spawn((
            Player,
            Team::Republic,
            Transform::from_translation(b(at[0] - 600.0, at[1], at[2] - 90.0)),
        ))
        .id();
    run(&mut app, 0.5);
    assert!(app.world().get::<rc_combat::Hittable>(player).is_some());

    // Folded up: it fires at nobody.
    run(&mut app, 2.0);
    assert_eq!(app.world().resource::<LevelTurrets>().stats.auto_shots, 0);

    // Its tag (`BDTurrets`, sent by the alarm script) switches it on.
    app.world_mut().write_message(ScriptEvent {
        name: "BDTurrets".into(),
        by: "test".into(),
    });
    run(&mut app, 3.0);
    let shots = app.world().resource::<LevelTurrets>().stats.auto_shots;
    assert!(shots > 5, "it fires once unfolded ({shots} shots)");
    assert!(
        app.world().resource::<Seen>().player_hurt > 0.0,
        "its bolts hit the player"
    );

    // Shot to pieces (health 256 on the map).
    let hit = app.world().resource::<LevelTurrets>().hits[i].expect("a target entity");
    hurt(&mut app, hit, 100.0);
    run(&mut app, 0.1);
    hurt(&mut app, hit, 500.0);
    run(&mut app, 0.5);
    let raised = &app.world().resource::<Seen>().raised;
    let first = raised.iter().position(|e| e == "TurretsShot");
    let last = raised.iter().position(|e| e == "Security00");
    assert!(first.is_some() && last.is_some(), "{raised:?}");
    assert!(first < last, "first hit before death: {raised:?}");
    assert_eq!(raised.iter().filter(|e| *e == "TurretsShot").count(), 1);
    let lt = app.world().resource::<LevelTurrets>();
    assert!(lt
        .stats
        .automatic
        .contains(&"destroyed CoreShipTurret1".into()));
    let after = lt.stats.auto_shots;
    run(&mut app, 2.0);
    assert_eq!(
        app.world().resource::<LevelTurrets>().stats.auto_shots,
        after,
        "a destroyed turret stops firing"
    );
}
