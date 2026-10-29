//! Lists what an animation package holds: every skeletal mesh with its
//! material slots, and every animation set with its sequences. For
//! filling in look files (`data/squad/looks.toml`).
//!
//! `cargo run -p rc_model --example rc_model_list -- <file.ukx> [--sequences]`

use rc_package::animation::read_mesh_animation;
use rc_package::class::object_path;
use rc_package::skeletal::read_skeletal_mesh;
use rc_package::Package;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(file) = args.first() else {
        eprintln!("usage: rc_model_list <file.ukx> [--sequences]");
        std::process::exit(2);
    };
    let sequences = args.iter().any(|a| a == "--sequences");
    let data = std::fs::read(file).expect("can't read the file");
    let pkg = Package::parse(&data).expect("not a package");
    for e in &pkg.exports {
        match pkg.export_class_name(e) {
            "SkeletalMesh" => {
                let name = pkg.name(e.object_name);
                match read_skeletal_mesh(&pkg, &data, e) {
                    Ok(m) => {
                        println!(
                            "mesh {name}: {} bones, {} triangles, {} material slots",
                            m.bones.len(),
                            m.triangles.len(),
                            m.textures.len()
                        );
                        for (i, &t) in m.textures.iter().enumerate() {
                            let used = m
                                .triangles
                                .iter()
                                .filter(|tri| {
                                    m.materials
                                        .get(tri.material as usize)
                                        .map_or(tri.material as usize, |mm| {
                                            mm.texture_index.max(0) as usize
                                        })
                                        == i
                                })
                                .count();
                            println!("  slot {i}: {} ({used} triangles)", object_path(&pkg, t));
                        }
                        for &a in &m.animations {
                            println!("  animation set {}", object_path(&pkg, a));
                        }
                    }
                    Err(err) => println!("mesh {name}: can't read: {err}"),
                }
            }
            "MeshAnimation" if sequences => {
                let name = pkg.name(e.object_name);
                match read_mesh_animation(&pkg, &data, e) {
                    Ok(a) => {
                        println!("animation set {name}: {} sequences", a.sequences.len());
                        for s in &a.sequences {
                            println!("  {} ({} frames at {}/s)", s.name, s.num_frames, s.rate);
                        }
                    }
                    Err(err) => println!("animation set {name}: can't read: {err}"),
                }
            }
            _ => {}
        }
    }
}
