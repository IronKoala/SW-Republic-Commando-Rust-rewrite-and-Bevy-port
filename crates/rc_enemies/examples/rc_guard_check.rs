//! rc_guard_check: does Kashyyyk's first level (yyy_01e) reach its end once
//! Grievous's bodyguards are killed? No window. Needs the game files:
//!
//! `RC_GAME_DIR=<game folder> cargo run --release -p rc_enemies --example rc_guard_check [MAP]`
//!
//! It runs the map's own level scripts (rc_scripts' Director) together with
//! its enemy spawners (rc_ai's Battle, each spawner making the enemy type
//! whose data/enemies file claims its class). It stands in for the rest of
//! the game: at 2 s it sends `Spawn_Guard` (in the game the squad's work on
//! the landing pad door sends it), and it kills every bodyguard 3 s after it
//! appears (in the game the player does). Every event the scripts and
//! spawners send each other is passed on, as rc_game does.
//!
//! The chain in the map: Spawn_Guard makes guard 1, whose death sends
//! GrievousGuard_dead; a Counter turns that into Spawn_Guard_Three, which
//! makes guard 3, whose death sends ThreeDeadGuards; a Dispatcher plays the
//! closing lines and sends EndLevel, which a script waits for to end the
//! level.
//!
//! Writes `logs/guard_check.log` and ends `GUARD CHECK: N passed, M failed`
//! (exit code 1 if any failed).

use rc_ai::params::AiParams;
use rc_ai::{Battle, BattleEvent};
use rc_movement::{TriangleWorld, Vec3};
use rc_nav::NavGraph;
use rc_package::Package;
use rc_scripts::{Director, Effect, Settings};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::sync::Arc;

const DT: f32 = 1.0 / 30.0;
const SECONDS: f32 = 240.0;
/// Seconds after a guard appears that the stand-in player kills it.
const KILL_AFTER: f32 = 3.0;

fn main() {
    let Ok(game) = std::env::var("RC_GAME_DIR") else {
        eprintln!("set RC_GAME_DIR to the game folder");
        std::process::exit(2);
    };
    let map_name = std::env::args().nth(1).unwrap_or_else(|| "yyy_01e".into());
    let game_data = std::path::Path::new(&game).join("GameData");
    let map_path = game_data.join("Maps").join(format!("{map_name}.ctm"));
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");

    std::fs::create_dir_all("logs").expect("can't create logs");
    let mut log = std::fs::File::create("logs/guard_check.log").expect("can't create the log");
    let mut line = |s: String| {
        println!("{s}");
        writeln!(log, "{s}").ok();
    };

    // Enemy types, as the game reads them.
    let mut types: Vec<Arc<AiParams>> = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(root.join("data/enemies"))
        .expect("can't read data/enemies")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    for p in &files {
        // As rc_enemies does: the built-in battle droid, then the file.
        let mut t = AiParams::battle_droid()
            .with_overrides(&std::fs::read_to_string(p).unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        t.kind.id = p
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_ascii_lowercase();
        types.push(Arc::new(t));
    }

    // The map: path network, spawners, collision, scripts.
    let data = std::fs::read(&map_path)
        .unwrap_or_else(|e| panic!("can't read {}: {e}", map_path.display()));
    let pkg = Package::parse(&data).expect("can't read the map package");
    let nav = NavGraph::from_map(&pkg, &data).expect("no path network");
    let factories = rc_ai::read_factories(&pkg, &data).expect("can't read the spawners");
    let mut claimed = Vec::new();
    for f in factories {
        if let Some(t) = types.iter().find(|t| t.kind.matches(&f.prototype)) {
            claimed.push((f, t.clone()));
        }
    }
    let guards: Vec<String> = claimed
        .iter()
        .filter(|(_, t)| t.kind.id == "grievous_guard")
        .map(|(f, _)| {
            format!(
                "{} (tag {}, makes {})",
                f.name,
                f.tag,
                f.offspring.tag.as_deref().unwrap_or("-")
            )
        })
        .collect();
    line(format!(
        "{map_name}: {} spawners claimed; bodyguard spawners: {guards:?}",
        claimed.len()
    ));
    let mut battle = Battle::with_types(base_arc(&types), nav, claimed);
    battle.scripted = true;

    let coll = rc_level::map_collision::load(&map_path, &game_data, 84.0)
        .expect("can't read the map's collision");
    let v = |p: [f32; 3]| Vec3::new(p[0], p[1], p[2]);
    let world = TriangleWorld::new(
        coll.triangles
            .iter()
            .map(|t| [v(t[0]), v(t[1]), v(t[2])])
            .collect(),
    );
    let level = rc_level::actors::read_level(&pkg, &data).expect("can't read the actors");
    let scripts = rc_level::scripts::read(&pkg, &data, &level);
    let settings = std::fs::read_to_string(root.join("data/scripts/scripts.toml"))
        .ok()
        .and_then(|t| Settings::default().with_overrides(&t).ok())
        .unwrap_or_default();
    let mut director = Director::new(&scripts, &coll.movers.triggers, settings);
    director.start();

    // What happened, for the checks.
    let mut sent: BTreeMap<String, f32> = BTreeMap::new();
    let mut guard_births: Vec<(u32, String, f32)> = Vec::new();
    let mut kill_at: HashMap<u32, f32> = HashMap::new();
    let mut guard_params: Option<Arc<AiParams>> = None;
    let mut leaving: Option<(String, f32)> = None;
    let mut fired_spawn = false;
    let (mut fired_tower, mut fired_seen) = (false, false);
    let mut tower_removed: Option<f32> = None;

    let mut time = 0.0f32;
    let mut to_scripts: Vec<(String, String)> = Vec::new();
    while time < SECONDS && leaving.is_none() {
        time += DT;
        if !fired_spawn && time >= 2.0 {
            fired_spawn = true;
            line(format!(
                "{time:6.2} s: stand-in: Spawn_Guard (the landing pad door work)"
            ));
            director.fire("Spawn_Guard", "rc_guard_check");
        }
        // The tower guard: the player walks into the tower trigger, then
        // sees Grievous (a LineOfSightTrigger sends GreivousVerified).
        if !fired_tower && time >= 10.0 {
            fired_tower = true;
            line(format!(
                "{time:6.2} s: stand-in: Mover_Enter_Tower (the tower trigger)"
            ));
            director.fire("Mover_Enter_Tower", "rc_guard_check");
        }
        if !fired_seen && time >= 14.0 {
            fired_seen = true;
            line(format!(
                "{time:6.2} s: stand-in: GreivousVerified (the line-of-sight trigger)"
            ));
            director.fire("GreivousVerified", "rc_guard_check");
        }
        // Scripts first; their events go to the spawners.
        for e in director.step(DT, None) {
            match e {
                Effect::Event { name, by } => {
                    sent.entry(name.to_ascii_lowercase()).or_insert(time);
                    line(format!("{time:6.2} s: scripts: event {name} (by {by})"));
                    for b in battle.trigger(&name) {
                        handle(
                            &b,
                            time,
                            &battle,
                            &mut line,
                            &mut to_scripts,
                            &mut guard_births,
                            &mut kill_at,
                            &mut guard_params,
                        );
                    }
                }
                Effect::ChangeLevel { url, by, .. } => {
                    line(format!(
                        "{time:6.2} s: scripts: LEAVE the level for {url} (by {by})"
                    ));
                    leaving = Some((url, time));
                }
                Effect::Action {
                    script,
                    index,
                    action,
                    waits,
                    owner,
                } => {
                    // DestroyPawn removes the script's characters, as
                    // rc_scripts_plugin's characters part does.
                    if action.class.to_ascii_lowercase().ends_with("destroypawn") {
                        if let Some(o) = &owner {
                            for id in battle.following(o) {
                                line(format!("{time:6.2} s: scripts: {o} removes character {id}"));
                                battle.remove(id);
                                kill_at.remove(&id);
                                if o.eq_ignore_ascii_case("SeeGuardScript") {
                                    tower_removed = Some(time);
                                }
                            }
                        }
                    }
                    if waits {
                        // Stand in for the game: everything it is asked to
                        // do is done at once.
                        director.action_done(&script, index);
                    }
                }
                _ => {}
            }
        }
        for b in battle.step(DT, &world, None) {
            handle(
                &b,
                time,
                &battle,
                &mut line,
                &mut to_scripts,
                &mut guard_births,
                &mut kill_at,
                &mut guard_params,
            );
        }
        // The stand-in player kills guards that are due.
        let due: Vec<u32> = kill_at
            .iter()
            .filter(|(_, t)| time >= **t)
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            kill_at.remove(&id);
            line(format!("{time:6.2} s: stand-in player kills guard {id}"));
            for b in battle.killed(id) {
                handle(
                    &b,
                    time,
                    &battle,
                    &mut line,
                    &mut to_scripts,
                    &mut guard_births,
                    &mut kill_at,
                    &mut guard_params,
                );
            }
        }
        for (name, by) in to_scripts.drain(..) {
            sent.entry(name.to_ascii_lowercase()).or_insert(time);
            director.fire(&name, &by);
        }
    }

    // Checks.
    let (mut passed, mut failed) = (0, 0);
    let mut check = |line: &mut dyn FnMut(String), ok: bool, what: String| {
        line(format!("{} {what}", if ok { "PASS" } else { "FAIL" }));
        if ok {
            passed += 1;
        } else {
            failed += 1;
        }
    };
    let at = |n: &str| sent.get(&n.to_ascii_lowercase()).copied();
    check(
        &mut line,
        !guard_births.is_empty(),
        format!("bodyguards appeared: {:?}", guard_births),
    );
    if let Some(p) = &guard_params {
        check(
            &mut line,
            p.combat.health == 2000.0 && p.combat.shields == 250.0,
            format!(
                "a bodyguard has the game's health 2000 and shield 250 (has {} and {})",
                p.combat.health, p.combat.shields
            ),
        );
    }
    for ev in [
        "GrievousGuard_dead",
        "Spawn_Guard_Three",
        "ThreeDeadGuards",
        "EndLevel",
    ] {
        check(
            &mut line,
            at(ev).is_some(),
            format!("event {ev} sent (at {:?} s)", at(ev)),
        );
    }
    check(
        &mut line,
        guard_births.len() >= 2,
        format!(
            "the second guard came after the first died ({} guards)",
            guard_births.len()
        ),
    );
    check(
        &mut line,
        tower_removed.is_some(),
        format!("the tower guard is gone after GreivousVerified (at {tower_removed:?} s)"),
    );
    match &leaving {
        Some((url, t)) => check(
            &mut line,
            true,
            format!("the level ends: leaves for {url} at {t:.1} s"),
        ),
        None => check(
            &mut line,
            false,
            format!("the level ends (no level change in {SECONDS} s)"),
        ),
    }
    line(format!("GUARD CHECK: {passed} passed, {failed} failed"));
    if failed > 0 {
        std::process::exit(1);
    }
}

/// The whole battle's limits come from the battle droid's file.
fn base_arc(types: &[Arc<AiParams>]) -> Arc<AiParams> {
    types
        .iter()
        .find(|t| t.kind.id == "battle_droid")
        .cloned()
        .unwrap_or_else(|| types[0].clone())
}

#[allow(clippy::too_many_arguments)]
fn handle(
    b: &BattleEvent,
    time: f32,
    battle: &Battle,
    line: &mut dyn FnMut(String),
    to_scripts: &mut Vec<(String, String)>,
    guard_births: &mut Vec<(u32, String, f32)>,
    kill_at: &mut HashMap<u32, f32>,
    guard_params: &mut Option<Arc<AiParams>>,
) {
    match b {
        BattleEvent::Spawned {
            id, spawner, feet, ..
        } => {
            let sp = &battle.spawners[*spawner];
            let kind = sp.params.kind.id.clone();
            let tag = battle
                .offspring(*id)
                .and_then(|o| o.tag)
                .unwrap_or_default();
            line(format!(
                "{time:6.2} s: spawner {} made {kind} {id} (tag {tag}) at ({:.0}, {:.0}, {:.0})",
                sp.factory.name, feet.x, feet.y, feet.z
            ));
            // The tower guard can't be hurt (its script makes it
            // invulnerable); the player kills the others.
            if kind == "grievous_guard" && !tag.eq_ignore_ascii_case("SeeGrievousGuard") {
                guard_births.push((*id, tag, time));
                kill_at.insert(*id, time + KILL_AFTER);
                *guard_params = Some(sp.params.clone());
            }
        }
        BattleEvent::Event { name, by } => {
            line(format!("{time:6.2} s: spawners: event {name} (by {by})"));
            to_scripts.push((name.clone(), by.clone()));
        }
        BattleEvent::SpawnerOn { spawner, by } => {
            line(format!(
                "{time:6.2} s: spawner {} on ({by:?})",
                battle.spawners[*spawner].factory.name
            ));
        }
        BattleEvent::SpawnerRefused { spawner } => {
            line(format!(
                "{time:6.2} s: spawner {} REFUSED",
                battle.spawners[*spawner].factory.name
            ));
        }
        _ => {}
    }
}
