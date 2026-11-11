//! Prints the light the light probe (`probe.rs`) finds at a map's player
//! start, and at points along the view from there, to compare with
//! screenshots of the original game.
//!
//! Usage: probe_check MAP [x y z]   (x y z: a Bevy position to ask about too)
//! RC_GAME_DIR points at the game folder.

use rc_level_viewer::lighting::LightingSettings;
use rc_level_viewer::probe::LightProbe;
use rc_level_viewer::{materials, scene};
use rc_package::Package;
use std::path::PathBuf;
use std::rc::Rc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let map = args.first().cloned().unwrap_or_else(|| "geo_01a".into());
    let game = PathBuf::from(std::env::var("RC_GAME_DIR").expect("set RC_GAME_DIR"));
    let gd = game.join("GameData");
    let data = std::fs::read(gd.join("Maps").join(format!("{map}.ctm"))).expect("map");
    let pkg = Package::parse(&data).expect("parse");
    let mut mats = materials::Materials::new(&gd);
    let sc = scene::build(Rc::new((pkg, data)), &mut mats).expect("scene");
    debug(&sc);
    let s = LightingSettings::default();
    let t = std::time::Instant::now();
    let (probe, stats) = LightProbe::build(&sc, s.vertex_colour_gain, s.lightmap_gain, 6.0);
    println!(
        "{map}: {} samples ({:?}) in {:.0} ms",
        probe.samples(),
        stats,
        t.elapsed().as_secs_f64() * 1000.0
    );
    let p = sc.spawn.position;
    let yaw = sc.spawn.yaw as f32 / 65536.0 * std::f32::consts::TAU;
    // Unreal yaw turns from +X towards +Y; in Bevy that's +X towards +Z.
    let fwd = [yaw.cos(), 0.0, yaw.sin()];
    for step in 0..6 {
        let d = step as f32 * 2.0;
        let q = [p[0] + fwd[0] * d, p[1], p[2] + fwd[2] * d];
        let l = probe.light_at(q);
        println!(
            "  {d:>4.0} m ahead of the start ({:.1}, {:.1}, {:.1}): {}",
            q[0],
            q[1],
            q[2],
            fmt(l)
        );
    }
    if args.len() >= 4 {
        let q: Vec<f32> = args[1..4].iter().map(|v| v.parse().unwrap()).collect();
        println!(
            "  at ({}, {}, {}): {}",
            q[0],
            q[1],
            q[2],
            fmt(probe.light_at([q[0], q[1], q[2]]))
        );
    }
}

fn fmt(l: Option<[f32; 3]>) -> String {
    match l {
        Some(c) => format!(
            "{:.2} {:.2} {:.2} (mean {:.2})",
            c[0],
            c[1],
            c[2],
            (c[0] + c[1] + c[2]) / 3.0
        ),
        None => "none".into(),
    }
}

#[allow(dead_code)]
fn debug(sc: &scene::Scene) {
    let mut n = [0usize; 6];
    for b in &sc.batches {
        if b.from_mesh {
            n[0] += 1;
            continue;
        }
        n[1] += 1;
        if b.lightmap_page.is_some() {
            n[2] += 1;
            n[3] += b.indices.len() / 3;
        }
        if b.lightmap_uvs.len() == b.positions.len() {
            n[4] += 1;
        }
        if b.sky {
            n[5] += 1;
        }
    }
    let p = sc.spawn.position;
    let mut best = (f32::MAX, [0.0f32; 3], false, false, false);
    for b in &sc.batches {
        for q in &b.positions {
            let d = ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2)).sqrt();
            if d < best.0 {
                best = (d, *q, b.sky, b.mover.is_some(), b.translucent);
            }
        }
    }
    println!("nearest corner to the start: {:?}", best);
    println!(
        "mesh batches {}, bsp batches {}, with page {} ({} tris), uv ok {}, sky {}",
        n[0], n[1], n[2], n[3], n[4], n[5]
    );
}
