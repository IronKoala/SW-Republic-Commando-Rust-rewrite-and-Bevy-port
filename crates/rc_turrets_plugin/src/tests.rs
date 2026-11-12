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
        event: None,
        first_damaged_event: None,
        deployed: None,
        health: None,
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
    assert_eq!(files.len(), 6);
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

/// The core ship's wall turrets (geo_04b) are placed on their sides
/// (pitch -16383, roll -32763): turned toward a target, the barrels must
/// point at it all the same.
#[test]
fn a_turret_on_its_side_points_at_its_target() {
    let Some(dir) = game() else {
        eprintln!("no game files: not checked");
        return;
    };
    let paths = GamePaths::new(dir, vec![]);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/turrets");
    let (files, _) = rc_turrets::params::load_folders(&[&root]);
    let p = &files["core_ship_turret"];
    let (kl, notes) = load_look(&paths, p).unwrap();
    assert!(
        kl.closed.is_some() && kl.deploy.is_some() && kl.dead.is_some(),
        "{notes:?}"
    );
    for rotation in [[0, 0, 0], [0, 16408, 0], [-16383, 0, -32763]] {
        let mut place = placed(0);
        place.rotation = rotation;
        let mut t = Turret::new(place, "core_ship_turret", p);
        // Somewhere the turret can see: out from its base, a little aside.
        let [f, r, u] = t.axes();
        let target: [f32; 3] = std::array::from_fn(|k| 600.0 * f[k] + 300.0 * r[k] + 200.0 * u[k]);
        for _ in 0..200 {
            let from = t.to_level(kl.aim_pivot);
            t.turn_toward_by(from, target, 0.05, kl.aim_rest);
        }
        let (_, g) = turret_bones(&kl, t.yaw, t.pitch);
        let piv = model_to_level(
            &kl.look,
            &t,
            p.gunner.half_height,
            g[kl.pitch_bone.unwrap()].pos,
        );
        let m = muzzle(&kl, &t, p.gunner.half_height);
        let gun: [f32; 3] = std::array::from_fn(|k| m[k] - piv[k]);
        let want: [f32; 3] = std::array::from_fn(|k| target[k] - piv[k]);
        let len = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let cos = (gun[0] * want[0] + gun[1] * want[1] + gun[2] * want[2]) / (len(gun) * len(want));
        eprintln!("rotation {rotation:?}: yaw {:.0} pitch {:.0}, gun {gun:?}, to target {want:?}, cos {cos:.3}", t.yaw, t.pitch);
        assert!(
            cos > 0.9,
            "rotation {rotation:?}: the barrels point {gun:?}, the target is {want:?}"
        );
    }
}
