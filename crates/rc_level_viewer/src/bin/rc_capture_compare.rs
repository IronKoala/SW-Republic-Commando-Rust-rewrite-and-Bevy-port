//! rc_capture_compare: compares two pictures saved by
//! `rc_level_viewer --capture`, by their colours, to check a change in how
//! things are drawn.
//!
//! Usage:
//!   rc_capture_compare A.bmp B.bmp --same
//!   rc_capture_compare FLAT.bmp BUMPED.bmp --bumped
//!
//! The picture is cut into a 4 x 4 grid and each square's average colour
//! (red, green, blue, 0-255) is compared, so a change that brightens,
//! darkens or tints any part of the view shows up, not only one that moves
//! outlines. "Detail" is how much neighbouring pixels differ on average:
//! bump maps should add detail without changing the colours.
//!
//! - `--same`: the two must match: every square's average within 2 and 99%
//!   of pixels within 3 on every channel. Used to check that the bump
//!   shader with `bump_strength = 0` draws exactly what the flat props did.
//! - `--bumped`: B is A with bump maps: every square's average within 4 of
//!   A's, at least 5% of pixels different, and the whole view's detail up
//!   by at least 3%.
//!
//! Prints one line per square and PASS or FAIL, and exits with 1 on FAIL.

use std::process::ExitCode;

struct Picture {
    w: usize,
    h: usize,
    /// Red, green, blue per pixel, top row first.
    px: Vec<[u8; 3]>,
}

/// Read a 24-bit .bmp as `rc_level_viewer` writes it.
fn read_bmp(path: &str) -> Result<Picture, String> {
    let d = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    if d.len() < 54 || &d[0..2] != b"BM" {
        return Err(format!("{path}: not a .bmp file"));
    }
    let u32_at = |i: usize| u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
    let offset = u32_at(10) as usize;
    let w = u32_at(18) as i32;
    let h = u32_at(22) as i32;
    let bits = u16::from_le_bytes([d[28], d[29]]);
    if bits != 24 || w <= 0 || h == 0 {
        return Err(format!(
            "{path}: only 24-bit pictures are read ({bits}-bit, {w}x{h})"
        ));
    }
    let (w, top_down, h) = (w as usize, h < 0, h.unsigned_abs() as usize);
    let row = (w * 3).div_ceil(4) * 4;
    if d.len() < offset + row * h {
        return Err(format!("{path}: file is cut short"));
    }
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        let src = if top_down { y } else { h - 1 - y };
        let r = &d[offset + src * row..];
        for x in 0..w {
            px.push([r[x * 3 + 2], r[x * 3 + 1], r[x * 3]]);
        }
    }
    Ok(Picture { w, h, px })
}

const GRID: usize = 4;

/// Average colour of each grid square, row by row.
fn square_averages(p: &Picture) -> Vec<[f64; 3]> {
    let mut out = Vec::new();
    for gy in 0..GRID {
        for gx in 0..GRID {
            let (y0, y1) = (gy * p.h / GRID, (gy + 1) * p.h / GRID);
            let (x0, x1) = (gx * p.w / GRID, (gx + 1) * p.w / GRID);
            let mut sum = [0.0f64; 3];
            for y in y0..y1 {
                for x in x0..x1 {
                    for k in 0..3 {
                        sum[k] += p.px[y * p.w + x][k] as f64;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)).max(1) as f64;
            out.push(sum.map(|s| s / n));
        }
    }
    out
}

/// Average difference between each pixel and its right and lower
/// neighbours (summed over red, green and blue).
fn detail(p: &Picture) -> f64 {
    let diff = |a: [u8; 3], b: [u8; 3]| {
        (0..3)
            .map(|k| (a[k] as f64 - b[k] as f64).abs())
            .sum::<f64>()
    };
    let mut sum = 0.0;
    let mut n = 0usize;
    for y in 0..p.h - 1 {
        for x in 0..p.w - 1 {
            let c = p.px[y * p.w + x];
            sum += diff(c, p.px[y * p.w + x + 1]) + diff(c, p.px[(y + 1) * p.w + x]);
            n += 2;
        }
    }
    sum / n.max(1) as f64
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(a), Some(b), Some(mode)) = (args.first(), args.get(1), args.get(2)) else {
        eprintln!("usage: rc_capture_compare A.bmp B.bmp --same|--bumped");
        return ExitCode::from(2);
    };
    let same = match mode.as_str() {
        "--same" => true,
        "--bumped" => false,
        _ => {
            eprintln!("the third argument must be --same or --bumped");
            return ExitCode::from(2);
        }
    };
    let (pa, pb) = match (read_bmp(a), read_bmp(b)) {
        (Ok(x), Ok(y)) => (x, y),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    if (pa.w, pa.h) != (pb.w, pb.h) {
        println!(
            "FAIL: sizes differ ({}x{} and {}x{})",
            pa.w, pa.h, pb.w, pb.h
        );
        return ExitCode::from(1);
    }
    let (sa, sb) = (square_averages(&pa), square_averages(&pb));
    let mut worst = 0.0f64;
    for (i, (x, y)) in sa.iter().zip(&sb).enumerate() {
        let d = (0..3).map(|k| (x[k] - y[k]).abs()).fold(0.0, f64::max);
        worst = worst.max(d);
        println!(
            "square row {} column {}: A {:>5.1} {:>5.1} {:>5.1}  B {:>5.1} {:>5.1} {:>5.1}  biggest difference {d:.1}",
            i / GRID + 1,
            i % GRID + 1,
            x[0],
            x[1],
            x[2],
            y[0],
            y[1],
            y[2]
        );
    }
    let close = pa
        .px
        .iter()
        .zip(&pb.px)
        .filter(|(x, y)| (0..3).all(|k| (x[k] as i32 - y[k] as i32).abs() <= 3))
        .count() as f64
        / pa.px.len() as f64;
    let changed =
        pa.px.iter().zip(&pb.px).filter(|(x, y)| x != y).count() as f64 / pa.px.len() as f64;
    let (da, db) = (detail(&pa), detail(&pb));
    let gain = db / da.max(1e-9) - 1.0;
    println!(
        "biggest square difference {worst:.1}; pixels within 3: {:.1}%; pixels changed: {:.1}%; detail A {da:.2}, B {db:.2} ({:+.1}%)",
        close * 100.0,
        changed * 100.0,
        gain * 100.0
    );
    let (ok, rule) = if same {
        (
            worst <= 2.0 && close >= 0.99,
            "--same: squares within 2 and 99% of pixels within 3",
        )
    } else {
        (
            worst <= 4.0 && changed >= 0.05 && gain >= 0.03,
            "--bumped: squares within 4, at least 5% of pixels changed, detail up at least 3%",
        )
    };
    println!("{} ({rule})", if ok { "PASS" } else { "FAIL" });
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
