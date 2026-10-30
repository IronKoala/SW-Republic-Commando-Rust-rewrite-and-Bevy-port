//! Explore: where the first-person arms sit in view for each animation.
//! `cargo run -p rc_model --example rc_arms_probe -- <game folder> [Mesh]`

use rc_model::{Model, ModelSpec};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let game = std::path::PathBuf::from(&args[0]);
    let mesh = args.get(1).cloned().unwrap_or("HudArmsDC17m".into());
    let names = [
        "ActionBreathe",
        "Idle",
        "Walk",
        "Run",
        "Fire",
        "Reload",
        "Holster",
        "Load",
        "AttachHolster",
        "AttachLoad",
        "ThrowGrenade",
        "MeleeAttack",
        "Aim",
        "AimBreathe",
    ];
    let spec = ModelSpec {
        package: "GameData/Animations/hudarms.ukx".into(),
        mesh,
        set: None,
        sequences: names
            .iter()
            .map(|n| (n.to_string(), n.to_string()))
            .collect(),
        skins: vec![],
    };
    let resolve = |r: &str| {
        // Any upper or lower case, as on Windows.
        let mut at = game.clone();
        for part in r.split('/') {
            let want = part.to_ascii_lowercase();
            let found = std::fs::read_dir(&at).ok().and_then(|d| {
                d.flatten()
                    .find(|e| e.file_name().to_string_lossy().to_ascii_lowercase() == want)
            });
            at = found.map_or(at.join(part), |e| e.path());
        }
        at
    };
    let (m, notes) = Model::load(&resolve, &spec).unwrap();
    for n in notes {
        println!("{n}");
    }
    for n in names {
        let Some(c) = m.clip(n) else { continue };
        for f in [0.0, c.num_frames * 0.5, c.num_frames - 1.0] {
            let pts = m.posed_from_origin(n, f);
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            let mut inview = 0;
            for p in &pts {
                let u = [p[0] * 50.0, p[2] * 50.0, p[1] * 50.0];
                for k in 0..3 {
                    lo[k] = lo[k].min(u[k]);
                    hi[k] = hi[k].max(u[k]);
                }
                // 85 degrees across 4:3, 69 up and down.
                if u[0] > 1.0 && (u[1] / u[0]).abs() < 0.9163 && (u[2] / u[0]).abs() < 0.687 {
                    inview += 1;
                }
            }
            println!(
                "{n:14} frame {f:5.1}: fwd {:6.1}..{:6.1} right {:6.1}..{:6.1} up {:6.1}..{:6.1}  in view {:.0}%",
                lo[0], hi[0], lo[1], hi[1], lo[2], hi[2],
                100.0 * inview as f32 / pts.len() as f32
            );
        }
    }
}
