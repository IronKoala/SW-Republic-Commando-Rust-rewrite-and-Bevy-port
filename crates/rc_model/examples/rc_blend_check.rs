//! Checks that characters change animation smoothly, from the numbers.
//!
//! For each pair of animations (idle to run, run to idle, walk to firing,
//! and so on) it plays the first for half a second, switches to the second
//! and plays that for half a second, at 30 frames a second. Every frame it
//! measures how far the point of the mesh that moved most went since the
//! frame before ("step", in Unreal units: 1 uu is about 2 cm).
//!
//! - `snap` is the step at the switch with no blending (how it was).
//! - `blend` is the largest step around the switch with blending.
//! - `own` is the largest step inside the two animations themselves, each
//!   played on its own: how much the character moves anyway.
//!
//! A switch passes when the change is no more sudden than the animations'
//! own motion (`blend` at most `own` times 1.25, plus 1 uu) or at most half
//! the snap, and the mesh stays in one piece while the two poses are mixed
//! (edges stretch by less than 10% on average).
//!
//! `cargo run --release -p rc_model --example rc_blend_check -- "<game folder>"`
//!
//! The blend times are the defaults in `data/squad/looks.toml` and
//! `data/enemies/battle_droid.toml`.

use rc_anim::{edges, stretch, Crossfade};
use rc_model::{Model, ModelSpec};
use std::path::{Path, PathBuf};

const FPS: f32 = 30.0;
const HALF: usize = 15;

/// Our name, the sequence, and the blend time into it.
type Anim = (&'static str, &'static str, f32);

struct Character {
    label: &'static str,
    package: &'static str,
    mesh: &'static str,
    anims: &'static [Anim],
    switches: &'static [(&'static str, &'static str)],
}

const CHARACTERS: [Character; 2] = [
    Character {
        label: "commando",
        package: "GameData/Animations/clone.ukx",
        mesh: "CloneCommando",
        anims: &[
            ("idle", "ActionBreathe", 0.2),
            ("walk", "WalkForward", 0.2),
            ("run", "RunForward", 0.2),
            ("walk_back", "WalkBackward", 0.2),
            ("step_left", "StepLeft", 0.2),
            ("fire", "Fire", 0.05),
            ("down", "WoundedFallDown", 0.25),
            ("revive", "HealImmobileLoop", 0.2),
        ],
        switches: &[
            ("idle", "walk"),
            ("walk", "run"),
            ("run", "idle"),
            ("idle", "fire"),
            ("fire", "idle"),
            ("walk", "step_left"),
            ("run", "walk_back"),
            ("idle", "down"),
            ("idle", "revive"),
            ("revive", "idle"),
        ],
    },
    Character {
        label: "battle droid",
        package: "GameData/Animations/droid.ukx",
        mesh: "BattleDroid",
        anims: &[
            ("idle", "ActionBreathe", 0.2),
            ("walk", "WalkForward", 0.2),
            ("run", "RunForward", 0.2),
            ("fire", "Fire", 0.05),
            ("crouch", "CrouchBreathe", 0.2),
            ("crouch_fire", "CrouchFire", 0.05),
            ("death", "Death", 0.1),
        ],
        switches: &[
            ("idle", "walk"),
            ("walk", "run"),
            ("run", "idle"),
            ("idle", "fire"),
            ("fire", "idle"),
            ("idle", "crouch"),
            ("crouch", "crouch_fire"),
            ("walk", "death"),
        ],
    },
];

/// Find a file under the game folder in any upper or lower case.
fn find(root: &Path, rel: &str) -> PathBuf {
    let mut at = root.to_path_buf();
    for part in rel.split(['/', '\\']) {
        let next = std::fs::read_dir(&at).ok().and_then(|d| {
            d.flatten()
                .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))
                .map(|e| e.path())
        });
        at = next.unwrap_or_else(|| at.join(part));
    }
    at
}

fn largest_step(a: &[[f32; 3]], b: &[[f32; 3]]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(p, q)| {
            let d = [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
            // Bevy units back to Unreal units.
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() * 50.0
        })
        .fold(0.0, f32::max)
}

/// Play `from` then `to`; every frame's points. With `blend` the switch is
/// smoothed over `to`'s blend time.
fn play(model: &Model, from: &str, to: (&str, f32), blend: bool) -> Vec<Vec<[f32; 3]>> {
    let mut fade = Crossfade::default();
    let mut out = Vec::new();
    let mut frame = 0.0f32;
    let mut current = from;
    for i in 0..2 * HALF {
        let dt = 1.0 / FPS;
        if i == HALF && to.0 != from {
            current = to.0;
            frame = 0.0;
            fade.start(if blend { to.1 } else { 0.0 });
        } else if i > 0 {
            fade.advance(dt);
            let clip = model.clip(current).expect("animation found");
            frame = if current == "down" || current == "death" {
                (frame + dt * clip.rate).min(clip.num_frames - 1.0)
            } else {
                (frame + dt * clip.rate).rem_euclid(clip.num_frames)
            };
        }
        let local = model.local_pose(current, frame);
        out.push(model.posed_local(fade.apply(local)));
    }
    out
}

fn main() {
    let Some(game) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: rc_blend_check <game folder>");
        std::process::exit(2);
    };
    let (mut passed, mut failed) = (0, 0);
    for c in &CHARACTERS {
        let spec = ModelSpec {
            package: c.package.into(),
            mesh: c.mesh.into(),
            set: None,
            sequences: c
                .anims
                .iter()
                .map(|a| (a.0.to_string(), a.1.to_string()))
                .collect(),
            skins: Vec::new(),
        };
        let model = match Model::load(&|rel| find(&game, rel), &spec) {
            Ok((m, _)) => m,
            Err(e) => {
                println!("FAIL {}: can't read the model: {e}", c.label);
                failed += 1;
                continue;
            }
        };
        let rest = model.posed_local(&model.skeleton.local);
        let mesh_edges = edges(&model.mesh);
        println!("{} ({} {}):", c.label, c.package, c.mesh);
        for &(a, b) in c.switches {
            let Some(&(_, _, time)) = c.anims.iter().find(|x| x.0 == b) else {
                continue;
            };
            if model.clip(a).is_none() || model.clip(b).is_none() {
                println!("  FAIL {a} -> {b}: animation not found");
                failed += 1;
                continue;
            }
            let snapped = play(&model, a, (b, time), false);
            let blended = play(&model, a, (b, time), true);
            let steps = |f: &[Vec<[f32; 3]>]| -> Vec<f32> {
                f.windows(2).map(|w| largest_step(&w[0], &w[1])).collect()
            };
            let (s, bl) = (steps(&snapped), steps(&blended));
            // Step i goes from frame i to i + 1; the switch is frame HALF.
            let switch_at = HALF - 1;
            let window = (time * FPS).ceil() as usize + 2;
            let around = switch_at..(switch_at + window).min(bl.len());
            let snap = s[switch_at];
            let blend = bl[around.clone()].iter().copied().fold(0.0, f32::max);
            // Each animation played on its own from its first frame.
            let alone = |name: &str| steps(&play(&model, name, (name, 0.0), false));
            let own = alone(a).into_iter().chain(alone(b)).fold(0.0, f32::max);
            let worst_stretch = blended[around.start + 1..=around.end]
                .iter()
                .map(|p| stretch(&rest, p, &mesh_edges).mean)
                .fold(0.0, f32::max);
            let ok = (blend <= own * 1.25 + 1.0 || blend <= 0.5 * snap) && worst_stretch < 0.1;
            println!(
                "  {} {a} -> {b} (blend {time:.2} s): snap {snap:.1} uu, blend {blend:.1} uu, own {own:.1} uu, stretch while mixing {:.3}",
                if ok { "ok  " } else { "FAIL" },
                worst_stretch
            );
            if ok {
                passed += 1;
            } else {
                failed += 1;
            }
        }
    }
    println!("BLEND CHECK: {passed} passed, {failed} failed");
    if failed > 0 {
        std::process::exit(1);
    }
}
