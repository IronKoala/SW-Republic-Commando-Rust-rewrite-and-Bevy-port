//! The flying gunships and fighters of geo_04a and geo_04d in a Bevy app
//! with no window: placed ones wait switched off until their tag's event,
//! factory ones appear when their factory's event comes, and each flies its
//! patrol's animation once and then sends its stop's event, which the
//! level script waits for to take the ship away. Needs the game files
//! (`RC_GAME_DIR`, with the maps, script packages and animation packages);
//! without them the test says so and passes.

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use rc_cast::{Cast, CastPlugin};
use rc_game_core::{
    CorePlugin, CurrentLevel, GameLog, GamePaths, GameState, RaiseEvent, ScriptEvent,
};
use std::time::Duration;

const STEP: f32 = 1.0 / 60.0;

#[derive(Resource, Default)]
struct Seen {
    /// Event name and the time it came.
    raised: Vec<(String, f32)>,
    clock: f32,
}

fn record(mut seen: ResMut<Seen>, mut r: MessageReader<RaiseEvent>) {
    seen.clock += STEP;
    let t = seen.clock;
    seen.raised.extend(r.read().map(|e| (e.name.clone(), t)));
}

fn game() -> Option<String> {
    let Ok(game) = std::env::var("RC_GAME_DIR") else {
        eprintln!("RC_GAME_DIR not set: skipped");
        return None;
    };
    for f in [
        "GameData/Maps/geo_04a.ctm",
        "GameData/Maps/geo_04d.ctm",
        "GameData/Animations/vehicles.ukx",
    ] {
        if !std::path::Path::new(&game).join(f).exists() {
            eprintln!("{f} not in {game}: skipped");
            return None;
        }
    }
    Some(game)
}

fn level(game: &str, map: &str) -> App {
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        TransformPlugin,
        AssetPlugin::default(),
    ))
    .init_asset::<Mesh>()
    .init_asset::<Image>()
    .init_asset::<StandardMaterial>()
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
        STEP,
    )))
    .add_message::<rc_combat::Died>()
    .add_plugins((CorePlugin, CastPlugin))
    .insert_resource(GamePaths::new(game, vec![data]))
    .insert_resource(GameLog::terminal())
    .insert_resource(CurrentLevel {
        map: Some(map.into()),
    })
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
    app.update();
    app
}

fn run(app: &mut App, seconds: f32) {
    for _ in 0..(seconds / STEP).round() as usize {
        app.update();
    }
}

fn send(app: &mut App, name: &str) {
    app.world_mut().write_message(ScriptEvent {
        name: name.into(),
        by: "test".into(),
    });
}

/// When `name` was raised, seconds after the test clock's `from`.
fn when(app: &App, name: &str, from: f32) -> Option<f32> {
    app.world()
        .resource::<Seen>()
        .raised
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, t)| t - from)
}

fn clock(app: &App) -> f32 {
    app.world().resource::<Seen>().clock
}

fn cast_names(app: &mut App) -> Vec<String> {
    let mut q = app.world_mut().query::<&Cast>();
    q.iter(app.world()).map(|c| c.name.clone()).collect()
}

#[test]
fn geo_04d_flybys_wait_fly_and_send_their_events() {
    let Some(game) = game() else { return };
    let mut app = level(&game, "geo_04d");
    let names = cast_names(&mut app);
    for n in ["GeoFighter1", "GunshipLow1", "GunshipLow3"] {
        assert!(
            names.contains(&n.to_string()),
            "{n} not in the cast: {names:?}"
        );
    }
    // The factory's gunship isn't there until its factory is switched on.
    assert!(!names.iter().any(|n| n == "PawnFactory8 made"));

    // Switched off: nothing happens without their events.
    run(&mut app, 6.0);
    assert_eq!(when(&app, "DestroyFighterFlyby01", 0.0), None);

    // Dispatcher3 sends FighterFlyby01: the fighter wakes, waits 0.2 s and
    // flies Walker01FlyBy (130 frames at 30 a second) once.
    let t0 = clock(&app);
    send(&mut app, "FighterFlyby01");
    send(&mut app, "GunshipFlyby02");
    run(&mut app, 3.0);
    assert_eq!(when(&app, "DestroyFighterFlyby01", t0), None, "too early");
    run(&mut app, 3.0);
    let t = when(&app, "DestroyFighterFlyby01", t0).expect("fighter's event never came");
    assert!((4.3..4.8).contains(&t), "fighter's event after {t:.2} s");
    let g = when(&app, "DestroyGunshipFlyby02", t0).expect("gunship's event never came");
    assert!((4.3..4.8).contains(&g), "gunship's event after {g:.2} s");
    // The other flybys weren't triggered.
    assert_eq!(when(&app, "DestroyGunshipFlyby03", 0.0), None);

    // ScriptedTrigger52 switches GunshipFactory00 on: its gunship (tag
    // Gunship00) appears and dives (Dive, 200 frames), then Strafecomplete.
    let t1 = clock(&app);
    send(&mut app, "GunshipFactory00");
    run(&mut app, 0.1);
    assert!(cast_names(&mut app)
        .iter()
        .any(|n| n == "PawnFactory8 made"));
    run(&mut app, 7.5);
    let s = when(&app, "Strafecomplete", t1).expect("Strafecomplete never came");
    assert!((6.6..7.1).contains(&s), "Strafecomplete after {s:.2} s");
    // A second switch-on makes no second gunship (capacity 1).
    send(&mut app, "GunshipFactory00");
    run(&mut app, 0.2);
    let made = cast_names(&mut app)
        .iter()
        .filter(|n| *n == "PawnFactory8 made")
        .count();
    assert_eq!(made, 1);
}

#[test]
fn geo_04a_gunship_chase_is_made_and_sends_its_events() {
    let Some(game) = game() else { return };
    let mut app = level(&game, "geo_04a");
    let t0 = clock(&app);
    // Trigger14 sends GunshipChase: both factories make their ship.
    send(&mut app, "GunshipChase");
    run(&mut app, 0.1);
    let names = cast_names(&mut app);
    for n in ["PawnFactory8 made", "PawnFactory1 made"] {
        assert!(names.contains(&n.to_string()), "{n} not made: {names:?}");
    }
    run(&mut app, 6.0);
    for e in ["DestroyGunshipChase00", "DestroyGunshipChase01"] {
        let t = when(&app, e, t0).unwrap_or_else(|| panic!("{e} never came"));
        // GunshipChase: 144 frames at 30 a second, after the 0.2 s wait.
        assert!((4.7..5.2).contains(&t), "{e} after {t:.2} s");
    }
}
