//! sight_triggers: lists every `LineOfSightTrigger` in the game's maps:
//! where it is, what the player must look at, how near and how straight
//! on, whether it starts switched on, what switches it on, and what waits
//! for its event (scripts, doors, spawners, other triggers). Reads only.
//!
//! Run: `cargo run -p rc_scripts --example sight_triggers [MAP...]`
//! (game folder from RC_GAME_DIR). Writes `logs/sight_triggers.log`.

use rc_level::scripts::{Val, Vals};
use rc_package::Package;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";

/// Every text value under a key that holds an event name, with its key.
fn event_values(vals: &Vals, key: &str, out: &mut Vec<(String, String)>) {
    for (k, v) in vals {
        let path = if key.is_empty() {
            k.clone()
        } else {
            format!("{key}.{k}")
        };
        match v {
            Val::Text(t) => {
                let base = k.split('[').next().unwrap_or(k).to_ascii_lowercase();
                if base.contains("event") && !t.eq_ignore_ascii_case("None") && !t.is_empty() {
                    out.push((path, t.clone()));
                }
            }
            Val::Struct(s) => event_values(s, &path, out),
            Val::Structs(list) => {
                for (i, s) in list.iter().enumerate() {
                    event_values(s, &format!("{path}[{i}]"), out);
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let game = std::env::var_os("RC_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_DIR));
    let data = game.join("GameData");
    let mut names: Vec<String> = std::env::args().skip(1).collect();
    if names.is_empty() {
        names = std::fs::read_dir(data.join("Maps"))
            .map(|d| {
                d.flatten()
                    .filter_map(|e| {
                        let p = e.path();
                        let ctm = p.extension()?.to_str()?.eq_ignore_ascii_case("ctm");
                        ctm.then(|| p.file_stem()?.to_str().map(str::to_string))?
                    })
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
    }
    let _ = std::fs::create_dir_all("logs");
    let mut log = std::fs::File::create("logs/sight_triggers.log").expect("log");
    let mut out = |s: String| {
        println!("{s}");
        let _ = writeln!(log, "{s}");
    };
    let mut total = 0;
    let mut maps_with = 0;
    for name in &names {
        let path = data.join("Maps").join(format!("{name}.ctm"));
        let Ok(bytes) = std::fs::read(&path) else {
            out(format!("{name}: can't read"));
            continue;
        };
        let Ok(pkg) = Package::parse(&bytes) else {
            out(format!("{name}: can't parse"));
            continue;
        };
        let Ok(level) = rc_level::actors::read_level(&pkg, &bytes) else {
            continue;
        };
        let map = rc_level::scripts::read(&pkg, &bytes, &level);
        // The level's collision, with doors and lifts where the map
        // places them, for the line checks.
        let world = rc_level::map_collision::load(&path, &data, 84.0)
            .ok()
            .map(|c| {
                let mut tris = c.triangles.clone();
                for t in c.mover_triangles.values() {
                    tris.extend_from_slice(t);
                }
                let v = |p: [f32; 3]| rc_movement::Vec3::new(p[0], p[1], p[2]);
                rc_movement::TriangleWorld::new(
                    tris.iter().map(|t| [v(t[0]), v(t[1]), v(t[2])]).collect(),
                )
            });
        let sights: Vec<_> = map
            .actors
            .iter()
            .filter(|a| a.obj.class.eq_ignore_ascii_case("LineOfSightTrigger"))
            .collect();
        if sights.is_empty() {
            continue;
        }
        maps_with += 1;
        total += sights.len();
        out(format!(
            "== {name}: {} line-of-sight triggers",
            sights.len()
        ));
        // Who sends what, and who waits on what (event name, lower case).
        let mut senders: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut waiters: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for a in &map.actors {
            let o = &a.obj;
            let mut ev = Vec::new();
            event_values(&o.values, "", &mut ev);
            for (k, e) in ev {
                senders
                    .entry(e.to_ascii_lowercase())
                    .or_default()
                    .push(format!("{} ({}) {k}", o.name, o.class));
            }
            if o.has_own_tag() {
                waiters
                    .entry(o.tag().to_ascii_lowercase())
                    .or_default()
                    .push(format!("{} ({}) by its Tag", o.name, o.class));
            }
            for (i, act) in a.actions.iter().enumerate() {
                let kind = act.class.strip_prefix("ACTION_").unwrap_or(&act.class);
                if let Some(e) = act.text("ExternalEvent") {
                    waiters
                        .entry(e.to_ascii_lowercase())
                        .or_default()
                        .push(format!("{} #{i} {kind}", o.name));
                }
                let mut ev = Vec::new();
                event_values(&act.values, "", &mut ev);
                for (k, e) in ev {
                    if k == "ExternalEvent" {
                        continue;
                    }
                    senders
                        .entry(e.to_ascii_lowercase())
                        .or_default()
                        .push(format!("{} #{i} {kind} {k}", o.name));
                }
            }
        }
        for s in sights {
            let o = &s.obj;
            let seen = o.text("SeenActorTag").unwrap_or("None");
            let seen_actor = map
                .actors
                .iter()
                .find(|b| b.obj.export != o.export && b.obj.tag().eq_ignore_ascii_case(seen));
            out(format!(
                "  {} at ({:.0}, {:.0}, {:.0}) Tag={} Event={} SeenActorTag={}{} MaxViewDist={} MaxViewAngle={} bEnabled={}",
                o.name,
                s.location[0],
                s.location[1],
                s.location[2],
                o.tag(),
                o.event().unwrap_or("None"),
                seen,
                seen_actor.map_or(String::new(), |b| format!(
                    " -> {} ({}) at ({:.0}, {:.0}, {:.0})",
                    b.obj.name, b.obj.class, b.location[0], b.location[1], b.location[2]
                )),
                o.float("MaxViewDist")
                    .map_or("3000 (class)".into(), |v| v.to_string()),
                o.int("MaxViewAngle")
                    .map_or("15 (class)".into(), |v| v.to_string()),
                o.flag("bEnabled", true),
            ));
            // Seen from the trigger's own spot, at eye height: what the
            // level designer most likely meant as the place to stand.
            let target = seen_actor.map_or(s.location, |b| b.location);
            let eye = rc_movement::Vec3::new(s.location[0], s.location[1], s.location[2] + 64.0);
            let at = rc_movement::Vec3::new(target[0], target[1], target[2]);
            let dist = (at - eye).length();
            let max = o.float("MaxViewDist").unwrap_or(3000.0);
            let view = match &world {
                None => "no collision read".to_string(),
                Some(w) => {
                    let clear = rc_scripts::sight::clear_line(w, eye, at);
                    if !clear {
                        use rc_movement::World;
                        if let Some(h) = w.sweep(rc_movement::Vec3::ZERO, eye, at) {
                            out(format!(
                                "      first hit at {:.0} of {dist:.0} uu",
                                h.time * dist
                            ));
                        }
                    }
                    format!(
                        "{dist:.0} uu, {}, line {}",
                        if dist <= max {
                            "near enough"
                        } else {
                            "TOO FAR"
                        },
                        if clear { "clear" } else { "BLOCKED" }
                    )
                }
            };
            out(format!("      looking from its own spot: {view}"));
            if o.has_own_tag() {
                for w in senders
                    .get(&o.tag().to_ascii_lowercase())
                    .into_iter()
                    .flatten()
                {
                    out(format!("      switched on by: {w}"));
                }
            }
            if let Some(e) = o.event() {
                let e = e.to_ascii_lowercase();
                let list = waiters.get(&e).cloned().unwrap_or_default();
                if list.is_empty() {
                    out("      nothing waits for its event".into());
                }
                for w in list {
                    out(format!("      heard by: {w}"));
                }
                let others: Vec<_> = senders
                    .get(&e)
                    .into_iter()
                    .flatten()
                    .filter(|s| !s.starts_with(&format!("{} ", o.name)))
                    .collect();
                for s in others {
                    out(format!("      the same event is also sent by: {s}"));
                }
            }
        }
    }
    out(format!(
        "{total} line-of-sight triggers in {maps_with} of {} maps",
        names.len()
    ));
}
