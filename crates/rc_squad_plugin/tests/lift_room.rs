//! Squad mates ride a lift with the player, with the real squad and doors
//! and lifts plugins and no game files (so it runs anywhere, including the
//! GitHub checks). The player and the squad stand on a wide platform; it
//! rises 400 uu, and everyone must arrive at the top on it. A second run
//! without the lifts part's `MovingFloors` shows what happened before: the
//! squad is left at the bottom.
//!
//! Run: cargo test --release -p rc_squad_plugin --test lift_room -- --nocapture

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use rc_combat::{Health, Hittable, Team};
use rc_game_core::units::to_bevy;
use rc_game_core::{
    CorePlugin, GameLog, GamePaths, GameState, LevelCollision, MovingFloors, Player, PlayerCamera,
};
use rc_level::actors::Placement;
use rc_level::movers::{Encroach, Glide, MapMovers, MoverInfo, Sounds, MAX_KEYS};
use rc_movement::{MoveInput, MoveParams, Pawn, TriangleWorld, Vec3 as UVec3};
use rc_movers::{MoverSet, Settings};
use rc_movers_plugin::{LevelMovers, MoversPlugin};
use rc_squad_plugin::{yaw_rotation, SquadPlugin, SquadSet, SquadState};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const DT: f32 = 1.0 / 60.0;
/// The platform: 1000 x 1000, top at z = 10, rising 400 in 3 s.
const HALF: f32 = 500.0;
const RISE: f32 = 400.0;

/// The 12 triangles of a box.
fn box_tris(lo: UVec3, hi: UVec3) -> Vec<[UVec3; 3]> {
    let c = |i: usize| {
        UVec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        )
    };
    let faces = [
        [0, 1, 3, 2],
        [4, 6, 7, 5],
        [0, 4, 5, 1],
        [2, 3, 7, 6],
        [0, 2, 6, 4],
        [1, 5, 7, 3],
    ];
    faces
        .iter()
        .flat_map(|f| [[c(f[0]), c(f[1]), c(f[2])], [c(f[0]), c(f[2]), c(f[3])]])
        .collect()
}

fn lift() -> MoverInfo {
    let mut key_pos = [[0.0; 3]; MAX_KEYS];
    key_pos[1] = [0.0, 0.0, RISE];
    MoverInfo {
        place: Placement {
            export: 1,
            class: "Mover".into(),
            name: "Mover37".into(),
            location: [0.0; 3],
            rotation: [0; 3],
            draw_scale: 1.0,
            draw_scale_3d: [1.0; 3],
            pre_pivot: [0.0; 3],
            static_mesh: 0,
            skins: vec![],
            hidden: false,
            unlit: false,
            static_mesh_instance: 0,
        },
        chain: vec!["Mover".into()],
        base_pos: [0.0; 3],
        base_rot: [0; 3],
        key_pos,
        key_rot: [[0; 3]; MAX_KEYS],
        num_keys: 2,
        key_num: 0,
        move_times: [3.0; MAX_KEYS],
        stay_open_time: 0.0,
        delay_time: 0.0,
        glide: Glide::Smooth,
        encroach: Encroach::Return,
        state: "TriggerToggle".into(),
        tag: "Lift1".into(),
        opening_event: String::new(),
        opened_event: String::new(),
        closing_event: String::new(),
        closed_event: String::new(),
        auto_door: false,
        trigger_once_only: false,
        sounds: Sounds {
            opening: None,
            opened: None,
            closing: None,
            closed: None,
            moving: None,
        },
        defaulted: vec![],
    }
}

/// A stand-in for the game's player: stands still, and moves the movers
/// first each frame, as `rc_game`'s player movement does.
#[derive(Resource)]
struct Body(Pawn);

fn move_player(
    movers: Res<LevelMovers>,
    collision: Res<LevelCollision>,
    mut body: ResMut<Body>,
    mut player: Query<&mut Transform, With<Player>>,
) {
    movers.step(DT, &mut body.0);
    body.0.step(DT, &MoveInput::default(), collision.0.as_ref());
    if let Ok(mut t) = player.single_mut() {
        t.translation = to_bevy(body.0.feet());
    }
}

fn app(ride: bool) -> App {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let log_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../logs/lift_room{}.log",
        if ride { "" } else { "_no_ride" }
    ));
    let floor = TriangleWorld::new(box_tris(
        UVec3::new(-3000.0, -3000.0, -20.0),
        UVec3::new(3000.0, 3000.0, 0.0),
    ));
    let plat = box_tris(UVec3::new(-HALF, -HALF, 0.0), UVec3::new(HALF, HALF, 10.0));
    let map = MapMovers {
        movers: vec![lift()],
        triggers: vec![],
        failures: vec![],
    };
    let set = MoverSet::new(map, Settings::default(), move |_| plat.clone());
    let movers = LevelMovers::new(set, Arc::new(floor));
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        TransformPlugin,
        CorePlugin,
        rc_combat::CombatPlugin,
        MoversPlugin,
        SquadPlugin,
    ))
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
        DT,
    )))
    .insert_resource(GamePaths::new(PathBuf::from("no-game-here"), vec![data]))
    .insert_resource(GameLog::create(&log_path).expect("log file"))
    .insert_resource(LevelCollision(movers.world()))
    .insert_resource(Body(Pawn::new(
        MoveParams::player_commando(),
        UVec3::new(0.0, 0.0, 10.5),
    )))
    .add_systems(Update, move_player.before(SquadSet));
    if ride {
        app.insert_resource(movers.floors());
    }
    app.insert_resource(movers);
    app.world_mut()
        .resource_mut::<NextState<GameState>>()
        .set(GameState::InGame);
    app.world_mut().spawn((
        Player,
        Health::new(200.0),
        Team::Republic,
        Hittable::COMMANDO,
        Transform::from_translation(to_bevy(UVec3::new(0.0, 0.0, 10.0)))
            .with_rotation(yaw_rotation(0.0)),
    ));
    app.world_mut().spawn((
        PlayerCamera,
        Transform::from_translation(to_bevy(UVec3::new(0.0, 0.0, 154.0)))
            .with_rotation(yaw_rotation(0.0)),
    ));
    app
}

fn run(app: &mut App, secs: f32) {
    for _ in 0..(secs / DT).round() as usize {
        app.update();
    }
}

/// Each squad mate's feet.
fn feet(app: &App) -> Vec<(String, UVec3)> {
    let state = app.world().resource::<SquadState>();
    let squad = state.squad.as_ref().expect("squad spawned");
    squad
        .members
        .iter()
        .map(|m| (m.settings.name.clone(), m.pawn.feet()))
        .collect()
}

/// Spawn, let the squad settle around the player, and put anyone who
/// isn't on the platform onto it (where they spawn is the squad's own
/// business; this checks the ride).
fn settle_on_the_lift(app: &mut App) {
    run(app, 1.0);
    let mut state = app.world_mut().resource_mut::<SquadState>();
    let squad = state.squad.as_mut().expect("squad spawned");
    assert_eq!(squad.members.len(), 3, "Sev, Fixer and Scorch");
    for (i, m) in squad.members.iter_mut().enumerate() {
        let f = m.pawn.feet();
        if f.x.abs() > HALF - 60.0 || f.y.abs() > HALF - 60.0 || (f.z - 10.0).abs() > 2.0 {
            let spot = UVec3::new(-150.0 + 150.0 * i as f32, -200.0, 10.0);
            println!(
                "{} spawned at {f:?}, off the platform; moved to {spot:?}",
                m.settings.name
            );
            m.pawn.location = spot + UVec3::new(0.0, 0.0, m.pawn.extent().z + 0.5);
            m.pawn.velocity = UVec3::ZERO;
        }
    }
    run(app, 1.0);
    for (name, f) in feet(app) {
        println!("before the ride: {name} feet {f:?}");
        assert!(
            (f.z - 10.0).abs() < 2.0,
            "{name} isn't standing on the lift: {f:?}"
        );
        assert!(
            f.x.abs() < HALF && f.y.abs() < HALF,
            "{name} is off the lift: {f:?}"
        );
    }
}

#[test]
fn the_squad_rides_the_lift_up_with_the_player() {
    let mut app = app(true);
    settle_on_the_lift(&mut app);
    {
        let state = app.world().resource::<SquadState>();
        for (i, r) in state.riding.iter().enumerate() {
            assert_eq!(r.as_deref(), Some("Mover37"), "member {i} not on the lift");
        }
    }
    assert!(app.world().get_resource::<MovingFloors>().is_some());
    assert_eq!(app.world().resource::<LevelMovers>().send("Lift1"), 1);
    // 3 s up, and a second to spare.
    let mut lowest_during = f32::MAX;
    for _ in 0..(4.0 / DT) as usize {
        app.update();
        let lift_top = app.world().resource::<LevelMovers>().read().movers[0]
            .pose
            .pos
            .z
            + 10.0;
        for (name, f) in feet(&app) {
            assert!(
                f.z > lift_top - 2.0,
                "{name} sank into or fell off the lift: feet {f:?}, lift top {lift_top}"
            );
            lowest_during = lowest_during.min(f.z - lift_top);
        }
    }
    let player = app.world().resource::<Body>().0.feet();
    println!("after the ride: player feet {player:?}");
    assert!((player.z - (10.0 + RISE)).abs() < 3.0, "player {player:?}");
    for (name, f) in feet(&app) {
        println!("after the ride: {name} feet {f:?}");
        assert!(
            (f.z - (10.0 + RISE)).abs() < 3.0,
            "{name} didn't arrive at the top: {f:?}"
        );
    }
    let state = app.world().resource::<SquadState>();
    for (i, r) in state.riding.iter().enumerate() {
        assert_eq!(
            r.as_deref(),
            Some("Mover37"),
            "member {i} not on the lift at the top"
        );
    }
    println!("lowest feet relative to the lift top during the ride: {lowest_during:.2} uu");
}

#[test]
fn without_moving_floors_the_squad_is_left_behind() {
    let mut app = app(false);
    settle_on_the_lift(&mut app);
    app.world().resource::<LevelMovers>().send("Lift1");
    run(&mut app, 4.0);
    let player = app.world().resource::<Body>().0.feet();
    assert!((player.z - (10.0 + RISE)).abs() < 3.0, "player {player:?}");
    let left = feet(&app).iter().filter(|(_, f)| f.z < RISE * 0.5).count();
    println!("without MovingFloors: {left} of 3 left at the bottom");
    assert!(left >= 1);
}
