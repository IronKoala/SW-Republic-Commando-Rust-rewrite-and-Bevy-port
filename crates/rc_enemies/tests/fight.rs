//! The enemies plugin in a Bevy app with no window, together with the real
//! combat plugin: a spawner in the movement test room makes a battle droid
//! that sees the player and shoots, its bolts hurt the player, and the
//! player's own shots destroy it. Checks the numbers, not the picture.

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use rc_ai::{AiParams, Battle, Factory, Mode, SpawnSpot, StartState};
use rc_combat::{Body, CombatPlugin, Damage, Dead, Health, Hittable, Team};
use rc_enemies::look::Pose;
use rc_enemies::{Droid, EnemiesPlugin, Fight};
use rc_game_core::{
    Action, ActionState, CorePlugin, GameLog, GamePaths, GameState, LevelCollision, MusicMood,
    Player, PlayerCamera,
};
use rc_movement::Vec3 as UVec3;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

const STEP: f32 = 1.0 / 60.0;

/// Unreal position to Bevy (same rule as rc_game_core::units).
fn b(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, z, y) / 50.0
}

struct Room {
    app: App,
    player: Entity,
}

impl Room {
    /// The player at (-500, 0) facing +X; a spawner 1500 uu ahead whose
    /// walk-in trigger is where the player stands.
    fn new() -> Room {
        Room::with(AiParams::battle_droid())
    }

    /// The same room with another enemy type in the spawner.
    fn with(params: AiParams) -> Room {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, TransformPlugin))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
                STEP,
            )))
            .add_plugins((CorePlugin, CombatPlugin, EnemiesPlugin))
            // No game folder: no droid mesh or sounds, which these checks
            // don't need.
            .insert_resource(GamePaths::new("/nonexistent-game", vec![data]))
            .insert_resource(GameLog::terminal())
            .insert_resource(LevelCollision(Arc::new(rc_movement::room::test_room())));
        let player = app
            .world_mut()
            .spawn((Player, Transform::from_translation(b(-500.0, 0.0, 0.0))))
            .id();
        app.world_mut().spawn((
            PlayerCamera,
            Transform::from_translation(b(-500.0, 0.0, 144.0)).looking_to(Vec3::X, Vec3::Y),
        ));
        let spot = UVec3::new(1000.0, 0.0, 80.0);
        let factory = Factory {
            name: "TestFactory".into(),
            tag: "TestDroids".into(),
            prototype: "BattleDroidTM".into(),
            location: spot,
            yaw: std::f32::consts::PI,
            spots: vec![SpawnSpot {
                location: spot,
                yaw: std::f32::consts::PI,
            }],
            capacity: 1,
            max_alive: 1,
            generate_forever: false,
            spawn_mode: 0,
            spawn_interval: 1.0,
            start_state: StartState::StationaryAttack,
            team: None,
            triggers: vec![(UVec3::new(-500.0, 0.0, 84.0), 300.0)],
        };
        let params = Arc::new(params);
        let battle = Battle::with_types(
            params.clone(),
            rc_nav::NavGraph::default(),
            vec![(factory, params)],
        );
        app.insert_resource(Fight::new(battle));
        app.world_mut()
            .resource_mut::<NextState<GameState>>()
            .set(GameState::InGame);
        Room { app, player }
    }

    fn seconds(&mut self, s: f32, held: &[Action]) {
        for _ in 0..(s / STEP).round() as usize {
            let now: HashSet<Action> = held.iter().copied().collect();
            self.app
                .world_mut()
                .resource_mut::<ActionState>()
                .update(now, Vec2::ZERO, Vec2::ZERO);
            self.app.update();
        }
    }

    fn droids(&mut self) -> Vec<Entity> {
        let w = self.app.world_mut();
        w.query_filtered::<Entity, With<Droid>>().iter(w).collect()
    }
}

#[test]
fn droid_spawns_spots_and_shoots_the_player() {
    let mut r = Room::new();
    r.seconds(0.5, &[]);
    let droids = r.droids();
    assert_eq!(droids.len(), 1, "one droid spawned");
    let d = droids[0];
    let w = r.app.world();
    assert_eq!(*w.get::<Health>(d).unwrap(), Health::new(100.0));
    assert_eq!(*w.get::<Team>(d).unwrap(), Team::Separatist);
    assert_eq!(w.get::<Hittable>(d).unwrap().half_height, 80.0);
    let feet = w.get::<Transform>(d).unwrap().translation;
    assert!(
        (feet - b(1000.0, 0.0, 0.0)).length() < 0.2,
        "droid feet at {feet}"
    );

    r.seconds(10.0, &[]);
    let fight = r.app.world().resource::<Fight>();
    let s = fight.stats.clone();
    println!("{s:?}");
    assert!(
        s.first_spotted.is_some_and(|t| t < 1.0),
        "spotted quickly: {s:?}"
    );
    assert!(s.shots >= 10, "{} shots in 10 s", s.shots);
    assert!(
        s.hits_on_player >= 1 && s.hits_on_player < s.shots,
        "some bolts hit, not all: {s:?}"
    );
    assert_eq!(*r.app.world().resource::<MusicMood>(), MusicMood::Battle);
    // The player's health or shield went down.
    let h = r.app.world().get::<Health>(r.player).unwrap();
    let sh = r.app.world().get::<rc_combat::Shield>(r.player).unwrap();
    assert!(
        sh.current < sh.max || h.current < h.max,
        "player unhurt: {h:?} {sh:?}"
    );
    // The droid stayed where it was told to (stationary attack) and faces the player.
    let t = r.app.world().get::<Transform>(d).unwrap();
    let forward = t.rotation * Vec3::X;
    assert!(
        forward.x < -0.9,
        "droid faces the player: forward {forward}"
    );
    assert_eq!(
        r.app.world().get::<Droid>(d).unwrap().pose == Pose::Death,
        false
    );
}

#[test]
fn players_shots_destroy_the_droid() {
    let mut r = Room::new();
    r.seconds(1.0, &[]);
    let d = r.droids()[0];
    r.seconds(8.0, &[Action::Fire]);
    let w = r.app.world();
    assert!(
        w.get::<Dead>(d).is_some(),
        "droid destroyed; health {:?}",
        w.get::<Health>(d)
    );
    let fight = w.resource::<Fight>();
    assert_eq!(fight.stats.droids_destroyed, 1);
    assert!(fight.stats.droids_hurt >= 2, "{:?}", fight.stats);
    assert_eq!(fight.battle.fighters[0].mode, Mode::Dead);
    assert_eq!(w.get::<Droid>(d).unwrap().pose, Pose::Death);
    // The fight is over: calm music.
    r.seconds(0.1, &[]);
    assert_eq!(*r.app.world().resource::<MusicMood>(), MusicMood::Calm);
}

/// An enemy type's settings, read from its file in data/enemies.
fn enemy_file(id: &str) -> AiParams {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../data/enemies/{id}.toml"));
    let mut p = AiParams::battle_droid()
        .with_overrides(&std::fs::read_to_string(path).unwrap())
        .unwrap();
    p.kind.id = id.into();
    p
}

#[test]
fn other_enemy_types_spawn_with_their_own_numbers() {
    let want = enemy_file("trandoshan");
    let mut r = Room::with(want.clone());
    r.seconds(0.5, &[]);
    let d = r.droids()[0];
    let w = r.app.world();
    assert_eq!(
        *w.get::<Health>(d).unwrap(),
        Health::new(want.combat.health)
    );
    assert_eq!(*w.get::<Body>(d).unwrap(), Body::Organic);
    assert_eq!(w.get::<Droid>(d).unwrap().kind, "trandoshan");
    assert_eq!(
        w.get::<Hittable>(d).unwrap().half_height,
        want.movement.collision_height
    );
    let name = w.get::<Name>(d).unwrap().to_string();
    assert!(name.starts_with("Trandoshan"), "{name}");
}

#[derive(Resource, Default)]
struct MateHits(u32);

fn count_mate_hits(
    mut hits: ResMut<MateHits>,
    mut d: bevy::ecs::message::MessageReader<Damage>,
    mates: Query<(), (With<Team>, Without<Player>, Without<Droid>)>,
) {
    hits.0 += d.read().filter(|d| mates.contains(d.target)).count() as u32;
}

#[test]
fn droids_shoot_squad_mates_too() {
    let mut r = Room::new();
    // A squad mate halfway between, nearer the droid than the player is.
    let mate = r
        .app
        .world_mut()
        .spawn((
            Name::new("mate"),
            Team::Republic,
            Hittable::COMMANDO,
            Health::new(100.0),
            Transform::from_translation(b(300.0, 150.0, 0.0)),
        ))
        .id();
    r.app
        .init_resource::<MateHits>()
        .add_systems(Last, count_mate_hits);
    r.seconds(10.0, &[]);
    let hits = r.app.world().resource::<MateHits>().0;
    let fight = r.app.world().resource::<Fight>();
    println!("mate hits {hits}, {:?}", fight.stats);
    assert!(hits >= 1, "the droid never hit the mate");
    assert_eq!(fight.battle.fighters[0].target_index, Some(1));
    assert!(r.app.world().get::<Health>(mate).unwrap().current < 100.0);
}

#[test]
fn a_fallen_droid_leaves_its_blaster() {
    let mut r = Room::new();
    r.seconds(1.0, &[]);
    r.seconds(8.0, &[Action::Fire]);
    let w = r.app.world_mut();
    let drops: Vec<String> = w
        .query::<&rc_combat::WeaponPickup>()
        .iter(w)
        .map(|p| p.weapon.clone())
        .collect();
    assert_eq!(drops, ["e5_blaster_pickup"]);
}
