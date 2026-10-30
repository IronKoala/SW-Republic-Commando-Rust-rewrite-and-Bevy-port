//! Squad context orders in the movement test room, with the real squad and
//! combat plugins and no game files (so it runs anywhere, including the
//! GitHub checks): a bacta dispenser heals the squad mate who uses it, and
//! a demolition charge (Scorch's job) blows up a stand-in droid next to it and reports
//! the job done.
//!
//! Run: cargo test --release -p rc_squad_plugin --test jobs_room -- --nocapture

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
use rc_squad::{MapSquadInfo, MarkerSpot, Order};
use rc_squad_plugin::{
    yaw_rotation, SquadJobDone, SquadMapData, SquadMate, SquadOrder, SquadPlugin, SquadState,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const DT: f32 = 1.0 / 60.0;

#[derive(Resource, Default)]
struct Done(Vec<SquadJobDone>);

fn collect(mut r: MessageReader<SquadJobDone>, mut done: ResMut<Done>) {
    done.0.extend(r.read().cloned());
}

fn marker(name: &str, anchor: &str, class: &str, stand: UVec3) -> MarkerSpot {
    MarkerSpot {
        name: name.into(),
        job: class.into(),
        anchor: anchor.into(),
        anchor_class: class.into(),
        stand,
        yaw: 0.0,
        aim: stand + UVec3::new(60.0, 0.0, 60.0),
        ..Default::default()
    }
}

fn app() -> App {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let log_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../logs/jobs_room.log");
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
    .insert_resource(GamePaths::new(PathBuf::from("no-game-here"), vec![data]))
    .insert_resource(GameLog::create(&log_path).expect("log file"))
    .insert_resource(LevelCollision(Arc::new(test_room())))
    .insert_resource(SquadMapData {
        info: MapSquadInfo {
            markers: vec![
                marker(
                    "SquadMarkerSpot0",
                    "BactaDispenserGEO0",
                    "BactaDispenserGEO",
                    UVec3::new(-300.0, 600.0, 0.0),
                ),
                marker(
                    "SquadMarkerSpot1",
                    "DemolitionCharge0",
                    "DemolitionCharge",
                    UVec3::new(1000.0, 0.0, 0.0),
                ),
            ],
            factories: vec![],
        },
        nav: None,
        notes: vec![],
    })
    .init_resource::<Done>()
    .add_systems(Update, collect);
    app.world_mut()
        .resource_mut::<NextState<GameState>>()
        .set(GameState::InGame);
    app.world_mut().spawn((
        Player,
        Health::new(200.0),
        Team::Republic,
        Hittable::COMMANDO,
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

fn mates(app: &mut App) -> Vec<(Entity, usize, f32)> {
    let mut q = app.world_mut().query::<(Entity, &SquadMate, &Health)>();
    let mut v: Vec<_> = q
        .iter(app.world())
        .map(|(e, m, h)| (e, m.index, h.current))
        .collect();
    v.sort_by_key(|m| m.1);
    v
}

#[test]
fn bacta_heals_and_a_demolition_charge_blows_up_what_is_near_it() {
    let mut app = app();
    run(&mut app, 1.0);
    {
        let state = app.world().resource::<SquadState>();
        let squad = state.squad.as_ref().expect("squad spawned");
        assert_eq!(squad.markers.len(), 2);
        assert_eq!(squad.markers[0].0.job, "Bacta");
        assert_eq!(squad.markers[1].0.job, "Demolition");
    }

    // Hurt everyone, then send someone to the bacta dispenser.
    for (e, _, _) in mates(&mut app) {
        app.world_mut().write_message(Damage {
            target: e,
            amount: 350.0,
            kind: DamageKind::Other,
            instigator: None,
            point: Vec3::ZERO,
            direction: Vec3::ZERO,
        });
    }
    run(&mut app, 0.2);
    let hurt = mates(&mut app);
    assert!(hurt.iter().all(|m| m.2 < 400.0), "{hurt:?}");
    app.world_mut().write_message(SquadOrder(Order::Marker(0)));
    run(&mut app, 8.0);
    let after = mates(&mut app);
    println!("health before {hurt:?}, after bacta {after:?}");
    assert_eq!(
        after.iter().filter(|m| m.2 == 400.0).count(),
        1,
        "exactly the one who used it is healed: {after:?}"
    );

    // A droid by the demolition charge.
    let droid = app
        .world_mut()
        .spawn((
            Health::new(100.0),
            Team::Separatist,
            Hittable {
                radius: 35.0,
                half_height: 80.0,
                center_up: 80.0,
            },
            Transform::from_translation(to_bevy(UVec3::new(1150.0, 150.0, 0.0))),
        ))
        .id();
    // Keep it from being shot first: it's a Neutral prop for the squad
    // until the blast (squad mates only shoot Separatists).
    app.world_mut().entity_mut(droid).insert(Team::Neutral);
    app.world_mut().write_message(SquadOrder(Order::Marker(1)));
    // Walk and set it (about 6 s), then the 10 s fuse.
    run(&mut app, 12.0);
    assert!(app.world().get::<Dead>(droid).is_none(), "blew too early");
    assert!(app.world().resource::<Done>().0.is_empty());
    run(&mut app, 8.0);
    assert!(app.world().get::<Dead>(droid).is_some(), "the blast missed");
    let done = &app.world().resource::<Done>().0;
    assert_eq!(done.len(), 1, "{done:?}");
    assert_eq!(done[0].anchor, "DemolitionCharge0");
    assert_eq!(done[0].job, "Demolition");
}
