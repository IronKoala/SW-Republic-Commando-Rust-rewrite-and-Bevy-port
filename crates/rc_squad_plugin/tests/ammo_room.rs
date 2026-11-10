//! Squad mates and the level's ammo boxes, with the real squad and combat
//! plugins and no game files (so it runs anywhere, including the GitHub
//! checks). A flat floor, the player standing still at the middle, and a
//! DC-17m box and a thermal detonator box 600-700 uu behind the squad.
//!
//! - As shipped (`[ammo] limited = "no"`, like the original), nobody counts
//!   shots and nobody goes near the boxes.
//! - With `limited` switched on, Sev (low on rifle shots) and Fixer (out of
//!   grenades) each walk to the box they need and take it; the box is
//!   hidden, and taken boxes are saved as taken like the player's.
//!
//! Run: cargo test --release -p rc_squad_plugin --test ammo_room -- --nocapture

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use rc_combat::ammo_box::AmmoBox;
use rc_combat::{Health, Hittable, Team};
use rc_game_core::pickups::{LevelAmmoBoxes, PlacedAmmoBox};
use rc_game_core::units::to_bevy;
use rc_game_core::{
    CorePlugin, GameLog, GamePaths, GameState, LevelCollision, Player, PlayerCamera,
};
use rc_movement::{TriangleWorld, Vec3 as UVec3};
use rc_squad_plugin::{yaw_rotation, SquadPlugin, SquadState};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const DT: f32 = 1.0 / 60.0;

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

/// A box as the map would place it: (export, name, class, ammo class, at).
fn placed(
    export: usize,
    name: &str,
    class: &str,
    ammo: &str,
    at: UVec3,
    amount: u32,
) -> PlacedAmmoBox {
    PlacedAmmoBox {
        export,
        name: name.into(),
        class: class.into(),
        // The middle of its touch cylinder: 15 uu above the floor.
        at: to_bevy(at + UVec3::new(0.0, 0.0, 15.0)),
        ammo: ammo.into(),
        amount,
        respawn: None,
        radius: 40.0,
        height: 15.0,
        message: String::new(),
        sound: None,
    }
}

const RIFLE_AT: UVec3 = UVec3::new(-700.0, -150.0, 0.0);
const GRENADES_AT: UVec3 = UVec3::new(-650.0, 250.0, 0.0);

fn app(name: &str) -> App {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let log_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../logs/{name}.log"));
    let floor = TriangleWorld::new(box_tris(
        UVec3::new(-3000.0, -3000.0, -20.0),
        UVec3::new(3000.0, 3000.0, 0.0),
    ));
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
    .insert_resource(LevelCollision(Arc::new(floor)))
    .insert_resource(LevelAmmoBoxes(vec![
        placed(
            1,
            "DC17mBlasterBox3",
            "DC17mBlasterBox",
            "DC17mBlasterAmmo",
            RIFLE_AT,
            300,
        ),
        placed(
            2,
            "ThermalDetonatorBox1",
            "ThermalDetonatorBox",
            "ThermalDetonatorAmmo",
            GRENADES_AT,
            5,
        ),
    ]));
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

fn boxes(app: &mut App) -> Vec<(String, bool)> {
    let mut q = app.world_mut().query::<&AmmoBox>();
    let mut v: Vec<(String, bool)> = q
        .iter(app.world())
        .map(|b| (b.name.clone(), b.taken))
        .collect();
    v.sort();
    v
}

#[test]
fn as_shipped_the_squad_leaves_boxes_alone() {
    let mut app = app("ammo_room_off");
    run(&mut app, 1.0);
    {
        let state = app.world().resource::<SquadState>();
        let squad = state.squad.as_ref().expect("squad spawned");
        assert!(
            !squad.settings.ammo.limited,
            "shipped squad.toml has limited = \"no\""
        );
        assert!(squad.members.iter().all(|m| m.ammo.is_empty()));
    }
    run(&mut app, 10.0);
    let b = boxes(&mut app);
    println!("boxes: {b:?}");
    assert_eq!(b.len(), 2, "both boxes placed");
    assert!(b.iter().all(|(_, taken)| !taken));
}

#[test]
fn with_limited_ammo_squad_mates_fetch_the_boxes_they_need() {
    let mut app = app("ammo_room");
    run(&mut app, 1.0);
    {
        let mut state = app.world_mut().resource_mut::<SquadState>();
        let squad = state.squad.as_mut().expect("squad spawned");
        squad.settings.ammo.limited = true;
        for m in &mut squad.members {
            m.ammo = squad.settings.ammo.starting();
        }
        // Sev nearly out of rifle shots, Fixer out of grenades.
        squad.members[0].ammo.insert("dc17m_blaster".into(), 20);
        squad.members[1].ammo.insert("thermal_detonator".into(), 0);
    }
    let mut taken_at = None;
    for f in 0..(15.0 / DT) as usize {
        app.update();
        if boxes(&mut app).iter().all(|(_, t)| *t) {
            taken_at = Some(f as f32 * DT);
            break;
        }
    }
    let b = boxes(&mut app);
    println!("boxes: {b:?}, all taken after {taken_at:?} s");
    let state = app.world().resource::<SquadState>();
    let squad = state.squad.as_ref().unwrap();
    for (i, m) in squad.members.iter().enumerate() {
        println!(
            "{}: {:?}, feet {:?}",
            m.settings.name,
            m.ammo,
            m.pawn.feet()
        );
        assert_eq!(m.fetching, None, "member {i} still fetching");
    }
    assert!(taken_at.is_some(), "boxes not taken: {b:?}");
    assert_eq!(squad.ammo_of(0, "dc17m_blaster"), Some(300));
    assert_eq!(squad.ammo_of(1, "thermal_detonator"), Some(5));
    // Scorch needed nothing and got nothing.
    assert_eq!(squad.ammo_of(2, "dc17m_blaster"), Some(120));

    // Then they go back to the player.
    run(&mut app, 8.0);
    let state = app.world().resource::<SquadState>();
    let squad = state.squad.as_ref().unwrap();
    for m in &squad.members {
        let f = m.pawn.feet();
        let d = (f.x * f.x + f.y * f.y).sqrt();
        println!("{} back to {d:.0} uu from the player", m.settings.name);
        assert!(d < 800.0, "{} stayed {d:.0} uu away", m.settings.name);
    }
}
