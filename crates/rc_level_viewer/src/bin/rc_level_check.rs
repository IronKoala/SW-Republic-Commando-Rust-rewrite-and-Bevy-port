//! rc_level_check: tries to load every map in the game, the same way the
//! level viewer does, without opening a window, and logs what worked and
//! what didn't for each one.
//!
//! Usage:
//!   rc_level_check [map names...]
//!
//! With no names it tries every `.ctm` file in `GameData/Maps` (from
//! RC_GAME_DIR or the default Steam path). Writes `logs/rc_level_check.log`
//! and ends with one line per map and a total. Exits with 1 if any map
//! failed to load.

use rc_level_viewer::materials::Materials;
use rc_level_viewer::scene;
use rc_package::Package;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";
const LOG_PATH: &str = "logs/rc_level_check.log";

struct Log(fs::File);

impl Log {
    fn line(&mut self, s: impl AsRef<str>) {
        println!("{}", s.as_ref());
        let _ = writeln!(self.0, "{}", s.as_ref());
    }
}

/// One map's result.
struct Row {
    name: String,
    ok: bool,
    summary: String,
}

fn check_map(path: &PathBuf, mats: &mut Materials, log: &mut Log) -> Row {
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let t = Instant::now();
    let fail = |summary: String| Row {
        name: name.clone(),
        ok: false,
        summary,
    };
    let data = match fs::read(path) {
        Ok(d) => d,
        Err(e) => return fail(format!("can't read the file: {e}")),
    };
    let pkg = match Package::parse(&data) {
        Ok(p) => p,
        Err(e) => return fail(format!("package not readable: {e}")),
    };
    let map = Rc::new((pkg, data));
    // A bug in a reader should show up as one failed map, not end the run.
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scene::build(map, mats)));
    let scene = match built {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return fail(format!("level not built: {e}")),
        Err(p) => {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            return fail(format!("CRASHED while building: {msg}"));
        }
    };
    let s = &scene.stats;
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    let mesh_fail: usize = s.mesh_failures.values().sum();
    let mat_fail: usize = s.material_failures.values().sum();
    log.line(format!("== {name}"));
    log.line(format!(
        "  BSP {} polygons ({} triangles), {} check problems; lightmaps {} on {} pages{}",
        s.bsp_polygons,
        s.bsp_triangles,
        s.bsp_checks.len(),
        s.lightmap_entries,
        s.lightmap_pages,
        match &s.lightmap_error {
            Some(e) => format!(", NOT READ: {e}"),
            None => String::new(),
        }
    ));
    for c in s.bsp_checks.iter().take(5) {
        log.line(format!("  BSP check problem: {c}"));
    }
    for c in s.lightmap_checks.iter().take(5) {
        log.line(format!("  lightmap check problem: {c}"));
    }
    log.line(format!(
        "  actors {} read, {} failed; props {} with a mesh, {} drawn, {} hidden, {} not shown; {} prop triangles",
        s.actors,
        s.actor_failures,
        s.mesh_actors,
        s.mesh_actors_drawn,
        s.hidden_actors,
        mesh_fail,
        s.mesh_triangles
    ));
    for (e, n) in &s.mesh_failures {
        log.line(format!("  mesh not shown ({n}x): {e}"));
    }
    log.line(format!(
        "  materials {} shown, {} not",
        s.materials_ok, mat_fail
    ));
    for (e, n) in &s.material_failures {
        log.line(format!("  material not shown ({n}x): {e}"));
    }
    log.line(format!(
        "  blocking volumes {} ({} stop players, {} stop the AI, {} unreadable, {} scaled); {} path points inside one, {} volumes near a path point",
        s.blocking_volumes,
        s.volumes_block_players,
        s.volumes_block_actors,
        s.volume_failures.len(),
        s.volumes_scaled,
        s.path_points_in_volumes,
        s.volumes_near_paths
    ));
    for f in &s.volume_failures {
        log.line(format!("  volume not read: {f}"));
    }
    let f = s.floor_test;
    log.line(format!(
        "  floor test: {} of {} path points have a floor 0-150 below, {} closer than 80; collision triangles {}; spawn {}; built in {ms:.0} ms",
        f.near, f.tested, f.too_close, s.collision_triangles, scene.spawn.from
    ));
    let mut notes = Vec::new();
    if s.actor_failures > 0 {
        notes.push(format!("{} actors unreadable", s.actor_failures));
    }
    if mesh_fail > 0 {
        notes.push(format!("{mesh_fail} props missing"));
    }
    if mat_fail > 0 {
        notes.push(format!("{mat_fail} materials missing"));
    }
    if !s.volume_failures.is_empty() {
        notes.push(format!("{} volumes unreadable", s.volume_failures.len()));
    }
    if s.path_points_in_volumes > 0 {
        notes.push(format!(
            "{} path points in volumes",
            s.path_points_in_volumes
        ));
    }
    if s.lightmap_error.is_some() {
        notes.push("lightmaps unreadable".into());
    }
    if !s.bsp_checks.is_empty() {
        notes.push(format!("{} BSP problems", s.bsp_checks.len()));
    }
    if !s.lightmap_checks.is_empty() {
        notes.push(format!("{} lightmap problems", s.lightmap_checks.len()));
    }
    Row {
        name,
        ok: true,
        summary: format!(
            "{} BSP polys, {} props, {} volumes, {} triangles drawn, {ms:.0} ms{}",
            s.bsp_polygons,
            s.mesh_actors_drawn,
            s.blocking_volumes,
            s.bsp_triangles + s.mesh_triangles,
            if notes.is_empty() {
                String::new()
            } else {
                format!("; {}", notes.join(", "))
            }
        ),
    }
}

fn main() {
    let game = std::env::var_os("RC_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_DIR));
    let game_data = game.join("GameData");
    let maps_dir = game_data.join("Maps");
    let names: Vec<String> = std::env::args().skip(1).collect();
    let mut maps: Vec<PathBuf> = if names.is_empty() {
        fs::read_dir(&maps_dir)
            .map(|d| {
                d.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ctm")))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        names
            .iter()
            .map(|n| maps_dir.join(format!("{n}.ctm")))
            .collect()
    };
    maps.sort();

    // Rule: never write inside the game install.
    let cwd = std::env::current_dir().expect("can't read the current folder");
    if let (Ok(c), Ok(g)) = (cwd.canonicalize(), game.canonicalize()) {
        if c.starts_with(&g) {
            eprintln!(
                "Refusing to run from inside the game folder ({}).",
                g.display()
            );
            std::process::exit(2);
        }
    }
    fs::create_dir_all("logs").expect("can't create the logs folder");
    let mut log = Log(fs::File::create(LOG_PATH).expect("can't create the log file"));
    log.line(format!(
        "rc_level_check {}: {} maps in {}",
        env!("CARGO_PKG_VERSION"),
        maps.len(),
        maps_dir.display()
    ));
    // Keep the texture and mesh packages open between maps: many maps share
    // them, and reading them once saves most of the time.
    let mut mats = Materials::new(&game_data);
    let t = Instant::now();
    let rows: Vec<Row> = maps
        .iter()
        .map(|m| check_map(m, &mut mats, &mut log))
        .collect();
    log.line("");
    log.line("SUMMARY");
    for r in &rows {
        log.line(format!(
            "  {:<18} {}  {}",
            r.name,
            if r.ok { "LOADED" } else { "FAILED" },
            r.summary
        ));
    }
    let ok = rows.iter().filter(|r| r.ok).count();
    log.line(format!(
        "{ok} of {} maps loaded, {} failed, in {:.1} s",
        rows.len(),
        rows.len() - ok,
        t.elapsed().as_secs_f64()
    ));
    if ok != rows.len() {
        std::process::exit(1);
    }
}
