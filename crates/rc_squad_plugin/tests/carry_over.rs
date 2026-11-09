//! The squad going on to the next map with the player, through the real
//! save plugin's level exit and the real squad plugin, in the movement test
//! room. No game files, so it runs anywhere (including the GitHub checks).
//!
//! Run: cargo test -p rc_squad_plugin --test carry_over -- --nocapture

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use rc_combat::{Dead, Health, Shield};
use rc_game_core::save::{SaveAppExt, SaveSlot};
use rc_game_core::units::to_bevy;
use rc_game_core::{
    ChangeLevel, CorePlugin, CurrentLevel, GameLog, GamePaths, GameState, LevelCollision, Player,
    PlayerCamera,
};
use rc_movement::room::test_room;
use rc_movement::Vec3 as UVec3;
use rc_save::{SaveFolder, SavePlugin, TravelSettings, LEVEL_START};
use rc_squad::Stance;
use rc_squad_plugin::carry::SquadCarry;
use rc_squad_plugin::{yaw_rotation, SquadMate, SquadPlugin, SquadState};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const DT: f32 = 1.0 / 60.0;

/// The level "loads" at once: from Loading straight to InGame.
fn fake_level(mut next: ResMut<NextState<GameState>>) {
    next.set(GameState::InGame);
}

fn app(saves: &std::path::Path) -> App {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let data = root.join("../../data");
    let log_path = root.join("../../logs/carry_over.log");
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        TransformPlugin,
        CorePlugin,
        rc_combat::CombatPlugin,
        SquadPlugin,
        SavePlugin,
    ))
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
        DT,
    )))
    .insert_resource(GamePaths::new(PathBuf::from("no-game-here"), vec![data]))
    .insert_resource(GameLog::create(&log_path).expect("log file"))
    .insert_resource(SaveFolder(saves.to_path_buf()))
    .insert_resource(LevelCollision(Arc::new(test_room())))
    .add_systems(OnEnter(GameState::Loading), fake_level);
    app.world_mut().resource_mut::<CurrentLevel>().map = Some("geo_01a".into());
    app.world_mut()
        .resource_mut::<NextState<GameState>>()
        .set(GameState::Loading);
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
    for _ in 0..(secs / DT).round().max(1.0) as usize {
        app.update();
    }
}

fn mate(app: &mut App, name: &str) -> Entity {
    let mut q = app.world_mut().query::<(Entity, &SquadMate)>();
    q.iter(app.world())
        .find(|(_, m)| m.name == name)
        .map(|(e, _)| e)
        .unwrap_or_else(|| panic!("no {name}"))
}

fn health(app: &mut App, name: &str) -> (f32, f32) {
    let e = mate(app, name);
    let h = app.world().get::<Health>(e).unwrap();
    (h.current, h.max)
}

fn stance(app: &App) -> Stance {
    app.world()
        .resource::<SquadState>()
        .squad
        .as_ref()
        .unwrap()
        .stance
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rc_squad_carry_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn leave(app: &mut App, map: &str) {
    app.world_mut().write_message(ChangeLevel {
        map: map.into(),
        portal: None,
        by: "test".into(),
    });
}

#[test]
fn the_squad_keeps_its_health_and_order_on_the_next_map() {
    let dir = temp_dir("keeps");
    let mut app = app(&dir);
    run(&mut app, 0.5);
    assert_eq!(
        app.world().resource::<TravelSettings>().carry,
        ["combat", "squad_carry"],
        "default carry list"
    );
    assert!(app.world().resource::<SquadState>().squad.is_some());

    // Fixer hurt, Scorch down (and not yet revived), Sev untouched; the
    // squad told to search and destroy.
    let fixer = mate(&mut app, "Fixer");
    app.world_mut().get_mut::<Health>(fixer).unwrap().current = 123.0;
    app.world_mut().get_mut::<Shield>(fixer).unwrap().current = 0.0;
    let scorch = mate(&mut app, "Scorch");
    app.world_mut().get_mut::<Health>(scorch).unwrap().current = 0.0;
    app.world_mut().entity_mut(scorch).insert(Dead);
    app.world_mut()
        .resource_mut::<SquadState>()
        .squad
        .as_mut()
        .unwrap()
        .stance = Stance::SearchAndDestroy;
    let (sev_before, max) = health(&mut app, "Sev");

    leave(&mut app, "geo_01b");
    run(&mut app, 0.5);
    assert_eq!(
        app.world().resource::<CurrentLevel>().map.as_deref(),
        Some("geo_01b")
    );
    let (sev, _) = health(&mut app, "Sev");
    let (fixer_h, _) = health(&mut app, "Fixer");
    let (scorch_h, _) = health(&mut app, "Scorch");
    println!("arrived: Sev {sev}, Fixer {fixer_h}, Scorch {scorch_h} (of {max})");
    assert_eq!(sev, sev_before);
    assert_eq!(fixer_h, 123.0);
    assert_eq!(scorch_h, max * 0.25, "a downed mate gets up with a quarter");
    let scorch = mate(&mut app, "Scorch");
    assert!(
        app.world().get::<Dead>(scorch).is_none(),
        "Scorch still down"
    );
    let fixer = mate(&mut app, "Fixer");
    let sh = app.world().get::<Shield>(fixer).unwrap();
    assert_eq!(sh.current, sh.max, "shields come back full");
    assert_eq!(stance(&app), Stance::SearchAndDestroy);
    let squad_down = app
        .world()
        .resource::<SquadState>()
        .squad
        .as_ref()
        .unwrap()
        .members
        .iter()
        .any(|m| m.down);
    assert!(!squad_down, "the squad still thinks someone is down");

    // The new map's start checkpoint has the carried health, so "retry
    // from checkpoint" doesn't heal them.
    let cp = rc_save::file::read(&rc_save::file::slot_path(&dir, &SaveSlot::Checkpoint)).unwrap();
    assert_eq!(cp.header.map, "geo_01b");
    assert_eq!(cp.header.checkpoint.as_deref(), Some(LEVEL_START));
    let fixer_saved = cp
        .part("squad")
        .and_then(|s| s.get("members").and_then(|m| m.as_array()).cloned())
        .unwrap_or_default()
        .iter()
        .find(|m| m.get("name").and_then(|n| n.as_str()) == Some("Fixer"))
        .and_then(|m| m.get("health").and_then(|h| h.as_float()));
    let carried = cp.part("squad_carry");
    println!("checkpoint: Fixer in [squad] {fixer_saved:?}, [squad_carry] {carried:?}");
    assert!(
        fixer_saved == Some(123.0) || carried.is_some_and(|c| c.to_string().contains("123")),
        "the checkpoint lost Fixer's carried health"
    );
    // An ordinary save file has no [squad_carry] once the squad is back.
    rc_save_now(&mut app);
    let quick = rc_save::file::read(&rc_save::file::slot_path(&dir, &SaveSlot::Quick)).unwrap();
    assert!(quick.part("squad").is_some());
    assert!(quick.part("squad_carry").is_none());

    // "Secure position" isn't carried (the spot is on the old map).
    app.world_mut()
        .resource_mut::<SquadState>()
        .squad
        .as_mut()
        .unwrap()
        .stance = Stance::SecurePosition;
    leave(&mut app, "geo_01c");
    run(&mut app, 0.5);
    assert_eq!(stance(&app), Stance::FormUp);
    assert_eq!(health(&mut app, "Fixer").0, 123.0);
    let _ = std::fs::remove_dir_all(&dir);
}

fn rc_save_now(app: &mut App) {
    app.world_mut().write_message(rc_game_core::save::SaveGame {
        slot: SaveSlot::Quick,
    });
    run(app, 0.1);
}

#[test]
fn a_carry_reads_back_and_rejects_a_bad_order() {
    let c: SquadCarry = toml::from_str(
        "stance = 'search_and_destroy'\n[[members]]\nname = 'Sev'\nhealth = 250.0\n",
    )
    .unwrap();
    assert_eq!(c.members[0].health, 250.0);
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_save_part::<SquadCarry>();
    let bad = SquadCarry {
        stance: Some("dance".into()),
        members: vec![],
    };
    use rc_game_core::save::SavePart;
    assert!(SquadCarry::load(app.world_mut(), bad).is_err());
}
