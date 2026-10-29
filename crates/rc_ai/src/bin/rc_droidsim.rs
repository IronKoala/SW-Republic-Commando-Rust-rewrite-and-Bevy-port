//! rc_droidsim: battle droids against a stand-in player on a real map, with
//! no window, logging the numbers.
//!
//! Usage: rc_droidsim [MAP] [--seconds N] [--settings FILE] [--goal NODE]
//!                    [--no-return-fire]
//!
//! The stand-in player starts at the map's PlayerStart and walks the path
//! network towards `--goal` (a path node name or tag; by default the walk-in
//! trigger of the battle droid spawner nearest the start). Spawners switch
//! on when it walks into their trigger, or comes within
//! `activate_unscripted_within` of a spawner that only scripts could start.
//! Droids then see, take cover, keep their distance and shoot. Their shots
//! are traced as straight lines (the weapons thread's bolts fly, which this
//! test doesn't model) against the level and the player's collision box,
//! and counted. Unless `--no-return-fire`, the player shoots back at the
//! nearest droid it can see, so droids also take hits and die.
//!
//! Map: a name from GameData/Maps (default geo_01a) or a .ctm path. Game
//! folder: RC_GAME_DIR or the default Steam path. Log: logs/rc_droidsim.log.
//! Ends with CHECK lines and exits 1 if any fails.

use rc_ai::{AiEvent, AiParams, Mode, Target};
use rc_movement::{MoveParams, Pawn, TriangleWorld, Vec3};
use rc_nav::{NavGraph, PathFollower};
use rc_package::Package;
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";
const DT: f32 = 1.0 / 30.0;
/// The stand-in player's return fire: one shot per this many seconds.
const PLAYER_SHOT_INTERVAL: f32 = 0.3;
/// Damage per player shot (a stand-in, not the DC-17's real number).
const PLAYER_SHOT_DAMAGE: f32 = 35.0;
/// Damage per droid bolt: BattleDroidBlasterProjTM's Damage.
const DROID_BOLT_DAMAGE: f32 = 2.0;

struct Log(Option<std::fs::File>);
impl Log {
    fn line(&mut self, s: impl AsRef<str>) {
        println!("{}", s.as_ref());
        if let Some(f) = self.0.as_mut() {
            let _ = writeln!(f, "{}", s.as_ref());
        }
    }
}

fn fmt(v: Vec3) -> String {
    format!("({:.0}, {:.0}, {:.0})", v.x, v.y, v.z)
}

/// Where along `origin + t * dir` a ray first passes within the player's
/// collision box (an upright cylinder), if it does within `max` uu.
fn ray_hits_cylinder(
    origin: Vec3,
    dir: Vec3,
    centre: Vec3,
    radius: f32,
    half_height: f32,
    max: f32,
) -> Option<f32> {
    // Closest approach in the flat plane.
    let o = origin - centre;
    let (dx, dy) = (dir.x, dir.y);
    let a = dx * dx + dy * dy;
    if a < 1e-8 {
        return None;
    }
    let b = 2.0 * (o.x * dx + o.y * dy);
    let c = o.x * o.x + o.y * o.y - radius * radius;
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    for t in [(-b - s) / (2.0 * a), (-b + s) / (2.0 * a)] {
        if t > 0.0 && t < max {
            let z = o.z + dir.z * t;
            if z.abs() <= half_height {
                return Some(t);
            }
        }
    }
    None
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut map = "geo_01a".to_string();
    let mut seconds = 90.0f32;
    let mut settings = PathBuf::from("data/enemies/battle_droid.toml");
    let mut goal_name: Option<String> = None;
    let mut return_fire = true;
    let mut from_player_start = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--seconds" => seconds = args.next().and_then(|s| s.parse().ok()).unwrap_or(seconds),
            "--settings" => settings = args.next().map(PathBuf::from).unwrap_or(settings),
            "--goal" => goal_name = args.next(),
            "--no-return-fire" => return_fire = false,
            "--from-player-start" => from_player_start = true,
            other => map = other.to_string(),
        }
    }
    let game =
        PathBuf::from(std::env::var("RC_GAME_DIR").unwrap_or_else(|_| DEFAULT_GAME_DIR.into()));
    let game_data = game.join("GameData");
    let map_path = if map.ends_with(".ctm") {
        PathBuf::from(&map)
    } else {
        game_data.join("Maps").join(format!("{map}.ctm"))
    };
    let _ = std::fs::create_dir_all("logs");
    let mut log = Log(std::fs::File::create("logs/rc_droidsim.log").ok());
    let mut failed = 0;
    let mut check = |log: &mut Log, ok: bool, what: String| {
        log.line(format!(
            "CHECK {}: {what}",
            if ok { "PASS" } else { "FAIL" }
        ));
        if !ok {
            failed += 1;
        }
    };
    let fail = |log: &mut Log, why: String| -> ! {
        log.line(format!("FAIL: {why}"));
        std::process::exit(1)
    };

    // Settings.
    let params = match std::fs::read_to_string(&settings) {
        Ok(t) => AiParams::battle_droid()
            .with_overrides(&t)
            .unwrap_or_else(|e| fail(&mut log, format!("{}: {e}", settings.display()))),
        Err(_) => {
            log.line(format!(
                "no {} here; using the built-in battle droid values",
                settings.display()
            ));
            AiParams::battle_droid()
        }
    };
    let params = Arc::new(params);

    // The map.
    let t0 = std::time::Instant::now();
    let data = std::fs::read(&map_path)
        .unwrap_or_else(|e| fail(&mut log, format!("can't read {}: {e}", map_path.display())));
    let pkg = Package::parse(&data).unwrap_or_else(|e| fail(&mut log, e.to_string()));
    let mut nav = NavGraph::from_map(&pkg, &data).unwrap_or_else(|e| fail(&mut log, e));
    // Barricade path points open when level scripts say so, which we don't
    // run yet; open them all so the stand-in player can move on.
    let opened = nav.blocked.len();
    nav.blocked.clear();
    let factories = rc_ai::read_factories(&pkg, &data).unwrap_or_else(|e| fail(&mut log, e));
    let player_params = MoveParams::player_commando();
    let coll = rc_level::map_collision::load(&map_path, &game_data, player_params.collision_height)
        .unwrap_or_else(|e| fail(&mut log, e));
    let v = |p: [f32; 3]| Vec3::new(p[0], p[1], p[2]);
    let world = TriangleWorld::new(
        coll.triangles
            .iter()
            .map(|t| [v(t[0]), v(t[1]), v(t[2])])
            .collect(),
    );
    log.line(format!(
        "map {}: {} collision triangles ({} BSP nodes, {} meshes, {} mesh failures), {} path nodes, {} links, {} spawners, loaded in {:.0} ms",
        map_path.display(),
        coll.triangles.len(),
        coll.bsp_nodes,
        coll.mesh_actors,
        coll.mesh_failures,
        nav.nodes.len(),
        nav.link_count(),
        factories.len(),
        t0.elapsed().as_secs_f64() * 1000.0
    ));
    let droid_factories: Vec<&rc_ai::Factory> = factories
        .iter()
        .filter(|f| f.prototype.starts_with("BattleDroid"))
        .collect();
    let mut by_class: BTreeMap<&str, usize> = BTreeMap::new();
    for f in &factories {
        *by_class.entry(f.prototype.as_str()).or_default() += 1;
    }
    log.line(format!("spawners by what they make: {by_class:?}"));
    log.line(format!(
        "{opened} blocked path points (barricades the level scripts open) opened for this test"
    ));
    let with_trigger = droid_factories
        .iter()
        .filter(|f| !f.triggers.is_empty())
        .count();
    let spots: usize = droid_factories.iter().map(|f| f.spots.len()).sum();
    log.line(format!(
        "battle droid spawners: {}; {} have walk-in triggers, the rest start from scripts; {} spawn points in all",
        droid_factories.len(),
        with_trigger,
        spots
    ));
    check(
        &mut log,
        !droid_factories.is_empty(),
        format!("battle droid spawners found ({})", droid_factories.len()),
    );

    // The stand-in player.
    let Some((start_feet, start_yaw)) = coll.start.map(|(p, y)| (v(p), y)) else {
        fail(&mut log, "no PlayerStart".into())
    };
    let mut player = Pawn::new(player_params.clone(), start_feet);
    // Where the player starts and walks to. The map's own start area is
    // walled in until level scripts open it, so by default the stand-in
    // starts on a path node 2500-4500 uu (by path) from a battle droid
    // spawner's walk-in trigger and walks into it.
    let goal = match &goal_name {
        Some(n) => nav
            .find(n)
            .map(|i| nav.nodes[i].location)
            .unwrap_or_else(|| fail(&mut log, format!("no path node called {n}"))),
        None => {
            let mut chosen = None;
            'outer: for f in &droid_factories {
                for &(c, _) in &f.triggers {
                    if from_player_start {
                        if nav.route(player.location, c).is_some() {
                            chosen = Some((c, f.tag.clone(), None));
                            break 'outer;
                        }
                        continue;
                    }
                    for (i, n) in nav.nodes.iter().enumerate() {
                        if !matches!(n.kind, rc_nav::NodeKind::Path) {
                            continue;
                        }
                        let straight = (n.location - c).length();
                        if !(2000.0..4500.0).contains(&straight) {
                            continue;
                        }
                        if let Some(r) = nav.route(n.location, c) {
                            if (2500.0..4500.0).contains(&r.length) {
                                chosen = Some((c, f.tag.clone(), Some(i)));
                                break 'outer;
                            }
                        }
                    }
                }
            }
            let (c, tag, start) = chosen.unwrap_or_else(|| {
                fail(&mut log, "no droid spawner trigger can be reached".into())
            });
            if let Some(i) = start {
                let feet =
                    nav.nodes[i].location - Vec3::new(0.0, 0.0, player_params.collision_height);
                player = Pawn::new(player_params.clone(), feet);
                log.line(format!(
                    "player starts at path node {} {}",
                    nav.nodes[i].name,
                    fmt(nav.nodes[i].location)
                ));
            }
            log.line(format!(
                "player goal: the walk-in trigger for {tag} at {}",
                fmt(c)
            ));
            c
        }
    };
    let route = nav
        .route(player.location, goal)
        .unwrap_or_else(|| fail(&mut log, "no route for the player".into()));
    log.line(format!(
        "player route: {} nodes, {:.0} uu, from {} facing {:.0} deg",
        route.nodes.len(),
        route.length,
        fmt(player.location),
        start_yaw.to_degrees()
    ));
    let mut player_path = PathFollower::new(route.points.clone());
    let mut player_health = 200.0f32; // PlayerCommandoDefaults MaxHealth
    let mut next_player_shot = 0.0f32;

    // Fight.
    let droid_factories: Vec<rc_ai::Factory> = droid_factories.into_iter().cloned().collect();
    let mut battle = rc_ai::Battle::new(params.clone(), nav, droid_factories);
    let mut health: HashMap<u32, f32> = HashMap::new();
    let mut spawned_at: HashMap<u32, (f32, Vec3)> = HashMap::new();
    let mut counts: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut shots = 0u32;
    let mut droid_hits = 0u32;
    let mut walls = 0u32;
    let mut deaths = 0u32;
    let mut route_lengths = Vec::new();
    let mut cover_reached = 0u32;
    let mut lowest_z = f32::INFINITY;
    let mut worst_tick_ms = 0.0f64;
    let mut total_tick_ms = 0.0f64;
    let mut ticks = 0u32;
    let mut first_spot: Option<f32> = None;
    let mut first_hit: Option<f32> = None;
    let mut time = 0.0f32;
    let mut next_snapshot = 0.0f32;
    let player_extent_r = player_params.collision_radius;
    let player_extent_h = player_params.collision_height;

    while time < seconds {
        let tick_start = std::time::Instant::now();
        // Player walks (stops when there).
        let input = player_path.update(&player, DT).unwrap_or_default();
        player.step(DT, &input, &world);
        if player_path.stuck_time > 2.0 {
            log.line(format!(
                "[{time:6.2}s] player stuck at {} walking to point {} of {} at {}",
                fmt(player.location),
                player_path.next_index(),
                player_path.points.len(),
                player_path.target().map(fmt).unwrap_or_default()
            ));
            let k = player_path.next_index();
            if k >= 1 && k < route.nodes.len() {
                let (a, b) = (route.nodes[k - 1], route.nodes[k]);
                let nav = &battle.nav;
                let flags: Vec<u32> = nav.links[a]
                    .iter()
                    .filter(|l| l.to == b)
                    .map(|l| l.flags)
                    .collect();
                log.line(format!(
                    "  link {} -> {} has reachFlags {flags:?}",
                    nav.nodes[a].name, nav.nodes[b].name
                ));
            }
            player_path.stuck_time = -1000.0;
        }

        // Spawners, droids thinking and acting, droids hearing shots.
        let target = Some(Target {
            centre: player.location,
            eye: player.eye(),
        });
        let mut events = battle.step(DT, &world, target);

        // The player shoots back at the nearest droid it can see.
        if return_fire && time >= next_player_shot {
            let eye = player.eye();
            let target = battle
                .alive()
                .filter(|f| (f.pawn.location - eye).length() < 6000.0)
                .filter(|f| rc_ai::brain::clear_line(&world, eye, f.eye()))
                .min_by(|a, b| {
                    (a.pawn.location - eye)
                        .length()
                        .total_cmp(&(b.pawn.location - eye).length())
                })
                .map(|f| f.id);
            if let Some(id) = target {
                next_player_shot = time + PLAYER_SHOT_INTERVAL;
                let h = health.entry(id).or_insert(params.combat.health);
                *h -= PLAYER_SHOT_DAMAGE;
                if *h <= 0.0 {
                    events.extend(battle.killed(id));
                    deaths += 1;
                    let f = battle.fighters.iter().find(|f| f.id == id).unwrap();
                    log.line(format!(
                        "[{time:6.2}s] droid {id} destroyed ({:.1} s after spawning, fired {} shots, walked {:.0} uu)",
                        time - spawned_at[&id].0,
                        f.shots_fired,
                        f.distance_walked
                    ));
                } else {
                    events.extend(battle.damaged(id, Some(player.location)));
                }
                events.extend(battle.hear(player.location));
            }
        }

        for e in events {
            let (id, e) = match e {
                rc_ai::BattleEvent::SpawnerOn {
                    spawner,
                    by_trigger,
                } => {
                    let f = &battle.spawners[spawner].factory;
                    log.line(format!(
                        "[{time:6.2}s] spawner {} ({}) switched on by {}",
                        f.name,
                        f.tag,
                        if by_trigger {
                            "its walk-in trigger"
                        } else {
                            "the player coming near (scripted in the original)"
                        }
                    ));
                    continue;
                }
                rc_ai::BattleEvent::Spawned {
                    id,
                    spawner,
                    feet,
                    yaw,
                } => {
                    let loc = feet + Vec3::new(0.0, 0.0, params.movement.collision_height);
                    log.line(format!(
                        "[{time:6.2}s] droid {id} spawned by {} at {} facing {:.0} deg",
                        battle.spawners[spawner].factory.name,
                        fmt(loc),
                        yaw.to_degrees()
                    ));
                    spawned_at.insert(id, (time, loc));
                    continue;
                }
                rc_ai::BattleEvent::Ai { id, event } => (id, event),
            };
            match &e {
                AiEvent::Fire {
                    origin, direction, ..
                } => {
                    shots += 1;
                    let max = 10000.0;
                    let wall = rc_ai::brain::trace(&world, *origin, *origin + *direction * max)
                        .map(|t| t * max);
                    let hit = ray_hits_cylinder(
                        *origin,
                        *direction,
                        player.location,
                        player_extent_r,
                        player_extent_h,
                        max,
                    );
                    match (hit, wall) {
                        (Some(th), w) if w.is_none_or(|tw| th < tw) => {
                            droid_hits += 1;
                            player_health -= DROID_BOLT_DAMAGE;
                            first_hit.get_or_insert(time);
                        }
                        (_, Some(_)) => walls += 1,
                        _ => {}
                    }
                    *counts.entry("shots").or_default() += 1;
                }
                AiEvent::Spotted { distance } => {
                    first_spot.get_or_insert(time);
                    log.line(format!(
                        "[{time:6.2}s] droid {id} spotted the player {distance:.0} uu away"
                    ));
                }
                AiEvent::ModeChanged { from, to } => {
                    log.line(format!(
                        "[{time:6.2}s] droid {id}: {} -> {}",
                        from.name(),
                        to.name()
                    ));
                }
                AiEvent::ManeuverChanged { from, to } => {
                    if *to == "in cover" {
                        cover_reached += 1;
                    }
                    log.line(format!("[{time:6.2}s] droid {id}: maneuver {from} -> {to}"));
                }
                AiEvent::Route { nodes, length } => {
                    route_lengths.push(*length);
                    log.line(format!(
                        "[{time:6.2}s] droid {id}: route of {nodes} nodes, {length:.0} uu"
                    ));
                }
                other => log.line(format!("[{time:6.2}s] droid {id}: {other:?}")),
            }
            *counts
                .entry(match e {
                    AiEvent::Fire { .. } => "fire events",
                    AiEvent::Spotted { .. } => "spotted",
                    AiEvent::LostSight => "lost sight",
                    AiEvent::ModeChanged { .. } => "mode changes",
                    AiEvent::ManeuverChanged { .. } => "maneuver changes",
                    AiEvent::Route { .. } => "routes",
                    AiEvent::NoRoute => "no route",
                    AiEvent::Stuck => "stuck",
                })
                .or_default() += 1;
        }
        for f in battle.alive() {
            lowest_z = lowest_z.min(f.pawn.location.z - (spawned_at[&f.id].1.z - 400.0));
        }

        let ms = tick_start.elapsed().as_secs_f64() * 1000.0;
        worst_tick_ms = worst_tick_ms.max(ms);
        total_tick_ms += ms;
        ticks += 1;

        if time >= next_snapshot {
            next_snapshot += 5.0;
            let alive: Vec<String> = battle
                .alive()
                .map(|f| {
                    format!(
                        "#{} {} {} at {} {:.0} uu away",
                        f.id,
                        f.mode.name(),
                        f.maneuver.name(),
                        fmt(f.pawn.location),
                        (f.pawn.location - player.location).length()
                    )
                })
                .collect();
            log.line(format!(
                "[{time:6.2}s] player at {} health {player_health:.0}; {} droids: {}",
                fmt(player.location),
                alive.len(),
                alive.join("; ")
            ));
        }
        time += DT;
    }
    let droids = &battle.fighters;

    // Results.
    log.line(format!("events: {counts:?}"));
    let total_route: f32 = route_lengths.iter().sum();
    log.line(format!(
        "{} droids spawned, {deaths} destroyed; {shots} droid shots: {droid_hits} hit the player, {walls} missed and hit the level behind or in front, {} missed into the sky; player took {:.0} damage",
        droids.len(),
        shots - droid_hits - walls,
        DROID_BOLT_DAMAGE * droid_hits as f32
    ));
    log.line(format!(
        "{} routes walked, {:.0} uu in all; {cover_reached} times a droid reached cover; first spotted at {:?} s, first hit at {:?} s",
        route_lengths.len(),
        total_route,
        first_spot,
        first_hit
    ));
    log.line(format!(
        "thinking + moving time per frame: average {:.3} ms, worst {:.3} ms ({} frames)",
        total_tick_ms / ticks.max(1) as f64,
        worst_tick_ms,
        ticks
    ));
    check(
        &mut log,
        !droids.is_empty(),
        format!("droids spawned ({})", droids.len()),
    );
    let on_floor = droids
        .iter()
        .filter(|f| f.pawn.mode == rc_movement::Mode::Walking || f.mode == Mode::Dead)
        .count();
    check(
        &mut log,
        on_floor == droids.len(),
        format!(
            "every droid standing on a floor at the end ({on_floor} of {})",
            droids.len()
        ),
    );
    check(&mut log, lowest_z > 0.0, format!("no droid fell more than 400 uu below where it spawned (lowest margin {lowest_z:.0} uu)"));
    check(
        &mut log,
        first_spot.is_some(),
        "a droid saw the player".into(),
    );
    check(&mut log, shots > 0, format!("droids fired ({shots} shots)"));
    let ratio = droid_hits as f32 / shots.max(1) as f32;
    check(
        &mut log,
        droid_hits > 0 && ratio < 0.95,
        format!(
            "some shots hit, not all ({droid_hits} of {shots}, {:.0}%)",
            ratio * 100.0
        ),
    );
    check(
        &mut log,
        worst_tick_ms < 50.0,
        format!("worst frame under 50 ms ({worst_tick_ms:.2} ms)"),
    );
    log.line(format!("DONE: {failed} checks failed"));
    std::process::exit(if failed == 0 { 0 } else { 1 });
}
