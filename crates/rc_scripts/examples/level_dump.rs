//! level_dump (scratch, for the level blockers audit): writes every map's
//! actors, events waited on, event senders and script actions as
//! tab-separated lines, for a script to sort through. Reads only.
//!
//! Run: RC_GAME_DIR=... cargo run --release -p rc_scripts --example level_dump -- MAP...

use rc_level::scripts::{Val, Vals};
use rc_package::Package;
use std::path::PathBuf;

fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

/// Every text value under a key ending in "Event" (not ExternalEvent),
/// with its path, walking structs and lists of structs.
fn events(prefix: &str, vals: &Vals, out: &mut Vec<(String, String)>) {
    for (k, v) in vals {
        let path = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match v {
            Val::Text(t) => {
                let base = k.split('[').next().unwrap_or(k).to_ascii_lowercase();
                if base.contains("event")
                    && base != "externalevent"
                    && !t.eq_ignore_ascii_case("None")
                    && !t.is_empty()
                {
                    out.push((path, t.clone()));
                }
            }
            Val::Struct(s) => events(&path, s, out),
            Val::Structs(list) => {
                for (i, s) in list.iter().enumerate() {
                    events(&format!("{path}[{i}]"), s, out);
                }
            }
            _ => {}
        }
    }
}

fn compact(vals: &Vals) -> String {
    vals.iter()
        .filter(|(k, _)| k.as_str() != "Comment")
        .map(|(k, v)| match v {
            Val::Text(t) => format!("{k}={t}"),
            Val::Struct(s) => format!("{k}={{{}}}", compact(s)),
            Val::Raw(_) => format!("{k}=<raw>"),
            other => format!("{k}={other:?}"),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() {
    let game = PathBuf::from(std::env::var_os("RC_GAME_DIR").expect("RC_GAME_DIR"));
    let data = game.join("GameData");
    for name in std::env::args().skip(1) {
        let path = data.join("Maps").join(format!("{name}.ctm"));
        let bytes = std::fs::read(&path).expect("read map");
        let pkg = Package::parse(&bytes).expect("parse");
        let level = rc_level::actors::read_level(&pkg, &bytes).expect("level");
        let map = rc_level::scripts::read(&pkg, &bytes, &level);
        // What the port's own readers recognise: movers, triggers, props
        // that can break.
        match rc_level::map_collision::load(&path, &data, 84.0) {
            Ok(l) => {
                for m in &l.movers.movers {
                    println!("RMOVER\t{name}\t{}\t{}", m.place.class, m.place.name);
                }
                for t in &l.movers.triggers {
                    println!(
                        "RTRIG\t{name}\t{}\t{}\t{}\t{}\t{}",
                        t.class, t.name, t.state, t.trigger_type, t.event
                    );
                }
                for b in &l.breakables {
                    println!(
                        "RBREAK\t{name}\t{}\t{}\t{}\t{}",
                        b.class,
                        b.name,
                        b.health,
                        b.event.clone().unwrap_or_default()
                    );
                }
                println!("RSTART\t{name}\t{:?}", l.start.map(|s| s.0));
            }
            Err(e) => println!("FAIL\t{name}\tcollision: {e}"),
        }
        for f in &map.failures {
            println!("FAIL\t{name}\t{}", clean(f));
        }
        for a in &map.actors {
            let o = &a.obj;
            println!(
                "ACTOR\t{name}\t{}\t{}\t{}\t{:.0},{:.0},{:.0}",
                o.class,
                o.name,
                o.tag(),
                a.location[0],
                a.location[1],
                a.location[2]
            );
            let mut ev = Vec::new();
            events("", &o.values, &mut ev);
            for (k, e) in ev {
                println!("SEND\t{name}\t{e}\t{}\t{}\t{k}", o.class, o.name);
            }
            if a.actions.is_empty() {
                println!("VALS\t{name}\t{}\t{}\t{}", o.class, o.name, clean(&compact(&o.values)));
            }
            for (i, act) in a.actions.iter().enumerate() {
                let kind = act.class.strip_prefix("ACTION_").unwrap_or(&act.class);
                println!(
                    "ACT\t{name}\t{}\t{i}\t{kind}\t{}",
                    o.name,
                    clean(&compact(&act.values))
                );
                if kind.eq_ignore_ascii_case("WaitForEvent") {
                    if let Some(e) = act.text("ExternalEvent") {
                        println!("WAIT\t{name}\t{e}\t{}\t{}\t{i}", o.class, o.name);
                    }
                } else {
                    let mut ev = Vec::new();
                    events("", &act.values, &mut ev);
                    for (k, e) in ev {
                        println!("SEND\t{name}\t{e}\tACTION_{kind}\t{}#{i}\t{k}", o.name);
                    }
                }
            }
        }
    }
}
