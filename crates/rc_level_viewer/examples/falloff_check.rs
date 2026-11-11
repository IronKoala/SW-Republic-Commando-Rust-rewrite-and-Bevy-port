//! Checks that geo_01a's light shafts get the game's falloff shader
//! settings (`HsFalloff`, see `rc_level_viewer::falloff`), read from the
//! game's own files. Needs the game's files (RC_GAME_DIR, or the default
//! Steam folder); no window.
//!
//! Run: `cargo run -p rc_level_viewer --example falloff_check`
//! It ends `FALLOFF CHECK: N passed, 0 failed`.

use rc_level_viewer::falloff::{FalloffDefaults, D3DBLEND_ONE, D3DBLEND_SRCCOLOR};
use rc_level_viewer::materials::Materials;
use rc_level_viewer::scene;
use rc_package::Package;
use std::path::PathBuf;
use std::rc::Rc;

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";

fn main() {
    let game = std::env::var_os("RC_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_DIR));
    let game_data = game.join("GameData");
    let (mut passed, mut failed) = (0, 0);
    let mut check = |ok: bool, what: String| {
        println!("{} {what}", if ok { "PASS" } else { "FAIL" });
        if ok {
            passed += 1;
        } else {
            failed += 1;
        }
    };
    let mut mats = Materials::new(&game_data);

    // Engine.u's class defaults: white tints, SRCCOLOR / ONE.
    match FalloffDefaults::read(&mut mats) {
        Ok(d) => {
            check(
                d.src_blend == D3DBLEND_SRCCOLOR && d.dest_blend == D3DBLEND_ONE,
                format!("HsFalloff blends SrcBlend {} DestBlend {} (want 3 SRCCOLOR, 2 ONE)", d.src_blend, d.dest_blend),
            );
            check(
                d.diffuse_tint == [255; 4] && d.gradient_tint == [255; 4],
                format!("HsFalloff tints default to white: {:?} {:?}", d.diffuse_tint, d.gradient_tint),
            );
        }
        Err(e) => check(false, format!("HsFalloff defaults not read: {e}")),
    }

    let path = game_data.join("Maps").join("geo_01a.ctm");
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            println!("FAIL can't read {}: {e}", path.display());
            println!("FALLOFF CHECK: 0 passed, 1 failed");
            std::process::exit(1);
        }
    };
    let pkg = Package::parse(&data).expect("geo_01a isn't a package");
    let s = scene::build(Rc::new((pkg, data)), &mut mats).expect("geo_01a not built");
    check(
        s.stats.falloff_error.is_none(),
        format!("no falloff error ({:?})", s.stats.falloff_error),
    );
    check(
        s.stats.prop_parts_falloff >= 2,
        format!("geo_01a: {} prop parts with the falloff shader (the sun beams and the tunnel light cones)", s.stats.prop_parts_falloff),
    );
    let falloffs: Vec<_> = s
        .batches
        .iter()
        .filter_map(|b| b.picture.as_ref()?.falloff.clone().map(|f| (b, f)))
        .collect();
    // The sun beams (`Arena_Textures.Unique.rayfalloff`): DiffuseTint
    // stored 182,238,252 (blue, green, red), default blend (squared).
    let rays = falloffs.iter().find(|(_, f)| f.gradient_name.ends_with("Unique.Falloff"));
    match rays {
        Some((b, f)) => {
            let want = [252.0 / 255.0, 238.0 / 255.0, 182.0 / 255.0];
            let close = (0..3).all(|k| (f.diffuse_tint[k] - want[k]).abs() < 0.002);
            check(close, format!("sun beams tinted {:?} (want red 252, green 238, blue 182 / 255)", f.diffuse_tint));
            check(f.squared(), "sun beams squared before adding (SrcBlend SRCCOLOR)".into());
            check(f.pan == [0.0; 2] && f.oscillate == [0.0; 2], format!("sun beams still: pan {:?}, oscillate {:?}", f.pan, f.oscillate));
            let g = &f.gradient;
            let row: Vec<u8> = (0..g.width).map(|x| g.pixels[x * 4]).collect();
            check(
                row.first() < row.last() && row.windows(2).all(|w| w[1] as i32 + 8 >= w[0] as i32),
                format!("gradient dark side-on, bright face-on: {} .. {}", row[0], row[row.len() - 1]),
            );
            let white = b.colours.iter().filter(|c| c[0] == 255).count();
            let black = b.colours.iter().filter(|c| c[0] == 0).count();
            check(
                white > 0 && black > 0,
                format!("sun beams carry their painted colours: {white} white corners, {black} black"),
            );
        }
        None => check(false, "sun beam material (gradient Unique.Falloff) not found".into()),
    }
    // The tunnel light cones (`Shader.VolumnLightTex`): SrcBlend ONE,
    // VPanRate 0.1.
    match falloffs.iter().find(|(_, f)| f.gradient_name.ends_with("InverseFalloff")) {
        Some((_, f)) => {
            check(!f.squared(), format!("light cones added as they are (SrcBlend {} = ONE)", f.src_blend));
            check(
                (f.pan[1] - 0.1).abs() < 1e-6,
                format!("light cones pan down at {} per second (want 0.1)", f.pan[1]),
            );
        }
        None => check(false, "light cone material (gradient InverseFalloff) not found".into()),
    }
    println!("FALLOFF CHECK: {passed} passed, {failed} failed");
    if failed > 0 {
        std::process::exit(1);
    }
}
