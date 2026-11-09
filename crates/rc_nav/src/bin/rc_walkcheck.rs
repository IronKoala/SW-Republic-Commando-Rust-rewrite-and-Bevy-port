//! rc_walkcheck: find places in a level where the player gets stuck while
//! walking.
//!
//! Usage: rc_walkcheck [MAP] [--near X,Y,Z RADIUS] [--fps N] [--link FROM TO]  (default geo_01a; a
//! name from GameData/Maps or a path to a .ctm file). The game folder is
//! RC_GAME_DIR or the default Steam path. Writes logs/rc_walkcheck.log.
//!
//! The level designers linked the AI path points with walkable links: an
//! AI can walk straight from one end to the other. So the player should be
//! able to as well. For every walkable link, a player (with the game's
//! movement, `rc_movement`) starts standing on one end and runs at the other
//! for twice the time it should take, steering straight at it every frame
//! (60 frames a second, or `--fps`). The run fails if the player doesn't arrive. It also
//! counts the sweeps that found the player's box already inside the level
//! ("stuck inside"), which the movement code can only creep out of.
//!
//! Exits with code 1 if any run that should work fails.

use rc_movement::{Hit, Mode, MoveInput, MoveParams, Pawn, TriangleWorld, Vec3, World};
use rc_nav::reach as flags;
use rc_nav::NavGraph;
use rc_package::Package;
use std::cell::Cell;
use std::io::Write;
use std::path::PathBuf;

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";

/// The level, counting the sweeps that start inside it.
struct Counting<'a> {
    world: &'a TriangleWorld,
    inside: Cell<u32>,
}

impl World for Counting<'_> {
    fn sweep(&self, extent: Vec3, start: Vec3, end: Vec3) -> Option<Hit> {
        let h = self.world.sweep(extent, start, end);
        if h.is_some_and(|h| h.start_solid) {
            self.inside.set(self.inside.get() + 1);
        }
        h
    }
}

struct Run {
    arrived: bool,
    secs: f32,
    inside: u32,
    /// Where the player was going slowest (after the first half second).
    slowest_at: Vec3,
    slowest_speed: f32,
    end: Vec3,
}

fn run(world: &TriangleWorld, params: &MoveParams, from: Vec3, to: Vec3, fps: f32) -> Run {
    let w = Counting { world, inside: Cell::new(0) };
    // Path points are where a character's centre stands; drop onto the
    // floor below first, like the game does when it spawns someone.
    let mut pawn = Pawn::new(params.clone(), from - Vec3::new(0.0, 0.0, params.collision_height));
    let still = MoveInput::default();
    for _ in 0..30 {
        pawn.step(1.0 / 60.0, &still, &w);
    }
    w.inside.set(0);
    let flat = |v: Vec3| Vec3::new(v.x, v.y, 0.0);
    let dist = (flat(to) - flat(pawn.location)).length();
    let limit = 2.0 * dist / params.ground_speed + 1.0;
    let mut t = 0.0;
    let mut slowest = (f32::MAX, pawn.location);
    while t < limit {
        let d = flat(to) - flat(pawn.location);
        if d.length() < params.collision_radius {
            return Run {
                arrived: true,
                secs: t,
                inside: w.inside.get(),
                slowest_at: slowest.1,
                slowest_speed: if slowest.0 == f32::MAX { 0.0 } else { slowest.0 },
                end: pawn.feet(),
            };
        }
        let input = MoveInput { forward: 1.0, yaw: d.y.atan2(d.x), ..Default::default() };
        pawn.step(1.0 / fps, &input, &w);
        t += 1.0 / fps;
        let speed = pawn.velocity.flat().length();
        if t > 0.5 && pawn.mode == Mode::Walking && speed < slowest.0 {
            slowest = (speed, pawn.feet());
        }
    }
    Run {
        arrived: false,
        secs: t,
        inside: w.inside.get(),
        slowest_at: slowest.1,
        slowest_speed: if slowest.0 == f32::MAX { 0.0 } else { slowest.0 },
        end: pawn.feet(),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = args.get(1).filter(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| "geo_01a".into());
    let mut near: Option<(Vec3, f32)> = None;
    if let Some(i) = args.iter().position(|a| a == "--near") {
        let p: Vec<f32> = args[i + 1].split(',').map(|s| s.parse().unwrap()).collect();
        near = Some((Vec3::new(p[0], p[1], p[2]), args[i + 2].parse().unwrap()));
    }
    let fps: f32 = args
        .iter()
        .position(|a| a == "--fps")
        .map(|i| args[i + 1].parse().unwrap())
        .unwrap_or(60.0);
    let only: Option<(String, String)> = args
        .iter()
        .position(|a| a == "--link")
        .map(|i| (args[i + 1].clone(), args[i + 2].clone()));
    let game = PathBuf::from(std::env::var("RC_GAME_DIR").unwrap_or_else(|_| DEFAULT_GAME_DIR.into()));
    let gd = game.join("GameData");
    let path = if arg.ends_with(".ctm") {
        PathBuf::from(&arg)
    } else {
        gd.join("Maps").join(format!("{arg}.ctm"))
    };
    let _ = std::fs::create_dir_all("logs");
    let mut log = std::fs::File::create("logs/rc_walkcheck.log").ok();
    let mut line = |s: String| {
        println!("{s}");
        if let Some(f) = log.as_mut() {
            let _ = writeln!(f, "{s}");
        }
    };

    let params = MoveParams::player_commando();
    let lvl = match rc_level::map_collision::load(&path, &gd, params.collision_height) {
        Ok(l) => l,
        Err(e) => {
            line(format!("can't load {}: {e}", path.display()));
            std::process::exit(1);
        }
    };
    let v = |p: [f32; 3]| Vec3::new(p[0], p[1], p[2]);
    // The level plus the parts scripts may switch off (they start solid).
    let mut tris: Vec<[Vec3; 3]> = lvl.triangles.iter().map(|t| [v(t[0]), v(t[1]), v(t[2])]).collect();
    for ts in lvl.part_triangles.values() {
        tris.extend(ts.iter().map(|t| [v(t[0]), v(t[1]), v(t[2])]));
    }
    let world = TriangleWorld::new(tris);
    let data = std::fs::read(&path).unwrap();
    let pkg = Package::parse(&data).unwrap();
    let graph = NavGraph::from_map(&pkg, &data).unwrap();
    if let Some((feet, _)) = lvl.start {
        line(format!("player start feet ({:.0}, {:.0}, {:.0})", feet[0], feet[1], feet[2]));
    }
    line(format!(
        "{}: {} collision triangles, {} path points, {} links",
        path.display(),
        world.triangles.len(),
        graph.nodes.len(),
        graph.link_count()
    ));

    let (mut tried, mut failed, mut inside_runs) = (0, 0, 0);
    let t0 = std::time::Instant::now();
    for (a, links) in graph.links.iter().enumerate() {
        for l in links {
            let (na, nb) = (&graph.nodes[a], &graph.nodes[l.to]);
            if !flags::walkable(l.flags) || l.flags & flags::DOOR != 0 || l.distance < 60.0 {
                continue;
            }
            // Jump points are for jumping, not walking.
            if na.class == "JumpDest" || nb.class == "JumpDest" {
                continue;
            }
            if graph.blocked.contains(&a) || graph.blocked.contains(&l.to) {
                continue;
            }
            if let Some(only) = &only {
                if na.name != only.0 || nb.name != only.1 {
                    continue;
                }
            }
            if let Some((c, r)) = near {
                if (na.location - c).length() > r && (nb.location - c).length() > r {
                    continue;
                }
            }
            // Big drops or climbs are jumps or falls, not walks.
            if (nb.location.z - na.location.z).abs() > 0.8 * l.distance {
                continue;
            }
            tried += 1;
            let r = run(&world, &params, na.location, nb.location, fps);
            if r.inside > 0 {
                inside_runs += 1;
            }
            // Running flat out takes distance / speed, plus a moment to
            // get going. Much longer than that means something held the
            // player back.
            let ideal = l.distance / params.ground_speed + 0.3;
            if !r.arrived || r.inside > 0 || r.secs > 1.5 * ideal || near.is_some() {
                if !r.arrived {
                    failed += 1;
                }
                line(format!(
                    "{} {} -> {} ({:.0} uu): {:.2} s, stuck inside {} times, slowest {:.0} uu/s at ({:.0}, {:.0}, {:.0}), ended at ({:.0}, {:.0}, {:.0})",
                    if !r.arrived {
                        "FAIL"
                    } else if r.inside > 0 || r.secs > 1.5 * ideal {
                        "SLOW"
                    } else {
                        "OK  "
                    },
                    na.name,
                    nb.name,
                    l.distance,
                    r.secs,
                    r.inside,
                    r.slowest_speed,
                    r.slowest_at.x,
                    r.slowest_at.y,
                    r.slowest_at.z,
                    r.end.x,
                    r.end.y,
                    r.end.z
                ));
            }
        }
    }
    line(format!(
        "SUMMARY: {tried} walks, {failed} didn't arrive, {inside_runs} were stuck inside the level at least once ({:.1} s)",
        t0.elapsed().as_secs_f32()
    ));
    if failed > 0 {
        std::process::exit(1);
    }
}
