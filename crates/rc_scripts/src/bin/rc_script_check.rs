//! rc_script_check: reads every map's level scripting and runs it without a
//! window, to check it from the numbers.
//!
//! Usage:
//!   rc_script_check [MAP ...] [--secs N] [--fire EVENT@SECONDS ...]
//!
//! For each map (all of `GameData/Maps` if none are named) it logs the
//! scripts, their actions and the actors around them, every action kind
//! and whether the director runs it itself or hands it to the game, then
//! sends the level-start events and runs the director for `--secs`
//! seconds (default 60) with no player, logging every event and effect.
//! `--fire Name@T` sends an event at T seconds, standing in for the game
//! (a door, a dispenser, a droid dying). Everything goes to
//! `logs/rc_script_check.log`. The game folder is RC_GAME_DIR or the
//! default Steam path; nothing is written there.

use rc_package::Package;
use rc_scripts::{Director, Effect, Settings};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";
const LOG: &str = "logs/rc_script_check.log";
const SETTINGS: &str = "data/scripts/scripts.toml";

/// Action kinds the director runs itself (the rest go to the game).
const OWN: &[&str] = &[
    "WaitForEvent",
    "TriggerEvent",
    "WaitForTimer",
    "WaitForRandomTimer",
    "Label",
    "GotoLabel",
    "GotoAction",
    "IfCondition",
    "IfRandomPct",
    "EndSection",
];

struct Log(fs::File);
impl Log {
    fn line(&mut self, s: impl AsRef<str>) {
        let _ = writeln!(self.0, "{}", s.as_ref());
    }
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
            Ok(s) => s,
            Err(e) => {
                log.line(format!("ERROR in {SETTINGS}: {e}"));
                std::process::exit(1);
            }
        },
        Err(_) => Settings::default(),
    };
    let mut names = Vec::new();
    let mut secs = 60.0f32;
    let mut fires: Vec<(String, f32)> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--secs" => secs = args.next().and_then(|s| s.parse().ok()).unwrap_or(secs),
            "--fire" => {
                if let Some((e, t)) = args.next().as_deref().and_then(|s| s.split_once('@')) {
                    fires.push((e.to_string(), t.parse().unwrap_or(0.0)));
                }
            }
            _ => names.push(a),
        }
    }
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
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let (mut maps, mut failed, mut scripts, mut actions, mut unreadable) = (0, 0, 0, 0, 0);
    let (mut dest_found, mut dest_missing) = (0, 0);
    for name in &names {
        maps += 1;
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
                failed += 1;
                log.line(format!("== {name}: ERROR {e}"));
                continue;
            }
        };
        let map = rc_level::scripts::read(&pkg, &bytes, &level);
        // Where scripts send characters: every DestinationTag / ObjectiveTag
        // should name an actor (by tag, else by name).
        let places = rc_level::scripts::places(&pkg, &bytes, &level);
        let mut missing = Vec::new();
        for s in map.scripts() {
            for a in &s.actions {
                for key in ["DestinationTag", "ObjectiveTag"] {
                    let Some(t) = a.text(key).filter(|t| !t.eq_ignore_ascii_case("None")) else {
                        continue;
                    };
                    if places
                        .iter()
                        .any(|p| p.tag.eq_ignore_ascii_case(t) || p.name.eq_ignore_ascii_case(t))
                    {
                        dest_found += 1;
                    } else {
                        dest_missing += 1;
                        missing.push(format!("{}.{key}={t}", s.obj.name));
                    }
                }
            }
        }
        if !missing.is_empty() {
            log.line(format!("   destinations not in the map: {missing:?}"));
        }
        // Triggers with their class defaults, as the movers part reads them.
        let triggers = rc_level::map_collision::load(&path, &data, 84.0)
            .map(|l| l.movers.triggers)
            .unwrap_or_default();
        unreadable += map.failures.len();
        let d = Director::new(&map, &triggers, settings.clone());
        let s = &d.stats;
        scripts += s.scripts + s.sequences;
        actions += s.actions;
        log.line(format!(
            "== {name}: {} actors, {} scripted triggers, {} sequences, {} actions, {} triggers, {} counters, {} conditions, {} checkpoints, {} props, {} unreadable; start events {:?}",
            map.actors.len(),
            s.scripts,
            s.sequences,
            s.actions,
            s.triggers,
            s.counters,
            s.conditions,
            s.checkpoints,
            s.props,
            map.failures.len(),
            d.start_events()
        ));
        for f in &map.failures {
            log.line(format!("   unreadable: {f}"));
        }
        for a in map.scripts() {
            log.line(format!(
                "   {} {} (tag {})",
                a.obj.class,
                a.obj.name,
                a.obj.tag()
            ));
            for (i, act) in a.actions.iter().enumerate() {
                let kind = act.class.strip_prefix("ACTION_").unwrap_or(&act.class);
                *kinds.entry(kind.to_string()).or_default() += 1;
                let vals: Vec<String> = act
                    .values
                    .iter()
                    .filter(|(k, _)| k.as_str() != "Comment")
                    .map(|(k, v)| match v {
                        rc_level::scripts::Val::Text(t) => format!("{k}={t}"),
                        other => format!("{k}={other:?}"),
                    })
                    .collect();
                log.line(format!("     {i:2} {kind} {}", vals.join(" ")));
            }
        }
        // Run it.
        let mut d = d;
        d.start();
        let dt = 0.05;
        let steps = (secs / dt).round() as usize;
        let mut fired = vec![false; fires.len()];
        for _ in 0..steps {
            for (k, (e, t)) in fires.iter().enumerate() {
                if !fired[k] && d.time >= *t {
                    fired[k] = true;
                    d.fire(e, "--fire");
                }
            }
            let t = d.time + dt;
            for e in d.step(dt, None) {
                let text = match e {
                    Effect::Note(n) => n,
                    Effect::Event { .. } => continue,
                    Effect::Untrigger { name, by } => format!("untrigger {name} (by {by})"),
                    Effect::Action {
                        script,
                        index,
                        action,
                        ..
                    } => {
                        let kind = action
                            .class
                            .strip_prefix("ACTION_")
                            .unwrap_or(&action.class)
                            .to_string();
                        let key = [
                            "Sound",
                            "TargetTag",
                            "CollideActorTag",
                            "DestroyTag",
                            "PromptText",
                            "ObjectiveNum",
                            "FactoryTag",
                            "ActorTag",
                        ]
                        .iter()
                        .find_map(|k| action.get(k).map(|v| format!(" {k}={v:?}")))
                        .unwrap_or_default();
                        format!("{script} #{index}: {kind}{key}")
                    }
                    Effect::PropDestroyed { name, tag, .. } => {
                        format!("prop {name} ({tag}) destroyed")
                    }
                    Effect::Checkpoint { name } => format!("checkpoint {name}: save"),
                };
                log.line(format!("   t={t:6.2} {text}"));
            }
        }
        log.line(format!(
            "   after {secs} s: {} events sent, {} actions run, {} scripts finished",
            d.stats.events_sent, d.stats.actions_run, d.stats.scripts_finished
        ));
        for (n, next, what) in d.script_states() {
            log.line(format!("   state {n}: at {next}, {what}"));
        }
    }
    log.line(
        "== action kinds over all maps (own = run by the director, game = handed to the game):",
    );
    for (k, n) in &kinds {
        let own = OWN.iter().any(|o| o.eq_ignore_ascii_case(k));
        log.line(format!(
            "   {k}: {n} ({})",
            if own { "own" } else { "game" }
        ));
    }
    let line = format!(
        "{maps} maps, {failed} unreadable; {scripts} scripts with {actions} actions; {} action kinds; {unreadable} actors or actions unreadable; {dest_found} places characters are sent to found, {dest_missing} not found",
        kinds.len()
    );
    log.line(&line);
    println!("{line}");
}
