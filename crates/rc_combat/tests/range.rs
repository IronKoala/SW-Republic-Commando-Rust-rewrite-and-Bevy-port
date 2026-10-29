//! The combat plugin in a Bevy app with no window: the player fires the
//! game's weapons at a target in the movement test room, and a droid fires
//! back. Checks the numbers, not the picture.

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use rc_combat::{
    Arsenal, CombatPlugin, Damage, Dead, Died, FireShot, Health, Hittable, Shield, ShotLanded,
    Team, WeaponLibrary,
};
use rc_game_core::{
    Action, ActionState, CorePlugin, GamePaths, GameState, LevelCollision, Player, PlayerCamera,
    PlayerVitals, WeaponReadout,
};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

const STEP: f32 = 1.0 / 60.0;

/// Unreal position to Bevy (same rule as rc_game_core::units).
fn b(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, z, y) / 50.0
}

#[derive(Resource, Default)]
struct Seen {
    landed: Vec<ShotLanded>,
    died: Vec<Died>,
    damage: Vec<Damage>,
}

fn record(
    mut seen: ResMut<Seen>,
    mut l: MessageReader<ShotLanded>,
    mut d: MessageReader<Died>,
    mut h: MessageReader<Damage>,
) {
    seen.landed.extend(l.read().cloned());
    seen.died.extend(d.read().cloned());
    seen.damage.extend(h.read().cloned());
}

struct Range {
    app: App,
    player: Entity,
    droid: Entity,
}

impl Range {
    fn new() -> Range {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, TransformPlugin))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
                STEP,
            )))
            .add_plugins((CorePlugin, CombatPlugin))
            // No game folder: no sounds or models, which these checks don't need.
            .insert_resource(GamePaths::new("/nonexistent-game", vec![data]))
            .insert_resource(LevelCollision(Arc::new(rc_movement::room::test_room())))
            .init_resource::<Seen>()
            .add_systems(Last, record);
        // The player stands at (-500, 0) facing +X, eye 144 uu above the feet.
        let player = app
            .world_mut()
            .spawn((Player, Transform::from_translation(b(-500.0, 0.0, 0.0))))
            .id();
        app.world_mut().spawn((
            PlayerCamera,
            Transform::from_translation(b(-500.0, 0.0, 144.0)).looking_to(Vec3::X, Vec3::Y),
        ));
        // A target 1000 uu ahead with a battle droid's numbers.
        let droid = app
            .world_mut()
            .spawn((
                Name::new("droid"),
                Health::new(100.0),
                Team::Separatist,
                Hittable {
                    radius: 40.0,
                    half_height: 84.0,
                    center_up: 84.0,
                },
                Transform::from_translation(b(500.0, 0.0, 0.0)),
            ))
            .id();
        app.world_mut()
            .resource_mut::<NextState<GameState>>()
            .set(GameState::InGame);
        let mut r = Range { app, player, droid };
        r.run(3, &[]);
        r
    }

    /// Run `frames` frames with `held` actions held.
    fn run(&mut self, frames: usize, held: &[Action]) {
        for _ in 0..frames {
            let now: HashSet<Action> = held.iter().copied().collect();
            self.app
                .world_mut()
                .resource_mut::<ActionState>()
                .update(now, Vec2::ZERO, Vec2::ZERO);
            self.app.update();
        }
    }

    fn seconds(&mut self, s: f32, held: &[Action]) {
        self.run((s / STEP).round() as usize, held);
    }

    fn arsenal(&self) -> Arsenal {
        self.app
            .world()
            .get::<Arsenal>(self.player)
            .unwrap()
            .clone()
    }

    fn seen(&mut self) -> Seen {
        std::mem::take(&mut *self.app.world_mut().resource_mut::<Seen>())
    }
}

#[test]
fn every_weapon_file_reads() {
    let r = Range::new();
    let lib = r.app.world().resource::<WeaponLibrary>();
    assert!(lib.errors.is_empty(), "{:?}", lib.errors);
    let ids: Vec<_> = lib.weapons.keys().cloned().collect();
    assert_eq!(
        ids,
        [
            "dc15s_pistol",
            "dc17m_blaster",
            "e5_blaster",
            "e5_blaster_tm"
        ]
    );
}

#[test]
fn player_gets_the_games_numbers() {
    let r = Range::new();
    let w = r.app.world();
    assert_eq!(*w.get::<Health>(r.player).unwrap(), Health::new(200.0));
    let s = w.get::<Shield>(r.player).unwrap();
    assert_eq!(
        (s.current, s.recharge_rate, s.recharge_delay),
        (100.0, 20.0, 3.0)
    );
    let a = r.arsenal();
    assert_eq!(a.weapons[0].0, "dc17m_blaster");
    assert_eq!(
        (a.weapons[0].1.shots_in_clip(), a.weapons[0].1.reserve),
        (60, 60)
    );
    let v = w.resource::<PlayerVitals>();
    assert_eq!((v.health, v.shield), (200.0, 100.0));
    let ro = w.resource::<WeaponReadout>();
    assert_eq!(
        (ro.clip, ro.clip_size, ro.reserve),
        (Some(60), Some(60), Some(60))
    );
    assert!(ro.on_target, "the crosshair is on the droid");
}

#[test]
fn dc17m_kills_a_droid_in_five_hits() {
    let mut r = Range::new();
    r.seen();
    r.seconds(1.0, &[Action::Fire]);
    r.seconds(1.0, &[]);
    let a = r.arsenal();
    let fired = a.weapons[0].1.shots_fired;
    assert!(
        (10..=11).contains(&fired),
        "about 10 shots a second, got {fired}"
    );
    let seen = r.seen();
    let hits = seen.damage.iter().filter(|d| d.target == r.droid).count();
    assert_eq!(
        hits, 5,
        "100 health / 20 damage = 5 hits, then the droid is down"
    );
    assert!(seen.damage.iter().all(|d| d.amount == 20.0));
    assert_eq!(seen.died.len(), 1);
    assert_eq!(seen.died[0].entity, r.droid);
    assert_eq!(seen.died[0].killer, Some(r.player));
    assert!(r.app.world().get::<Dead>(r.droid).is_some());
    // The other shots fly past the fallen droid into the far wall (x = 4000).
    let walls: Vec<_> = seen.landed.iter().filter(|l| l.on_level).collect();
    assert_eq!(walls.len(), fired as usize - 5);
    for l in &walls {
        let x = l.point.x * 50.0;
        assert!((x - 4000.0).abs() < 1.0, "wall hit at x {x}");
    }
}

#[test]
fn shots_land_around_the_crosshair() {
    let mut r = Range::new();
    r.app.world_mut().entity_mut(r.droid).insert(Dead);
    r.seen();
    r.seconds(2.0, &[Action::Fire]);
    r.seconds(1.0, &[]);
    let seen = r.seen();
    assert!(
        (20..=21).contains(&seen.landed.len()),
        "{}",
        seen.landed.len()
    );
    // The crosshair is on the far wall at (4000, 0, 144). Bolts leave the
    // muzzle (127 forward, 30 right, 10 down from the eye) and fly to it,
    // off by up to 15 uu per 1000 uu: about 66 uu at 4370 uu.
    let (mut most_y, mut most_z) = (0.0f32, 0.0f32);
    for l in &seen.landed {
        let (x, y, z) = (l.point.x * 50.0, l.point.z * 50.0, l.point.y * 50.0);
        assert!((x - 4000.0).abs() < 1.0);
        most_y = most_y.max(y.abs());
        most_z = most_z.max((z - 144.0).abs());
    }
    assert!(most_y < 67.0 && most_z < 67.0, "spread {most_y} {most_z}");
    assert!(
        most_y > 20.0 && most_z > 20.0,
        "there is some spread: {most_y} {most_z}"
    );
}

#[test]
fn bolts_take_time_to_arrive() {
    let mut r = Range::new();
    r.seen();
    r.run(1, &[Action::Fire]);
    // The droid's near side is ~960 uu ahead of the eye, the muzzle 127 uu
    // ahead: ~830 uu, which a bolt from rest at 60000 uu/s² covers in
    // about 0.166 s (10 frames).
    let mut frames = 0;
    while r.app.world().resource::<Seen>().damage.is_empty() {
        r.run(1, &[]);
        frames += 1;
        assert!(frames < 60);
    }
    assert!((9..=12).contains(&frames), "hit after {frames} frames");
}

#[test]
fn magazine_empties_and_reloads() {
    let mut r = Range::new();
    // Move the droid away so it doesn't matter.
    r.app.world_mut().entity_mut(r.droid).insert(Dead);
    r.seconds(7.0, &[Action::Fire]);
    let a = r.arsenal();
    let w = &a.weapons[0].1;
    // 60 shots in 6 s, then a 1.5 s reload has started.
    assert_eq!(w.shots_fired, 60);
    assert!(w.reloading.is_some());
    r.seconds(1.6, &[]);
    let a = r.arsenal();
    assert_eq!(
        (a.weapons[0].1.shots_in_clip(), a.weapons[0].1.reserve),
        (60, 0)
    );
    let ro = r.app.world().resource::<WeaponReadout>();
    assert_eq!((ro.clip, ro.reserve), (Some(60), Some(0)));
}

#[test]
fn pistol_is_one_shot_per_press_and_recharges() {
    let mut r = Range::new();
    r.run(1, &[Action::NextWeapon]);
    r.run(1, &[]);
    assert_eq!(r.arsenal().current, 1);
    r.seconds(1.0, &[Action::Fire]);
    assert_eq!(
        r.arsenal().weapons[1].1.shots_fired,
        1,
        "holding fires once"
    );
    for _ in 0..6 {
        r.run(1, &[Action::Fire]);
        r.seconds(0.25, &[]);
    }
    let charge = r.arsenal().weapons[1].1.clip;
    assert!(charge < 5.0);
    r.seconds(4.0, &[]);
    assert_eq!(r.arsenal().weapons[1].1.clip, 5.0);
    // Two 55-damage bolts take the 100-health droid down.
    let seen = r.seen();
    assert_eq!(seen.died.len(), 1);
    assert_eq!(
        seen.damage.iter().filter(|d| d.target == r.droid).count(),
        2
    );
}

#[test]
fn droid_fire_hits_shield_then_health() {
    let mut r = Range::new();
    r.seen();
    // The droid fires its E-5 at the player's chest five times.
    let from = b(500.0, 0.0, 120.0);
    let to = b(-500.0, 0.0, 100.0);
    for _ in 0..5 {
        r.app.world_mut().write_message(FireShot {
            shooter: r.droid,
            origin: from,
            direction: (to - from).normalize(),
            weapon: "e5_blaster".into(),
        });
        r.seconds(0.3, &[]);
    }
    let seen = r.seen();
    let on_player: Vec<_> = seen
        .damage
        .iter()
        .filter(|d| d.target == r.player)
        .collect();
    assert_eq!(on_player.len(), 5);
    let w = r.app.world();
    // 5 x 24 = 120: the shield takes 100, health the other 20.
    assert_eq!(w.get::<Shield>(r.player).unwrap().current, 0.0);
    assert_eq!(w.get::<Health>(r.player).unwrap().current, 180.0);
    // Nothing for 3 s, then 20 a second.
    r.seconds(2.9, &[]);
    assert_eq!(r.app.world().get::<Shield>(r.player).unwrap().current, 0.0);
    r.seconds(2.0, &[]);
    let s = r.app.world().get::<Shield>(r.player).unwrap().current;
    assert!((s - 38.0).abs() < 4.0, "about 2 s of recharge: {s}");
    let v = r.app.world().resource::<PlayerVitals>();
    assert_eq!(v.health, 180.0);
}

/// With the real game (RC_GAME_DIR set to the install, or a copy of its
/// System, Sounds, StaticMeshes and Textures folders): every sound the
/// weapon files name is found, and the DC-17m's model is built.
#[test]
fn real_game_sounds_and_model() {
    let Some(game) = std::env::var_os("RC_GAME_DIR") else {
        eprintln!("RC_GAME_DIR not set: skipped (not tested)");
        return;
    };
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        TransformPlugin,
        bevy::asset::AssetPlugin::default(),
    ))
    .init_asset::<bevy::audio::AudioSource>()
    .init_asset::<Mesh>()
    .init_asset::<StandardMaterial>()
    .init_asset::<Image>()
    .add_plugins((CorePlugin, CombatPlugin))
    .insert_resource(GamePaths::new(game, vec![data]));
    app.world_mut().spawn((Player, Transform::default()));
    let cam = app
        .world_mut()
        .spawn((PlayerCamera, Transform::default()))
        .id();
    app.world_mut()
        .resource_mut::<NextState<GameState>>()
        .set(GameState::InGame);
    for _ in 0..3 {
        app.update();
    }
    let lib = app.world().resource::<WeaponLibrary>();
    let mut wanted: Vec<String> = vec![];
    for p in lib.weapons.values() {
        wanted.extend(p.fire_sounds.iter().cloned());
        wanted.extend(p.reload_sound.iter().cloned());
        wanted.extend(p.empty_sound.iter().cloned());
    }
    wanted.sort();
    wanted.dedup();
    let missing: Vec<_> = wanted
        .iter()
        .filter(|n| !lib.sounds.contains_key(*n))
        .collect();
    assert!(missing.is_empty(), "sounds not found: {missing:?}");
    eprintln!("sounds loaded: {}", lib.sounds.len());
    // The DC-17m in hand: a child of the camera with one mesh per section.
    let children = app
        .world()
        .get::<Children>(cam)
        .expect("model under the camera");
    let model = children[0];
    let parts = app.world().get::<Children>(model).map_or(0, |c| c.len());
    assert!(parts >= 1, "model has {parts} parts");
    let meshes = app.world().resource::<Assets<Mesh>>();
    let mut tris = 0;
    for part in app.world().get::<Children>(model).unwrap() {
        let h = &app.world().get::<Mesh3d>(*part).unwrap().0;
        tris += meshes.get(h).unwrap().indices().unwrap().len() / 3;
    }
    eprintln!("DC-17m model: {parts} parts, {tris} triangles");
    assert!(tris > 100);
}
