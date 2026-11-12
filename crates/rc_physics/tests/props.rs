//! The physics part on a made-up level (no game files): a prop falls and
//! rests on the floor, a MegaForce wakes and pushes a prop that stood
//! still, and a level script's `SpawnActor` makes a prop that falls.

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use rc_game_core::units::UU_PER_UNIT;
use rc_game_core::{GameLog, ScriptEvent, ScriptSpawnActor};
use rc_level::karma::{Impulse, KParams, KarmaBody, MegaForce, ScriptSpawn, Shape};
use rc_level_plugin::LevelKarma;
use rc_physics::{PhysicsPropsPlugin, Prop};

fn cube(half: f32) -> Shape {
    Shape::Convex(
        (0..8)
            .map(|i| {
                let s = |b: usize| if i & b == 0 { -half } else { half };
                [s(1), s(2), s(4)]
            })
            .collect(),
    )
}

fn body(name: &str, at: [f32; 3], karma_now: bool) -> KarmaBody {
    KarmaBody {
        export: None,
        class: "KarmaProp".into(),
        name: name.into(),
        tag: name.into(),
        event: None,
        location: at,
        rotation: [0; 3],
        scale: [1.0; 3],
        mesh_path: Vec::new(),
        params: KParams {
            start_enabled: true,
            friction: 0.4,
            restitution: 0.4,
            ..KParams::default()
        },
        shapes: vec![cube(25.0)],
        // A 1 Karma unit cube of mass 1: I = 1/6.
        inertia: [1.0 / 6.0, 0.0, 0.0, 1.0 / 6.0, 0.0, 1.0 / 6.0],
        com_nudge: [0.0; 3],
        take_shot: false,
        damage_threshold: 0,
        impact_sounds: Vec::new(),
        impact_interval: 0.5,
        impact_volume: 1.0,
        life_time: 0.0,
        blocks_players: false,
        blocks_karma: true,
        karma_now,
        trigger_makes_karma: true,
        collision_radius: 1.0,
        health: 0,
        is_karma_prop: true,
    }
}

fn find(app: &mut App, name: &str) -> Vec3 {
    let mut q = app.world_mut().query::<(&Prop, &Transform)>();
    let t = q
        .iter(app.world())
        .find(|(p, _)| p.body.name == name)
        .map(|(_, t)| t.translation)
        .unwrap_or_else(|| panic!("{name} not made"));
    // Back to Unreal (x, y, z).
    Vec3::new(t.x, t.z, t.y) * UU_PER_UNIT
}

fn run(app: &mut App, secs: f32) {
    for _ in 0..(secs * 60.0) as usize {
        app.update();
    }
}

#[test]
fn props_fall_get_pushed_and_get_made() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, AssetPlugin::default()))
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0 / 60.0),
        ))
        .insert_resource(GameLog::terminal())
        .add_plugins(PhysicsPropsPlugin);
    let w = 5000.0;
    let floor = vec![
        [[-w, -w, 0.0], [w, -w, 0.0], [w, w, 0.0]],
        [[-w, -w, 0.0], [w, w, 0.0], [-w, w, 0.0]],
    ];
    let mut spawned = body("Spawned", [-1000.0, 0.0, 800.0], true);
    spawned.tag = "SpawnedTag".into();
    app.insert_resource(LevelKarma {
        bodies: vec![
            body("Faller", [0.0, 0.0, 500.0], true),
            body("Still", [1000.0, 0.0, 25.0], false),
        ],
        spawns: vec![ScriptSpawn {
            action: 7,
            script: "Script1".into(),
            class: "Test.Rock".into(),
            body: Ok(spawned),
        }],
        megaforces: vec![MegaForce {
            name: "MegaForce1".into(),
            tag: "Push".into(),
            location: [900.0, 0.0, 25.0],
            rotation: [0; 3],
            kind: Impulse::Sphere,
            distance: 300.0,
            amount: 2000.0,
            at_centre: false,
            emitter_direction: false,
            auto_activate: true,
        }],
        ground: floor,
        ..default()
    });
    app.finish();
    app.cleanup();
    run(&mut app, 4.0);
    let f = find(&mut app, "Faller");
    assert!(
        (f.z - 25.0).abs() < 4.0,
        "the falling cube rests on the floor: {f}"
    );
    let s = find(&mut app, "Still");
    assert!(
        (s.x - 1000.0).abs() < 1.0,
        "the still prop hasn't moved: {s}"
    );

    app.world_mut().write_message(ScriptEvent {
        name: "Push".into(),
        by: "test".into(),
    });
    run(&mut app, 1.0);
    let s = find(&mut app, "Still");
    assert!(
        s.x > 1200.0,
        "the MegaForce pushed it away from itself: {s}"
    );

    app.world_mut().write_message(ScriptSpawnActor {
        action: 7,
        class: "Test.Rock".into(),
        by: "Script1 #0".into(),
    });
    run(&mut app, 0.2);
    let a = find(&mut app, "Spawned");
    run(&mut app, 3.0);
    let b = find(&mut app, "Spawned");
    assert!(
        a.z > 600.0 && (b.z - 25.0).abs() < 4.0,
        "the made prop fell from 800 to the floor: {a} then {b}"
    );
}
