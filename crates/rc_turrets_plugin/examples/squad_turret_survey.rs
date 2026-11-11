//! squad_turret_survey: for every squad "Man turret" marker in a folder of
//! maps, which turret the squad mate sent there gets on (the turret the
//! marker's anchor names, else the nearest within 512 uu), or that none
//! does and they fight from the spot. Reads only; writes
//! `logs/squad_turret_survey.log` (and prints it).
//!
//! Run: cargo run --release -p rc_turrets_plugin --example squad_turret_survey -- <Maps folder> [map ...]

use rc_squad::{MarkerSpot, SquadSettings};
use rc_turrets::{for_marker, Turret};
use std::io::Write;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).expect("give the Maps folder").clone();
    let only: Vec<String> = args[2..].iter().map(|s| s.to_ascii_lowercase()).collect();
    let settings = std::fs::read_to_string("data/squad/squad.toml")
        .ok()
        .and_then(|t| SquadSettings::default().with_overrides(&t).ok())
        .unwrap_or_default();
    let (kinds, errors) = rc_turrets::params::load_folders(&[Path::new("data/turrets")]);
    let _ = std::fs::create_dir_all("logs");
    let mut log = std::fs::File::create("logs/squad_turret_survey.log").expect("log");
    let mut line = |s: String| {
        println!("{s}");
        let _ = writeln!(log, "{s}");
    };
    for e in errors {
        line(format!("ERROR in a turret file: {e}"));
    }
    let mut maps: Vec<_> = std::fs::read_dir(&dir)
        .expect("read folder")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ctm")))
        .filter(|p| {
            only.is_empty()
                || only.iter().any(|o| {
                    p.file_stem()
                        .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(o))
                })
        })
        .collect();
    maps.sort();
    let (mut total, mut found) = (0, 0);
    for path in maps {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let data = std::fs::read(&path).expect("read map");
        let Ok(pkg) = rc_package::Package::parse(&data) else {
            line(format!("{name}: can't parse"));
            continue;
        };
        let Ok(info) = rc_squad::map::read(&pkg, &data) else {
            continue;
        };
        // The job each marker gets, as `Squad::job_of` picks it.
        let job_of = |m: &MarkerSpot| {
            let key = [&m.marker_class, &m.anchor_class, &m.name]
                .into_iter()
                .find(|s| !s.is_empty())?;
            settings.jobs.iter().find(|j| j.fits(key))
        };
        let posts: Vec<&MarkerSpot> = info
            .markers
            .iter()
            .filter(|m| job_of(m).is_some_and(|j| j.mount))
            .collect();
        if posts.is_empty() {
            continue;
        }
        let list: Vec<Turret> = match rc_level::actors::read_level(&pkg, &data) {
            Ok(level) => {
                let claims = |c: &str| rc_turrets::params::for_class(&kinds, c).is_some();
                rc_turrets::map::read(&pkg, &data, &level, &claims)
                    .0
                    .into_iter()
                    .filter_map(|p| {
                        let (id, params) = rc_turrets::params::for_class(&kinds, &p.class)?;
                        Some(Turret::new(p, id, params))
                    })
                    .collect()
            }
            Err(_) => Vec::new(),
        };
        for spot in posts {
            let job = job_of(spot).map(|j| j.name.clone()).unwrap_or_default();
            let s = spot.stand;
            let pick = for_marker(&list, &spot.anchor, [s.x, s.y, s.z], 512.0);
            total += 1;
            let to = match pick {
                Some(i) => {
                    found += 1;
                    let t = &list[i];
                    let d = ((t.place.location[0] - s.x).powi(2)
                        + (t.place.location[1] - s.y).powi(2))
                    .sqrt();
                    format!(
                        "gets on {} ({}, {:.0} uu away, player may use: {})",
                        t.place.name, t.kind, d, t.player_usable
                    )
                }
                None => "NO TURRET: fights from the spot".into(),
            };
            line(format!(
                "{name}: {} ({} {}, anchor {} {}) at ({:.0}, {:.0}, {:.0}): {to}",
                spot.name,
                job,
                if spot.marker_class.is_empty() {
                    "-"
                } else {
                    &spot.marker_class
                },
                spot.anchor,
                spot.anchor_class,
                s.x,
                s.y,
                s.z
            ));
        }
    }
    line(format!("{found} of {total} turret posts find a turret"));
}
