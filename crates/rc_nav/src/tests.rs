use crate::*;
use rc_movement::room::{test_room, LANES_X, LANE_STAIRS};
use rc_movement::{MoveParams, Pawn, Vec3};

fn node(name: &str, x: f32, y: f32, z: f32) -> NavNode {
    NavNode {
        name: name.into(),
        class: "PathNode".into(),
        tag: name.into(),
        location: Vec3::new(x, y, z),
        export: 0,
        kind: NodeKind::Path,
    }
}

fn two_way(g: &mut NavGraph, a: usize, b: usize) {
    let d = (g.nodes[a].location - g.nodes[b].location).length();
    g.links[a].push(Link { to: b, distance: d, flags: reach::WALK });
    g.links[b].push(Link { to: a, distance: d, flags: reach::WALK });
}

/// A square A-B-C-D with a long way round (A-B-C) and a short cut (A-D-C)
/// that is a fly-only link from D to C.
fn square() -> NavGraph {
    let mut g = NavGraph {
        nodes: vec![
            node("A", 0.0, 0.0, 84.0),
            node("B", 1000.0, 0.0, 84.0),
            node("C", 1000.0, 1000.0, 84.0),
            node("D", 0.0, 1000.0, 84.0),
            node("Island", 5000.0, 5000.0, 84.0),
        ],
        links: vec![Vec::new(); 5],
        ..Default::default()
    };
    two_way(&mut g, 0, 1);
    two_way(&mut g, 1, 2);
    two_way(&mut g, 0, 3);
    g.links[3].push(Link { to: 2, distance: 1000.0, flags: reach::FLY });
    g
}

#[test]
fn shortest_walkable_route() {
    let g = square();
    // A to C: A-D-C is the same length but D-C can only be flown.
    assert_eq!(g.node_path(0, 2), Some(vec![0, 1, 2]));
    // C to D: C-B-A-D (no walkable C-D link).
    assert_eq!(g.node_path(2, 3), Some(vec![2, 1, 0, 3]));
    assert_eq!(g.node_path(0, 4), None);
    assert_eq!(g.islands(), (2, 4));
}

#[test]
fn avoiding_a_node_takes_the_other_way() {
    let mut g = square();
    two_way(&mut g, 3, 2); // make D-C walkable too
    let p = g.node_path_avoiding(0, 2, |n| if n == 1 { 5000.0 } else { 0.0 }).unwrap();
    assert_eq!(p, vec![0, 3, 2]);
}

#[test]
fn blocked_nodes_are_avoided_until_unblocked() {
    let mut g = square();
    two_way(&mut g, 3, 2);
    g.nodes[1].tag = "Barricade".into();
    assert_eq!(g.set_blocked_by_tag("barricade", true), 1);
    assert_eq!(g.node_path(0, 2), Some(vec![0, 3, 2]));
    assert_eq!(g.node_path(0, 1), None);
    g.set_blocked_by_tag("Barricade", false);
    assert_eq!(g.node_path(0, 1), Some(vec![0, 1]));
}

#[test]
fn route_points_end_at_the_goal() {
    let g = square();
    let from = Vec3::new(-50.0, 10.0, 84.0);
    let to = Vec3::new(1010.0, 990.0, 84.0);
    let r = g.route(from, to).unwrap();
    assert_eq!(r.nodes, vec![0, 1, 2]);
    assert_eq!(*r.points.last().unwrap(), to);
    assert!((r.length - (51.0 + 1000.0 + 1000.0 + 14.1)).abs() < 2.0, "{}", r.length);
    assert_eq!(g.find_path(from, to).unwrap(), r.points);
}

#[test]
fn nearest_node_prefers_the_same_floor() {
    let mut g = square();
    g.nodes.push(node("Above", 0.0, 0.0, 84.0 + 300.0));
    g.links.push(Vec::new());
    // 250 up and 0 sideways from A's spot is nearer "Above" in straight
    // line terms (50 vs 250), but height counts double: A wins only when
    // closer; check both ways.
    assert_eq!(g.nearest_node(Vec3::new(0.0, 0.0, 84.0 + 100.0)), Some(0));
    assert_eq!(g.nearest_node(Vec3::new(0.0, 0.0, 84.0 + 250.0)), Some(5));
}

/// Walk a battle-droid-sized pawn up the test room's stairs along a route
/// using the player's physics. Checks the follower reaches the end and
/// isn't slower than the droid's top speed allows.
#[test]
fn follower_walks_a_pawn_up_the_stairs() {
    let world = test_room();
    let mut params = MoveParams::player_commando();
    params.ground_speed = 375.0; // battle droid
    params.accel_rate = 1000.0;
    params.collision_radius = 35.0;
    params.collision_height = 80.0;
    let mut pawn = Pawn::new(params, Vec3::new(LANES_X - 300.0, LANE_STAIRS, 0.0));
    let top = Vec3::new(LANES_X + 400.0, LANE_STAIRS, 128.0 + 80.0);
    let points = vec![Vec3::new(LANES_X - 60.0, LANE_STAIRS, 80.0), top];
    let mut f = PathFollower::new(points);
    let dt = 1.0 / 60.0;
    let mut t = 0.0;
    while t < 10.0 {
        match f.update(&pawn, dt) {
            Some(input) => {
                pawn.step(dt, &input, &world);
            }
            None => break,
        }
        t += dt;
    }
    assert!(f.finished(), "not finished after {t:.1}s at {:?}", pawn.location);
    assert!((pawn.feet().z - 128.0).abs() < 2.0, "feet at {}", pawn.feet().z);
    // About 700 uu (less the arrival radius) at 375 uu/s is 1.9 s plus speeding up; allow up to 3 s.
    assert!(t < 3.0, "took {t:.2}s");
    assert!(f.travelled > 660.0 && f.travelled < 760.0, "travelled {}", f.travelled);
}

#[test]
fn steer_faces_the_target() {
    let pawn = Pawn::new(MoveParams::player_commando(), Vec3::ZERO);
    let (input, arrived) = steer(&pawn, Vec3::new(0.0, 500.0, 84.0), 24.0, false);
    assert!(!arrived);
    assert!((input.yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
    assert_eq!(input.forward, 1.0);
    let (_, arrived) = steer(&pawn, pawn.location + Vec3::new(10.0, 0.0, 0.0), 24.0, false);
    assert!(arrived);
}
