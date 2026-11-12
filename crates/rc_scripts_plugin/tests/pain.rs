//! Pain volumes in a Bevy app with no window: the combat part's damage
//! rules with the pain part's timing, on hand-made volumes whose numbers
//! are copied from real maps (geo_04b's pits, ras_04c's fire, geo_04c's
//! crusher). No game files needed.

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use rc_combat::{
    CombatPlugin, Damage, DamageKind, DamageRules, Dead, Health, Hittable, Killed, Shield,
};
use rc_game_core::{
    CorePlugin, GameClass, GamePaths, GameState, LevelCollision, Player, PlayerVitals, ScriptEvent,
};
use rc_level::pain::PainVolume;
use rc_level::volumes::BlockingVolume;
use rc_scripts_plugin::pain::{self, DamageClass, LevelPain, PainClasses};
use std::sync::Arc;
use std::time::Duration;

const STEP: f32 = 0.05;

/// Unreal position to Bevy (same rule as rc_game_core::units).
fn b(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, z, y) / 50.0
}

fn boxed(name: &str, lo: [f32; 3], hi: [f32; 3]) -> BlockingVolume {
    let c = |x: f32, y: f32, z: f32| [x, y, z];
    let (a, b) = (lo, hi);
    BlockingVolume {
        export: 0,
        name: name.into(),
        location: [
            (a[0] + b[0]) / 2.0,
            (a[1] + b[1]) / 2.0,
            (a[2] + b[2]) / 2.0,
        ],
        block_players: false,
        block_actors: false,
        block_traces: false,
        polygons: vec![
            vec![
                c(a[0], a[1], a[2]),
                c(b[0], a[1], a[2]),
                c(b[0], b[1], a[2]),
                c(a[0], b[1], a[2]),
            ],
            vec![
                c(a[0], a[1], b[2]),
                c(b[0], a[1], b[2]),
                c(b[0], b[1], b[2]),
                c(a[0], b[1], b[2]),
            ],
            vec![
                c(a[0], a[1], a[2]),
                c(b[0], a[1], a[2]),
                c(b[0], a[1], b[2]),
                c(a[0], a[1], b[2]),
            ],
            vec![
                c(a[0], b[1], a[2]),
                c(b[0], b[1], a[2]),
                c(b[0], b[1], b[2]),
                c(a[0], b[1], b[2]),
            ],
            vec![
                c(a[0], a[1], a[2]),
                c(a[0], b[1], a[2]),
                c(a[0], b[1], b[2]),
                c(a[0], a[1], b[2]),
            ],
            vec![
                c(b[0], a[1], a[2]),
                c(b[0], b[1], a[2]),
                c(b[0], b[1], b[2]),
                c(b[0], a[1], b[2]),
            ],
        ],
    }
}

fn volume(name: &str, tag: &str, dps: f32, ty: &str, proximity: Option<&str>) -> PainVolume {
    let (lo, hi) = ([-500.0, -500.0, -2000.0], [500.0, 500.0, -300.0]);
    PainVolume {
        export: 0,
        name: name.into(),
        class: "PhysicsVolume".into(),
        tag: tag.into(),
        pain_causing: true,
        damage_per_sec: dps,
        damage_type: ty.into(),
        proximity: proximity.map(str::to_string),
        shape: boxed(name, lo, hi),
        bounds: (lo, hi),
    }
}

/// The damage classes' defaults as the game's packages have them
/// (`rc_probe --class DamageFallToDeath` and so on).
fn classes() -> Vec<(&'static str, DamageClass)> {
    vec![
        (
            "DamageFallToDeath",
            DamageClass {
                kind: DamageKind::Fall,
                rules: DamageRules {
                    bypass_shields: true,
                    kills: true,
                    kills_player: true,
                },
            },
        ),
        (
            "Crushed",
            DamageClass {
                kind: DamageKind::Other,
                rules: DamageRules {
                    bypass_shields: false,
                    kills: false,
                    kills_player: true,
                },
            },
        ),
        (
            "CTDamageFire",
            DamageClass {
                kind: DamageKind::Fire,
                rules: DamageRules::default(),
            },
        ),
    ]
}

#[derive(Resource, Default)]
struct Seen(Vec<(f32, Damage)>);

fn record(time: Res<Time>, mut seen: ResMut<Seen>, mut d: MessageReader<Damage>) {
    let t = time.elapsed_secs();
    seen.0.extend(d.read().map(|d| (t, d.clone())));
}

struct Room {
    app: App,
    player: Entity,
    droid: Entity,
}

impl Room {
    fn new(volumes: Vec<PainVolume>) -> Room {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, TransformPlugin))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
                STEP,
            )))
            .add_plugins((CorePlugin, CombatPlugin))
            .insert_resource(GamePaths::new("/nonexistent-game", vec![data]))
            .insert_resource(LevelCollision(Arc::new(rc_movement::room::test_room())))
            .add_message::<ScriptEvent>()
            .init_resource::<PainClasses>()
            .insert_resource(rc_game_core::GameLog::terminal())
            .insert_resource(LevelPain::from_parts(
                volumes,
                &classes(),
                &[
                    &[
                        "PlayerCommando",
                        "CloneCommando",
                        "Republic",
                        "CTPawn",
                        "Pawn",
                    ],
                    &["BattleDroidTM", "BattleDroid", "Droid", "CTPawn", "Pawn"],
                ],
            ))
            .init_resource::<Seen>()
            .add_systems(Update, pain::run.before(rc_combat::CombatSet::ApplyDamage))
            .add_systems(Last, record);
        let player = app
            .world_mut()
            .spawn((
                Player,
                GameClass("PlayerCommando".into()),
                Transform::from_translation(b(0.0, 0.0, 0.0)),
            ))
            .id();
        let droid = app
            .world_mut()
            .spawn((
                Name::new("droid"),
                GameClass("BattleDroidTM".into()),
                Health::new(100.0),
                Hittable::COMMANDO,
                Transform::from_translation(b(2000.0, 0.0, 0.0)),
            ))
            .id();
        app.world_mut()
            .resource_mut::<NextState<GameState>>()
            .set(GameState::InGame);
        let mut r = Room { app, player, droid };
        r.seconds(0.2);
        r
    }

    fn seconds(&mut self, s: f32) {
        for _ in 0..(s / STEP).round() as usize {
            self.app.update();
        }
    }

    /// Drop a character to the bottom of the pit (feet at z -1000).
    fn into_pit(&mut self, e: Entity) {
        self.app
            .world_mut()
            .get_mut::<Transform>(e)
            .unwrap()
            .translation = b(0.0, 0.0, -1000.0);
    }

    fn hits_on(&self, e: Entity) -> Vec<(f32, f32)> {
        self.app
            .world()
            .resource::<Seen>()
            .0
            .iter()
            .filter(|(_, d)| d.target == e)
            .map(|(t, d)| (*t, d.amount))
            .collect()
    }

    fn health(&self, e: Entity) -> f32 {
        self.app.world().get::<Health>(e).unwrap().current
    }
}

#[test]
fn a_fall_to_death_kills_the_player_outright_past_the_shield() {
    // geo_04b PhysicsVolume0: 1000 a second, DamageFallToDeath,
    // CloneCommando only.
    let mut r = Room::new(vec![volume(
        "PhysicsVolume0",
        "PhysicsVolume",
        1000.0,
        "DamageFallToDeath",
        Some("CloneCommando"),
    )]);
    let shield_before = r.app.world().get::<Shield>(r.player).map(|s| s.current);
    assert!(r.health(r.player) > 0.0);
    r.into_pit(r.player);
    let droid = r.droid;
    r.into_pit(droid);
    r.seconds(0.2);
    let hits = r.hits_on(r.player);
    assert_eq!(hits.len(), 1, "hurt once on entering: {hits:?}");
    assert_eq!(hits[0].1, 1000.0);
    let w = r.app.world();
    assert!(w.get::<Dead>(r.player).is_some(), "dead");
    assert!(
        w.get::<Killed>(r.player).is_some(),
        "dead for good, not down"
    );
    assert!(
        !w.resource::<PlayerVitals>().incapacitated,
        "not waiting for a revive"
    );
    assert_eq!(
        w.get::<Shield>(r.player).map(|s| s.current),
        shield_before,
        "the shield took none of it"
    );
    // The volume hurts commandos only: the droid beside the player is fine.
    assert!(r.hits_on(r.droid).is_empty());
    assert_eq!(r.health(r.droid), 100.0);
}

#[test]
fn a_fire_hurts_once_a_second_while_inside_and_stops_on_its_event() {
    // ras_04c PhysicsVolume2: 75 a second, CTDamageFire, anyone.
    let mut r = Room::new(vec![volume(
        "PhysicsVolume2",
        "Flames",
        75.0,
        "CTDamageFire",
        None,
    )]);
    r.into_pit(r.player);
    r.seconds(3.1);
    let hits = r.hits_on(r.player);
    assert_eq!(hits.len(), 4, "on entering, then at 1, 2 and 3 s: {hits:?}");
    for w in hits.windows(2) {
        assert!(
            (w[1].0 - w[0].0 - 1.0).abs() < STEP * 1.5,
            "a second apart: {hits:?}"
        );
    }
    assert!(hits.iter().all(|h| h.1 == 75.0));
    // An event with the volume's tag switches it off.
    r.app.world_mut().write_message(ScriptEvent {
        name: "flames".into(),
        by: "test".into(),
    });
    r.seconds(2.5);
    assert_eq!(r.hits_on(r.player).len(), 4, "no more once switched off");
    // And on again: the timer starts, the next hit comes a second later.
    r.app.world_mut().write_message(ScriptEvent {
        name: "Flames".into(),
        by: "test".into(),
    });
    r.seconds(1.2);
    assert_eq!(r.hits_on(r.player).len(), 5);
}

#[test]
fn being_crushed_kills_a_downed_player() {
    // geo_04c PhysicsVolume5: 1000 a second, Crushed, any Pawn.
    let mut r = Room::new(vec![volume(
        "PhysicsVolume5",
        "PhysicsVolume",
        1000.0,
        "Crushed",
        Some("Pawn"),
    )]);
    // Down already (shot), waiting for a revive.
    let p = r.player;
    r.app.world_mut().get_mut::<Health>(p).unwrap().current = 0.0;
    r.app.world_mut().entity_mut(p).insert(Dead);
    r.seconds(0.1);
    assert!(r.app.world().resource::<PlayerVitals>().incapacitated);
    r.into_pit(p);
    r.seconds(0.2);
    assert!(
        r.app.world().get::<Killed>(p).is_some(),
        "crushed while down: dead for good"
    );
    assert!(!r.app.world().resource::<PlayerVitals>().incapacitated);
    // A droid isn't killed outright by Crushed (only the player is), so it
    // is just hurt: 100 health, so it dies the ordinary way.
    let d = r.droid;
    r.into_pit(d);
    r.seconds(0.2);
    assert!(r.app.world().get::<Dead>(d).is_some());
    assert!(r.app.world().get::<Killed>(d).is_none());
}
