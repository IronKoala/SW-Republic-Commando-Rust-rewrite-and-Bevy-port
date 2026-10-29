//! `rc_probe --textures`: reads every `Texture` object, checks its mipmaps,
//! decodes its pixels and checks them (see `pixels.rs`). With an export
//! folder, also writes each texture's full-size picture as a PNG.

use crate::pixels::{self, Outcome};
use crate::{find_packages, Log};
use rc_package::texture::{self, format_name};
use rc_package::Package;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Instant;

/// How many individual texture lines to log per package.
const DETAIL_LINES: usize = 5;

/// Mip 0 vs. shrunk mip 1 differences above this (0-255 scale) are logged.
const SUSPICIOUS_DIFFERENCE: f64 = 20.0;

#[derive(Default)]
struct PixelStats {
    decoded: usize,
    skipped: BTreeMap<String, usize>,
    failed: usize,
    /// Mip-difference buckets: under 2, 2-5, 5-10, 10-20, 20 and over.
    buckets: [usize; 5],
    no_mip1: usize,
    written: usize,
}

pub(crate) fn scan(log: &mut Log, target: &Path, export_dir: Option<&Path>) -> bool {
    let mut files = Vec::new();
    if target.is_dir() {
        find_packages(target, &mut files);
        files.sort();
    } else {
        files.push(target.to_path_buf());
    }
    log.line(format!("found {} package files", files.len()));

    let t = Instant::now();
    let (mut ok, mut with_problems, mut failed, mut bad_packages) =
        (0usize, 0usize, 0usize, 0usize);
    let mut formats: BTreeMap<String, usize> = BTreeMap::new();
    let mut sizes: BTreeMap<(i32, i32), usize> = BTreeMap::new();
    let mut pixel_bytes = 0usize;
    let mut px = PixelStats::default();
    for path in &files {
        let shown = path
            .strip_prefix(target)
            .ok()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(path)
            .display()
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
        let textures: Vec<_> = pkg
            .exports
            .iter()
            .filter(|e| pkg.export_class_name(e) == "Texture")
            .collect();
        if textures.is_empty() {
            continue;
        }
        let (mut p_ok, mut p_problems, mut p_failed, mut shown_lines) = (0, 0, 0, 0);
        for e in textures {
            let name = pkg.name(e.object_name);
            match texture::read_texture(&pkg, &data, e) {
                Ok(tex) => {
                    let problems = tex.check(&pkg);
                    *formats
                        .entry(format!("{:>2} {}", tex.format, format_name(tex.format)))
                        .or_default() += 1;
                    if let Some(m) = tex.mips.first() {
                        *sizes.entry((m.width, m.height)).or_default() += 1;
                    }
                    pixel_bytes += tex.mips.iter().map(|m| m.data_len).sum::<usize>();
                    if !tex.mips.is_empty() {
                        match pixels::decode(&pkg, &data, &tex) {
                            Outcome::Decoded(d) => {
                                px.decoded += 1;
                                match d.mip_difference {
                                    None => px.no_mip1 += 1,
                                    Some(diff) => {
                                        let bucket = match diff {
                                            x if x < 2.0 => 0,
                                            x if x < 5.0 => 1,
                                            x if x < 10.0 => 2,
                                            x if x < 20.0 => 3,
                                            _ => 4,
                                        };
                                        px.buckets[bucket] += 1;
                                        if diff >= SUSPICIOUS_DIFFERENCE {
                                            log.line(format!(
                                                "  PIXELS {name} ({}): mip 1 differs from shrunk mip 0 by {diff:.1}",
                                                format_name(tex.format)
                                            ));
                                        }
                                    }
                                }
                                if let Some(dir) = export_dir {
                                    let stem = path
                                        .file_stem()
                                        .map(|s| s.to_string_lossy().to_string())
                                        .unwrap_or_default();
                                    let folder = dir.join(stem);
                                    let file = folder.join(format!("{name}.png"));
                                    let written = fs::create_dir_all(&folder).and_then(|_| {
                                        fs::write(&file, rc_image::png::encode(&d.mip0))
                                    });
                                    match written {
                                        Ok(()) => px.written += 1,
                                        Err(e) => log.line(format!(
                                            "  FAILED writing {}: {e}",
                                            file.display()
                                        )),
                                    }
                                }
                                if shown_lines < DETAIL_LINES {
                                    let [r, g, b, a] = d.mip0.mean();
                                    log.line(format!(
                                        "          {name}: average colour R {r:.0} G {g:.0} B {b:.0} A {a:.0}, mip difference {}",
                                        d.mip_difference.map_or("n/a".into(), |x| format!("{x:.1}"))
                                    ));
                                }
                            }
                            Outcome::Skipped(reason) => *px.skipped.entry(reason).or_default() += 1,
                            Outcome::Failed(err) => {
                                px.failed += 1;
                                log.line(format!("  FAILED {name}: decoding pixels: {err}"));
                            }
                        }
                    }
                    if problems.is_empty() {
                        p_ok += 1;
                        if shown_lines < DETAIL_LINES {
                            shown_lines += 1;
                            let m = &tex.mips[0];
                            log.line(format!(
                                "          {name}: {}x{} {} {} mips, mip 0 at offset {} ({} bytes)",
                                m.width,
                                m.height,
                                format_name(tex.format),
                                tex.mips.len(),
                                m.data_offset,
                                m.data_len
                            ));
                        }
                    } else {
                        p_problems += 1;
                        log.line(format!(
                            "  CHECK  {name} ({}): {}",
                            format_name(tex.format),
                            problems.join("; ")
                        ));
                    }
                }
                Err(err) => {
                    p_failed += 1;
                    log.line(format!("  FAILED {name}: {err}"));
                }
            }
        }
        log.line(format!(
            "{shown}: {p_ok} textures OK, {p_problems} with problems, {p_failed} failed"
        ));
        ok += p_ok;
        with_problems += p_problems;
        failed += p_failed;
    }

    log.line("--- texture formats ---");
    for (f, n) in &formats {
        log.line(format!("  {n:>6}  {f}"));
    }
    let mut sizes: Vec<_> = sizes.into_iter().collect();
    sizes.sort_by_key(|s| std::cmp::Reverse(s.1));
    log.line("--- most common sizes ---");
    for ((w, h), n) in sizes.iter().take(8) {
        log.line(format!("  {n:>6}  {w}x{h}"));
    }
    log.line("--- pixel decoding ---");
    log.line(format!("decoded: {}  failed: {}", px.decoded, px.failed));
    for (reason, n) in &px.skipped {
        log.line(format!("  skipped {n:>5}: {reason}"));
    }
    log.line("mip 1 vs. mip 0 shrunk to half, average difference per channel (0-255):");
    for (label, n) in ["under 2", "2 to 5", "5 to 10", "10 to 20", "20 or more"]
        .iter()
        .zip(px.buckets)
    {
        log.line(format!("  {label:>10}: {n}"));
    }
    log.line(format!("  no usable mip 1: {}", px.no_mip1));
    if let Some(dir) = export_dir {
        log.line(format!(
            "PNG files written: {} to {}",
            px.written,
            dir.display()
        ));
    }
    log.line("--- texture summary ---");
    log.line(format!(
        "textures: {}  OK: {ok}  with problems: {with_problems}  failed: {failed}  packages that failed to open: {bad_packages}",
        ok + with_problems + failed
    ));
    log.line(format!(
        "pixel data: {} MB, time: {:.0} ms",
        pixel_bytes / (1024 * 1024),
        t.elapsed().as_secs_f64() * 1000.0
    ));
    with_problems == 0 && failed == 0 && bad_packages == 0 && px.failed == 0 && ok > 0
}
