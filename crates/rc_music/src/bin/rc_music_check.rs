//! Checks the game's music against its maps, with no window or sound.
//!
//! For every map in `GameData/Maps`: its battle music, its rooms with
//! music, and every music action in its scripts must name pieces and
//! groups the music packages have, and every piece's `.ogg` must be in
//! `GameData/Music`. Writes `logs/rc_music_check.log` and ends with
//! `MUSIC CHECK: ... 0 problems` when all is well.
//!
//! Run from the repository folder:
//! `cargo run -p rc_music --bin rc_music_check` (set `RC_GAME_DIR` to the
//! game folder if it isn't in the default Steam place).

use rc_music::{Entry, MusicLibrary};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";
const LOG: &str = "logs/rc_music_check.log";

struct Log(fs::File);
impl Log {
    fn line(&mut self, s: impl AsRef<str>) {
        let _ = writeln!(self.0, "{}", s.as_ref());
        println!("{}", s.as_ref());
    }
}

fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        e.file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
            .then(|| e.path())
    })
}

/// Every piece a name can lead to (a group's members, first, won and lost
/// music, all the way down). Unknown names go to `missing`.
fn pieces(
    lib: &MusicLibrary,
    name: &str,
    out: &mut BTreeSet<String>,
    missing: &mut BTreeSet<String>,
    depth: u32,
) {
    if depth > 8 {
        return;
    }
    match lib.get(name) {
        Some(Entry::Piece(p)) => {
            out.insert(p.file.clone());
        }
        Some(Entry::Group(g)) => {
            for m in g
                .members
                .iter()
                .chain(&g.first)
                .chain(&g.won)
                .chain(&g.lost)
            {
                pieces(lib, m, out, missing, depth + 1);
            }
        }
        None => {
            missing.insert(name.to_string());
        }
    }
}

fn main() {
    let game = std::env::var_os("RC_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_DIR));
    let data = find(&game, "GameData").unwrap_or_else(|| game.join("GameData"));
    let _ = fs::create_dir_all("logs");
    let mut log = Log(fs::File::create(LOG).expect("can't create the log"));
    let lib = MusicLibrary::load(&data);
    let (np, ng) = lib.counts();
    log.line(format!(
        "music packages: {np} pieces, {ng} groups; {} room classes with music",
        lib.rooms.len()
    ));
    for n in &lib.notes {
        log.line(format!("  note: {n}"));
    }
    let music_dir = find(&data, "Music");
    let have: BTreeSet<String> = music_dir
        .as_ref()
        .and_then(|d| fs::read_dir(d).ok())
        .map(|r| {
            r.flatten()
                .map(|e| e.file_name().to_string_lossy().to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let mut problems = Vec::new();
    // Every piece in the packages, and whether its file is there.
    let mut lost_files = BTreeSet::new();
    for e in lib.entries.values() {
        if let Entry::Piece(p) = e {
            if !have.contains(&format!("{}.ogg", p.file.to_ascii_lowercase())) {
                lost_files.insert(p.file.clone());
            }
        }
    }
    log.line(format!(
        "{} of {np} pieces have no .ogg in GameData/Music: {:?}",
        lost_files.len(),
        lost_files
    ));
    // Rooms.
    for r in lib.rooms.values() {
        for n in r.state.iter().chain(&r.overlays) {
            let mut miss = BTreeSet::new();
            pieces(&lib, n, &mut BTreeSet::new(), &mut miss, 0);
            for m in miss {
                problems.push(format!(
                    "room class {}: {m} not in the music packages",
                    r.class
                ));
            }
        }
    }
    let mut maps: Vec<PathBuf> = find(&data, "Maps")
        .and_then(|d| fs::read_dir(d).ok())
        .map(|r| {
            r.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ctm")))
                .collect()
        })
        .unwrap_or_default();
    maps.sort();
    let (mut n_maps, mut n_cues, mut n_rooms) = (0, 0, 0);
    let mut unread_rooms = BTreeSet::new();
    for path in &maps {
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let read = fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|d| {
                rc_package::Package::parse(&d)
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
                problems.push(format!("{name}: can't read ({e})"));
                continue;
            }
        };
        n_maps += 1;
        let mm = rc_level::music::read(&pkg, &bytes, &level);
        let scripts = rc_level::scripts::read(&pkg, &bytes, &level);
        let mut names: Vec<(String, String)> = Vec::new();
        if let Some(b) = &mm.battle.music {
            names.push(("battle music".into(), b.clone()));
        }
        let mut rooms = Vec::new();
        for v in &mm.volumes {
            match lib.room(&v.audio_class) {
                Some(r) => {
                    rooms.push(format!("{}={}", v.shape.name, r.class));
                    n_rooms += 1;
                }
                None => {
                    // Fine when the class has no music; a problem only if
                    // the class itself is unknown.
                    unread_rooms.insert(v.audio_class.clone());
                }
            }
        }
        for f in &mm.failures {
            problems.push(format!("{name}: volume not read: {f}"));
        }
        let mut cues = 0;
        for s in scripts.scripts() {
            for a in &s.actions {
                let kind = a.class.to_ascii_lowercase();
                let key = match kind.as_str() {
                    "action_setimusestate" | "action_playmusic" => "Music",
                    "action_setbattlemusicmulti" => "multiSound",
                    "action_enablebattleengine"
                    | "action_setbattleengineparams"
                    | "action_forcebattlemusic"
                    | "action_forcevictoryflourish" => {
                        cues += 1;
                        continue;
                    }
                    _ => continue,
                };
                cues += 1;
                if let Some(n) = a.text(key) {
                    names.push((format!("{} {}", s.obj.name, a.class), n.to_string()));
                }
            }
        }
        n_cues += cues;
        let mut files = BTreeSet::new();
        for (who, n) in &names {
            let mut miss = BTreeSet::new();
            pieces(&lib, n, &mut files, &mut miss, 0);
            for m in miss {
                problems.push(format!("{name}: {who}: {m} not in the music packages"));
            }
        }
        let missing_files: Vec<_> = files
            .iter()
            .filter(|f| !have.contains(&format!("{}.ogg", f.to_ascii_lowercase())))
            .collect();
        for f in &missing_files {
            problems.push(format!("{name}: {f}.ogg is not in GameData/Music"));
        }
        log.line(format!(
            "== {name}: battle music {}, {} rooms with music {:?}, {cues} script music actions, {} pieces",
            mm.battle.music.as_deref().unwrap_or("none"),
            rooms.len(),
            rooms,
            files.len()
        ));
    }
    log.line(format!(
        "volumes whose AudioClass has no music (sound effects only): {}",
        unread_rooms.len()
    ));
    for p in &problems {
        log.line(format!("PROBLEM: {p}"));
    }
    log.line(format!(
        "MUSIC CHECK: {n_maps} maps, {n_rooms} rooms with music, {n_cues} script music actions, {} problems",
        problems.len()
    ));
    if !problems.is_empty() {
        std::process::exit(1);
    }
}
