//! Checks that invisible walls (blocking volumes) really stop the player.
//!
//! Usage: cargo run --release -p rc_walk --example volume_walk_test -- GAMEDATA MAP...
//!
//! For each volume that stops players and is small enough, a commando
//! starts at a nearby AI path point and runs straight at the volume's middle
//! for 4 seconds, with the game's movement (`rc_movement`). It's done twice:
//! once with the volumes in the collision and once without. With them the
//! player should never get inside; without them some runs should, which
//! shows the test can tell the difference.

use rc_movement::{MoveInput, MoveParams, Pawn, TriangleWorld, Vec3};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let gd = std::path::PathBuf::from(&a[1]);
    let mut totals = [0usize; 3];
    for name in &a[2..] {
        let map = gd.join("Maps").join(format!("{name}.ctm"));
        let params = MoveParams::player_commando();
        let lvl = rc_level::map_collision::load(&map, &gd, params.collision_height).unwrap();
        let data = std::fs::read(&map).unwrap();
        let pkg = rc_package::Package::parse(&data).unwrap();
        let level = rc_level::actors::read_level(&pkg, &data).unwrap();
        let vols = rc_level::volumes::blocking_volumes(&pkg, &data, &level);
        let nvol: usize = vols
            .volumes
            .iter()
            .filter(|v| v.block_players)
            .map(|v| v.triangles().count())
            .sum();
        let v3 = |p: [f32; 3]| Vec3::new(p[0], p[1], p[2]);
        let conv = |t: &[[f32; 3]; 3]| [v3(t[0]), v3(t[1]), v3(t[2])];
        let with = TriangleWorld::new(lvl.triangles.iter().map(conv).collect());
        let without = TriangleWorld::new(
            lvl.triangles[..lvl.triangles.len() - nvol]
                .iter()
                .map(conv)
                .collect(),
        );
        let mut pts = vec![];
        for &i in level.actors.iter().flatten() {
            if pkg.export_class_name(&pkg.exports[i]) == "PathNode" {
                pts.push(
                    rc_level::actors::read_placement(&pkg, &data, i)
                        .unwrap()
                        .location,
                );
            }
        }
        let (mut tried, mut inside_with, mut inside_without) = (0, 0, 0);
        for v in vols.volumes.iter().filter(|v| v.block_players) {
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            for q in v.polygons.iter().flatten() {
                for c in 0..3 {
                    lo[c] = lo[c].min(q[c]);
                    hi[c] = hi[c].max(q[c]);
                }
            }
            // Only volumes that reach the floor around a path point and are no
            // wider than 1500 units, so a 4 s walk can get into them.
            if hi[0] - lo[0] > 1500.0 || hi[1] - lo[1] > 1500.0 {
                continue;
            }
            let Some(start) = pts
                .iter()
                .filter(|p| {
                    !v.contains(**p)
                        && p[2] > lo[2]
                        && p[2] < hi[2]
                        && (0..2).all(|c| p[c] > lo[c] - 400.0 && p[c] < hi[c] + 400.0)
                })
                .next()
            else {
                continue;
            };
            let centre = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, start[2]];
            let yaw = (centre[1] - start[1]).atan2(centre[0] - start[0]);
            tried += 1;
            for (w, count) in [(&with, &mut inside_with), (&without, &mut inside_without)] {
                let mut pawn = Pawn::new(
                    params.clone(),
                    Vec3::new(start[0], start[1], start[2] - params.collision_height),
                );
                let input = MoveInput {
                    forward: 1.0,
                    yaw,
                    ..Default::default()
                };
                let mut entered = false;
                for _ in 0..240 {
                    pawn.step(1.0 / 60.0, &input, w as &dyn rc_movement::World);
                    let c = pawn.feet();
                    if v.contains([c.x, c.y, c.z + params.collision_height]) {
                        entered = true;
                    }
                }
                if entered {
                    *count += 1;
                }
            }
        }
        println!("{name}: walked at {tried} volumes; got inside {inside_with} with volumes in the collision, {inside_without} without");
        totals[0] += tried;
        totals[1] += inside_with;
        totals[2] += inside_without;
    }
    println!(
        "TOTAL: walked at {}; inside with volumes {}, without {}",
        totals[0], totals[1], totals[2]
    );
}
