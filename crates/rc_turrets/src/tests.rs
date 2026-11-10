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
        event: None,
        first_damaged_event: None,
        deployed: None,
        health: None,
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

const AUTOMATIC: &str = r#"
name = "Test automatic turret"
classes = ["CoreShipTurret"]

[model]
mesh = "CoreShipTurret"
pose = "Fire"

[aim]
speed = 40000
min_yaw = -65535
max_yaw = 65535
min_pitch = -65535
max_pitch = 65535
unlimited_yaw = true

[gunner]
height = 0
player_usable = false
reach = 0
radius = 60
half_height = 90
exit = [0, 0, 0]

[auto]
weapon = "core_ship_turret"
shot_delay = 0.15
range = 10000

[automatic]
fire_for = 4
cool_down = 1
health = 256
damage_table = "DV_TurretsCoreshipAuto"
"#;

fn automatic() -> Turret {
    let p = TurretParams::from_text(AUTOMATIC).unwrap();
    let mut place = placed();
    place.class = "CoreShipTurret".into();
    place.tag = "BDTurrets".into();
    place.event = Some("Security00".into());
    place.first_damaged_event = Some("TurretsShot".into());
    Turret::new(place, "core_ship_turret", &p)
}

#[test]
fn an_automatic_file_needs_a_weapon_and_a_known_side() {
    let p = TurretParams::from_text(AUTOMATIC).unwrap();
    let a = p.automatic.as_ref().unwrap();
    assert!(!a.deployed);
    assert_eq!(a.side, "separatist");
    let no_weapon = AUTOMATIC.replace("weapon = \"core_ship_turret\"\n", "");
    assert!(TurretParams::from_text(&no_weapon).is_err());
    let bad_side = AUTOMATIC.replace("[automatic]\n", "[automatic]\nside = \"droids\"\n");
    assert!(TurretParams::from_text(&bad_side).is_err());
}

#[test]
fn an_automatic_turret_waits_for_its_event_and_the_next_one_switches_it_off() {
    let mut t = automatic();
    assert!(!t.is_working());
    // Off: it doesn't fire even with a target.
    assert!(!t.auto_fire(0.0, true, 0.15, 4.0, 1.0));
    assert_eq!(t.trigger(1.0, 2.0), Switched::On);
    assert!(t.is_working());
    // Unfolding for 2 s: no aiming or firing yet.
    assert!(!t.is_ready(2.5));
    assert!(!t.auto_fire(2.5, true, 0.15, 4.0, 1.0));
    assert!(t.is_ready(3.0));
    assert!(t.auto_fire(3.0, true, 0.15, 4.0, 1.0));
    assert_eq!(t.trigger(4.0, 2.0), Switched::Off);
    assert!(!t.is_working());
    // A map can deploy it from the start.
    let p = TurretParams::from_text(AUTOMATIC).unwrap();
    let mut place = placed();
    place.deployed = Some(true);
    assert!(Turret::new(place, "x", &p).is_working());
}

#[test]
fn nobody_gets_on_an_automatic_turret() {
    let mut t = automatic();
    t.player_usable = true;
    assert!(!t.usable_by_player());
    assert_eq!(t.mount(Gunner::Player), Mounted::NotUsable);
    assert_eq!(t.mount(Gunner::Character("S".into())), Mounted::NotUsable);
}

#[test]
fn it_fires_every_shot_delay_then_cools_down() {
    let mut t = automatic();
    t.trigger(0.0, 0.0);
    let dt = 0.01;
    let mut shots = Vec::new();
    let mut now = 0.0f32;
    while now < 6.0 {
        if t.auto_fire(now, true, 0.15, 4.0, 1.0) {
            shots.push(now);
        }
        now += dt;
    }
    // First shot at once, then one every 0.15 s or a frame more.
    assert_eq!(shots[0], 0.0);
    let gap = shots[1] - shots[0];
    assert!((0.15..0.17).contains(&gap), "{gap}");
    // Nothing between 4 s (cooling down starts) and 5 s (it ends).
    let quiet: Vec<_> = shots.iter().filter(|&&s| s > 4.01 && s < 4.99).collect();
    assert!(quiet.is_empty(), "{quiet:?}");
    assert!(shots.iter().any(|&s| s >= 5.0), "fires again after cooling");
    let before = shots.iter().filter(|&&s| s < 4.0).count();
    assert!((24..=27).contains(&before), "{before} shots in 4 s");
}

#[test]
fn no_target_no_shots() {
    let mut t = automatic();
    t.trigger(0.0, 0.0);
    assert!(!t.auto_fire(0.0, false, 0.15, 4.0, 1.0));
    assert!(t.automatic.as_ref().unwrap().firing_since.is_none());
}

#[test]
fn the_first_hit_and_destruction_send_their_events_once() {
    let mut t = automatic();
    assert_eq!(t.hurt(0.0), Hurt::default(), "no damage, no event");
    let h = t.hurt(100.0);
    assert_eq!(h.events, vec!["TurretsShot".to_string()]);
    assert!(!h.destroyed);
    assert!(t.hurt(100.0).events.is_empty());
    let h = t.hurt(100.0);
    assert!(h.destroyed);
    assert_eq!(h.events, vec!["Security00".to_string()]);
    assert!(!t.is_working());
    assert_eq!(
        t.trigger(9.0, 0.0),
        Switched::Nothing,
        "a destroyed turret stays off"
    );
    assert_eq!(t.hurt(100.0), Hurt::default());
}

#[test]
fn a_turret_with_no_health_cant_be_hurt() {
    let p = TurretParams::from_text(AUTOMATIC).unwrap();
    let mut place = placed();
    place.health = Some(0.0);
    let mut t = Turret::new(place, "x", &p);
    assert_eq!(t.hurt(1000.0), Hurt::default());
    // A turret a gunner works isn't hurt this way either.
    let mut g = Turret::new(placed(), "w", &params());
    assert_eq!(g.hurt(1000.0), Hurt::default());
}
