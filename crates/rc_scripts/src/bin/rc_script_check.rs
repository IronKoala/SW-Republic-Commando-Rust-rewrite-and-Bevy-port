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
    // Sound packages read so far: the lower-case names of their sounds.
    let mut packages: BTreeMap<String, Option<std::collections::HashSet<String>>> = BTreeMap::new();
    let (mut sounds_total, mut sounds_found) = (0, 0);
    let mut classes: BTreeMap<String, usize> = BTreeMap::new();
    // Map -> the maps its exits lead to.
    let mut exits: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // Map -> its teleporters' tags (places to arrive at), and every
    // (from, to, portal) an exit names.
    let mut arrivals: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut portals: Vec<(String, String, String)> = Vec::new();
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
        let (triggers, tags) = rc_level::map_collision::load(&path, &data, 84.0)
            .map(|l| {
                let tags = l.arrivals.iter().map(|a| a.0.to_ascii_lowercase()).collect();
                (l.movers.triggers, tags)
            })
            .unwrap_or_default();
        arrivals.insert(name.to_ascii_lowercase(), tags);
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
        // The sounds the scripts play: where each one lives, and whether
        // that package really has it.
        let mut sound_names: Vec<String> = Vec::new();
        for a in map.scripts() {
            for act in &a.actions {
                if rc_scripts::sound_action(act) {
                    if let Some(n) = act.text("Sound") {
                        if !sound_names.iter().any(|m| m.eq_ignore_ascii_case(n)) {
                            sound_names.push(n.to_string());
                        }
                    }
                }
            }
        }
        let (mut found, mut lost) = (0, Vec::new());
        for n in &sound_names {
            match map.sounds.get(&n.to_ascii_lowercase()) {
                Some(r) => {
                    let pkg_name = r.package.clone().unwrap_or_else(|| name.clone());
                    let ok = packages
                        .entry(pkg_name.to_ascii_lowercase())
                        .or_insert_with(|| sound_names_in(&data, &pkg_name))
                        .as_ref()
                        .map(|set| set.contains(&n.to_ascii_lowercase()));
                    *classes.entry(r.class.clone()).or_default() += 1;
                    match ok {
                        Some(true) => found += 1,
                        Some(false) => lost.push(format!("{n} (not in {pkg_name})")),
                        None => lost.push(format!("{n} ({pkg_name} not readable)")),
                    }
                }
                None => lost.push(format!("{n} (the map doesn't say where)")),
            }
        }
        sounds_total += sound_names.len();
        sounds_found += found;
        log.line(format!(
            "   sounds: {} played by scripts, {found} found in their package{}",
            sound_names.len(),
            if lost.is_empty() {
                String::new()
            } else {
                format!("; missing: {}", lost.join(", "))
            }
        ));
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
        // Where the level can be left for: walk-in exits and scripts.
        let mut leads = Vec::new();
        for (tp, url, at, open) in d.exits() {
            log.line(format!(
                "   exit {tp} at ({:.0}, {:.0}, {:.0}) to {url}{}",
                at[0],
                at[1],
                at[2],
                if open { "" } else { " (closed until a script or event opens it)" }
            ));
            leads.push(url);
        }
        for a in map.scripts() {
            for act in &a.actions {
                let kind = act.class.strip_prefix("ACTION_").unwrap_or(&act.class);
                let url = match kind {
                    "ChangeLevel" => act.text("URL"),
                    "ShowDebriefingScreen" => act.text("NewLevel"),
                    _ => None,
                };
                if let Some(url) = url.filter(|_| !act.flag("bDisabled", false)) {
                    log.line(format!("   exit by script {} to {url}", a.obj.name));
                    leads.push(url.to_string());
                }
            }
        }
        for url in leads {
            match rc_scripts::level_url(&url) {
                Some((m, portal)) => {
                    if let Some(p) = portal {
                        portals.push((name.to_ascii_lowercase(), m.clone(), p));
                    }
                    exits.entry(name.to_ascii_lowercase()).or_default().push(m)
                }
                None => log.line(format!("   ERROR: can't read the level address {url:?}")),
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
                    Effect::ChangeLevel { url, by } => format!("{by}: leave for {url}"),
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
    // Follow the campaign from the prologue, as a player would.
    let have: Vec<String> = names.iter().map(|n| n.to_ascii_lowercase()).collect();
    let mut missing = 0;
    for (from, to) in &exits {
        for m in to {
            if !have.contains(m) {
                missing += 1;
                log.line(format!("== ERROR: {from} leads to {m}, which isn't in GameData/Maps"));
            }
        }
    }
    // Where each exit's portal puts the player on the next map.
    let (mut at_teleporter, mut at_start) = (0, 0);
    for (from, to, p) in &portals {
        let found = arrivals
            .get(to)
            .is_some_and(|t| t.contains(&p.to_ascii_lowercase()));
        if found {
            at_teleporter += 1;
        } else {
            at_start += 1;
        }
        log.line(format!(
            "== {from} -> {to}#{p}: {}",
            if found {
                "arrives at the teleporter with that tag"
            } else {
                "no teleporter has that tag: arrives at the player start"
            }
        ));
    }
    println!("{at_teleporter} exits arrive at a teleporter, {at_start} at the next map's player start");
    if have.iter().any(|n| n == "pro") {
        let mut chain = vec!["pro".to_string()];
        loop {
            let last = chain.last().unwrap().clone();
            let next = exits.get(&last).and_then(|v| v.first()).cloned();
            match next {
                Some(n) if !chain.contains(&n) => chain.push(n),
                _ => break,
            }
        }
        log.line(format!(
            "== campaign from the prologue, following each map's exit: {} maps: {}",
            chain.len(),
            chain.join(" -> ")
        ));
        println!("campaign: {} maps from pro to {}", chain.len(), chain.last().unwrap());
    }
    let ends: Vec<&String> = have
        .iter()
        .filter(|n| !exits.contains_key(*n))
        .collect();
    log.line(format!("== maps with no exit: {ends:?}"));
    println!("{missing} exits lead to a map that isn't there");
    let line = format!(
        "{maps} maps, {failed} unreadable; {scripts} scripts with {actions} actions; {} action kinds; {unreadable} actors or actions unreadable; {sounds_found} of {sounds_total} script sounds found; {dest_found} places characters are sent to found, {dest_missing} not found",
        kinds.len()
    );
    log.line(&line);
    log.line(format!("== script sounds by kind (per map): {classes:?}"));
    println!("{line}");
}

/// The lower-case names of every sound in a sound package
/// (`GameData/Sounds/<name>.uax`), or in a map when `name` is one.
fn sound_names_in(data: &std::path::Path, name: &str) -> Option<std::collections::HashSet<String>> {
    // File names differ in case from what the maps say (`GEO_Voice`).
    let find = |dir: &str, file: String| {
        fs::read_dir(data.join(dir)).ok()?.flatten().find_map(|e| {
            e.file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(&file)
                .then(|| e.path())
        })
    };
    let path =
        find("Sounds", format!("{name}.uax")).or_else(|| find("Maps", format!("{name}.ctm")))?;
    let bytes = fs::read(path).ok()?;
    let pkg = Package::parse(&bytes).ok()?;
    Some(
        pkg.exports
            .iter()
            .filter(|e| {
                matches!(
                    pkg.export_class_name(e),
                    "Sound" | "SoundMultiple" | "SoundStreamed"
                )
            })
            .map(|e| pkg.name(e.object_name).to_ascii_lowercase())
            .collect(),
    )
}
