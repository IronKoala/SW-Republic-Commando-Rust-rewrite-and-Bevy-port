//! A squad mate sent to a turret marker ("Man turret") gets on the turret
//! and fires it at an enemy, with the real squad, combat and turret
//! plugins in the movement test room and no game files (so it runs
//! anywhere, including the GitHub checks). The turret is the shipped
//! Wookiee heavy turret file, placed by hand.
//!
//! Checks: the squad mate gets on once at the spot and is held at the
//! gunner's place; the turret fires its own shells while their own gun
//! stays quiet; F4 takes them off and puts them back on their feet; and a
//! turret the player is on is left to the player until they get off.
//!
//! Run: cargo test --release -p rc_turrets_plugin --test squad_turret -- --nocapture

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use rc_combat::{FireShot, Health, Hittable, Team};
use rc_game_core::units::to_bevy;
use rc_game_core::{
    CorePlugin, GameLog, GamePaths, GameState, LevelCollision, OnTurret, Player, PlayerCamera,
};
use rc_movement::room::test_room;
use rc_movement::Vec3 as UVec3;
use rc_squad::{MapSquadInfo, MarkerSpot, Order};
use rc_squad_plugin::{yaw_rotation, SquadMapData, SquadMate, SquadOrder, SquadPlugin, SquadState};
use rc_turrets::{Gunner, PlacedTurret, Turret};
use rc_turrets_plugin::{LevelTurrets, TurretKinds, TurretsPlugin};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const DT: f32 = 1.0 / 60.0;
const TURRET_AT: [f32; 3] = [900.0, 0.0, 125.0];

/// Every shot fired: who, with what.
#[derive(Resource, Default)]
struct Shots(Vec<(Entity, String)>);

fn collect(mut r: MessageReader<FireShot>, mut shots: ResMut<Shots>) {
    shots
        .0
        .extend(r.read().map(|s| (s.shooter, s.weapon.clone())));
}

fn placed() -> PlacedTurret {
    PlacedTurret {
        export: 0,
        name: "WookieeHeavyTurret4".into(),
        class: "WookieeHeavyTurret".into(),
        tag: "WookieeHeavyTurret".into(),
        location: TURRET_AT,
        rotation: [0, 0, 0],
        control_event: Some("SquadOnTurret".into()),
        player_control_event: None,
        release_event: Some("SquadOffTurret".into()),
        player_usable: Some(false),
        min_yaw: None,
        max_yaw: None,
        min_pitch: None,
        max_pitch: None,
        unlimited_yaw: None,
        speed: None,
        exit_spot: None,
        exit_offset: None,
        event: None,
        first_damaged_event: None,
        deployed: None,
        health: None,
    }
}

fn app() -> App {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let log_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../logs/squad_turret.log");
    let (kinds, errors) = rc_turrets::params::load_folders(&[&data.join("turrets")]);
    assert!(errors.is_empty(), "{errors:?}");
    let (id, params) = rc_turrets::params::for_class(&kinds, "WookieeHeavyTurret").unwrap();
    let turret = Turret::new(placed(), id, params);
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        TransformPlugin,
        CorePlugin,
        rc_combat::CombatPlugin,
        SquadPlugin,
        TurretsPlugin,
    ))
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
        DT,
    )))
    .insert_resource(GamePaths::new(PathBuf::from("no-game-here"), vec![data]))
    .insert_resource(GameLog::create(&log_path).expect("log file"))
    .insert_resource(LevelCollision(Arc::new(test_room())))
    .insert_resource(LevelTurrets::without_models(
        TurretKinds(kinds.clone()),
        vec![turret],
    ))
    .insert_resource(SquadMapData {
        info: MapSquadInfo {
            // As yyy_06d's: a squad spot whose anchor is the turret.
            markers: vec![MarkerSpot {
                name: "SquadMarkerSpot161".into(),
                job: "WookieeHeavyTurret".into(),
                anchor: "WookieeHeavyTurret4".into(),
                anchor_class: "WookieeHeavyTurret".into(),
                stand: UVec3::new(780.0, 0.0, 0.0),
                yaw: 0.0,
                aim: UVec3::new(TURRET_AT[0], TURRET_AT[1], TURRET_AT[2]),
                ..Default::default()
            }],
            ..Default::default()
        },
        ..Default::default()
    })
    .init_resource::<Shots>()
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

/// The squad mate on the turret post: entity, name, body centre, whether
/// they carry `OnTurret`.
fn posted(app: &mut App) -> Option<(Entity, String, UVec3, bool)> {
    let i = {
        let squad = app.world().resource::<SquadState>().squad.as_ref()?;
        (0..squad.members.len()).find(|&i| squad.members[i].marker == Some(0))?
    };
    let loc = app.world().resource::<SquadState>().squad.as_ref()?.members[i]
        .pawn
        .location;
    let mut q = app
        .world_mut()
        .query::<(Entity, &SquadMate, Option<&OnTurret>)>();
    q.iter(app.world())
        .find(|(_, m, _)| m.index == i)
        .map(|(e, m, on)| (e, m.name.clone(), loc, on.is_some()))
}

fn gunner(app: &App) -> Gunner {
    app.world().resource::<LevelTurrets>().list[0]
        .gunner
        .clone()
}

#[test]
fn a_squad_mate_mans_the_turret_fires_it_and_gets_off_when_recalled() {
    let mut app = app();
    run(&mut app, 1.0);
    {
        let squad = app
            .world()
            .resource::<SquadState>()
            .squad
            .as_ref()
            .expect("squad");
        assert_eq!(squad.markers[0].0.job, "Man turret");
    }
    app.world_mut().write_message(SquadOrder(Order::Marker(0)));
    run(&mut app, 10.0);
    let (e, name, loc, on) = posted(&mut app).expect("someone took the post");
    println!(
        "{name} at {loc:?}, on the turret: {on}, gunner {:?}",
        gunner(&app)
    );
    assert!(on, "{name} isn't on the turret");
    assert_eq!(gunner(&app), Gunner::Squad(name.clone()));
    // Held at the gunner's place: 120 above the turret's centre.
    let body = app.world().resource::<LevelTurrets>().list[0].gunner_body();
    assert!(
        (loc.x - body[0]).abs() < 1.0
            && (loc.y - body[1]).abs() < 1.0
            && (loc.z - body[2]).abs() < 1.0,
        "{loc:?} vs {body:?}"
    );
    let events = app.world().resource::<LevelTurrets>().stats.events.clone();
    assert_eq!(events, ["SquadOnTurret"]);

    // An enemy ahead, tough enough to last.
    app.world_mut().spawn((
        Health::new(100000.0),
        Team::Separatist,
        Hittable {
            radius: 35.0,
            half_height: 80.0,
            center_up: 80.0,
        },
        Transform::from_translation(to_bevy(UVec3::new(3500.0, 300.0, 0.0))),
    ));
    app.world_mut().resource_mut::<Shots>().0.clear();
    run(&mut app, 7.0);
    let shots = app.world().resource::<Shots>().0.clone();
    let turret = shots
        .iter()
        .filter(|(s, w)| *s == e && w == "heavy_turret")
        .count();
    let own = shots
        .iter()
        .filter(|(s, w)| *s == e && w != "heavy_turret")
        .count();
    let others = shots.iter().filter(|(s, _)| *s != e).count();
    let tur = &app.world().resource::<LevelTurrets>().list[0];
    println!(
        "{name}: {turret} turret shells, {own} own shots; the others fired {others}; turret yaw {:.0} pitch {:.0}",
        tur.yaw, tur.pitch
    );
    // Wookiee heavy turret: a shell every 2 s from each barrel; no model
    // here, so one start point per shot.
    assert!(turret >= 3, "the turret didn't fire: {shots:?}");
    assert_eq!(own, 0, "{name} fired their own gun from the turret");
    // Turned toward the droid: to the left (+Y) a little.
    assert!(tur.yaw > 500.0 && tur.yaw < 8194.0, "{}", tur.yaw);

    // F4: off the turret, back on their feet.
    app.world_mut().write_message(SquadOrder(Order::Cancel));
    run(&mut app, 0.5);
    let feet = {
        let squad = app.world().resource::<SquadState>().squad.as_ref().unwrap();
        let m = squad
            .members
            .iter()
            .find(|m| m.settings.name == name)
            .unwrap();
        assert!(m.mounted.is_none());
        m.pawn.feet()
    };
    assert_eq!(gunner(&app), Gunner::Nobody);
    let mut q = app.world_mut().query::<(Entity, &OnTurret)>();
    assert_eq!(q.iter(app.world()).count(), 0);
    println!("{name} off at {feet:?}");
    assert!(feet.z.abs() < 5.0, "not on the floor: {feet:?}");
    let events = app.world().resource::<LevelTurrets>().stats.events.clone();
    assert_eq!(events, ["SquadOnTurret", "SquadOffTurret"]);
}

#[test]
fn a_turret_the_player_is_on_waits_for_the_player() {
    let mut app = app();
    run(&mut app, 1.0);
    // Someone else on it (the player, as far as the turret knows).
    app.world_mut().resource_mut::<LevelTurrets>().list[0].gunner = Gunner::Player;
    app.world_mut().write_message(SquadOrder(Order::Marker(0)));
    run(&mut app, 10.0);
    let (_, name, loc, on) = posted(&mut app).expect("someone took the post");
    assert!(!on, "{name} got on a taken turret");
    assert!(loc.z < 150.0, "{name} isn't standing at the spot: {loc:?}");
    // The player gets off: the squad mate asks again within 2 s and gets on.
    app.world_mut().resource_mut::<LevelTurrets>().list[0].gunner = Gunner::Nobody;
    run(&mut app, 2.5);
    let (_, _, _, on) = posted(&mut app).unwrap();
    assert!(on, "{name} didn't get on once it was free");
    assert_eq!(gunner(&app), Gunner::Squad(name));
}
