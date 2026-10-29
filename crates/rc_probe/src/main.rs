//! rc_probe: proves we can read Republic Commando's data from the installed game.
//!
//! Opens game packages READ-ONLY, parses their header and tables, and writes
//! what it found to the screen and to `logs/rc_probe.log` in the folder you run
//! it from. It never writes anything inside the game folder.
//!
//! Usage:
//!   rc_probe                 open GameData/System/Core.u in the game folder
//!   rc_probe <file>          open one package and log its details
//!   rc_probe <folder>        open every package under that folder, one line each
//!   rc_probe --textures [path]  read every texture in a package or folder
//!                            (default: the whole GameData folder), decode and
//!                            check its pixels, log to `logs/rc_textures.log`
//!   rc_probe --meshes [path]  read every static mesh (3D model) in a package
//!                            or folder (default: GameData/StaticMeshes) and
//!                            check it, log to `logs/rc_meshes.log`
//!   rc_probe --export-textures <path>  the same, and also write each texture
//!                            as a PNG to `extracted/textures/<package>/`
//!   rc_probe --characters [path]  read every character mesh and animation in
//!                            a package or folder (default: GameData/Animations),
//!                            play the animations and measure them, log to
//!                            `logs/rc_characters.log`
//!
//! The game folder comes from the RC_GAME_DIR environment variable, or the
//! default Steam path below. (Env-var idea from muffinmxn/gang-beasts-rust.)

mod characters;
mod meshes;
mod pixels;
mod textures;

use rc_package::{ObjectRef, Package};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";
const PACKAGE_EXTENSIONS: &[&str] = &["u", "ctm", "utx", "uax", "usx", "ukx"];
const LOG_PATH: &str = "logs/rc_probe.log";
const TEXTURE_LOG_PATH: &str = "logs/rc_textures.log";
const MESH_LOG_PATH: &str = "logs/rc_meshes.log";
const CHARACTER_LOG_PATH: &str = "logs/rc_characters.log";
/// Gitignored folder for anything extracted from the game.
const EXTRACT_DIR: &str = "extracted/textures";

pub(crate) struct Log {
    file: fs::File,
}

impl Log {
    pub(crate) fn line(&mut self, s: impl AsRef<str>) {
        let s = s.as_ref();
        println!("{s}");
        let _ = writeln!(self.file, "{s}");
    }
}

fn game_dir() -> PathBuf {
    std::env::var_os("RC_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_DIR))
}

fn main() {
    let game = game_dir();
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    let export_mode = args.first().is_some_and(|a| a == "--export-textures");
    let texture_mode = export_mode || args.first().is_some_and(|a| a == "--textures");
    let mesh_mode = args.first().is_some_and(|a| a == "--meshes");
    let character_mode = args.first().is_some_and(|a| a == "--characters");
    if texture_mode || mesh_mode || character_mode {
        args.remove(0);
    }
    if export_mode && args.is_empty() {
        eprintln!("--export-textures needs a package file or folder, for example:");
        eprintln!(
            r#"  rc_probe --export-textures "{}\GameData\Textures\weapontextures.utx""#,
            game.display()
        );
        std::process::exit(2);
    }
    let target = match args.first() {
        Some(arg) => PathBuf::from(arg),
        None if texture_mode => game.join("GameData"),
        None if mesh_mode => game.join("GameData").join("StaticMeshes"),
        None if character_mode => game.join("GameData").join("Animations"),
        None => game.join("GameData").join("System").join("Core.u"),
    };
    let log_path = if texture_mode {
        TEXTURE_LOG_PATH
    } else if mesh_mode {
        MESH_LOG_PATH
    } else if character_mode {
        CHARACTER_LOG_PATH
    } else {
        LOG_PATH
    };

    // Rule: never write inside the game install. Refuse to run from in there.
    let cwd = std::env::current_dir().expect("can't read the current folder");
    if let (Ok(c), Ok(g)) = (cwd.canonicalize(), game.canonicalize()) {
        if c.starts_with(&g) {
            eprintln!("Refusing to run from inside the game folder ({}), because the log would be written there.", g.display());
            eprintln!("Run it from the project folder instead.");
            std::process::exit(2);
        }
    }

    fs::create_dir_all("logs").expect("can't create the logs folder");
    let file = fs::File::create(log_path).expect("can't create the log file");
    let mut log = Log { file };
    log.line(format!("rc_probe {}", env!("CARGO_PKG_VERSION")));
    log.line(format!("game folder: {}", game.display()));
    log.line(format!("target:      {}", target.display()));

    let ok = if texture_mode {
        let export_dir = export_mode.then(|| Path::new(EXTRACT_DIR));
        textures::scan(&mut log, &target, export_dir)
    } else if mesh_mode {
        meshes::scan(&mut log, &target)
    } else if character_mode {
        characters::scan(&mut log, &target)
    } else if target.is_dir() {
        scan_folder(&mut log, &target)
    } else {
        probe_one(&mut log, &target)
    };
    log.line(format!("log written to {log_path}"));
    std::process::exit(if ok { 0 } else { 1 });
}

fn read_package(path: &Path) -> Result<(Package, f64), String> {
    let t = Instant::now();
    let data = fs::read(path).map_err(|e| format!("can't read file: {e}"))?;
    let pkg = Package::parse(&data).map_err(|e| e.to_string())?;
    Ok((pkg, t.elapsed().as_secs_f64() * 1000.0))
}

fn probe_one(log: &mut Log, path: &Path) -> bool {
    let (pkg, ms) = match read_package(path) {
        Ok(v) => v,
        Err(e) => {
            log.line(format!("FAILED: {e}"));
            return false;
        }
    };
    let s = &pkg.summary;
    log.line(format!(
        "file size: {} bytes, parsed in {ms:.2} ms",
        pkg.file_size
    ));
    log.line("--- header ---");
    log.line(format!("tag:              {:#010X}", s.tag));
    log.line(format!("package version:  {}", s.version));
    log.line(format!("licensee version: {}", s.licensee_version));
    log.line(format!(
        "looks like Republic Commando: {}",
        s.looks_like_republic_commando()
    ));
    log.line(format!("package flags:    {:#010X}", s.package_flags));
    log.line(format!(
        "names:   {:>6} at offset {:>9} (table ends at {})",
        s.name_count, s.name_offset, pkg.name_table_end
    ));
    log.line(format!(
        "imports: {:>6} at offset {:>9} (table ends at {})",
        s.import_count, s.import_offset, pkg.import_table_end
    ));
    log.line(format!(
        "exports: {:>6} at offset {:>9} (table ends at {})",
        s.export_count, s.export_offset, pkg.export_table_end
    ));
    log.line(format!(
        "guid: {}",
        s.guid
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<String>()
    ));
    log.line(format!("generations: {}", s.generations.len()));
    for (i, g) in s.generations.iter().enumerate() {
        log.line(format!(
            "  gen {i}: {} exports, {} names",
            g.export_count, g.name_count
        ));
    }

    log.line("--- first 20 names ---");
    for (i, n) in pkg.names.iter().take(20).enumerate() {
        log.line(format!("  [{i:>4}] {:<32} flags {:#010X}", n.name, n.flags));
    }

    log.line("--- first 10 imports (ClassPackage.ClassName ObjectName in Package) ---");
    for (i, m) in pkg.imports.iter().take(10).enumerate() {
        log.line(format!(
            "  [{i:>4}] {}.{} {} in {}",
            pkg.name(m.class_package),
            pkg.name(m.class_name),
            pkg.name(m.object_name),
            pkg.object_name(m.package_index)
        ));
    }

    log.line("--- first 10 exports (Class ObjectName, data offset+size) ---");
    for (i, e) in pkg.exports.iter().take(10).enumerate() {
        log.line(format!(
            "  [{i:>4}] {} {}  offset {} size {}  flags {:#010X}{}",
            pkg.export_class_name(e),
            pkg.name(e.object_name),
            e.serial_offset,
            e.serial_size,
            e.object_flags,
            e.rc_unknown
                .map(|u| format!("  rc_unknown {u}"))
                .unwrap_or_default()
        ));
    }

    let mut by_class: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &pkg.exports {
        *by_class.entry(pkg.export_class_name(e)).or_default() += 1;
    }
    let mut by_class: Vec<_> = by_class.into_iter().collect();
    by_class.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    log.line(format!(
        "--- exports by class ({} classes, top 15) ---",
        by_class.len()
    ));
    for (class, n) in by_class.iter().take(15) {
        log.line(format!("  {n:>6}  {class}"));
    }

    let imports_from_self = pkg
        .exports
        .iter()
        .filter(|e| matches!(ObjectRef::from_index(e.class_index), ObjectRef::Export(_)))
        .count();
    log.line(format!(
        "exports whose class is defined in this same package: {imports_from_self}"
    ));

    report_checks(log, &pkg)
}

fn report_checks(log: &mut Log, pkg: &Package) -> bool {
    let problems = pkg.check();
    if problems.is_empty() {
        log.line("--- checks: all passed ---");
        true
    } else {
        log.line(format!(
            "--- checks: {} problems (first 20 shown) ---",
            problems.len()
        ));
        for p in problems.iter().take(20) {
            log.line(format!("  {p}"));
        }
        false
    }
}

pub(crate) fn find_packages(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_packages(&path, out);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if PACKAGE_EXTENSIONS
                .iter()
                .any(|p| p.eq_ignore_ascii_case(ext))
            {
                out.push(path);
            }
        }
    }
}

fn scan_folder(log: &mut Log, dir: &Path) -> bool {
    let mut files = Vec::new();
    find_packages(dir, &mut files);
    files.sort();
    log.line(format!("found {} package files", files.len()));
    log.line("result  version/licensee  names imports exports  ms  file");

    let t = Instant::now();
    let (mut ok, mut failed, mut with_problems) = (0, 0, 0);
    let mut versions: BTreeMap<(u16, u16), usize> = BTreeMap::new();
    for path in &files {
        let shown = path.strip_prefix(dir).unwrap_or(path).display();
        match read_package(path) {
            Ok((pkg, ms)) => {
                let s = &pkg.summary;
                *versions.entry((s.version, s.licensee_version)).or_default() += 1;
                let problems = pkg.check();
                let result = if problems.is_empty() {
                    ok += 1;
                    "OK    "
                } else {
                    with_problems += 1;
                    "CHECK "
                };
                log.line(format!(
                    "{result}  {:>3}/{:<3}  {:>6} {:>6} {:>6}  {ms:>7.2}  {shown}",
                    s.version, s.licensee_version, s.name_count, s.import_count, s.export_count
                ));
                for p in problems.iter().take(3) {
                    log.line(format!("          problem: {p}"));
                }
            }
            Err(e) => {
                failed += 1;
                log.line(format!("FAILED  {shown}: {e}"));
            }
        }
    }
    log.line("--- summary ---");
    log.line(format!("packages: {}  parsed and passed checks: {ok}  parsed with problems: {with_problems}  failed to parse: {failed}", files.len()));
    log.line(format!(
        "total time: {:.0} ms",
        t.elapsed().as_secs_f64() * 1000.0
    ));
    for ((v, l), n) in versions {
        log.line(format!("  version {v} licensee {l}: {n} files"));
    }
    failed == 0 && with_problems == 0 && !files.is_empty()
}
