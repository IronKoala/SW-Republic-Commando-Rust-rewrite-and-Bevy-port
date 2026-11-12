//! Squad mates leaving and rejoining the squad, with the real squad
//! plugin and no game files (so it runs anywhere, including the GitHub
//! checks). A flat floor and a made-up map like ras_02c / ras_02b in
//! one: Sev is placed in the map (in the squad, called `Delta07` by the
//! scripts), Fixer is made by a spawner on the event `BringFixer` (called
//! `Delta40`, out of the squad until a script adds him), and Scorch isn't
//! in this map at all. The test plays the level scripts' part by sending
//! the messages the scripts part sends (`ScriptEvent`, `CharacterOrder`,
//! `SquadMembership`).
//!
//! Run: cargo test --release -p rc_squad_plugin --test rejoin_room -- --nocapture

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use rc_combat::{Health, Hittable, Team};
use rc_game_core::save::SavePart;
use rc_game_core::units::to_bevy;
use rc_game_core::{
    CharacterAct, CharacterOrder, CharacterOrderDone, CorePlugin, GameLog, GamePaths, GameState,
    LevelCollision, Player, PlayerCamera, ScriptEvent, ScriptTags, SquadMembership,
};
use rc_movement::{TriangleWorld, Vec3 as UVec3};
use rc_squad::{MapSquadInfo, PlacedMate, Presence, SquadFactory};
use rc_squad_plugin::save::SquadSave;
use rc_squad_plugin::{
    mate_half_height, yaw_rotation, SquadMapData, SquadMate, SquadPlugin, SquadState,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const DT: f32 = 1.0 / 60.0;

fn floor() -> TriangleWorld {
    let (lo, hi) = (-4000.0, 4000.0);
    TriangleWorld::new(vec![
        [
            UVec3::new(lo, lo, 0.0),
            UVec3::new(hi, lo, 0.0),
            UVec3::new(hi, hi, 0.0),
        ],
        [
            UVec3::new(lo, lo, 0.0),
            UVec3::new(hi, hi, 0.0),
            UVec3::new(lo, hi, 0.0),
        ],
    ])
}

/// Sev placed 1000 uu ahead (beyond form-up's 768 uu, so he comes over) of the player; Fixer's spawner 1200 uu to the
/// side. Locations are actors' middles, as maps give them.
fn map() -> MapSquadInfo {
    let mid = mate_half_height();
    MapSquadInfo {
        placed: vec![PlacedMate {
            actor: "Commando07Delta0".into(),
            class: "Commando07Delta".into(),
            location: UVec3::new(1000.0, 0.0, mid),
            yaw: 0.0,
            tag: "Delta07".into(),
            script: Some("SevScript".into()),
            event: Some("SevDown".into()),
            revive_event: Some("SevUp".into()),
        }],
        factories: vec![SquadFactory {
            actor: "PawnFactory2".into(),
            class: "Commando40Delta".into(),
            location: UVec3::new(0.0, 1200.0, mid),
            yaw: 0.0,
            tag: "BringFixer".into(),
            pawn_tag: "Delta40".into(),
            script: Some("SS-Commando".into()),
            pawn_event: Some("FixerDown".into()),
        }],
        has_squad_actor: true,
        ..Default::default()
    }
}

#[derive(Resource, Default)]
struct Done(Vec<CharacterOrderDone>);

fn collect(mut r: MessageReader<CharacterOrderDone>, mut done: ResMut<Done>) {
    done.0.extend(r.read().cloned());
}

fn app() -> App {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let log_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../logs/rejoin_room.log");
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
    .insert_resource(LevelCollision(Arc::new(floor())))
    .insert_resource(SquadMapData {
        info: map(),
        read: true,
        ..Default::default()
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
        Transform::from_translation(to_bevy(UVec3::new(0.0, 0.0, 0.0)))
            .with_rotation(yaw_rotation(0.0)),
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

fn move_player(app: &mut App, feet: UVec3) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut Transform, With<Player>>();
    for mut t in q.iter_mut(app.world_mut()) {
        t.translation = to_bevy(feet);
    }
}

/// (presence, feet) of the mate called `name`.
fn mate(app: &App, name: &str) -> (Presence, UVec3) {
    let squad = app
        .world()
        .resource::<SquadState>()
        .squad
        .as_ref()
        .expect("squad spawned");
    let m = squad
        .members
        .iter()
        .find(|m| m.settings.name == name)
        .unwrap_or_else(|| panic!("no {name}"));
    (m.presence, m.pawn.feet())
}

/// The body of the mate called `name` and what scripts call them.
fn body(app: &mut App, name: &str) -> Option<(Entity, String)> {
    let mut q = app.world_mut().query::<(Entity, &SquadMate, &ScriptTags)>();
    q.iter(app.world())
        .find(|(_, m, _)| m.name == name)
        .map(|(e, _, t)| (e, t.tag.clone().unwrap_or_default()))
}

fn dist_2d(a: UVec3, b: UVec3) -> f32 {
    UVec3::new(a.x - b.x, a.y - b.y, 0.0).length()
}

#[test]
fn squad_mates_leave_and_rejoin_as_the_scripts_say() {
    let mut app = app();
    run(&mut app, 0.5);

    // From the map: Sev where he's placed, in the squad; Fixer and Scorch
    // not in the level, with no body.
    let (p, sev) = mate(&app, "Sev");
    println!("start: Sev {p:?} at {sev:?}");
    assert_eq!(p, Presence::InSquad);
    assert_eq!(mate(&app, "Fixer").0, Presence::Gone);
    assert_eq!(mate(&app, "Scorch").0, Presence::Gone);
    let (sev_body, sev_tag) = body(&mut app, "Sev").expect("Sev has a body");
    assert_eq!(sev_tag, "Delta07", "scripts call Sev by his Tag");
    assert!(body(&mut app, "Fixer").is_none(), "Fixer has no body yet");
    assert!(
        body(&mut app, "Scorch").is_none(),
        "Scorch isn't in this map"
    );

    // In the squad, Sev comes to the player.
    run(&mut app, 4.0);
    let (_, sev) = mate(&app, "Sev");
    println!("after 4 s: Sev at {sev:?}");
    assert!(
        dist_2d(sev, UVec3::ZERO) < 400.0,
        "Sev didn't come to the player: {sev:?}"
    );

    // The spawner goes off: Fixer comes in at it, out of the squad.
    app.world_mut().write_message(ScriptEvent {
        name: "BringFixer".into(),
        by: "test".into(),
    });
    run(&mut app, 0.2);
    let (p, fixer) = mate(&app, "Fixer");
    println!("Fixer comes in: {p:?} at {fixer:?}");
    assert_eq!(p, Presence::Away);
    assert!(dist_2d(fixer, UVec3::new(0.0, 1200.0, 0.0)) < 5.0);
    let (fixer_body, fixer_tag) = body(&mut app, "Fixer").expect("Fixer has a body");
    assert_eq!(fixer_tag, "Delta40");
    // A second time does nothing (the spawner makes one).
    app.world_mut().write_message(ScriptEvent {
        name: "BringFixer".into(),
        by: "test".into(),
    });
    run(&mut app, 2.0);
    let (_, fixer2) = mate(&app, "Fixer");
    assert!(
        dist_2d(fixer2, fixer) < 5.0,
        "Fixer, out of the squad, wandered: {fixer2:?}"
    );

    // His script walks him to a door; the script waits for him.
    let door = UVec3::new(800.0, 1200.0, mate_half_height());
    app.world_mut().write_message(CharacterOrder {
        target: fixer_body,
        act: CharacterAct::MoveTo {
            to: [door.x, door.y, door.z],
            walk: false,
            face: Some(0.0),
        },
        script: "SS-Commando".into(),
        index: 9,
        waits: true,
    });
    run(&mut app, 6.0);
    let (_, fixer) = mate(&app, "Fixer");
    let done = &app.world().resource::<Done>().0;
    println!("Fixer after his walk: {fixer:?}; done {done:?}");
    assert!(
        dist_2d(fixer, door) < 70.0,
        "Fixer didn't get to the door: {fixer:?}"
    );
    assert!(
        done.iter()
            .any(|d| d.script == "SS-Commando" && d.index == 9),
        "the script never heard Fixer got there"
    );

    // Sev leaves the squad; the player walks off; Sev stays.
    app.world_mut().write_message(SquadMembership {
        target: sev_body,
        join: false,
        by: "SevScript #1".into(),
    });
    run(&mut app, 0.2);
    let (p, sev_left_at) = mate(&app, "Sev");
    assert_eq!(p, Presence::Away);
    move_player(&mut app, UVec3::new(-1500.0, 0.0, 0.0));
    run(&mut app, 5.0);
    let (_, sev) = mate(&app, "Sev");
    println!("Sev out of the squad, player away: Sev at {sev:?}");
    assert!(
        dist_2d(sev, sev_left_at) < 5.0,
        "Sev followed after leaving: {sev:?}"
    );

    // Orders while out of the squad: a squad order doesn't move him.
    // A save keeps who is where.
    let save = SquadSave::save(app.world_mut()).expect("a squad to save");
    let presence: Vec<(String, String)> = save
        .members
        .iter()
        .map(|m| (m.name.clone(), m.presence.clone()))
        .collect();
    println!("saved: {presence:?}");
    assert_eq!(
        presence,
        vec![
            ("Sev".to_string(), "away".to_string()),
            ("Fixer".to_string(), "away".to_string()),
            ("Scorch".to_string(), "gone".to_string()),
        ]
    );
    assert_eq!(
        save.members[1].spawners_used,
        vec!["PawnFactory2".to_string()]
    );

    // Both rejoin (`ChangeSquadMember` with bAddMember) and come to the
    // player.
    for e in [sev_body, fixer_body] {
        app.world_mut().write_message(SquadMembership {
            target: e,
            join: true,
            by: "test".into(),
        });
    }
    run(&mut app, 12.0);
    let player = UVec3::new(-1500.0, 0.0, 0.0);
    for name in ["Sev", "Fixer"] {
        let (p, f) = mate(&app, name);
        println!("{name} rejoined: {p:?} at {f:?}");
        assert_eq!(p, Presence::InSquad);
        assert!(
            dist_2d(f, player) < 400.0,
            "{name} didn't follow after rejoining: {f:?}"
        );
    }

    // A script takes Sev out of the level (`DestroyPawn`): no body.
    app.world_mut().write_message(CharacterOrder {
        target: sev_body,
        act: CharacterAct::Remove,
        script: "SevScript".into(),
        index: 7,
        waits: false,
    });
    run(&mut app, 0.2);
    assert_eq!(mate(&app, "Sev").0, Presence::Gone);
    assert!(body(&mut app, "Sev").is_none(), "Sev's body is still there");
}

#[test]
fn a_map_that_cant_be_read_puts_all_three_with_the_player() {
    let mut app = app();
    app.world_mut().resource_mut::<SquadMapData>().read = false;
    run(&mut app, 0.5);
    for name in ["Sev", "Fixer", "Scorch"] {
        let (p, f) = mate(&app, name);
        assert_eq!(p, Presence::InSquad, "{name}");
        assert!(dist_2d(f, UVec3::ZERO) < 300.0, "{name} at {f:?}");
        assert!(body(&mut app, name).is_some(), "{name} has no body");
    }
}

#[derive(Resource, Default)]
struct Raised(Vec<(String, String)>);

fn collect_raised(mut r: MessageReader<rc_game_core::RaiseEvent>, mut got: ResMut<Raised>) {
    got.0
        .extend(r.read().map(|e| (e.name.clone(), e.by.clone())));
}

fn put_down(app: &mut App, e: Entity, down: bool) {
    let mut m = app.world_mut().entity_mut(e);
    if down {
        m.get_mut::<Health>().unwrap().current = 0.0;
        m.insert(rc_combat::Dead);
    } else {
        let mut h = m.get_mut::<Health>().unwrap();
        h.current = h.max;
        m.remove::<rc_combat::Dead>();
    }
}

/// Going down and being revived send the map's events for that mate: a
/// placed one its `Event` / `ReviveEvent`, one a spawner made the
/// spawner's `PawnEventFromFactory` and nothing on revive.
#[test]
fn squad_mates_send_their_down_and_revived_events() {
    let mut app = app();
    app.init_resource::<Raised>()
        .add_systems(Update, collect_raised);
    run(&mut app, 0.5);
    let (sev, _) = body(&mut app, "Sev").expect("Sev has a body");
    put_down(&mut app, sev, true);
    run(&mut app, 0.1);
    put_down(&mut app, sev, false);
    run(&mut app, 0.1);
    app.world_mut().write_message(ScriptEvent {
        name: "BringFixer".into(),
        by: "test".into(),
    });
    run(&mut app, 0.5);
    let (fixer, _) = body(&mut app, "Fixer").expect("Fixer came in");
    put_down(&mut app, fixer, true);
    run(&mut app, 0.1);
    put_down(&mut app, fixer, false);
    run(&mut app, 0.1);
    let got: Vec<(String, String)> = app
        .world()
        .resource::<Raised>()
        .0
        .iter()
        .filter(|(n, _)| !n.ends_with("Event") && !n.starts_with("Player"))
        .cloned()
        .collect();
    println!("raised: {got:?}");
    assert_eq!(
        got,
        [
            ("SevDown".to_string(), "Sev".to_string()),
            ("SevUp".to_string(), "Sev".to_string()),
            ("FixerDown".to_string(), "Fixer".to_string()),
        ]
    );
}
