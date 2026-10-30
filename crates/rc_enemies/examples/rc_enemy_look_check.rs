//! Reads every enemy type's model the way the game does (data/enemies/*.toml
//! `[look]`) and checks each one: the mesh is found, every animation the file
//! names is found (a missing one is listed as a NOTE: that pose plays the
//! idle animation), and the skin is a real picture rather than a grey
//! stand-in. No window. Needs the game files:
//!
//! `RC_GAME_DIR=<game folder> cargo run -p rc_enemies --example rc_enemy_look_check`
//!
//! Writes `logs/enemy_look_check.log` and ends `LOOK CHECK: N passed, M failed`.

use rc_ai::params::AiParams;
use rc_enemies::look::{DroidLook, Pose};
use rc_game_core::GamePaths;
use std::io::Write;

fn main() {
    let Ok(game) = std::env::var("RC_GAME_DIR") else {
        eprintln!("set RC_GAME_DIR to the game folder");
        std::process::exit(2);
    };
    let paths = GamePaths::new(game, Vec::new());
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/enemies");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("can't read data/enemies")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    std::fs::create_dir_all("logs").expect("can't create logs");
    let mut log = std::fs::File::create("logs/enemy_look_check.log").expect("can't create the log");
    let mut line = |s: String| {
        println!("{s}");
        writeln!(log, "{s}").ok();
    };
    let (mut passed, mut failed) = (0, 0);
    let mut check = |line: &mut dyn FnMut(String), ok: bool, what: String| {
        line(format!("{} {what}", if ok { "PASS" } else { "FAIL" }));
        if ok {
            passed += 1;
        } else {
            failed += 1;
        }
    };
    for file in files {
        let id = file.file_stem().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(&file).expect("can't read an enemy file");
        let params = match AiParams::battle_droid().with_overrides(&text) {
            Ok(p) => p,
            Err(e) => {
                check(&mut line, false, format!("{id}: settings file: {e}"));
                continue;
            }
        };
        let l = &params.look;
        line(format!(
            "--- {id}: {} mesh {} skin {:?}",
            l.package, l.mesh, l.skin
        ));
        let (look, notes) = match DroidLook::load(&paths, l) {
            Ok(x) => x,
            Err(e) => {
                check(&mut line, false, format!("{id}: model loads ({e})"));
                continue;
            }
        };
        for n in &notes {
            line(format!("  {n}"));
        }
        check(&mut line, true, format!("{id}: model loads"));
        let named = Pose::ALL
            .iter()
            .filter(|p| !p.sequence(l).is_empty())
            .count();
        // A missing animation isn't a failure: that pose plays the idle
        // one (the notes above name which). The model still shows.
        line(format!(
            "{} {id}: {} of {named} named animations found",
            if look.clips.len() == named {
                "PASS"
            } else {
                "NOTE"
            },
            look.clips.len()
        ));
        // A stand-in skin is a flat grey; a real one has colour in it.
        let ok = match &look.skin {
            Some((name, img)) => {
                let [r, g, b, _] = img.mean();
                let spread = r.max(g).max(b) - r.min(g).min(b);
                check(
                    &mut line,
                    !name.to_ascii_lowercase().contains("proxy") && spread > 3.0,
                    format!(
                        "{id}: skin {name} is a real picture (average R {r:.0} G {g:.0} B {b:.0})"
                    ),
                );
                true
            }
            None => false,
        };
        if !ok {
            check(&mut line, false, format!("{id}: skin loads"));
        }
        let points = look.posed(Pose::Idle, 0.0);
        let (lo, hi) = points
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| {
                (lo.min(p[1]), hi.max(p[1]))
            });
        let tall_uu = (hi - lo) * 50.0;
        let want_uu = 2.0 * params.movement.collision_height;
        check(
            &mut line,
            tall_uu > 0.5 * want_uu && tall_uu < 2.0 * want_uu,
            format!("{id}: standing {tall_uu:.0} uu tall (collision height {want_uu:.0} uu)"),
        );
    }
    line(format!("LOOK CHECK: {passed} passed, {failed} failed"));
    if failed > 0 {
        std::process::exit(1);
    }
}
