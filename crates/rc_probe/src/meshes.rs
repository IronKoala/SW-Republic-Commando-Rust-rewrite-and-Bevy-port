//! `rc_probe --meshes`: reads every `StaticMesh` object and runs its checks
//! (`StaticMesh::check` in rc_package), one summary line per package.

use crate::{find_packages, Log};
use rc_package::static_mesh;
use rc_package::Package;
use std::fs;
use std::path::Path;
use std::time::Instant;

/// How many individual mesh lines to log per package.
const DETAIL_LINES: usize = 3;

pub(crate) fn scan(log: &mut Log, target: &Path) -> bool {
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
    let (mut vertices, mut triangles) = (0usize, 0usize);
    let mut with_zero_normals = Vec::new();
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
        let (mut n, mut details) = (0usize, 0usize);
        let (pkg_ok, pkg_problems, pkg_failed) = (ok, with_problems, failed);
        for e in &pkg.exports {
            if pkg.export_class_name(e) != "StaticMesh" {
                continue;
            }
            n += 1;
            let name = pkg.name(e.object_name);
            match static_mesh::read_static_mesh(&pkg, &data, e) {
                Ok(m) => {
                    vertices += m.vertices.len();
                    triangles += m.triangle_count();
                    if m.zero_normals() > 0 {
                        with_zero_normals.push(format!("{name} ({})", m.zero_normals()));
                    }
                    let problems = m.check();
                    if problems.is_empty() {
                        ok += 1;
                        if details < DETAIL_LINES {
                            details += 1;
                            let size: Vec<String> = (0..3)
                                .map(|k| {
                                    format!("{:.1}", m.bounding_box.max[k] - m.bounding_box.min[k])
                                })
                                .collect();
                            log.line(format!(
                                "  {name}: {} vertices, {} triangles, {} sections, size {}",
                                m.vertices.len(),
                                m.triangle_count(),
                                m.sections.len(),
                                size.join(" x ")
                            ));
                        }
                    } else {
                        with_problems += 1;
                        log.line(format!("  PROBLEM {name}: {}", problems.join("; ")));
                    }
                }
                Err(err) => {
                    failed += 1;
                    log.line(format!("  FAILED {name}: {err}"));
                }
            }
        }
        if n > 0 {
            log.line(format!(
                "{shown}: {n} static meshes, {} ok, {} with problems, {} failed to read",
                ok - pkg_ok,
                with_problems - pkg_problems,
                failed - pkg_failed
            ));
        }
    }
    log.line("--- summary ---");
    log.line(format!(
        "static meshes: {} ok, {with_problems} with problems, {failed} failed to read; {bad_packages} packages unreadable",
        ok
    ));
    log.line(format!(
        "meshes with some zero-length normals (allowed): {}{}",
        with_zero_normals.len(),
        if with_zero_normals.is_empty() {
            String::new()
        } else {
            format!(": {}", with_zero_normals.join(", "))
        }
    ));
    log.line(format!(
        "total {vertices} vertices, {triangles} triangles, {:.0} ms",
        t.elapsed().as_secs_f64() * 1000.0
    ));
    with_problems == 0 && failed == 0 && bad_packages == 0
}
