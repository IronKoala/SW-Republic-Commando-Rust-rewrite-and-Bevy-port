use super::*;
use bevy::state::app::StatesPlugin;
use std::collections::HashSet;

#[test]
fn action_names_round_trip_and_are_unique() {
    let names: HashSet<_> = Action::ALL.iter().map(|a| a.name()).collect();
    assert_eq!(names.len(), Action::ALL.len());
    for a in Action::ALL {
        assert_eq!(Action::from_name(a.name()), Some(a));
    }
}

#[test]
fn action_state_tracks_presses() {
    let mut s = ActionState::default();
    s.update([Action::Fire].into(), Vec2::ZERO, Vec2::ZERO);
    assert!(s.pressed(Action::Fire) && s.just_pressed(Action::Fire));
    s.update([Action::Fire].into(), Vec2::ZERO, Vec2::ZERO);
    assert!(s.pressed(Action::Fire) && !s.just_pressed(Action::Fire));
    s.update(HashSet::new(), Vec2::new(3.0, 4.0), Vec2::ZERO);
    assert!(s.just_released(Action::Fire));
    assert!((s.movement.length() - 1.0).abs() < 1e-5);
}

#[test]
fn units_round_trip() {
    let u = rc_movement::Vec3::new(100.0, -50.0, 168.0);
    let b = units::to_bevy(u);
    assert_eq!(b, Vec3::new(2.0, 3.36, -1.0));
    let back = units::to_unreal(b);
    assert!((back.x - u.x).abs() < 1e-3 && (back.y - u.y).abs() < 1e-3);
}

#[test]
fn paths_find_files_ignoring_case_and_mods_win() {
    let root = std::env::temp_dir().join(format!("rc_game_core_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let game = root.join("game");
    std::fs::create_dir_all(game.join("GameData/Maps")).unwrap();
    std::fs::write(game.join("GameData/Maps/GEO_01A.ctm"), b"x").unwrap();
    let base = root.join("data");
    let modd = root.join("mods/my_mod");
    std::fs::create_dir_all(base.join("weapons")).unwrap();
    std::fs::create_dir_all(modd.join("weapons")).unwrap();
    std::fs::write(base.join("weapons/dc15s.toml"), b"base").unwrap();
    std::fs::write(modd.join("weapons/dc15s.toml"), b"mod").unwrap();
    let p = GamePaths::new(&game, vec![modd.clone(), base.clone()]);
    assert!(p.map_path("geo_01a").exists());
    assert_eq!(
        std::fs::read(p.data_file("weapons/dc15s.toml").unwrap()).unwrap(),
        b"mod"
    );
    assert_eq!(p.data_files("weapons/dc15s.toml").len(), 2);
    assert!(p.data_file("weapons/none.toml").is_none());
    let _ = std::fs::remove_dir_all(&root);
}

#[derive(Resource, Default)]
struct Ticks(u32);

fn tick(mut t: ResMut<Ticks>) {
    t.0 += 1;
}

#[test]
fn pausing_stops_gameplay_without_leaving_the_level() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, CorePlugin))
        .init_resource::<Ticks>()
        .add_systems(Update, tick.in_set(GameSet::Simulate).run_if(playing));
    app.update();
    assert_eq!(
        app.world().resource::<Ticks>().0,
        0,
        "no ticks outside a level"
    );
    app.world_mut()
        .resource_mut::<NextState<GameState>>()
        .set(GameState::InGame);
    app.update();
    app.update();
    let before = app.world().resource::<Ticks>().0;
    assert!(before >= 1);
    app.world_mut()
        .resource_mut::<NextState<Play>>()
        .set(Play::Paused);
    app.update();
    app.update();
    let paused = app.world().resource::<Ticks>().0;
    assert!(paused <= before + 1);
    assert_eq!(
        *app.world().resource::<State<GameState>>().get(),
        GameState::InGame
    );
}
