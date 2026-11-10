use super::*;

const FILE: &str = r#"
name = "Test turret"
classes = ["WookieeHeavyTurret"]

[model]
mesh = "WookieeHeavyTurret"
pose = "Fire"

[aim]
speed = 10000
min_yaw = -8194
max_yaw = 8194
min_pitch = -8194
max_pitch = 8194

[gunner]
height = 120
reach = 100
radius = 80
half_height = 125
exit = [-120, 0, -50]
weapon = "heavy_turret"
"#;

fn params() -> TurretParams {
    TurretParams::from_text(FILE).unwrap()
}

fn placed() -> PlacedTurret {
    PlacedTurret {
        export: 7,
        name: "WookieeHeavyTurret0".into(),
        class: "WookieeHeavyTurret".into(),
        tag: "WookieeCannon1".into(),
        location: [100.0, 200.0, 300.0],
        rotation: [0, 16384, 0],
        control_event: Some("MountedTurret4".into()),
        player_control_event: Some("PlayerOn".into()),
        release_event: Some("Released".into()),
        player_usable: None,
        min_yaw: None,
        max_yaw: None,
        min_pitch: None,
        max_pitch: Some(16384),
        unlimited_yaw: None,
        speed: None,
        exit_spot: None,
        exit_offset: None,
    }
}

#[test]
fn file_reads_and_rejects_misspellings() {
    let p = params();
    assert_eq!(p.model.yaw_bone, "pivotYaw");
    assert_eq!(p.model.package, "GameData/Animations/turrets.ukx");
    assert!(p.gunner.player_usable);
    assert_eq!(p.gunner.prompt, "PRESS @ TO MOUNT TURRET");
    let bad = FILE.replace("reach = 100", "raech = 100");
    let e = TurretParams::from_text(&bad).unwrap_err();
    assert!(e.contains("raech"), "{e}");
}

#[test]
fn the_map_overrides_the_class() {
    let t = Turret::new(placed(), "wookiee_heavy_turret", &params());
    assert_eq!(t.limits.max_pitch, 16384);
    assert_eq!(t.limits.min_pitch, -8194);
    assert_eq!(t.limits.speed, 10000.0);
    assert!(t.player_usable);
    let mut p = placed();
    p.player_usable = Some(false);
    assert!(!Turret::new(p, "x", &params()).player_usable);
}

#[test]
fn getting_on_sends_the_player_event_then_the_control_event() {
    let mut t = Turret::new(placed(), "x", &params());
    assert_eq!(
        t.mount(Gunner::Player),
        Mounted::On(vec!["PlayerOn".into(), "MountedTurret4".into()])
    );
    assert_eq!(t.gunner, Gunner::Player);
    assert!(!t.usable_by_player());
    // Someone else can't get on; getting on again does nothing new.
    assert_eq!(t.mount(Gunner::Character("Seq".into())), Mounted::Taken);
    assert_eq!(t.mount(Gunner::Player), Mounted::On(vec![]));
    assert_eq!(t.leave(), Mounted::Off(vec!["Released".into()]));
    assert_eq!(t.leave(), Mounted::NotOn);
    assert!(t.usable_by_player());
}

#[test]
fn a_character_sends_only_the_control_event() {
    let mut t = Turret::new(placed(), "x", &params());
    assert_eq!(
        t.mount(Gunner::Character("ScriptedSequence1".into())),
        Mounted::On(vec!["MountedTurret4".into()])
    );
    assert_eq!(t.mount(Gunner::Player), Mounted::Taken);
}

#[test]
fn the_player_cant_use_a_turret_that_isnt_usable() {
    let mut p = placed();
    p.player_usable = Some(false);
    let mut t = Turret::new(p, "x", &params());
    assert_eq!(t.mount(Gunner::Player), Mounted::NotUsable);
    // A character still can (the last level's Wookiee turrets).
    assert!(matches!(
        t.mount(Gunner::Character("S".into())),
        Mounted::On(_)
    ));
}

#[test]
fn turning_is_limited_by_speed_and_limits() {
    let mut t = Turret::new(placed(), "x", &params());
    // A huge mouse move in 0.1 s turns at most 1000 units.
    assert!(!t.turn(50000.0, 0.0, 0.1));
    assert_eq!(t.yaw, 1000.0);
    // Keep going: stops at the limit.
    for _ in 0..20 {
        t.turn(50000.0, 50000.0, 0.1);
    }
    assert_eq!(t.yaw, 8194.0);
    assert_eq!(t.pitch, 16384.0);
    assert!(t.turn(1.0, 0.0, 0.1));
    for _ in 0..40 {
        t.turn(-50000.0, -50000.0, 0.1);
    }
    assert_eq!(t.yaw, -8194.0);
    assert_eq!(t.pitch, -8194.0);
}

#[test]
fn a_whole_turn_of_limits_turns_all_the_way_round() {
    // yyy_06d's Wookiee turrets: MinYaw -65536, MaxYaw 65536.
    let mut p = placed();
    p.min_yaw = Some(-65536);
    p.max_yaw = Some(65536);
    let mut t = Turret::new(p, "x", &params());
    for _ in 0..50 {
        t.turn(50000.0, 0.0, 0.1);
    }
    // 50 * 1000 units = 50000, wrapped past half a turn.
    assert_eq!(t.yaw, 50000.0 - 65536.0);
}

#[test]
fn the_view_adds_the_turrets_own_facing() {
    let mut t = Turret::new(placed(), "x", &params());
    let (yaw, pitch) = t.view();
    assert!((yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    assert_eq!(pitch, 0.0);
    t.turn(-1000.0, 1000.0, 1.0);
    let (yaw, pitch) = t.view();
    assert!((yaw - (std::f32::consts::FRAC_PI_2 - to_radians(1000.0))).abs() < 1e-5);
    assert!((pitch - to_radians(1000.0)).abs() < 1e-5);
}

#[test]
fn the_gunner_sits_above_and_gets_off_behind() {
    let t = Turret::new(placed(), "x", &params());
    assert_eq!(t.gunner_body(), [100.0, 200.0, 420.0]);
    // Facing +Y (yaw 16384): 120 behind is -Y.
    let e = t.exit_point();
    assert!((e[0] - 100.0).abs() < 1e-3 && (e[1] - 80.0).abs() < 1e-3 && e[2] == 250.0);
    let mut p = placed();
    p.exit_spot = Some([1.0, 2.0, 3.0]);
    assert_eq!(Turret::new(p, "x", &params()).exit_point(), [1.0, 2.0, 3.0]);
}

#[test]
fn turning_toward_a_point_and_settling() {
    let mut t = Turret::new(placed(), "x", &params());
    // Facing +Y; a target to the front left (towards -X) at the same height.
    let from = [100.0, 200.0, 300.0];
    for _ in 0..30 {
        t.turn_toward(from, [0.0, 300.0, 300.0], 0.1);
    }
    assert!((t.yaw - 8192.0).abs() < 5.0, "{}", t.yaw);
    assert!(t.pitch.abs() < 1.0);
    let mut settled = false;
    for _ in 0..30 {
        settled = t.settle(0.1);
    }
    assert!(settled);
}

#[test]
fn scripts_find_the_first_turret_with_a_tag() {
    let p = params();
    let mut a = placed();
    a.name = "WookieeHeavyTurret4".into();
    let b = placed();
    let list = vec![Turret::new(a, "x", &p), Turret::new(b, "x", &p)];
    assert_eq!(first_tagged(&list, "wookieecannon1"), Some(0));
    assert_eq!(first_tagged(&list, "nothing"), None);
}

#[test]
fn wrap_keeps_half_a_turn_each_way() {
    assert_eq!(wrap(32768.0), 32768.0);
    assert_eq!(wrap(32769.0), -32767.0);
    assert_eq!(wrap(-40000.0), 25536.0);
}

#[test]
fn a_squad_marker_finds_its_turret_by_name_or_nearby() {
    let p = params();
    let mut a = placed();
    a.name = "WookieeHeavyTurret0".into();
    let mut b = placed();
    b.name = "WookieeHeavyTurret4".into();
    b.location = [2000.0, 0.0, 300.0];
    let list = vec![Turret::new(a, "x", &p), Turret::new(b, "x", &p)];
    assert_eq!(
        for_marker(&list, "wookieeheavyturret4", [0.0; 3], 500.0),
        Some(1)
    );
    // An anchor that isn't a turret: the nearest one close enough.
    assert_eq!(
        for_marker(&list, "Something", [1900.0, 50.0, 250.0], 500.0),
        Some(1)
    );
    assert_eq!(for_marker(&list, "", [5000.0, 0.0, 0.0], 500.0), None);
}

#[test]
fn a_squad_mate_gets_on_like_a_character_and_saves_by_name() {
    let mut t = Turret::new(placed(), "x", &params());
    let sev = Gunner::Squad("Sev".into());
    assert!(matches!(t.mount(sev.clone()), Mounted::On(_)));
    // Asking again (after loading a save) sends nothing more.
    assert_eq!(t.mount(sev.clone()), Mounted::On(Vec::new()));
    assert_eq!(t.mount(Gunner::Player), Mounted::Taken);
    assert!(!t.usable_by_player());
    for g in [
        Gunner::Nobody,
        Gunner::Player,
        sev,
        Gunner::Character("WookieeTurretScript".into()),
    ] {
        assert_eq!(Gunner::from_save_name(&g.save_name()), g);
    }
}
