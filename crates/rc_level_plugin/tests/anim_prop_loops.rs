//! geo_04b's anti-air turret (`AATurret`, an `AnimProp`) in a Bevy app
//! with no window: it is drawn with the animation set the level links to
//! its model (`CustomMeshSets`), loops its `AnimHealthy` animation
//! (`TurretBreathe`, 800 frames) from the start and plays that
//! animation's own firing sounds each time round. A rope's tag starts its
//! `AnimTriggered` animation. Needs the game files (`RC_GAME_DIR`);
//! without them the test says so and passes.

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use rc_game_core::{GameLog, GamePaths, ScriptEvent};
use rc_level_plugin::anim_props::{self, AnimProp};
use std::time::Duration;

const STEP: f32 = 1.0 / 30.0;

#[test]
fn anti_air_turret_loops_its_own_animation() {
    let Ok(game) = std::env::var("RC_GAME_DIR") else {
        eprintln!("RC_GAME_DIR not set: skipped");
        return;
    };
    let root = std::path::Path::new(&game);
    if !root.join("GameData/Maps/geo_04b.ctm").exists() {
        eprintln!("geo_04b not in {game}: skipped");
        return;
    }
    let lvl = rc_level::map_collision::load(
        &root.join("GameData/Maps/geo_04b.ctm"),
        &root.join("GameData"),
        78.0,
    )
    .expect("geo_04b reads");
    assert_eq!(
        lvl.custom_mesh_sets,
        vec![("AATurret".to_string(), "cGEO04AATurretSet".to_string())]
    );
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, AssetPlugin::default()))
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .init_asset::<AudioSource>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            STEP,
        )))
        .add_message::<rc_game_core::PropAnim>()
        .add_message::<ScriptEvent>()
        .init_state::<rc_game_core::GameState>()
        .insert_resource(GamePaths::new(&game, vec![data]))
        .insert_resource(GameLog::terminal())
        .add_systems(Update, anim_props::animate);
    let paths = app.world().resource::<GamePaths>().clone();
    let report = anim_props::spawn(
        app.world_mut(),
        &paths,
        &lvl.anim_props,
        &lvl.prop_anims,
        &lvl.custom_mesh_sets,
    );
    eprintln!("{report:?}");
    let find = |app: &mut App, name: &str| {
        let mut q = app.world_mut().query::<&AnimProp>();
        q.iter(app.world())
            .find(|p| p.name == name)
            .cloned()
            .unwrap_or_else(|| panic!("{name} is not drawn"))
    };
    let aa = find(&mut app, "AATurret0");
    assert_eq!(
        aa.playing.as_ref().map(|p| p.0.as_str()),
        Some("TurretBreathe")
    );
    assert!(aa.looping);
    // Its base at the bottom of its collision cylinder (22 uu, the
    // AnimProp class's CollisionHeight; bAlignBottom).
    assert!((aa.place.location[2] - (1292.9331 - 22.0)).abs() < 0.01);
    let clip = aa
        .model
        .clip("TurretBreathe")
        .expect("TurretBreathe read")
        .clone();
    assert_eq!(clip.num_frames, 800.0);
    let len = clip.num_frames / clip.rate;
    let sounds = aa.model.sounds.get("TurretBreathe").map_or(0, |s| s.len());
    assert_eq!(sounds, 18, "its firing sounds (AnimNotify_Sound)");

    // A little more than one time round: still playing, wrapped back near
    // the start, and every firing sound heard once plus the first again.
    let steps = ((len + 1.0) / STEP).round() as usize;
    for _ in 0..steps {
        app.update();
    }
    let aa = find(&mut app, "AATurret0");
    let (anim, frame) = aa.playing.clone().expect("still playing");
    assert_eq!(anim, "TurretBreathe");
    assert!(frame < 60.0, "wrapped round to frame {frame}");
    assert!(
        aa.heard.len() >= sounds + 1,
        "heard {} sounds: {:?}",
        aa.heard.len(),
        aa.heard
    );

    // A rope starts its rappel animation when its tag's event comes.
    let rope = find(&mut app, "Rope0");
    let trig = rope.anims.as_ref().and_then(|a| a.triggered.clone());
    assert_eq!(
        trig.as_ref().map(|t| t.0.as_str()),
        Some("CloneEntrance1760")
    );
    app.world_mut().write_message(ScriptEvent {
        name: "CloneRope07".into(),
        by: "test".into(),
    });
    app.update();
    let rope = find(&mut app, "Rope0");
    if rope.model.clip("CloneEntrance1760").is_some() {
        assert_eq!(
            rope.playing.as_ref().map(|p| p.0.as_str()),
            Some("CloneEntrance1760")
        );
    } else {
        eprintln!("Rope0: CloneEntrance1760 not in the staged files");
    }
}
