//! rc_dispenser_check: do the maps' droid dispensers make their droids the
//! way the original does? No window. Needs the game files:
//!
//! `RC_GAME_DIR=<game folder> cargo run --release -p rc_enemies --example rc_dispenser_check [MAP ...]`
//!
//! For each map (by default every map that has a battle droid or super
//! battle droid dispenser), it reads the map's spawners as the game does,
//! the dispensers' model (`Dispensers.ukx`) and the droids' looks, and
//! gives the battle the same animation numbers the game does
//! (`rc_enemies::dispensers::motion`). Then, with no level scripts, for
//! each dispenser in turn it stands in for the level: it sends the
//! dispenser's tag (and its online tag after the intro, when it has one),
//! runs the battle against a stand-in player 1500 uu in front of it for
//! 20 s, then hurts it in three steps until it is destroyed.
//!
//! Checks: it plays its intro, warms up and goes online; it makes its
//! first droids (NumInitDroids, or fewer when MaxInWorld says so) one
//! after another; each droid rides its slot, is let go when the slot's
//! animation says (AnimNotify_ReleaseDroid) and finishes unfolding near
//! the machine, not inside the level; it shows its damage looks at 66 %
//! and 33 %, and when destroyed sends its map's event and takes the droid
//! still on its slot with it.
//!
//! Writes `logs/dispenser_check.log` and ends `DISPENSER CHECK: N passed,
//! M failed` (exit code 1 if any failed).

use rc_ai::params::AiParams;
use rc_ai::{Battle, BattleEvent, DispenserStep, Phase, Target};
use rc_enemies::dispensers::{animations, motion, unfold_from, PACKAGE};
use rc_enemies::look::DroidLook;
use rc_game_core::GamePaths;
use rc_model::{Model, ModelSpec};
use rc_movement::{TriangleWorld, Vec3, World};
use rc_nav::NavGraph;
use rc_package::Package;
use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

const DT: f32 = 1.0 / 30.0;
/// Seconds each dispenser runs before it is hurt.
const RUN: f32 = 20.0;

const MAPS: &[&str] = &[
    "geo_05b", "ras_02b", "ras_02e", "ras_03a", "ras_04a", "ras_04b", "ras_04c", "yyy_04f",
    "yyy_05d", "yyy_05e", "yyy_06a", "yyy_06c", "yyy_35b", "yyy_35c",
];

fn main() {
    let Ok(game) = std::env::var("RC_GAME_DIR") else {
        eprintln!("set RC_GAME_DIR to the game folder");
        std::process::exit(2);
    };
    let maps: Vec<String> = {
        let a: Vec<String> = std::env::args().skip(1).collect();
        if a.is_empty() {
            MAPS.iter().map(|s| s.to_string()).collect()
        } else {
            a
        }
    };
    let paths = GamePaths::new(&game, Vec::new());
    let game_data = std::path::Path::new(&game).join("GameData");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::create_dir_all("logs").expect("can't create logs");
    let mut log = std::fs::File::create("logs/dispenser_check.log").expect("can't create the log");
    let mut line = |s: String| {
        println!("{s}");
        writeln!(log, "{s}").ok();
    };
    let (mut passed, mut failed) = (0u32, 0u32);
    let mut check = |line: &mut dyn FnMut(String), ok: bool, what: String| {
        line(format!("{} {what}", if ok { "PASS" } else { "FAIL" }));
        if ok {
            passed += 1;
        } else {
            failed += 1;
        }
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
    let base = types
        .iter()
        .find(|t| t.kind.id == "battle_droid")
        .cloned()
        .unwrap_or_else(|| types[0].clone());
    let mut looks: HashMap<String, Option<DroidLook>> = HashMap::new();
    let mut models: HashMap<String, Option<Arc<Model>>> = HashMap::new();

    for map_name in &maps {
        let map_path = game_data.join("Maps").join(format!("{map_name}.ctm"));
        let Ok(data) = std::fs::read(&map_path) else {
            check(
                &mut line,
                false,
                format!("{map_name}: map not found at {}", map_path.display()),
            );
            continue;
        };
        let pkg = Package::parse(&data).expect("can't read the map package");
        let nav = NavGraph::from_map(&pkg, &data).expect("no path network");
        let factories = rc_ai::read_factories(&pkg, &data).expect("can't read the spawners");
        let mut claimed = Vec::new();
        for f in factories {
            if f.dispenser.is_none() {
                continue;
            }
            match types.iter().find(|t| t.kind.matches(&f.prototype)) {
                Some(t) => claimed.push((f, t.clone())),
                None => line(format!(
                    "{map_name}: dispenser {} makes {:?}, which no data/enemies file claims: left out",
                    f.name, f.prototype
                )),
            }
        }
        if claimed.is_empty() {
            line(format!("{map_name}: no BD or SBD dispensers"));
            continue;
        }
        let coll = match rc_level::map_collision::load(&map_path, &game_data, 84.0) {
            Ok(c) => c,
            Err(e) => {
                check(
                    &mut line,
                    false,
                    format!("{map_name}: collision not read: {e}"),
                );
                continue;
            }
        };
        line(format!(
            "{map_name}: collision from {} BSP nodes, {} props ({} props not read: their packages are missing), {} terrains{}",
            coll.bsp_nodes,
            coll.mesh_actors,
            coll.mesh_failures,
            coll.terrains,
            if coll.terrain_failures.is_empty() {
                String::new()
            } else {
                format!(" (not read: {:?})", coll.terrain_failures)
            }
        ));
        let v = |p: [f32; 3]| Vec3::new(p[0], p[1], p[2]);
        let world = TriangleWorld::new(
            coll.triangles
                .iter()
                .map(|t| [v(t[0]), v(t[1]), v(t[2])])
                .collect(),
        );
        let n = claimed.len();
        let mut battle = Battle::with_types(base.clone(), nav, claimed);
        battle.scripted = true;

        // The animation numbers, as the game gives them.
        for i in 0..battle.spawners.len() {
            let sp = &battle.spawners[i];
            let Some(d) = sp.factory.dispenser.clone() else {
                continue;
            };
            let kind = sp.params.kind.id.clone();
            let look = looks.entry(kind.clone()).or_insert_with(|| {
                match DroidLook::load(&paths, &sp.params.look) {
                    Ok((l, _)) => Some(l),
                    Err(e) => {
                        println!("{kind}: look not read: {e}");
                        None
                    }
                }
            });
            let Some((unfold, note)) = look.as_ref().and_then(|l| unfold_from(l, &kind)) else {
                check(
                    &mut line,
                    false,
                    format!("{map_name}: {kind} has no DispUnfold"),
                );
                continue;
            };
            let anims = animations(&d);
            let key = format!("{}:{}", d.mesh, anims.join(","));
            let model = models
                .entry(key)
                .or_insert_with(|| {
                    let spec = ModelSpec {
                        package: PACKAGE.into(),
                        mesh: d.mesh.clone(),
                        sequences: anims.iter().map(|a| (a.clone(), a.clone())).collect(),
                        ..Default::default()
                    };
                    match Model::load(&|rel| paths.game_file(rel), &spec) {
                        Ok((m, _)) => Some(Arc::new(m)),
                        Err(e) => {
                            println!("{}: model not read: {e}", d.mesh);
                            None
                        }
                    }
                })
                .clone();
            let Some(model) = model else {
                check(
                    &mut line,
                    false,
                    format!("{map_name}: {} model {} not read", sp.factory.name, d.mesh),
                );
                continue;
            };
            let (m, notes) = motion(&model, &d, &unfold);
            line(format!(
                "{map_name}: {} ({}, tag {}, online tag {}): makes {kind}, {} slots, {} at first then {} alive, every {:.1}-{:.1} s, max in world {}, intro {} {:.2} s, warm-up {:.1} s, sends {} when destroyed{}",
                sp.factory.name,
                d.class,
                sp.factory.tag,
                d.online_tag.as_deref().unwrap_or("-"),
                d.num_dispensers,
                d.num_init_droids,
                sp.factory.max_alive,
                sp.factory.spawn_interval,
                sp.factory.spawn_interval_max.max(sp.factory.spawn_interval),
                sp.factory.max_in_world,
                d.intro_anim.as_deref().unwrap_or("-"),
                m.intro_seconds,
                d.warm_up_time,
                d.damaged.event.as_deref().unwrap_or("nothing"),
                if d.hidden { " (hidden)" } else { "" }
            ));
            line(format!("    {note}"));
            for n in notes {
                line(format!("    {n}"));
            }
            battle.set_dispenser_motion(i, m);
        }

        // Each dispenser in turn.
        for i in 0..n {
            let (name, tag, online_tag, d, centre, yaw, max_in_world) = {
                let sp = &battle.spawners[i];
                let Some(d) = sp.factory.dispenser.clone() else {
                    continue;
                };
                if sp.motion.is_none() {
                    continue;
                }
                (
                    sp.factory.name.clone(),
                    sp.factory.tag.clone(),
                    d.online_tag.clone(),
                    d,
                    sp.factory.location,
                    sp.factory.yaw,
                    sp.factory.max_in_world,
                )
            };
            let who = format!("{map_name} {name}");
            // How far below `p` the floor is (within 5000 uu).
            let down = |p: Vec3| {
                let from = p + Vec3::new(0.0, 0.0, 10.0);
                let to = p - Vec3::new(0.0, 0.0, 5000.0);
                let from = from + Vec3::new(0.0, 0.0, 4.0);
                world
                    .sweep(Vec3::new(4.0, 4.0, 4.0), from, to)
                    .map(|h| p.z - (from.z + (to.z - from.z) * h.time))
            };
            let ahead = Vec3::new(yaw.cos(), yaw.sin(), 0.0);
            let player_centre = centre + ahead * 1500.0;
            let player = Target {
                centre: player_centre,
                eye: player_centre + Vec3::new(0.0, 0.0, 60.0),
            };
            let start = battle.time;
            // Where each droid was when it was let go and when it finished
            // unfolding.
            let mut places: HashMap<(u32, bool), Vec3> = HashMap::new();
            // Droids of its kind already about: MaxInWorld counts them.
            let kind = battle.spawners[i].params.kind.id.clone();
            let about = battle.alive().filter(|f| f.params.kind.id == kind).count() as u32;
            let mut events = battle.trigger(&tag);
            let mut seen: Vec<(f32, BattleEvent)> = Vec::new();
            let mut online_sent = false;
            while battle.time < start + RUN {
                for e in events.drain(..) {
                    note_place(&battle, &e, &mut places);
                    seen.push((battle.time - start, e));
                }
                if let Some(t) = &online_tag {
                    let waiting = matches!(
                        battle.spawners[i].disp.as_ref().map(|s| s.phase.clone()),
                        Some(Phase::WaitOnline)
                    );
                    if waiting && !online_sent {
                        online_sent = true;
                        line(format!("    {who}: waits for {t} after its intro: sent"));
                        events.extend(battle.trigger(t));
                    }
                }
                events.extend(battle.step_with(DT, &world, Some(player), &[]));
            }
            for e in events.drain(..) {
                note_place(&battle, &e, &mut places);
                seen.push((battle.time - start, e));
            }
            // Where its intro carried it: its bottom should be on the floor.
            let placed = centre;
            let centre = battle.dispenser_location(i);
            let bottom = centre - Vec3::new(0.0, 0.0, d.collision_height);
            let gap = down(bottom);
            line(format!(
                "    {who}: placed at ({:.0}, {:.0}, {:.0}), its intro carried it to ({:.0}, {:.0}, {:.0}); its bottom is {}",
                placed.x,
                placed.y,
                placed.z,
                centre.x,
                centre.y,
                centre.z,
                gap.map(|g| format!("{g:.0} uu above the floor")).unwrap_or("over no floor within 5000 uu".into())
            ));
            // Not a failure of the dispenser: the port's level collision
            // still lacks some floors (props that don't block yet, movers).
            let rides_mover = d.attach_tag.is_some();
            if let Some(g) = gap.filter(|_| !rides_mover) {
                check(
                    &mut line,
                    g.abs() <= 40.0,
                    format!("{who}: stands on the floor after its intro ({g:.0} uu above it)"),
                );
            } else {
                line(format!(
                    "NOTE {who}: the port's collision has no floor right under it after its intro{}",
                    if rides_mover { " (it rides a mover; this check doesn't move the movers)" } else { "" }
                ));
            }
            let mut looks_seen = Vec::new();
            let mut made: Vec<(f32, u32, usize)> = Vec::new();
            let mut released: HashMap<u32, f32> = HashMap::new();
            let mut unfolded: HashMap<u32, f32> = HashMap::new();
            for (t, e) in &seen {
                if let BattleEvent::Dispenser { spawner, step } = e {
                    if *spawner != i {
                        continue;
                    }
                    match step {
                        DispenserStep::Intro { anim } => line(format!(
                            "    {t:5.2}s intro {}",
                            anim.as_deref().unwrap_or("ActionBreathe")
                        )),
                        DispenserStep::Look { which } => {
                            looks_seen.push(*which);
                            line(format!("    {t:5.2}s look {which}"));
                        }
                        DispenserStep::Dispense { id, slot } => {
                            made.push((*t, *id, *slot));
                            line(format!("    {t:5.2}s droid {id} on slot {}", slot + 1));
                        }
                        DispenserStep::Released { id } => {
                            released.insert(*id, *t);
                            line(format!(
                                "    {t:5.2}s droid {id} let go{}",
                                places
                                    .get(&(*id, false))
                                    .map(|p| format!(" at ({:.0}, {:.0}, {:.0})", p.x, p.y, p.z))
                                    .unwrap_or_default()
                            ));
                        }
                        DispenserStep::Unfolded { id } => {
                            unfolded.insert(*id, *t);
                            line(format!("    {t:5.2}s droid {id} unfolded"));
                        }
                        other => line(format!("    {t:5.2}s {other:?}")),
                    }
                }
            }
            check(
                &mut line,
                looks_seen.contains(&"warm_up") && looks_seen.contains(&"online"),
                format!("{who}: warms up and goes online ({looks_seen:?})"),
            );
            let first = if max_in_world > 0.0 {
                d.num_init_droids
                    .min((max_in_world as u32).saturating_sub(about))
            } else {
                d.num_init_droids
            } as usize;
            check(
                &mut line,
                made.len() >= first.max(if about as f32 >= max_in_world && max_in_world > 0.0 { 0 } else { 1 }),
                format!(
                    "{who}: made {} droids in {RUN:.0} s (first wave {first}; {about} of its kind already about)",
                    made.len()
                ),
            );
            // Slot timing: let go at the slot animation's release moment.
            let slots = battle.spawners[i].motion.clone().unwrap().slots;
            let mut timing_ok = true;
            for (t, id, slot) in &made {
                if let Some(r) = released.get(id) {
                    let want = slots[*slot].release * slots[*slot].seconds;
                    if ((r - t) - want).abs() > 2.0 * DT {
                        timing_ok = false;
                        line(format!(
                            "    droid {id}: let go after {:.2} s, slot says {want:.2} s",
                            r - t
                        ));
                    }
                }
            }
            check(
                &mut line,
                timing_ok,
                format!("{who}: droids are let go when their slot's animation says"),
            );
            // Where the unfolded droids stand.
            let mut place_ok = true;
            let mut n_unfolded = 0;
            for id in unfolded.keys() {
                n_unfolded += 1;
                let Some(&feet) = places.get(&(*id, true)) else {
                    continue;
                };
                let flat = ((feet.x - centre.x).powi(2) + (feet.y - centre.y).powi(2)).sqrt();
                let drop = down(feet);
                line(format!(
                    "    droid {id} unfolded at ({:.0}, {:.0}, {:.0}), {flat:.0} uu from the dispenser's centre, {:.0} uu above its bottom, {}",
                    feet.x,
                    feet.y,
                    feet.z,
                    feet.z - bottom.z,
                    drop.map(|d| format!("{d:.0} uu above the floor")).unwrap_or("no floor within 5000 uu".into())
                ));
                // Its feet where the machine's bottom is, give or take.
                if flat > 600.0 || (feet.z - bottom.z).abs() > 40.0 {
                    place_ok = false;
                }
            }
            check(
                &mut line,
                n_unfolded > 0 && place_ok,
                format!(
                    "{who}: {n_unfolded} droids unfolded beside the machine, level with its bottom"
                ),
            );

            // Hurt it: 60 %, 30 %, then 0.
            let max = d.max_health;
            let mut hurt = Vec::new();
            for h in [max * 0.6, max * 0.3, 0.0] {
                hurt.extend(battle.dispenser_hurt(i, h));
            }
            let mut steps = Vec::new();
            let mut sent = Vec::new();
            let mut killed = 0;
            for e in &hurt {
                match e {
                    BattleEvent::Dispenser {
                        step: DispenserStep::Look { which },
                        ..
                    } => steps.push(*which),
                    BattleEvent::Dispenser {
                        step: DispenserStep::KilledOnSlot { .. },
                        ..
                    } => killed += 1,
                    BattleEvent::Event { name, .. } => sent.push(name.clone()),
                    _ => {}
                }
            }
            check(
                &mut line,
                steps == ["slightly_damaged", "heavily_damaged", "damaged"],
                format!("{who}: damage looks {steps:?}"),
            );
            check(
                &mut line,
                d.damaged.event.as_ref().is_none_or(|e| sent.contains(e)),
                format!(
                    "{who}: destroyed, sends {sent:?}; {killed} droid(s) destroyed on its slot"
                ),
            );
            check(
                &mut line,
                matches!(
                    battle.spawners[i].disp.as_ref().map(|s| s.phase.clone()),
                    Some(Phase::Destroyed)
                ) && !battle.spawners[i].active,
                format!("{who}: stays destroyed and makes no more"),
            );
        }
    }
    line(format!("DISPENSER CHECK: {passed} passed, {failed} failed"));
    if failed > 0 {
        std::process::exit(1);
    }
}

/// Note where a droid's feet are when it is let go (`false`) or finishes
/// unfolding (`true`).
fn note_place(battle: &Battle, e: &BattleEvent, places: &mut HashMap<(u32, bool), Vec3>) {
    let (id, done) = match e {
        BattleEvent::Dispenser {
            step: DispenserStep::Released { id },
            ..
        } => (*id, false),
        BattleEvent::Dispenser {
            step: DispenserStep::Unfolded { id },
            ..
        } => (*id, true),
        _ => return,
    };
    if let Some(f) = battle.fighters.iter().find(|f| f.id == id) {
        places.insert((id, done), f.pawn.feet());
    }
}
