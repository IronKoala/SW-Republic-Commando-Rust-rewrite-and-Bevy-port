//! `rc_probe --sounds`: reads every sound object, decodes the embedded `.wav`
//! files and measures them (length, peak, loudness, checksum). With an export
//! folder, also writes each sound as a `.wav` file to `extracted/sounds/`.

use crate::{find_packages, Log};
use rc_audio::Wav;
use rc_package::properties::Value;
use rc_package::sound;
use rc_package::Package;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::Instant;

/// How many individual sound lines to show on screen per package. The log
/// file gets every line.
const DETAIL_LINES: usize = 5;

#[derive(Default)]
struct Totals {
    sounds: usize,
    decoded: usize,
    failed: usize,
    with_problems: usize,
    silent: usize,
    seconds: f64,
    written: usize,
    /// (format tag, channels, sample rate, bits) -> count
    formats: BTreeMap<(u16, u16, u32, u16), usize>,
    file_types: BTreeMap<String, usize>,
    streamed: usize,
    streamed_found: usize,
    streamed_failed: usize,
    multiple: usize,
    multiple_problems: usize,
}

pub(crate) fn scan(
    log: &mut Log,
    target: &Path,
    game_data: &Path,
    export_dir: Option<&Path>,
) -> bool {
    let mut files = Vec::new();
    if target.is_dir() {
        find_packages(target, &mut files);
        files.sort();
    } else {
        files.push(target.to_path_buf());
    }
    log.line(format!("found {} package files", files.len()));
    let music = list_music(&game_data.join("Music"));
    log.line(format!(
        "music folder: {} .ogg files in {}",
        music.len(),
        game_data.join("Music").display()
    ));

    let t = Instant::now();
    let mut tot = Totals::default();
    let mut bad_packages = 0;
    for path in &files {
        let shown = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let data = match fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                bad_packages += 1;
                log.line(format!("FAILED  {shown}: can't read file: {e}"));
                continue;
            }
        };
        let pkg = match Package::parse(&data) {
            Ok(p) => p,
            Err(e) => {
                bad_packages += 1;
                log.line(format!("FAILED  {shown}: {e}"));
                continue;
            }
        };
        let has_sounds = pkg.exports.iter().any(|e| {
            matches!(
                pkg.export_class_name(e),
                "Sound" | "SoundStreamed" | "SoundMultiple"
            )
        });
        if !has_sounds {
            continue;
        }
        log.line(format!("--- {shown} ---"));
        let package_name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let mut shown_lines = 0;
        for e in &pkg.exports {
            let name = pkg.name(e.object_name);
            match pkg.export_class_name(e) {
                "Sound" => {
                    tot.sounds += 1;
                    let line = match sound::read_sound(&pkg, &data, e) {
                        Err(err) => {
                            tot.failed += 1;
                            format!("  FAILED   Sound {name}: {err}")
                        }
                        Ok(s) => {
                            *tot.file_types.entry(s.file_type.clone()).or_default() += 1;
                            let mut problems = s.check();
                            let bytes = s.bytes(&data);
                            match Wav::parse(bytes) {
                                Err(err) => {
                                    tot.failed += 1;
                                    format!("  FAILED   Sound {name} ({}): {err}", s.file_type)
                                }
                                Ok(w) => {
                                    tot.decoded += 1;
                                    let st = w.stats();
                                    *tot.formats
                                        .entry((
                                            w.format_tag,
                                            w.channels,
                                            w.sample_rate,
                                            w.bits_per_sample,
                                        ))
                                        .or_default() += 1;
                                    tot.seconds += w.seconds();
                                    if st.peak == 0.0 {
                                        tot.silent += 1;
                                    }
                                    if w.trailing_bytes != 0 {
                                        problems.push(format!(
                                            "{} bytes after the WAV chunks",
                                            w.trailing_bytes
                                        ));
                                    }
                                    if let Some(dir) = export_dir {
                                        let out =
                                            dir.join(&package_name).join(format!("{name}.wav"));
                                        if fs::create_dir_all(out.parent().unwrap())
                                            .and_then(|_| fs::write(&out, bytes))
                                            .is_ok()
                                        {
                                            tot.written += 1;
                                        }
                                    }
                                    let result = if problems.is_empty() {
                                        "OK      "
                                    } else {
                                        tot.with_problems += 1;
                                        "CHECK   "
                                    };
                                    let mut l = format!(
                                        "  {result} Sound {name}: {} Hz, {} ch, {} bit, {} frames, {:.3} s, peak {:.3} ({:.1} dB), rms {:.1} dB, clipped {}, checksum {:016x}",
                                        w.sample_rate,
                                        w.channels,
                                        w.bits_per_sample,
                                        w.frames(),
                                        w.seconds(),
                                        st.peak,
                                        st.peak_db(),
                                        st.rms_db(),
                                        st.clipped,
                                        st.checksum
                                    );
                                    for p in problems {
                                        l.push_str(&format!("\n           problem: {p}"));
                                    }
                                    l
                                }
                            }
                        }
                    };
                    let important = !line.starts_with("  OK");
                    if important || shown_lines < DETAIL_LINES {
                        log.line(line);
                        shown_lines += 1;
                    } else {
                        log.file_only(line);
                    }
                }
                "SoundStreamed" => {
                    tot.streamed += 1;
                    match sound::read_streamed(&pkg, &data, e) {
                        Err(err) => {
                            tot.streamed_failed += 1;
                            log.line(format!("  FAILED   SoundStreamed {name}: {err}"))
                        }
                        Ok(s) => {
                            let found = music.contains(
                                s.file_name.to_ascii_lowercase().trim_end_matches(".ogg"),
                            );
                            if found {
                                tot.streamed_found += 1;
                            }
                            let looping = matches!(
                                s.properties
                                    .iter()
                                    .find(|p| pkg.name(p.name) == "Looping")
                                    .map(|p| &p.value),
                                Some(Value::Bool(true))
                            );
                            log.file_only(format!(
                                "  STREAM   {name}: file \"{}\" looping {looping} leftover {}  {}",
                                s.file_name,
                                s.leftover,
                                if found {
                                    "found in Music/"
                                } else {
                                    "NOT in Music/"
                                }
                            ));
                        }
                    }
                }
                "SoundMultiple" => {
                    tot.multiple += 1;
                    match sound::read_multiple(&pkg, &data, e) {
                        Ok(m) => {
                            let bad_ref = m
                                .sounds
                                .iter()
                                .any(|&i| pkg.object_name(i).starts_with("<bad"));
                            if m.leftover != 0 || bad_ref {
                                tot.multiple_problems += 1;
                            }
                            if m.leftover != 0 || bad_ref || tot.multiple <= 3 {
                                let names: Vec<_> = m
                                    .sounds
                                    .iter()
                                    .take(4)
                                    .map(|&i| pkg.object_name(i))
                                    .collect();
                                log.line(format!(
                                    "  MULTI    {name}: {} sounds {names:?}..., second list {:?}, leftover {}",
                                    m.sounds.len(),
                                    m.extra,
                                    m.leftover
                                ));
                            }
                        }
                        Err(err) => {
                            tot.multiple_problems += 1;
                            log.line(format!("  FAILED   SoundMultiple {name}: {err}"));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    log.line("--- summary ---");
    log.line(format!(
        "Sound objects: {}  decoded: {}  with problems: {}  failed: {}  silent: {}",
        tot.sounds, tot.decoded, tot.with_problems, tot.failed, tot.silent
    ));
    log.line(format!(
        "total length of decoded sounds: {:.1} s",
        tot.seconds
    ));
    for (t, n) in &tot.file_types {
        log.line(format!("  file type {t}: {n}"));
    }
    for ((tag, ch, rate, bits), n) in &tot.formats {
        log.line(format!(
            "  format tag {tag}, {ch} ch, {rate} Hz, {bits} bit: {n}"
        ));
    }
    log.line(format!(
        "SoundStreamed objects: {}  read: {}  failed: {}  file found in Music/: {}",
        tot.streamed,
        tot.streamed - tot.streamed_failed,
        tot.streamed_failed,
        tot.streamed_found
    ));
    log.line(format!(
        "SoundMultiple objects: {}  with problems: {}",
        tot.multiple, tot.multiple_problems
    ));
    if export_dir.is_some() {
        log.line(format!("wav files written: {}", tot.written));
    }
    log.line(format!("bad packages: {bad_packages}"));
    log.line(format!(
        "total time: {:.0} ms",
        t.elapsed().as_secs_f64() * 1000.0
    ));
    bad_packages == 0
        && tot.failed == 0
        && tot.with_problems == 0
        && tot.streamed_failed == 0
        && tot.multiple_problems == 0
}

/// Lower-case names (without ".ogg") of the files in the Music folder.
fn list_music(dir: &Path) -> BTreeSet<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return BTreeSet::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let ext = p.extension()?.to_str()?.to_ascii_lowercase();
            (ext == "ogg").then(|| p.file_stem()?.to_str().map(|s| s.to_ascii_lowercase()))?
        })
        .collect()
}
