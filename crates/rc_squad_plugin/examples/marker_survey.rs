//! marker_survey: lists every squad marker in a folder of maps, to check
//! which `[job.*]` each one gets. Reads only.
//!
//! Run: cargo run --release -p rc_squad_plugin --example marker_survey -- <Maps folder>

use rc_squad::{MarkerSpot, Squad, SquadSettings};
use std::collections::BTreeMap;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).expect("give the Maps folder").clone();
    let dump = (args.get(2).map(|s| s.as_str()) == Some("--dump"))
        .then(|| (args[3].clone(), args[4].clone()));
    let settings = SquadSettings::default();
    let mut by_class: BTreeMap<String, (usize, Option<String>)> = BTreeMap::new();
    let mut maps: Vec<_> = std::fs::read_dir(&dir)
        .expect("read folder")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ctm")))
        .collect();
    maps.sort();
    let mut markers: BTreeMap<String, usize> = BTreeMap::new();
    for path in maps {
        let data = std::fs::read(&path).expect("read map");
        let Ok(pkg) = rc_package::Package::parse(&data) else {
            println!("{}: can't parse", path.display());
            continue;
        };
        let info = match rc_squad::map::read(&pkg, &data) {
            Ok(i) => i,
            Err(e) => {
                println!("{}: {e}", path.display());
                continue;
            }
        };
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        // Marker actors of their own (CTMarker classes), spots or not.
        if let Ok(level) = rc_level::actors::read_level(&pkg, &data) {
            for &a in level.actors.iter().flatten() {
                let class = pkg.export_class_name(&pkg.exports[a]).to_string();
                if class.starts_with("Marker") {
                    *markers.entry(class.clone()).or_default() += 1;
                }
                // `--dump <map> <class>`: every setting of those actors.
                if dump.as_ref().is_some_and(|(m, c)| {
                    name.starts_with(m.as_str()) && c.eq_ignore_ascii_case(&class)
                }) {
                    let p = rc_level::actors::read_placement(&pkg, &data, a);
                    println!(
                        "{name} {} {class} at {:?}",
                        pkg.object_name((a + 1) as i32),
                        p.map(|p| p.location)
                    );
                    if let Ok(props) = rc_level::actors::read_actor_properties(&pkg, &data, a) {
                        for q in props {
                            let v = match &q.value {
                                rc_package::properties::Value::Object(o) => {
                                    format!("-> {}", pkg.object_name(*o))
                                }
                                rc_package::properties::Value::Name(n) => {
                                    format!("'{}'", pkg.name(*n))
                                }
                                v => format!("{v:?}").chars().take(120).collect(),
                            };
                            println!("    {}[{}] = {v}", pkg.name(q.name), q.array_index);
                        }
                    }
                }
            }
        }
        let mut groups: BTreeMap<String, Vec<&MarkerSpot>> = BTreeMap::new();
        for m in &info.markers {
            groups
                .entry(format!("{} {}", m.marker_class, m.anchor))
                .or_default()
                .push(m);
        }
        let mut line = format!("{name}: {} spots", info.markers.len());
        for (anchor, spots) in &groups {
            // The same order as `Squad::job_of`: order marker, anchor, name.
            let sp = spots[0];
            let class = [&sp.marker_class, &sp.anchor_class, &sp.name]
                .into_iter()
                .find(|s| !s.is_empty())
                .cloned()
                .unwrap_or_default();
            let job = settings
                .jobs
                .iter()
                .find(|j| j.fits(&class))
                .map(|j| j.key.clone());
            let e = by_class.entry(class.clone()).or_insert((0, job.clone()));
            e.0 += spots.len();
            line += &format!(
                " | {anchor} ({class}) x{} -> {}",
                spots.len(),
                job.as_deref().unwrap_or("NO JOB")
            );
        }
        println!("{line}");
    }
    println!("\nby order marker class, else anchor class (markers, job):");
    for (c, (n, j)) in &by_class {
        println!("  {c:30} {n:4}  {}", j.as_deref().unwrap_or("NO JOB"));
    }
    println!("\nmarker actors (CTMarker classes) in the maps:");
    for (c, n) in &markers {
        println!("  {c:30} {n:4}");
    }
    let _ = Squad::job_of;
}
