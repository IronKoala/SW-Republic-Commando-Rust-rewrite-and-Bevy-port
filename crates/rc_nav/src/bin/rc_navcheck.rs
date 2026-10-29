//! rc_navcheck: read a map's path network and check it with numbers.
//!
//! Usage: rc_navcheck [MAP]   (default geo_01a; a name from GameData/Maps or
//! a path to a .ctm file). The game folder is RC_GAME_DIR or the default
//! Steam path. Writes logs/rc_navcheck.log and exits with code 1 if a check
//! fails.
//!
//! Checks:
//! - every reach spec reads and both its ends are nodes we kept;
//! - each link's stored distance matches the straight line between its
//!   nodes (within 2 uu), which shows positions and links were read right;
//! - one big walkable island holds most nodes;
//! - routes between far-apart nodes exist, and are never shorter than the
//!   straight line (they can't be).

use rc_nav::{reach, NavGraph, NodeKind};
use rc_package::Package;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "geo_01a".into());
    let game = PathBuf::from(std::env::var("RC_GAME_DIR").unwrap_or_else(|_| DEFAULT_GAME_DIR.into()));
    let path = if arg.ends_with(".ctm") {
        PathBuf::from(&arg)
    } else {
        game.join("GameData").join("Maps").join(format!("{arg}.ctm"))
    };
    let _ = std::fs::create_dir_all("logs");
    let mut log = std::fs::File::create("logs/rc_navcheck.log").ok();
    let mut line = |s: String| {
        println!("{s}");
        if let Some(f) = log.as_mut() {
            let _ = writeln!(f, "{s}");
        }
    };
    let mut failed = 0;
    let mut check = |ok: bool, what: String, line: &mut dyn FnMut(String)| {
        line(format!("CHECK {}: {what}", if ok { "PASS" } else { "FAIL" }));
        if !ok {
            failed += 1;
        }
    };

    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            line(format!("can't read {}: {e}", path.display()));
            std::process::exit(1);
        }
    };
    let pkg = Package::parse(&data).expect("map package");
    let t = std::time::Instant::now();
    let (g, stats) = match NavGraph::from_map_with_stats(&pkg, &data) {
        Ok(x) => x,
        Err(e) => {
            line(format!("FAIL: {e}"));
            std::process::exit(1);
        }
    };
    line(format!(
        "map {}: {} nodes, {} links, {} reach specs ({} unreadable, {} dangling), read in {:.1} ms",
        path.display(),
        g.nodes.len(),
        g.link_count(),
        stats.reach_specs,
        stats.unreadable,
        stats.dangling,
        t.elapsed().as_secs_f64() * 1000.0
    ));
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for n in &g.nodes {
        *kinds.entry(n.class.clone()).or_default() += 1;
    }
    line(format!("nodes by class: {kinds:?}"));
    line(format!("links by reachFlags: {:?}", stats.flag_counts));
    for n in g.nodes.iter().filter(|n| n.is_cover()).take(5) {
        if let NodeKind::Cover { strafe_left, strafe_right, distance_to_cover, facing } = n.kind {
            line(format!(
                "cover {} '{}' at ({:.0}, {:.0}, {:.0}): step out left {strafe_left:.0}, right {strafe_right:.0}, cover {distance_to_cover:.0} away, faces {:.0} deg",
                n.name, n.tag, n.location.x, n.location.y, n.location.z, facing.to_degrees()
            ));
        }
    }

    check(stats.unreadable == 0 && stats.dangling == 0, format!("all reach specs read and connect kept nodes ({} unreadable, {} dangling)", stats.unreadable, stats.dangling), &mut line);

    let mut worst = 0.0f32;
    let mut off = 0;
    for (a, ls) in g.links.iter().enumerate() {
        for l in ls {
            let straight = (g.nodes[l.to].location - g.nodes[a].location).length();
            let d = (l.distance - straight).abs();
            worst = worst.max(d);
            if d > 2.0 {
                off += 1;
            }
        }
    }
    check(off == 0, format!("stored link distance = straight line between its nodes (worst difference {worst:.2} uu, {off} links over 2 uu)"), &mut line);

    let (islands, biggest) = g.islands();
    check(
        biggest * 2 > g.nodes.len(),
        format!("one big walkable island: {islands} islands, the biggest has {biggest} of {} nodes", g.nodes.len()),
        &mut line,
    );

    // Routes: from the first node to the five farthest nodes it can reach.
    let walk_nodes: Vec<usize> = (0..g.nodes.len())
        .filter(|&i| g.links[i].iter().any(|l| reach::walkable(l.flags)))
        .collect();
    if let Some(&start) = walk_nodes.first() {
        let mut far: Vec<(usize, f32)> = walk_nodes
            .iter()
            .map(|&i| (i, (g.nodes[i].location - g.nodes[start].location).length()))
            .collect();
        far.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut found = 0;
        let mut shorter = 0;
        let t = std::time::Instant::now();
        let mut tried = 0;
        for &(goal, straight) in far.iter() {
            if found >= 5 || tried >= 40 {
                break;
            }
            tried += 1;
            let Some(r) = g.route(g.nodes[start].location, g.nodes[goal].location) else {
                continue;
            };
            found += 1;
            if r.length + 1.0 < straight {
                shorter += 1;
            }
            line(format!(
                "route {} -> {}: {} nodes, {:.0} uu along the paths, {:.0} uu straight ({:.2}x)",
                g.nodes[start].name,
                g.nodes[goal].name,
                r.nodes.len(),
                r.length,
                straight,
                r.length / straight.max(1.0)
            ));
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        check(found >= 5 && shorter == 0, format!("5 long routes found ({found} of {tried} tried, {shorter} shorter than a straight line), {ms:.1} ms in all"), &mut line);
    }
    line(format!("DONE: {failed} checks failed"));
    std::process::exit(if failed == 0 { 0 } else { 1 });
}
