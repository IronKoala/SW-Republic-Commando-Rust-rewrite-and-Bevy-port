//! geo_03c's Geonosian turret gunner: the spawner tagged
//! `SpawnTurretGunner` gives its character a patrol route of one point,
//! the sonic turret `SonicTurret1`. A turret's anchor is its first exit
//! spot (`NavPts[0]`, here `SquadMarkerSpot21`), so the gunner runs there
//! and then sends `GunnerGetOnTurret`, which its level script waits for
//! before putting it on the turret. Needs the game files (`RC_GAME_DIR`);
//! without them the test says so and passes.

use rc_ai::{PatrolMode, PatrolRun, PatrolStep};

#[test]
fn geo_03c_gunner_runs_to_the_turret_then_says_so() {
    let Ok(game) = std::env::var("RC_GAME_DIR") else {
        eprintln!("RC_GAME_DIR not set: skipped");
        return;
    };
    let root = std::path::Path::new(&game).join("GameData");
    let map = root.join("Maps/geo_03c.ctm");
    if !map.exists() {
        eprintln!("geo_03c not in {game}: skipped");
        return;
    }
    let data = std::fs::read(&map).unwrap();
    let pkg = rc_package::Package::parse(&data).unwrap();
    let mut factories = rc_ai::read_factories(&pkg, &data).unwrap();
    let (lib, _) = rc_enemies::read_class_library(&root);
    let is_a = |c: &str, base: &str| lib.chain(c).iter().any(|x| x.eq_ignore_ascii_case(base));
    let notes = rc_ai::resolve_patrol_anchors(&pkg, &data, &mut factories, &is_a);
    eprintln!("notes: {notes:?}");
    let f = factories
        .iter()
        .find(|f| f.tag == "SpawnTurretGunner")
        .expect("the gunner's spawner");
    let o = &f.offspring;
    assert_eq!(o.script.as_deref(), Some("Gunner"));
    assert_eq!(o.patrol_mode, PatrolMode::Once);
    assert_eq!(o.patrol.len(), 1);
    let p = &o.patrol[0];
    assert_eq!(p.node.as_deref(), Some("SonicTurret1"));
    assert!(p.run);
    assert_eq!(p.event.as_deref(), Some("GunnerGetOnTurret"));
    let at = p.anchor.expect("anchor from the turret's NavPts[0]");
    // SquadMarkerSpot21 in the map.
    assert!(
        (at.x - 3773.9).abs() < 1.0 && (at.y + 40467.1).abs() < 1.0,
        "{at:?}"
    );

    let mut run = PatrolRun::new(o.patrol.clone(), o.patrol_mode);
    let mut steps = Vec::new();
    while let Some(s) = run.next(&mut |n| n / 2) {
        steps.push(s);
        if steps.len() > 10 {
            break;
        }
    }
    assert!(matches!(steps[0], PatrolStep::Wait(w) if (w - 0.2).abs() < 1e-6));
    assert!(steps
        .iter()
        .any(|s| matches!(s, PatrolStep::Move { run: true, .. })));
    assert!(matches!(steps.last(), Some(PatrolStep::Event(e)) if e == "GunnerGetOnTurret"));
    assert!(run.finished());
}
