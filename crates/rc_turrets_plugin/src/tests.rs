//! Checks the turret models turn the right way. They need the game's
//! files (RC_GAME_DIR, or the default Steam folder) and pass without
//! checking anything when they aren't there.

use super::*;
use rc_turrets::PlacedTurret;
use std::path::{Path, PathBuf};

fn game() -> Option<PathBuf> {
    let dir = std::env::var_os("RC_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando")
        });
    dir.join("GameData/Animations/turrets.ukx")
        .exists()
        .then_some(dir)
}

fn placed(yaw: i32) -> PlacedTurret {
    PlacedTurret {
        export: 0,
        name: "T".into(),
        class: "T".into(),
        tag: "T".into(),
        location: [0.0, 0.0, 0.0],
        rotation: [0, yaw, 0],
        control_event: None,
        player_control_event: None,
        release_event: None,
        player_usable: None,
        min_yaw: None,
        max_yaw: None,
        min_pitch: None,
        max_pitch: None,
        unlimited_yaw: None,
        speed: None,
        exit_spot: None,
        exit_offset: None,
    }
}

/// Where the muzzles are on average, in the level, with the gun turned.
fn muzzle(kl: &KindLook, t: &Turret, half: f32) -> [f32; 3] {
    let (_, global) = turret_bones(kl, t.yaw, t.pitch);
    let mut sum = [0.0; 3];
    for &m in &kl.muzzles {
        let p = model_to_level(&kl.look, t, half, global[m].pos);
        for k in 0..3 {
            sum[k] += p[k] / kl.muzzles.len() as f32;
        }
    }
    sum
}

#[test]
fn turret_models_point_and_turn_the_way_they_aim() {
    let Some(dir) = game() else {
        eprintln!("no game files: not checked");
        return;
    };
    let paths = GamePaths::new(dir, vec![]);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/turrets");
    let (files, errors) = rc_turrets::params::load_folders(&[&root]);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(files.len(), 3);
    for (id, p) in &files {
        let (kl, notes) = load_look(&paths, p).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(
            kl.yaw_bone.is_some() && kl.pitch_bone.is_some(),
            "{id}: {notes:?}"
        );
        assert_eq!(kl.muzzles.len(), p.model.muzzles.len(), "{id}: {notes:?}");
        // Facing +Y in the level (yaw 16384).
        let mut t = Turret::new(placed(16384), id, p);
        let pivot = |t: &Turret| {
            let (_, g) = turret_bones(&kl, t.yaw, t.pitch);
            model_to_level(
                &kl.look,
                t,
                p.gunner.half_height,
                g[kl.yaw_bone.unwrap()].pos,
            )
        };
        let base = pivot(&t);
        let rest = muzzle(&kl, &t, p.gunner.half_height);
        let d = [rest[0] - base[0], rest[1] - base[1]];
        eprintln!("{id}: pivot {base:?}, muzzles at rest {rest:?}");
        assert!(
            d[1] > d[0].abs(),
            "{id}: the gun doesn't point the turret's way: {d:?}"
        );
        // Turned left (towards -X when facing +Y) and up.
        t.yaw = 4000.0;
        t.pitch = 2000.0;
        let turned = muzzle(&kl, &t, p.gunner.half_height);
        eprintln!("{id}: turned left and up: {turned:?}");
        assert!(turned[0] < rest[0] - 1.0, "{id}: yaw turns the wrong way");
        assert!(turned[2] > rest[2] + 1.0, "{id}: pitch turns the wrong way");
    }
}
