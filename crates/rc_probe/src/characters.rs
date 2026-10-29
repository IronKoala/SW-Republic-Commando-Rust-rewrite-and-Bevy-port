//! `rc_probe --characters`: reads every `SkeletalMesh` (characters) and every
//! `MeshAnimation`, checks them, and plays each mesh's animations on the CPU
//! to measure whether the result holds together (see `rc_anim::stretch`).

use crate::{find_packages, Log};
use rc_anim::{edges, globals, skin, stretch, weights_per_point, Clip, Convention, Skeleton};
use rc_package::animation::{read_mesh_animation, MeshAnimation};
use rc_package::skeletal::{bounds, read_skeletal_mesh, SkeletalMesh};
use rc_package::{ObjectRef, Package};
use std::fs;
use std::path::Path;
use std::time::Instant;

/// Frames sampled per sequence when measuring stretch.
const SAMPLES_PER_SEQUENCE: usize = 8;

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
    let mut totals = Totals::default();
    for path in &files {
        let shown = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let data = match fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                log.line(format!("FAILED  {shown}: can't read file: {e}"));
                totals.bad_packages += 1;
                continue;
            }
        };
        let pkg = match Package::parse(&data) {
            Ok(p) => p,
            Err(e) => {
                log.line(format!("FAILED  {shown}: {e}"));
                totals.bad_packages += 1;
                continue;
            }
        };
        scan_package(log, &shown, &pkg, &data, &mut totals);
    }
    log.line("--- character summary ---");
    log.line(format!(
        "skeletal meshes: {} read, {} with problems, {} failed",
        totals.meshes_ok, totals.meshes_problems, totals.meshes_failed
    ));
    log.line(format!(
        "animation sets:  {} read, {} with problems, {} failed; {} sequences, {} frames in total",
        totals.anims_ok,
        totals.anims_problems,
        totals.anims_failed,
        totals.sequences,
        totals.frames
    ));
    log.line(format!(
        "stretch test, sequences measured: {}; worst average stretch {:.4}",
        totals.measured, totals.worst_mean
    ));
    log.line(format!(
        "packages that failed to open: {}; time {:.0} ms",
        totals.bad_packages,
        t.elapsed().as_secs_f64() * 1000.0
    ));
    totals.meshes_failed == 0 && totals.anims_failed == 0 && totals.bad_packages == 0
}

#[derive(Default)]
struct Totals {
    meshes_ok: usize,
    meshes_problems: usize,
    meshes_failed: usize,
    anims_ok: usize,
    anims_problems: usize,
    anims_failed: usize,
    sequences: usize,
    frames: i64,
    measured: usize,
    worst_mean: f32,
    bad_packages: usize,
}

fn fmt3(v: [f32; 3]) -> String {
    format!("({:.1}, {:.1}, {:.1})", v[0], v[1], v[2])
}

fn scan_package(log: &mut Log, shown: &str, pkg: &Package, data: &[u8], totals: &mut Totals) {
    let mut anims: Vec<(usize, MeshAnimation)> = Vec::new();
    for (i, e) in pkg.exports.iter().enumerate() {
        if pkg.export_class_name(e) != "MeshAnimation" {
            continue;
        }
        let name = pkg.name(e.object_name);
        match read_mesh_animation(pkg, data, e) {
            Ok(a) => {
                let problems = a.check();
                let frames: i64 = a.sequences.iter().map(|s| s.num_frames as i64).sum();
                totals.sequences += a.sequences.len();
                totals.frames += frames;
                let first = a
                    .sequences
                    .first()
                    .map(|s| {
                        format!(
                            ", first \"{}\" {} frames at {} fps",
                            s.name, s.num_frames, s.rate
                        )
                    })
                    .unwrap_or_default();
                log.line(format!(
                    "{}  {shown} anim {name}: {} bones, {} sequences, {frames} frames{first}",
                    if problems.is_empty() {
                        "ok     "
                    } else {
                        "PROBLEM"
                    },
                    a.bones.len(),
                    a.sequences.len()
                ));
                for p in problems.iter().take(5) {
                    log.line(format!("          {p}"));
                }
                if problems.is_empty() {
                    totals.anims_ok += 1;
                } else {
                    totals.anims_problems += 1;
                }
                anims.push((i, a));
            }
            Err(err) => {
                totals.anims_failed += 1;
                log.line(format!("FAILED   {shown} anim {name}: {err}"));
            }
        }
    }
    for e in &pkg.exports {
        if pkg.export_class_name(e) != "SkeletalMesh" {
            continue;
        }
        let name = pkg.name(e.object_name);
        let mesh = match read_skeletal_mesh(pkg, data, e) {
            Ok(m) => m,
            Err(err) => {
                totals.meshes_failed += 1;
                log.line(format!("FAILED   {shown} mesh {name}: {err}"));
                continue;
            }
        };
        let problems = mesh.check();
        let (lo, hi) = mesh.points_bounds();
        log.line(format!(
            "{}  {shown} mesh {name}: {} bones, {} points, {} wedges, {} triangles, {} influences, {} materials, {} LODs, {} sockets, {} bytes not read",
            if problems.is_empty() { "ok     " } else { "PROBLEM" },
            mesh.bones.len(),
            mesh.points.len(),
            mesh.wedges.len(),
            mesh.triangles.len(),
            mesh.influences.len(),
            mesh.materials.len(),
            mesh.lods.len(),
            mesh.sockets.len(),
            mesh.leftover
        ));
        log.line(format!(
            "          points from {} to {}; stored box {} to {}; scale {} origin {} rotation {:?}",
            fmt3(lo),
            fmt3(hi),
            fmt3(mesh.bounding_box.min),
            fmt3(mesh.bounding_box.max),
            fmt3(mesh.mesh_scale),
            fmt3(mesh.mesh_origin),
            mesh.rot_origin
        ));
        for p in problems.iter().take(5) {
            log.line(format!("          {p}"));
        }
        if problems.is_empty() {
            totals.meshes_ok += 1;
        } else {
            totals.meshes_problems += 1;
        }
        bone_fit(log, &mesh);
        // Animations linked from the mesh, if they are in this package.
        for &link in &mesh.animations {
            let ObjectRef::Export(ai) = ObjectRef::from_index(link) else {
                log.line(format!(
                    "          linked animation {} is in another package, not tested",
                    pkg.object_name(link)
                ));
                continue;
            };
            if let Some((_, anim)) = anims.iter().find(|(i, _)| *i == ai) {
                measure(log, &mesh, pkg.object_name(link), anim, totals);
            }
        }
    }
}

/// In the reference pose, each bone should sit near the points it moves.
/// Logs the median distance from a bone to the middle of its points, as a
/// share of the mesh's height.
fn bone_fit(log: &mut Log, mesh: &SkeletalMesh) {
    let skel = Skeleton::from_mesh(mesh, Convention::InvertRoot);
    let (lo, hi) = mesh.points_bounds();
    let height = (hi[2] - lo[2])
        .max(hi[0] - lo[0])
        .max(hi[1] - lo[1])
        .max(1e-3);
    let mut sums = vec![([0f32; 3], 0usize); skel.global.len()];
    for inf in &mesh.influences {
        if inf.weight > 0.5 {
            if let (Some(s), Some(p)) = (
                sums.get_mut(inf.bone as usize),
                mesh.points.get(inf.point as usize),
            ) {
                for (acc, v) in s.0.iter_mut().zip(p) {
                    *acc += v;
                }
                s.1 += 1;
            }
        }
    }
    let mut d: Vec<f32> = sums
        .iter()
        .zip(&skel.global)
        .filter(|(s, _)| s.1 > 0)
        .map(|(s, g)| {
            let c = s.0.map(|v| v / s.1 as f32);
            rc_anim::math::length(rc_anim::math::sub(c, g.pos)) / height
        })
        .collect();
    d.sort_by(|a, b| a.total_cmp(b));
    if let Some(&m) = d.get(d.len() / 2) {
        let (blo, bhi) = bounds(&skel.global.iter().map(|g| g.pos).collect::<Vec<_>>());
        log.line(format!(
            "          bones from {} to {}; bone to its points: median {:.3} of mesh height ({} bones measured)",
            fmt3(blo),
            fmt3(bhi),
            m,
            d.len()
        ));
    }
}

/// Plays each sequence with each rotation convention and logs how much the
/// mesh stretches. The right convention should stretch far less.
fn measure(
    log: &mut Log,
    mesh: &SkeletalMesh,
    anim_name: &str,
    anim: &MeshAnimation,
    totals: &mut Totals,
) {
    let weights = weights_per_point(mesh);
    let edge_list = edges(mesh);
    for conv in Convention::ALL {
        let skel = Skeleton::from_mesh(mesh, conv);
        let mut mean = 0f32;
        let mut over = 0f32;
        let mut n = 0usize;
        let mut worst = (0f32, String::new());
        let mut matched = 0usize;
        for seq in &anim.sequences {
            let clip = Clip::new(anim, seq, &skel);
            matched = matched.max(clip.matched_bones);
            let mut seq_mean = 0f32;
            for k in 0..SAMPLES_PER_SEQUENCE {
                let frame = clip.num_frames * k as f32 / SAMPLES_PER_SEQUENCE as f32;
                let pose = globals(&skel.parents, &clip.sample(&skel, frame));
                let posed = skin(&mesh.points, &weights, &skel, &pose);
                let s = stretch(&mesh.points, &posed, &edge_list);
                seq_mean += s.mean;
                over += s.over_half;
                n += 1;
            }
            seq_mean /= SAMPLES_PER_SEQUENCE as f32;
            mean += seq_mean * SAMPLES_PER_SEQUENCE as f32;
            if seq_mean > worst.0 {
                worst = (seq_mean, seq.name.clone());
            }
            if conv == Convention::InvertRoot {
                totals.measured += 1;
                totals.worst_mean = totals.worst_mean.max(seq_mean);
            }
        }
        if n == 0 {
            continue;
        }
        log.line(format!(
            "          {anim_name} with {conv:?}: {matched} bones matched by name; average edge stretch {:.4}, edges stretched >50%: {:.2}%, worst sequence {} ({:.4})",
            mean / n as f32,
            over / n as f32 * 100.0,
            worst.1,
            worst.0
        ));
    }
}
