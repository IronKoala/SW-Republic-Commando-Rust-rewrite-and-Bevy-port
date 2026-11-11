//! Decode a game video and report on it, without the game.
//!
//!   rc_video_check <file.avi> [--yuv out.yuv] [--compare ref.yuv] [--loops N]
//!
//! Prints the size, frame rate, frame count and how long one frame
//! takes to decode. `--yuv` writes every decoded frame as raw planar
//! 4:2:0 (what `ffmpeg -i file.avi -f rawvideo -pix_fmt yuv420p` writes),
//! and `--compare` checks every pixel against such a file and prints the
//! largest difference per frame. `--loops N` plays the video N times in
//! a row to check the loop back to the first frame.

use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "usage: rc_video_check <file.avi> [--yuv out.yuv] [--compare ref.yuv] [--loops N]"
        );
        std::process::exit(2);
    }
    let path = &args[1];
    let mut yuv_out = None;
    let mut compare = None;
    let mut loops = 1u32;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--yuv" => {
                yuv_out = args.get(i + 1).cloned();
                i += 1;
            }
            "--compare" => {
                compare = args.get(i + 1).cloned();
                i += 1;
            }
            "--loops" => {
                loops = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(1);
                i += 1;
            }
            other => {
                eprintln!("unknown option {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            println!("FAIL: cannot read {path}: {e}");
            std::process::exit(1);
        }
    };
    let mut player = match rc_video::VideoPlayer::open(data) {
        Ok(p) => p,
        Err(e) => {
            println!("FAIL: {e}");
            std::process::exit(1);
        }
    };
    let info = player.info.clone();
    println!(
        "video: {}x{} {} at {:.3} fps, {} frames, {:.2} s",
        info.width,
        info.height,
        info.codec_name(),
        info.fps(),
        info.frames.len(),
        info.seconds()
    );
    let reference = compare.map(|p| std::fs::read(p).expect("read compare file"));
    let frame_bytes = (info.width * info.height * 3 / 2) as usize;
    let mut yuv_file = yuv_out.map(|p| std::fs::File::create(p).expect("create yuv file"));
    let total = info.frames.len() * loops as usize;
    let t = Instant::now();
    let mut worst = 0i32;
    let mut exact = 0usize;
    let mut compared = 0usize;
    for n in 0..total {
        if let Err(e) = player.step() {
            println!("FAIL: {e}");
            println!("stats: {:?}", player.stats());
            std::process::exit(1);
        }
        let yuv = player.yuv().unwrap();
        if let Some(f) = yuv_file.as_mut() {
            use std::io::Write;
            f.write_all(&yuv).unwrap();
        }
        if let Some(r) = &reference {
            let k = n % info.frames.len();
            if let Some(rf) = r.get(k * frame_bytes..(k + 1) * frame_bytes) {
                let mut m = 0i32;
                for (a, b) in yuv.iter().zip(rf) {
                    let d = (*a as i32 - *b as i32).abs();
                    m = m.max(d);
                    if d == 0 {
                        exact += 1;
                    }
                }
                compared += yuv.len();
                worst = worst.max(m);
                if n < 3 || n % 50 == 0 || m > 4 {
                    println!("frame {n}: largest difference {m}");
                }
            }
        }
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0 / total as f64;
    println!(
        "decoded {total} frames, {ms:.2} ms per frame, looped {} times",
        player.loops
    );
    println!("stats: {:?}", player.stats());
    if compared > 0 {
        println!(
            "compare: {:.2}% of samples identical, largest difference {worst} (of 255)",
            100.0 * exact as f64 / compared as f64
        );
        if worst > 4 {
            println!("FAIL: differs from the reference by more than 4");
            std::process::exit(1);
        }
    }
    println!("OK");
}
