//! rc_mover_check: read every door, lift and trigger in the game's maps,
//! run each mover through a full open and close without a window, and log
//! the numbers: positions over time, how long opening and closing took
//! against what the map says, and a ride on every lift.
//!
//! Usage: `rc_mover_check [MAP ...]` (default: every map in
//! `GameData/Maps`). The game folder is RC_GAME_DIR or the default Steam
//! path. Settings come from `data/movers/movers.toml` if it's there.
//! Writes `logs/rc_mover_check.log`. Exits with 1 if a check failed.

use rc_movement::{MoveInput, MoveParams, Pawn, TriangleWorld, Vec3};
use rc_movers::{Behaviour, MoverEvent, MoverSet, Settings};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";
const SETTINGS: &str = "data/movers/movers.toml";
const LOG: &str = "logs/rc_mover_check.log";
const DT: f32 = 1.0 / 60.0;

struct Log(fs::File);

impl Log {
    fn line(&mut self, s: impl AsRef<str>) {
        println!("{}", s.as_ref());
        let _ = writeln!(self.0, "{}", s.as_ref());
    }
}

#[derive(Default)]
struct Totals {
    maps: usize,
    map_failures: usize,
    movers: usize,
    with_collision: usize,
    triggers: usize,
    by_state: BTreeMap<String, usize>,
    by_class: BTreeMap<String, usize>,
    unknown_states: BTreeMap<String, usize>,
    timing_checked: usize,
    timing_failed: usize,
    too_slow: usize,
    rides: usize,
    rides_failed: usize,
    rides_blocked: usize,
    no_sender: usize,
    sounds: usize,
}

fn v(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], p[1], p[2])
}

fn main() {
    let game = std::env::var_os("RC_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_DIR));
    let data = game.join("GameData");
    let _ = fs::create_dir_all("logs");
    let mut log = Log(fs::File::create(LOG).expect("can't create the log"));
    let settings = match fs::read_to_string(SETTINGS) {
        Ok(t) => match Settings::default().with_overrides(&t) {
            Ok(s) => {
                log.line(format!("settings from {SETTINGS}"));
                s
            }
            Err(e) => {
                log.line(format!("ERROR in {SETTINGS}: {e}"));
                std::process::exit(1);
            }
        },
        Err(_) => {
            log.line(format!("no {SETTINGS}; built-in settings"));
            Settings::default()
        }
    };
    let mut names: Vec<String> = std::env::args().skip(1).collect();
    if names.is_empty() {
        let mut all: Vec<String> = fs::read_dir(data.join("Maps"))
            .map(|d| {
                d.flatten()
                    .filter_map(|e| {
                        let p = e.path();
                        let ctm = p.extension()?.to_str()?.eq_ignore_ascii_case("ctm");
                        ctm.then(|| p.file_stem().map(|s| s.to_string_lossy().to_string()))
                            .flatten()
                    })
                    .collect()
            })
            .unwrap_or_default();
        all.sort();
        names = all;
    }
    let params = MoveParams::player_commando();
    let mut t = Totals::default();
    for name in &names {
        t.maps += 1;
        let path = data.join("Maps").join(format!("{name}.ctm"));
        let lvl = match rc_level::map_collision::load(&path, &data, params.collision_height) {
            Ok(l) => l,
            Err(e) => {
                t.map_failures += 1;
                log.line(format!("== {name}: ERROR {e}"));
                continue;
            }
        };
        log.line(format!(
            "== {name}: {} movers, {} triggers, {} unreadable",
            lvl.movers.movers.len(),
            lvl.movers.triggers.len(),
            lvl.movers.failures.len()
        ));
        for f in &lvl.movers.failures {
            log.line(format!("  unreadable: {f}"));
        }
        if lvl.movers.movers.is_empty() && lvl.movers.triggers.is_empty() {
            continue;
        }
        let tris: Vec<[Vec3; 3]> = lvl.triangles.iter().map(|t| [v(t[0]), v(t[1]), v(t[2])]).collect();
        let fixed = TriangleWorld::new(tris);
        let mt = lvl.mover_triangles;
        let tri_of = |e: usize| -> Vec<[Vec3; 3]> {
            mt.get(&e)
                .map(|ts| ts.iter().map(|t| [v(t[0]), v(t[1]), v(t[2])]).collect())
                .unwrap_or_default()
        };
        let set = MoverSet::new(lvl.movers.clone(), settings.clone(), tri_of);
        for u in &set.unhandled_triggers {
            log.line(format!("  trigger not run (its state isn't handled yet): {u}"));
        }
        t.triggers += lvl.movers.triggers.len();
        for tr in &lvl.movers.triggers {
            let hits = set
                .movers
                .iter()
                .filter(|m| !tr.event.is_empty() && m.info.tag.eq_ignore_ascii_case(&tr.event))
                .count();
            log.line(format!(
                "  trigger {} ({}) at ({:.0}, {:.0}, {:.0}) radius {:.0} height {:.0} type {} state {:?} event {:?} -> {} movers{}",
                tr.name,
                tr.class,
                tr.location[0],
                tr.location[1],
                tr.location[2],
                tr.radius,
                tr.height,
                tr.trigger_type,
                tr.state,
                tr.event,
                hits,
                if tr.once_only { ", once only" } else { "" }
            ));
        }
        for (i, m) in set.movers.iter().enumerate() {
            let info = &m.info;
            t.movers += 1;
            t.with_collision += m.collision.is_some() as usize;
            *t.by_state.entry(info.state.clone()).or_default() += 1;
            *t.by_class.entry(info.chain[0].clone()).or_default() += 1;
            if !m.known_state {
                *t.unknown_states.entry(info.state.clone()).or_default() += 1;
            }
            let senders = lvl
                .movers
                .triggers
                .iter()
                .filter(|tr| tr.event.eq_ignore_ascii_case(&info.tag))
                .count()
                + set
                    .movers
                    .iter()
                    .filter(|o| {
                        let i = &o.info;
                        [&i.opening_event, &i.opened_event, &i.closing_event, &i.closed_event]
                            .iter()
                            .any(|e| !e.is_empty() && e.eq_ignore_ascii_case(&info.tag))
                    })
                    .count();
            let needs_sender = matches!(
                m.behaviour,
                Behaviour::TriggerOpenTimed
                    | Behaviour::TriggerToggle
                    | Behaviour::TriggerControl
                    | Behaviour::TriggerPound
                    | Behaviour::TriggerLoop
            );
            if needs_sender && senders == 0 {
                t.no_sender += 1;
            }
            let s = &info.sounds;
            t.sounds += [&s.opening, &s.opened, &s.closing, &s.closed, &s.moving]
                .iter()
                .filter(|x| x.is_some())
                .count();
            let (lo, hi) = m
                .collision
                .as_ref()
                .map(|c| c.bounds())
                .unwrap_or((Vec3::ZERO, Vec3::ZERO));
            log.line(format!(
                "  mover {i} {} class {} state {:?} -> {}{}{} | {} keys from key {}, travel {:.0} uu{}, MoveTimes {:?} s, StayOpenTime {:.2} s, DelayTime {:.2}, {:?} glide, {:?} when blocked | tag {:?}, events opening {:?} opened {:?} closing {:?} closed {:?}, {} senders{} | {} collision triangles, box ({:.0}, {:.0}, {:.0})-({:.0}, {:.0}, {:.0}){}",
                info.place.name,
                info.chain.join(" < "),
                info.state,
                m.behaviour.name(),
                if m.known_state { "" } else { " (unknown state)" },
                if info.auto_door { ", auto door" } else { "" },
                info.num_keys,
                info.key_num,
                info.travel(),
                if info.turns() { ", turns" } else { "" },
                &info.move_times[..(info.num_keys.max(2) as usize - 1)],
                info.stay_open_time,
                info.delay_time,
                info.glide,
                info.encroach,
                info.tag,
                info.opening_event,
                info.opened_event,
                info.closing_event,
                info.closed_event,
                senders,
                if needs_sender && senders == 0 { " (NONE: opened by a script?)" } else { "" },
                m.collision.as_ref().map_or(0, |c| c.triangles.len()),
                lo.x, lo.y, lo.z, hi.x, hi.y, hi.z,
                if info.defaulted.is_empty() { String::new() } else { format!(" | engine defaults used for {}", info.defaulted.join(", ")) },
            ));
            for k in 0..info.num_keys as usize {
                let (p, r) = info.key(k);
                log.line(format!(
                    "    key {k}: ({:.1}, {:.1}, {:.1}) turn ({}, {}, {})",
                    p[0], p[1], p[2], r[0], r[1], r[2]
                ));
            }
            log.line(format!(
                "    sounds: opening {:?}, opened {:?}, closing {:?}, closed {:?}, moving {:?}",
                s.opening, s.opened, s.closing, s.closed, s.moving
            ));
        }
        // Run each mover on its own, with the player far away.
        for i in 0..set.movers.len() {
            let m = &set.movers[i];
            if m.info.num_keys < 2 {
                continue;
            }
            let want = m.info.open_time();
            if want > 60.0 {
                t.too_slow += 1;
                log.line(format!(
                    "  run mover {i} {}: takes {want:.0} s to open, too slow to run here; skipped",
                    m.info.place.name
                ));
                continue;
            }
            let mut one = MoverSet::new(lvl.movers.only(i), settings.clone(), |_| Vec::new());
            let mut far = Pawn::new(params.clone(), Vec3::new(1e6, 1e6, 1e6));
            // A mover that starts at its last key (open) is run closing
            // first, unless it goes round in a circuit.
            let starts_open = one.movers[0].key >= one.movers[0].last_key() && !one.movers[0].is_circuit();
            let mut ev = if starts_open { one.close(0) } else { one.open(0) };
            let (mut open_s, mut close_s) = (None, None);
            let mut trace = Vec::new();
            let limit = (2.0 * want + one.movers[0].info.stay_open_time + 2.0).min(120.0);
            let mut f = 0;
            while (f as f32) * DT < limit {
                f += 1;
                ev.extend(one.step(DT, &fixed, &mut far));
                if f % 6 == 0 {
                    let p = one.movers[0].pose.pos;
                    trace.push(format!("{:.1}s ({:.0},{:.0},{:.0})", f as f32 * DT, p.x, p.y, p.z));
                }
                for e in ev.drain(..) {
                    match e {
                        MoverEvent::Opened { secs, .. } => open_s = open_s.or(Some(secs)),
                        MoverEvent::Closed { secs, .. } => close_s = close_s.or(Some(secs)),
                        _ => {}
                    }
                }
            }
            let ok = |x: Option<f32>| x.is_some_and(|s| (s - want).abs() <= 2.0 * DT);
            let b = one.movers[0].behaviour;
            let circuit = one.movers[0].is_circuit();
            // Does it come back by itself within the time we ran?
            let back = (b.timed() && 2.0 * want + one.movers[0].info.stay_open_time + 0.5 < limit)
                || (b == Behaviour::Loop && !circuit);
            let pass = if starts_open { ok(close_s) } else { ok(open_s) && (!back || ok(close_s)) };
            let timed = back;
            t.timing_checked += 1;
            t.timing_failed += !pass as usize;
            log.line(format!(
                "  run mover {i} {}{}: opened in {} s, closed in {} s, want {want:.2} s -> {}",
                m.info.place.name,
                if starts_open { " (starts open)" } else if circuit { " (goes round)" } else { "" },
                open_s.map_or("never".into(), |s| format!("{s:.2}")),
                close_s.map_or(if timed || starts_open { "never".into() } else { "(not waited for)".into() }, |s| format!("{s:.2}")),
                if pass { "PASS" } else { "FAIL" }
            ));
            log.line(format!("    positions: {}", trace.join(" ")));
        }
        // Ride every lift: stand on it, open it, and see if the player
        // goes with it.
        let ext = Vec3::new(params.collision_radius, params.collision_radius, params.collision_height);
        for i in 0..set.movers.len() {
            if !set.movers[i].is_lift(params.collision_radius) {
                continue;
            }
            let info = &set.movers[i].info;
            let mut one = MoverSet::new(lvl.movers.only(i), settings.clone(), |e| tri_of(e));
            let Some((spot, clear)) = one.movers[0].ride_clearance(&fixed, ext) else {
                log.line(format!("  ride mover {i} {}: no floor on top found, skipped", info.place.name));
                continue;
            };
            if clear < 0.999 {
                t.rides_blocked += 1;
                log.line(format!(
                    "  ride mover {i} {}: the level is in the way {:.0}% of the way along (a door into a ceiling, or a lift with no room on top), skipped",
                    info.place.name,
                    clear * 100.0
                ));
                continue;
            }
            let feet_z = spot.z;
            let mut p = Pawn::new(params.clone(), spot);
            let w = one.world(&fixed);
            // Settle onto it.
            for _ in 0..30 {
                p.step(DT, &MoveInput::default(), &w);
            }
            let start_gap = p.feet().z - feet_z;
            let lift_start = one.movers[0].pose.pos.z;
            let feet_start = p.feet().z;
            let mut ev = one.open(0);
            let mut worst = 0.0f32;
            let mut boarded = false;
            let mut trace = Vec::new();
            let want = info.open_time();
            if want > 60.0 {
                continue;
            }
            let frames = ((want + 0.5) / DT) as usize;
            for f in 1..=frames {
                ev.extend(one.step(DT, &fixed, &mut p));
                let w = one.world(&fixed);
                p.step(DT, &MoveInput::default(), &w);
                let lift = one.movers[0].pose.pos.z - lift_start;
                let feet = p.feet().z - feet_start;
                worst = worst.max((lift - feet).abs());
                if f % 12 == 0 {
                    trace.push(format!("{:.1}s lift {:+.0} feet {:+.0}", f as f32 * DT, lift, feet));
                }
                boarded |= ev.iter().any(|e| matches!(e, MoverEvent::Boarded { .. }));
                ev.clear();
            }
            let lift_moved = one.movers[0].pose.pos.z - lift_start;
            let feet_moved = p.feet().z - feet_start;
            let pass = (lift_moved - feet_moved).abs() < 4.0 && worst < 16.0 && lift_moved.abs() > 32.0;
            t.rides += 1;
            t.rides_failed += !pass as usize;
            log.line(format!(
                "  ride mover {i} {}: settled {:.1} uu above its top; lift moved {:+.0} uu, player feet moved {:+.0} uu, worst gap on the way {:.1} uu, boarded {} -> {}",
                info.place.name,
                start_gap,
                lift_moved,
                feet_moved,
                worst,
                boarded,
                if pass { "PASS" } else { "FAIL" }
            ));
            log.line(format!("    {}", trace.join(", ")));
        }
    }
    log.line("== totals");
    log.line(format!(
        "{} maps ({} failed), {} movers ({} with collision), {} triggers, {} sounds named",
        t.maps, t.map_failures, t.movers, t.with_collision, t.triggers, t.sounds
    ));
    log.line(format!("movers by InitialState: {:?}", t.by_state));
    log.line(format!("movers by class: {:?}", t.by_class));
    log.line(format!("unknown states (run as {}): {:?}", settings.unknown_state.name(), t.unknown_states));
    log.line(format!("trigger-driven movers with no trigger or mover sending their tag: {}", t.no_sender));
    log.line(format!(
        "timing: {} of {} movers opened and closed in their MoveTimes ({} take over 60 s and weren't run)",
        t.timing_checked - t.timing_failed,
        t.timing_checked,
        t.too_slow
    ));
    log.line(format!(
        "lift rides: {} of {} passed ({} more movers that look like lifts have the level in the way and weren't ridden)",
        t.rides - t.rides_failed,
        t.rides,
        t.rides_blocked
    ));
    if t.timing_failed > 0 || t.rides_failed > 0 || t.map_failures > 0 {
        std::process::exit(1);
    }
}
