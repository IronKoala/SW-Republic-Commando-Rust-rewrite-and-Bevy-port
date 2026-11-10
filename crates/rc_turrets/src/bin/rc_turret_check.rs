//! rc_turret_check: reads the turrets in maps and runs their level scripts
//! without a window, to check the turret rules against the real maps.
//!
//! Usage:
//!   rc_turret_check [MAP ...] [--secs N] [--mount NAME@SECONDS] [--fire EVENT@SECONDS ...]
//!
//! For each map (yyy_06d, ras_03b and ras_03c if none are named) it logs
//! every turret: its class, the data file that claims it, its tag, events,
//! limits and exit spot. Then it runs the level's scripts for `--secs`
//! seconds (default 120), carrying out the turret steps the way the game
//! does (`AttachToTurret` puts a script's characters on a turret,
//! `SetTurretUsable` changes who may use it). With `--mount NAME@T` the
//! player gets on that turret at T seconds (by name, or `any` for the
//! first one the player may use), sending its events into the scripts.
//! `--fire Name@T` sends any other event. Everything goes to
//! `logs/rc_turret_check.log`. The game folder is RC_GAME_DIR or the
//! default Steam path; nothing is written there.

use rc_package::Package;
use rc_scripts::{Director, Effect, Settings};
use rc_turrets::{params, Gunner, Mounted, Turret};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";
const LOG: &str = "logs/rc_turret_check.log";
const SETTINGS: &str = "data/scripts/scripts.toml";

struct Log(fs::File);
impl Log {
    fn line(&mut self, s: impl AsRef<str>) {
        let _ = writeln!(self.0, "{}", s.as_ref());
    }
}

fn at(s: &str) -> Option<(String, f32)> {
    let (a, t) = s.split_once('@')?;
    Some((a.to_string(), t.parse().ok()?))
}

fn main() {
    let game = std::env::var_os("RC_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_DIR));
    let data = game.join("GameData");
    let _ = fs::create_dir_all("logs");
    let mut log = Log(fs::File::create(LOG).expect("can't create the log"));
    let settings = fs::read_to_string(SETTINGS)
        .ok()
        .and_then(|t| Settings::default().with_overrides(&t).ok())
        .unwrap_or_default();
    let (files, errors) = params::load_folders(&[Path::new("data/turrets")]);
    for e in &errors {
        log.line(format!("ERROR in a turret file: {e}"));
    }
    log.line(format!(
        "turret files: {:?}",
        files.keys().collect::<Vec<_>>()
    ));
    let mut names = Vec::new();
    let mut secs = 120.0f32;
    let mut mount: Option<(String, f32)> = None;
    let mut fires: Vec<(String, f32)> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--secs" => secs = args.next().and_then(|s| s.parse().ok()).unwrap_or(secs),
            "--mount" => mount = args.next().as_deref().and_then(at),
            "--fire" => fires.extend(args.next().as_deref().and_then(at)),
            _ => names.push(a),
        }
    }
    if names.is_empty() {
        names = vec!["yyy_06d".into(), "ras_03b".into(), "ras_03c".into()];
    }
    for name in &names {
        let path = data.join("Maps").join(format!("{name}.ctm"));
        let read = fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|d| {
                Package::parse(&d)
                    .map(|p| (p, d))
                    .map_err(|e| e.to_string())
            })
            .and_then(|(p, d)| {
                rc_level::actors::read_level(&p, &d)
                    .map(|l| (p, d, l))
                    .map_err(|e| e.to_string())
            });
        let (pkg, bytes, level) = match read {
            Ok(x) => x,
            Err(e) => {
                log.line(format!("== {name}: ERROR {e}"));
                continue;
            }
        };
        // Every actor whose class looks like a turret, to show the ones no
        // file claims.
        let (all, _) = rc_turrets::map::read(&pkg, &bytes, &level, &|c: &str| {
            c.to_ascii_lowercase().contains("turret") || c.eq_ignore_ascii_case("EWebCannon")
        });
        let mut list: Vec<Turret> = Vec::new();
        log.line(format!("== {name}: {} turret actors", all.len()));
        for p in all {
            match params::for_class(&files, &p.class) {
                Some((id, f)) => {
                    let t = Turret::new(p, id, f);
                    log.line(format!(
                        "   {} ({}, file {}): tag {}, at {:?} yaw {}, player usable {}, control {:?}, player control {:?}, release {:?}, yaw {}..{}{} pitch {}..{} speed {}, exit {:?}",
                        t.place.name,
                        t.place.class,
                        t.kind,
                        t.place.tag,
                        t.place.location,
                        t.place.rotation[1],
                        t.player_usable,
                        t.place.control_event,
                        t.place.player_control_event,
                        t.place.release_event,
                        t.limits.min_yaw,
                        t.limits.max_yaw,
                        if t.limits.unlimited_yaw { " (unlimited)" } else { "" },
                        t.limits.min_pitch,
                        t.limits.max_pitch,
                        t.limits.speed,
                        t.exit_point(),
                    ));
                    list.push(t);
                }
                None => log.line(format!(
                    "   {} ({}): no turret file claims this class, left out",
                    p.name, p.class
                )),
            }
        }
        let map = rc_level::scripts::read(&pkg, &bytes, &level);
        let triggers = rc_level::map_collision::load(&path, &data, 84.0)
            .map(|l| l.movers.triggers)
            .unwrap_or_default();
        let mut d = Director::new(&map, &triggers, settings.clone());
        d.start();
        // Scripts that wait for a turret's event at some point: their
        // names, to show where they got to at the end.
        let mut waits_on: Vec<String> = Vec::new();
        let dt = 0.1f32;
        let mut now = 0.0f32;
        let mut mounted = false;
        let mut fired = vec![false; fires.len()];
        while now < secs {
            let mut send: Vec<(String, String)> = Vec::new();
            if let Some((who, t)) = &mount {
                if !mounted && now >= *t {
                    mounted = true;
                    let i = list.iter().position(|x| {
                        if who.eq_ignore_ascii_case("any") {
                            x.usable_by_player()
                        } else {
                            x.place.name.eq_ignore_ascii_case(who)
                        }
                    });
                    match i {
                        Some(i) => {
                            let r = list[i].mount(Gunner::Player);
                            log.line(format!(
                                "t={now:.1} the player gets on {}: {r:?}",
                                list[i].place.name
                            ));
                            if let Mounted::On(events) = r {
                                for e in events {
                                    send.push((e, list[i].place.name.clone()));
                                }
                            }
                        }
                        None => log.line(format!("t={now:.1} no turret {who} for the player")),
                    }
                }
            }
            for (k, (e, t)) in fires.iter().enumerate() {
                if !fired[k] && now >= *t {
                    fired[k] = true;
                    send.push((e.clone(), "rc_turret_check".into()));
                }
            }
            for (e, by) in send {
                log.line(format!("t={now:.1} send {e} (from {by})"));
                d.fire(&e, &by);
            }
            for (script, _, what) in d.script_states() {
                let w = what.to_ascii_lowercase();
                let turret_event = list.iter().any(|t| {
                    [
                        &t.place.control_event,
                        &t.place.player_control_event,
                        &t.place.release_event,
                    ]
                    .into_iter()
                    .flatten()
                    .any(|e| w.ends_with(&format!("for {}", e.to_ascii_lowercase())))
                });
                if turret_event && !waits_on.contains(&script) {
                    waits_on.push(script);
                }
            }
            for effect in d.step(dt, None) {
                match effect {
                    Effect::Event { name: e, by } => {
                        log.line(format!("t={now:.1} event {e} (by {by})"));
                    }
                    Effect::Action {
                        script,
                        index,
                        action,
                        owner,
                        waits,
                    } => {
                        let kind = action
                            .class
                            .strip_prefix("ACTION_")
                            .unwrap_or(&action.class)
                            .to_ascii_lowercase();
                        match kind.as_str() {
                            "attachtoturret" => {
                                let tag = action.text("DestinationTag").unwrap_or("");
                                let attach = action.flag("Attach", true);
                                let who = owner.clone().unwrap_or_default();
                                match rc_turrets::first_tagged(&list, tag) {
                                    Some(i) => {
                                        let t = &mut list[i];
                                        let r = if attach {
                                            t.mount(Gunner::Character(who.clone()))
                                        } else {
                                            t.leave()
                                        };
                                        log.line(format!(
                                            "t={now:.1} {script} #{index}: {who} {} {} (tag {tag}): {r:?}",
                                            if attach { "gets on" } else { "gets off" },
                                            t.place.name
                                        ));
                                        let name = t.place.name.clone();
                                        if let Mounted::On(ev) | Mounted::Off(ev) = r {
                                            for e in ev {
                                                log.line(format!(
                                                    "t={now:.1} send {e} (from {name})"
                                                ));
                                                d.fire(&e, &name);
                                            }
                                        }
                                    }
                                    None => log.line(format!(
                                        "t={now:.1} {script} #{index}: no turret tagged {tag}"
                                    )),
                                }
                            }
                            "setturretusable" => {
                                let tag = action.text("TurretTag").unwrap_or("");
                                let usable = action.flag("IsPlayerUsable", true);
                                let mut n = 0;
                                for t in list
                                    .iter_mut()
                                    .filter(|t| t.place.tag.eq_ignore_ascii_case(tag))
                                {
                                    t.player_usable = usable;
                                    n += 1;
                                }
                                log.line(format!(
                                    "t={now:.1} {script} #{index}: {n} turrets tagged {tag} usable by the player: {usable}"
                                ));
                            }
                            _ => {}
                        }
                        if waits {
                            d.action_done(&script, index);
                        }
                    }
                    _ => {}
                }
            }
            now += dt;
        }
        for t in &list {
            log.line(format!(
                "   at the end: {} gunner {:?}, player usable {}",
                t.place.name, t.gunner, t.player_usable
            ));
        }
        // Scripts that wait for, or were woken by, a turret's events.
        let sent: Vec<String> = list
            .iter()
            .flat_map(|t| {
                [
                    t.place.control_event.clone(),
                    t.place.player_control_event.clone(),
                    t.place.release_event.clone(),
                ]
            })
            .flatten()
            .map(|e| e.to_ascii_lowercase())
            .collect();
        for (script, next, what) in d.script_states() {
            let w = what.to_ascii_lowercase();
            if sent.iter().any(|e| w.ends_with(&format!("for {e}"))) || waits_on.contains(&script) {
                log.line(format!("   script {script}: next {next}, {what}"));
            }
        }
    }
    println!("wrote {LOG}");
}
