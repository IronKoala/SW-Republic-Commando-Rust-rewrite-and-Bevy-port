//! rejoin_survey: for every map in a folder, where its squad mates come
//! from (placed or made by a spawner, what scripts call them, in the squad
//! at the start or not) and whether every `ACTION_ChangeSquadMember` in
//! its scripts names one of them. Reads only.
//!
//! Run: cargo run --release -p rc_squad_plugin --example rejoin_survey -- <Maps folder>
//!
//! Ends `REJOIN SURVEY: N maps, M ChangeSquadMember actions, K name nobody`.

use rc_squad::SquadSettings;

fn main() {
    let dir = std::env::args().nth(1).expect("give the Maps folder");
    let mut maps: Vec<_> = std::fs::read_dir(&dir)
        .expect("read folder")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ctm")))
        .collect();
    maps.sort();
    let classes: Vec<(String, String)> = SquadSettings::default()
        .members
        .iter()
        .map(|m| (m.name.clone(), m.class.clone()))
        .collect();
    let (mut n_maps, mut n_actions, mut n_bad) = (0, 0, 0);
    for path in maps {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let data = std::fs::read(&path).expect("read map");
        let Ok(pkg) = rc_package::Package::parse(&data) else {
            println!("{name}: can't parse");
            continue;
        };
        let Ok(info) = rc_squad::map::read(&pkg, &data) else {
            println!("{name}: squad not read");
            continue;
        };
        let Ok(level) = rc_level::actors::read_level(&pkg, &data) else {
            continue;
        };
        let scripts = rc_level::scripts::read(&pkg, &data, &level);
        let mut actions = Vec::new();
        for a in &scripts.actors {
            for (i, act) in a.actions.iter().enumerate() {
                if act.class.eq_ignore_ascii_case("ACTION_ChangeSquadMember") {
                    actions.push((a.obj.name.clone(), i, act.clone()));
                }
            }
        }
        if !info.wants_squad() && actions.is_empty() {
            continue;
        }
        n_maps += 1;
        let mut tags: Vec<String> = Vec::new();
        let mut line = Vec::new();
        for (who, class) in &classes {
            match info.mate(class) {
                Some(src @ rc_squad::MateSource::Placed(_)) => {
                    tags.push(src.tag().to_ascii_lowercase());
                    line.push(format!("{who}: placed, called {}, in the squad", src.tag()));
                }
                Some(src @ rc_squad::MateSource::Factory(f)) => {
                    tags.push(src.tag().to_ascii_lowercase());
                    line.push(format!(
                        "{who}: spawner {} on '{}', called {}",
                        f.actor,
                        f.tag,
                        src.tag()
                    ));
                }
                None => line.push(format!("{who}: not in the map")),
            }
        }
        // Every spawner's name for what it makes counts (yyy_06d makes
        // second copies of Fixer and Scorch).
        for f in &info.factories {
            tags.push(f.pawn_tag.to_ascii_lowercase());
            if !line.iter().any(|l| l.contains(&f.actor)) {
                line.push(format!(
                    "also spawner {} on '{}' makes a {} called {}",
                    f.actor, f.tag, f.class, f.pawn_tag
                ));
            }
        }
        println!(
            "{name}{}: {}",
            if info.wants_squad() {
                ""
            } else {
                " (no squad)"
            },
            line.join("; ")
        );
        for (who, i, act) in actions {
            let tag = act.text("MemberTag").unwrap_or("");
            let add = act.flag("bAddMember", true);
            let off = act.flag("bDisabled", false);
            n_actions += 1;
            let found = tags.contains(&tag.to_ascii_lowercase());
            let verdict = if off {
                "switched off in the map"
            } else if !info.wants_squad() {
                "map has no squad"
            } else if found {
                "ok"
            } else {
                n_bad += 1;
                "NAMES NOBODY"
            };
            println!(
                "    {who} #{i}: {} {tag}: {verdict}",
                if add { "add" } else { "remove" }
            );
        }
    }
    println!(
        "REJOIN SURVEY: {n_maps} maps, {n_actions} ChangeSquadMember actions, {n_bad} name nobody"
    );
}
