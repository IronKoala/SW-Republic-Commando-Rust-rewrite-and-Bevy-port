use super::*;

/// The turn of a placed actor matches how its drawn model is turned
/// (Unreal axes swapped to Bevy's): yaw 90 degrees takes the model's X
/// (forward) to Unreal Y, which is Bevy Z.
#[test]
fn turn_matches_unreal_axes() {
    let q = turn([0, 16384, 0]);
    let fwd = q * Vec3::X;
    assert!((fwd - Vec3::Z).length() < 1e-4, "{fwd}");
    // And points: local (1, 2, 3) uu, turned by the same rotation in
    // Unreal, then swapped, equals turning the swapped point.
    let rot = [1000, 20000, -3000];
    let ax = rc_level::actors::rotation_axes(rot);
    let l = [1.0, 2.0, 3.0];
    let w = [0, 1, 2].map(|i| l[0] * ax[0][i] + l[1] * ax[1][i] + l[2] * ax[2][i]);
    let got = turn(rot) * sw(l);
    assert!((got - sw(w)).length() < 1e-4, "{got} vs {:?}", sw(w));
}

/// A diagonal tensor swaps its Y and Z terms; mass and scale multiply.
#[test]
fn inertia_swaps_y_and_z() {
    let m = inertia([1.0, 0.0, 0.0, 2.0, 0.0, 3.0], 2.0, 1.0);
    assert_eq!(m.x_axis, Vec3::new(2.0, 0.0, 0.0));
    assert_eq!(m.y_axis, Vec3::new(0.0, 6.0, 0.0));
    assert_eq!(m.z_axis, Vec3::new(0.0, 0.0, 4.0));
    let m = inertia([1.0, 0.0, 0.0, 1.0, 0.0, 1.0], 1.0, 2.0);
    assert_eq!(m.x_axis.x, 4.0);
}

/// Karma: no friction below 0.01.
#[test]
fn tiny_friction_is_none() {
    assert_eq!(friction(0.005), 0.0);
    assert_eq!(friction(0.4), 0.4);
}

/// A convex hull becomes one Avian shape; a box with an offset a compound.
#[test]
fn shapes_become_colliders() {
    let cube: Vec<[f32; 3]> = (0..8)
        .map(|i| {
            [
                if i & 1 == 0 { -50.0 } else { 50.0 },
                if i & 2 == 0 { -50.0 } else { 50.0 },
                if i & 4 == 0 { -50.0 } else { 50.0 },
            ]
        })
        .collect();
    assert!(collider(&[Shape::Convex(cube)]).is_some());
    let b = Shape::Box {
        centre: [0.0, 0.0, 10.0],
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        size: [100.0, 50.0, 20.0],
    };
    let c = collider(&[b]).unwrap();
    let aabb = c.aabb(Vec3::ZERO, Quat::IDENTITY, 0.0);
    // 100 x 50 x 20 uu, 10 uu up: Bevy 2 x 0.4 x 1, raised 0.2.
    assert!((aabb.max.x - 1.0).abs() < 1e-3);
    assert!((aabb.max.y - 0.4).abs() < 1e-3, "{aabb:?}");
    assert!((aabb.max.z - 0.5).abs() < 1e-3);
    assert!(collider(&[]).is_none());
}
