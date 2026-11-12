//! The combat plugin in a Bevy app with no window: the player fires the
//! game's weapons at a target in the movement test room, and a droid fires
//! back. Checks the numbers, not the picture.

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use rc_combat::{
    Arsenal, Blast, Body, CombatPlugin, Damage, DamageTable, Dead, Died, FireShot, Grenades,
    Health, Hittable, Shield, ShotLanded, Stunned, Team, ThrowGrenade, WeaponLibrary,
};
use rc_game_core::{
    Action, ActionState, CorePlugin, GamePaths, GameState, LevelCollision, Player, PlayerCamera,
    PlayerVitals, WeaponReadout,
};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

const STEP: f32 = 1.0 / 60.0;

/// The battle droid's damage table (`DamageVariance`), from the game.
const DROID_TABLE: &str = "DV_DroidsBattleDroidDamage";

/// Unreal position to Bevy (same rule as rc_game_core::units).
fn b(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, z, y) / 50.0
}

#[derive(Resource, Default)]
struct Seen {
    landed: Vec<ShotLanded>,
    died: Vec<Died>,
    damage: Vec<Damage>,
    blasts: Vec<Blast>,
    taken: Vec<rc_combat::WeaponTaken>,
}

fn record(
    mut seen: ResMut<Seen>,
    mut l: MessageReader<ShotLanded>,
    mut d: MessageReader<Died>,
    mut h: MessageReader<Damage>,
    mut x: MessageReader<Blast>,
    mut t: MessageReader<rc_combat::WeaponTaken>,
) {
    seen.taken.extend(t.read().cloned());
    seen.blasts.extend(x.read().cloned());
    seen.landed.extend(l.read().cloned());
    seen.died.extend(d.read().cloned());
    seen.damage.extend(h.read().cloned());
}

struct Range {
    app: App,
    player: Entity,
    droid: Entity,
}

impl Range {
    fn new() -> Range {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, TransformPlugin))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
                STEP,
            )))
            .add_plugins((CorePlugin, CombatPlugin))
            // No game folder: no sounds or models, which these checks don't need.
            .insert_resource(GamePaths::new("/nonexistent-game", vec![data]))
            .insert_resource(LevelCollision(Arc::new(rc_movement::room::test_room())))
            .init_resource::<Seen>()
            .add_systems(Last, record);
        // The player stands at (-500, 0) facing +X, eye 144 uu above the feet.
        let player = app
            .world_mut()
            .spawn((Player, Transform::from_translation(b(-500.0, 0.0, 0.0))))
            .id();
        app.world_mut().spawn((
            PlayerCamera,
            Transform::from_translation(b(-500.0, 0.0, 144.0)).looking_to(Vec3::X, Vec3::Y),
        ));
        // A target 1000 uu ahead with a battle droid's numbers.
        let droid = app
            .world_mut()
            .spawn((
                Name::new("droid"),
                Health::new(100.0),
                Team::Separatist,
                Body::Droid,
                DamageTable(DROID_TABLE.into()),
                Hittable {
                    radius: 40.0,
                    half_height: 84.0,
                    center_up: 84.0,
                },
                Transform::from_translation(b(500.0, 0.0, 0.0)),
            ))
            .id();
        app.world_mut()
            .resource_mut::<NextState<GameState>>()
            .set(GameState::InGame);
        let mut r = Range { app, player, droid };
        r.run(3, &[]);
        r
    }

    /// Run `frames` frames with `held` actions held.
    fn run(&mut self, frames: usize, held: &[Action]) {
        for _ in 0..frames {
            let now: HashSet<Action> = held.iter().copied().collect();
            self.app
                .world_mut()
                .resource_mut::<ActionState>()
                .update(now, Vec2::ZERO, Vec2::ZERO);
            self.app.update();
        }
    }

    fn seconds(&mut self, s: f32, held: &[Action]) {
        self.run((s / STEP).round() as usize, held);
    }

    fn arsenal(&self) -> Arsenal {
        self.app
            .world()
            .get::<Arsenal>(self.player)
            .unwrap()
            .clone()
    }

    /// The droid's damage table multiplier for damage type `ty`.
    fn droid_takes(&self, ty: &str) -> f32 {
        self.app
            .world()
            .resource::<WeaponLibrary>()
            .damage_types
            .multiplier(Some(DROID_TABLE), ty)
    }

    fn seen(&mut self) -> Seen {
        std::mem::take(&mut *self.app.world_mut().resource_mut::<Seen>())
    }
}

#[test]
fn every_weapon_file_reads() {
    let r = Range::new();
    let lib = r.app.world().resource::<WeaponLibrary>();
    assert!(lib.errors.is_empty(), "{:?}", lib.errors);
    // The turrets' weapons: twin barrels, cooling down after a few shots.
    let heavy = &lib.weapons["heavy_turret"];
    assert!(heavy.mirror_fire);
    assert_eq!((heavy.clip_size, heavy.reload_time), (3, 3.0));
    assert_eq!(heavy.grenade.as_deref(), Some("turret_shell"));
    assert!(!lib.weapons["missile_turret"].mirror_fire);
    let ids: Vec<_> = lib.weapons.keys().cloned().collect();
    assert_eq!(
        ids,
        [
            "acp_array_gun",
            "bowcaster",
            "concussion_rifle",
            "core_ship_turret",
            "dc15_blaster_clone",
            "dc15_blaster_clone_tm",
            "dc15s_pistol",
            "dc17m_antiarmor",
            "dc17m_blaster",
            "dc17m_sniper",
            "droideka_blaster",
            "e5_blaster",
            "e5_blaster_pickup",
            "e5_blaster_tm",
            "elite_beam",
            "elite_beam_enemy",
            "eweb_turret",
            "grievous_missile_rack",
            "gunship_turret",
            "heavy_repeater_mp",
            "heavy_turret",
            "missile_turret",
            "rocket_launcher",
            "sbd_blaster",
            "scav_beam",
            "spider_droid_cannon",
            "trandoshan_rifle",
            "trandoshan_shotgun",
            "trandoshan_smg"
        ]
    );
}

#[test]
fn player_gets_the_games_numbers() {
    let r = Range::new();
    let w = r.app.world();
    assert_eq!(*w.get::<Health>(r.player).unwrap(), Health::new(200.0));
    let s = w.get::<Shield>(r.player).unwrap();
    assert_eq!(
        (s.current, s.recharge_rate, s.recharge_delay),
        (100.0, 20.0, 3.0)
    );
    let a = r.arsenal();
    assert_eq!(a.weapons[0].0, "dc17m_blaster");
    // Given full at the start, as the original's Weapon.GiveAmmo does.
    assert_eq!(
        (a.weapons[0].1.shots_in_clip(), a.weapons[0].1.reserve),
        (60, 240)
    );
    let v = w.resource::<PlayerVitals>();
    assert_eq!((v.health, v.shield), (200.0, 100.0));
    let ro = w.resource::<WeaponReadout>();
    assert_eq!(
        (ro.clip, ro.clip_size, ro.reserve),
        (Some(60), Some(60), Some(240))
    );
    assert!(ro.on_target, "the crosshair is on the droid");
}

#[test]
fn dc17m_kills_a_droid_in_five_hits() {
    let mut r = Range::new();
    r.seen();
    r.seconds(1.0, &[Action::Fire]);
    r.seconds(1.0, &[]);
    let a = r.arsenal();
    let fired = a.weapons[0].1.shots_fired;
    assert!(
        (10..=11).contains(&fired),
        "about 10 shots a second, got {fired}"
    );
    let seen = r.seen();
    let hits = seen.damage.iter().filter(|d| d.target == r.droid).count();
    assert_eq!(
        hits, 5,
        "100 health / 20 damage = 5 hits, then the droid is down"
    );
    assert!(seen.damage.iter().all(|d| d.amount == 20.0));
    assert_eq!(seen.died.len(), 1);
    assert_eq!(seen.died[0].entity, r.droid);
    assert_eq!(seen.died[0].killer, Some(r.player));
    assert!(r.app.world().get::<Dead>(r.droid).is_some());
    // The other shots fly past the fallen droid into the far wall (x = 4000).
    let walls: Vec<_> = seen.landed.iter().filter(|l| l.on_level).collect();
    assert_eq!(walls.len(), fired as usize - 5);
    for l in &walls {
        let x = l.point.x * 50.0;
        assert!((x - 4000.0).abs() < 1.0, "wall hit at x {x}");
    }
}

#[test]
fn shots_land_around_the_crosshair() {
    let mut r = Range::new();
    r.app.world_mut().entity_mut(r.droid).insert(Dead);
    r.seen();
    r.seconds(2.0, &[Action::Fire]);
    r.seconds(1.0, &[]);
    let seen = r.seen();
    assert!(
        (20..=21).contains(&seen.landed.len()),
        "{}",
        seen.landed.len()
    );
    // The crosshair is on the far wall at (4000, 0, 144). Bolts leave the
    // muzzle (127 forward, 30 right, 10 down from the eye) and fly to it,
    // off by up to 15 uu per 1000 uu: about 66 uu at 4370 uu.
    let (mut most_y, mut most_z) = (0.0f32, 0.0f32);
    for l in &seen.landed {
        let (x, y, z) = (l.point.x * 50.0, l.point.z * 50.0, l.point.y * 50.0);
        assert!((x - 4000.0).abs() < 1.0);
        most_y = most_y.max(y.abs());
        most_z = most_z.max((z - 144.0).abs());
    }
    assert!(most_y < 67.0 && most_z < 67.0, "spread {most_y} {most_z}");
    assert!(
        most_y > 20.0 && most_z > 20.0,
        "there is some spread: {most_y} {most_z}"
    );
}

#[test]
fn bolts_take_time_to_arrive() {
    let mut r = Range::new();
    r.seen();
    r.run(1, &[Action::Fire]);
    // The droid's near side is ~960 uu ahead of the eye, the muzzle 127 uu
    // ahead: ~830 uu, which a bolt from rest at 60000 uu/s² covers in
    // about 0.166 s (10 frames).
    let mut frames = 0;
    while r.app.world().resource::<Seen>().damage.is_empty() {
        r.run(1, &[]);
        frames += 1;
        assert!(frames < 60);
    }
    assert!((9..=12).contains(&frames), "hit after {frames} frames");
}

#[test]
fn magazine_empties_and_reloads() {
    let mut r = Range::new();
    // Move the droid away so it doesn't matter.
    r.app.world_mut().entity_mut(r.droid).insert(Dead);
    r.seconds(7.0, &[Action::Fire]);
    let a = r.arsenal();
    let w = &a.weapons[0].1;
    // 60 shots in 6 s, then a 1.5 s reload has started.
    assert_eq!(w.shots_fired, 60);
    assert!(w.reloading.is_some());
    r.seconds(1.6, &[]);
    let a = r.arsenal();
    assert_eq!(
        (a.weapons[0].1.shots_in_clip(), a.weapons[0].1.reserve),
        (60, 180)
    );
    let ro = r.app.world().resource::<WeaponReadout>();
    assert_eq!((ro.clip, ro.reserve), (Some(60), Some(180)));
}

#[test]
fn pistol_is_one_shot_per_press_and_recharges() {
    let mut r = Range::new();
    // The pistol is last in the cycle: one step back from the rifle.
    r.run(1, &[Action::PrevWeapon]);
    r.run(1, &[]);
    assert_eq!(r.arsenal().current, 3);
    r.seconds(1.0, &[Action::Fire]);
    assert_eq!(
        r.arsenal().weapons[3].1.shots_fired,
        1,
        "holding fires once"
    );
    for _ in 0..6 {
        r.run(1, &[Action::Fire]);
        r.seconds(0.25, &[]);
    }
    let charge = r.arsenal().weapons[3].1.clip;
    assert!(charge < 5.0);
    r.seconds(4.0, &[]);
    assert_eq!(r.arsenal().weapons[3].1.clip, 5.0);
    // Two 55-damage bolts take the 100-health droid down.
    let seen = r.seen();
    assert_eq!(seen.died.len(), 1);
    assert_eq!(
        seen.damage.iter().filter(|d| d.target == r.droid).count(),
        2
    );
}

#[test]
fn droid_fire_hits_shield_then_health() {
    let mut r = Range::new();
    r.seen();
    // The droid fires its E-5 at the player's chest ten times.
    let from = b(500.0, 0.0, 120.0);
    let to = b(-500.0, 0.0, 100.0);
    for _ in 0..10 {
        r.app.world_mut().write_message(FireShot {
            shooter: r.droid,
            origin: from,
            direction: (to - from).normalize(),
            weapon: "e5_blaster".into(),
        });
        r.seconds(0.3, &[]);
    }
    let seen = r.seen();
    let on_player: Vec<_> = seen
        .damage
        .iter()
        .filter(|d| d.target == r.player)
        .collect();
    assert_eq!(on_player.len(), 10);
    // The player's damage table: droid bolts (CTDamageEnergyDroids, a kind
    // of CTDamageEnergy) do x0.5.
    assert!(on_player.iter().all(|d| d.amount == 12.0), "{on_player:?}");
    let w = r.app.world();
    // 10 x 24 x 0.5 = 120: the shield takes 100, health the other 20.
    assert_eq!(w.get::<Shield>(r.player).unwrap().current, 0.0);
    assert_eq!(w.get::<Health>(r.player).unwrap().current, 180.0);
    // Nothing for 3 s, then 20 a second.
    r.seconds(2.9, &[]);
    assert_eq!(r.app.world().get::<Shield>(r.player).unwrap().current, 0.0);
    r.seconds(2.0, &[]);
    let s = r.app.world().get::<Shield>(r.player).unwrap().current;
    assert!((s - 38.0).abs() < 4.0, "about 2 s of recharge: {s}");
    let v = r.app.world().resource::<PlayerVitals>();
    assert_eq!(v.health, 180.0);
}

/// With the real game (RC_GAME_DIR set to the install, or a copy of its
/// System, Sounds, StaticMeshes and Textures folders): every sound the
/// weapon and grenade files name is found, groups (SoundMultiple) included,
/// and the DC-17m's model is built.
#[test]
fn real_game_sounds_and_model() {
    let Some(game) = std::env::var_os("RC_GAME_DIR") else {
        eprintln!("RC_GAME_DIR not set: skipped (not tested)");
        return;
    };
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        TransformPlugin,
        bevy::asset::AssetPlugin::default(),
    ))
    .init_asset::<bevy::audio::AudioSource>()
    .init_asset::<Mesh>()
    .init_asset::<StandardMaterial>()
    .init_asset::<Image>()
    .add_plugins((CorePlugin, CombatPlugin))
    .insert_resource(GamePaths::new(game, vec![data]));
    app.world_mut().spawn((Player, Transform::default()));
    let cam = app
        .world_mut()
        .spawn((PlayerCamera, Transform::default()))
        .id();
    app.world_mut()
        .resource_mut::<NextState<GameState>>()
        .set(GameState::InGame);
    for _ in 0..3 {
        app.update();
    }
    let lib = app.world().resource::<WeaponLibrary>();
    let mut wanted: Vec<String> = vec![];
    for p in lib.weapons.values() {
        wanted.extend(p.fire_sounds.iter().cloned());
        wanted.extend(p.reload_sound.iter().cloned());
        wanted.extend(p.empty_sound.iter().cloned());
    }
    for p in lib.grenades.values() {
        wanted.extend(p.throw_sound.iter().cloned());
        wanted.extend(p.bounce_sound.iter().cloned());
        wanted.extend(p.blast_sound.iter().cloned());
    }
    // Every grenade that blows up has a blast sound.
    for (id, p) in &lib.grenades {
        assert!(p.blast_sound.is_some(), "{id} has no blast sound");
    }
    wanted.sort();
    wanted.dedup();
    let missing: Vec<_> = wanted
        .iter()
        .filter(|n| !lib.sounds.contains_key(*n))
        .collect();
    assert!(missing.is_empty(), "sounds not found: {missing:?}");
    // Groups: the repeater's Uzi_Fire holds one sound, the EMP blast two.
    assert_eq!(lib.sounds["Uzi_Fire"].variants.len(), 1);
    assert_eq!(lib.sounds["exp_empGren"].variants.len(), 2);
    eprintln!("sounds loaded: {}", lib.sounds.len());
    // The DC-17m in hand: a child of the camera with one mesh per section.
    let children = app
        .world()
        .get::<Children>(cam)
        .expect("model under the camera");
    let model = children[0];
    let parts = app.world().get::<Children>(model).map_or(0, |c| c.len());
    assert!(parts >= 1, "model has {parts} parts");
    let meshes = app.world().resource::<Assets<Mesh>>();
    let mut tris = 0;
    for part in app.world().get::<Children>(model).unwrap() {
        let h = &app.world().get::<Mesh3d>(*part).unwrap().0;
        tris += meshes.get(h).unwrap().indices().unwrap().len() / 3;
    }
    eprintln!("DC-17m model: {parts} parts, {tris} triangles");
    assert!(tris > 100);
}

#[test]
fn every_grenade_file_reads_and_the_player_carries_them() {
    let r = Range::new();
    let lib = r.app.world().resource::<WeaponLibrary>();
    assert!(lib.errors.is_empty(), "{:?}", lib.errors);
    let ids: Vec<_> = lib.grenades.keys().cloned().collect();
    assert_eq!(
        ids,
        [
            "concussion_rifle_blast",
            "dc17m_antiarmor_round",
            "emp_grenade",
            "flash_grenade",
            "grievous_missile",
            "rocket_launcher_rocket",
            "sonic_detonator",
            "spider_droid_cannon_round",
            "thermal_detonator",
            "thermal_detonator_enemy",
            "turret_shell"
        ]
    );
    let g = r.app.world().get::<Grenades>(r.player).unwrap();
    assert_eq!(g.carried[0].0, "thermal_detonator");
    assert_eq!(
        g.carried[0].1,
        lib.grenades["thermal_detonator"].start_count
    );
    let ro = r.app.world().resource::<WeaponReadout>();
    assert_eq!(
        ro.grenade,
        Some(("Thermal Detonator".to_string(), g.carried[0].1))
    );
}

/// Drop a grenade at rest at `at` (Unreal units) and wait for its blast.
fn drop_grenade(r: &mut Range, id: &str, at: (f32, f32, f32)) -> Seen {
    r.seen();
    let origin = b(at.0, at.1, at.2);
    r.app.world_mut().write_message(ThrowGrenade {
        thrower: r.player,
        origin,
        velocity: Vec3::ZERO,
        grenade: id.into(),
    });
    let mut frames = 0;
    while r.app.world().resource::<Seen>().blasts.is_empty() {
        r.run(1, &[]);
        frames += 1;
        assert!(frames < 60 * 10, "no blast after 10 s");
    }
    r.run(2, &[]);
    r.seen()
}

#[test]
fn thermal_blast_falls_off_with_distance() {
    let mut r = Range::new();
    let p = r.app.world().resource::<WeaponLibrary>().grenades["thermal_detonator"].clone();
    // On the floor 200 uu in front of the droid (centre at x 500, z 84),
    // so the floor holds it still until the fuse runs out.
    let seen = drop_grenade(&mut r, "thermal_detonator", (300.0, 0.0, 10.0));
    let blast = &seen.blasts[0];
    let bx = blast.point.x * 50.0;
    assert!((bx - 300.0).abs() < 1.0, "lay still: went off at x {bx}");
    let on_droid: Vec<_> = seen.damage.iter().filter(|d| d.target == r.droid).collect();
    assert_eq!(on_droid.len(), 1);
    // Blast at about (300, 0, 0.5); droid centre (500, 0, 84).
    let c = Vec3::new(500.0, 0.0, 84.0);
    let pt = Vec3::new(blast.point.x, blast.point.z, blast.point.y) * 50.0;
    let dist = (c - pt).length();
    // Falloff, times the droid's table for the thermal's damage type
    // (CTDamageExplosionThermalDet, a kind of CTDamageExplosion: x0.75).
    let m = r.droid_takes(&p.damage_type);
    assert_eq!(m, 0.75);
    let want = p.damage * (1.0 - (dist - 40.0) / p.radius) * m;
    assert!(
        (on_droid[0].amount - want).abs() < 0.5,
        "damage {} at {dist:.0} uu, expected {want:.1}",
        on_droid[0].amount
    );
    // The player is 800 uu away: out of reach for a radius under 760.
    if p.radius + 40.0 < 800.0 {
        assert!(seen.damage.iter().all(|d| d.target != r.player));
    }
    // Battle droids don't burn (CTDamageFire x0 in their table).
    assert!(r.app.world().get::<Stunned>(r.droid).is_none());
}

#[test]
fn thermal_sets_an_organic_alight_close_up() {
    let mut r = Range::new();
    // An organic with no damage table, 100 uu from the blast.
    r.app
        .world_mut()
        .entity_mut(r.droid)
        .remove::<DamageTable>()
        .insert((Body::Organic, Health::new(1000.0)));
    let p = r.app.world().resource::<WeaponLibrary>().grenades["thermal_detonator"].clone();
    let st = p.status.clone().expect("the thermal burns");
    drop_grenade(&mut r, "thermal_detonator", (420.0, 0.0, 10.0));
    let s = r
        .app
        .world()
        .get::<Stunned>(r.droid)
        .expect("burning")
        .clone();
    assert!(!s.stops, "burning doesn't stop it");
    assert_eq!(s.damage_per_sec, st.damage_per_sec);
    assert!(s.left > 0.0 && s.left < st.max_time, "{}", s.left);
    // It hurts 15 a second until it wears off.
    r.seconds(2.0, &[]);
    let seen = r.seen();
    let burn: f32 = seen
        .damage
        .iter()
        .filter(|d| d.target == r.droid && d.kind == rc_combat::DamageKind::Fire)
        .map(|d| d.amount)
        .sum();
    assert!((burn - 30.0).abs() < 4.0, "about 2 s of burning: {burn}");
}

#[test]
fn flash_stuns_organics_but_not_droids() {
    let mut r = Range::new();
    let p = r.app.world().resource::<WeaponLibrary>().grenades["flash_grenade"].clone();
    let seen = drop_grenade(&mut r, "flash_grenade", (300.0, 0.0, 10.0));
    assert!(seen
        .damage
        .iter()
        .all(|d| d.amount == 0.0 || d.target != r.droid));
    assert!(
        r.app.world().get::<Stunned>(r.droid).is_none(),
        "droids ignore CTDamageStun"
    );
    // An organic without a table is stunned, for up to 18 s.
    r.app
        .world_mut()
        .entity_mut(r.droid)
        .remove::<DamageTable>()
        .insert(Body::Organic);
    drop_grenade(&mut r, "flash_grenade", (300.0, 0.0, 10.0));
    let s = r.app.world().get::<Stunned>(r.droid).expect("stunned");
    let max = p.status.as_ref().unwrap().max_time;
    assert!(s.stops && s.left > 0.0 && s.left <= max, "{}", s.left);
}

#[test]
fn emp_electrocutes_droids() {
    let mut r = Range::new();
    let p = r.app.world().resource::<WeaponLibrary>().grenades["emp_grenade"].clone();
    let st = p.status.clone().expect("the EMP electrocutes");
    drop_grenade(&mut r, "emp_grenade", (300.0, 0.0, 10.0));
    // No blast damage; the droid is electrocuted (stopped) for up to 9 s,
    // at 50 damage a second.
    let s = r
        .app
        .world()
        .get::<Stunned>(r.droid)
        .expect("electrocuted")
        .clone();
    assert!(s.stops);
    assert!(s.left > 0.0 && s.left <= st.max_time, "{}", s.left);
    let m = r.droid_takes(&st.damage_type);
    assert_eq!(s.damage_per_sec, st.damage_per_sec * m);
    // 100 health at 50 a second: down in about 2 s.
    r.seconds(2.5, &[]);
    let seen = r.seen();
    assert_eq!(seen.died.len(), 1, "electrocuted to death");
}

#[test]
fn player_throws_the_selected_grenade() {
    let mut r = Range::new();
    let before = r.app.world().get::<Grenades>(r.player).unwrap().carried[0].1;
    assert!(before > 0);
    r.seen();
    r.run(1, &[Action::ThrowGrenade]);
    r.run(1, &[]);
    let g = r.app.world().get::<Grenades>(r.player).unwrap();
    assert_eq!(g.carried[0].1, before - 1);
    // In the air, moving forward and up.
    let mut q = r.app.world_mut().query::<&rc_combat::GrenadeFlight>();
    let f = q.single(r.app.world()).expect("one grenade in flight");
    assert!(f.flight.velocity.x > 0.0);
    r.seconds(6.0, &[]);
    let seen = r.seen();
    assert_eq!(seen.blasts.len(), 1, "it went off");
    let ro = r.app.world().resource::<WeaponReadout>();
    assert_eq!(ro.grenade.as_ref().map(|g| g.1), Some(before - 1));
}

#[test]
fn sniper_zooms_and_hits_hard() {
    let mut r = Range::new();
    let lib = r.app.world().resource::<WeaponLibrary>();
    let p = lib.weapons["dc17m_sniper"].clone();
    r.run(1, &[Action::NextWeapon]);
    r.run(1, &[]);
    assert_eq!(r.arsenal().weapons[r.arsenal().current].0, "dc17m_sniper");
    r.run(1, &[Action::Zoom]);
    r.run(1, &[]);
    assert_eq!(
        r.app.world().resource::<WeaponReadout>().zoom_fov,
        p.zoom_fovs.first().copied()
    );
    r.seen();
    r.run(1, &[Action::Fire]);
    r.seconds(1.0, &[]);
    let seen = r.seen();
    let d: Vec<_> = seen.damage.iter().filter(|d| d.target == r.droid).collect();
    assert_eq!(d.len(), 1);
    // Zoomed: x1.72, then the droid's table for CTDamageBallisticDC17Sniper
    // (a kind of CTDamageBallistic: x0.4). 175 x 1.72 x 0.4 = 120.4.
    let m = r.droid_takes(&p.damage_type);
    assert_eq!(m, 0.4);
    let want = p.damage * p.zoom_damage * m;
    assert!(
        (d[0].amount - want).abs() < 1e-3,
        "{} vs {want}",
        d[0].amount
    );
    assert!((want - 120.4).abs() < 1e-3);
    // Switching weapon leaves zoom.
    r.run(1, &[Action::NextWeapon]);
    r.run(2, &[]);
    assert_eq!(r.app.world().resource::<WeaponReadout>().zoom_fov, None);
}

#[test]
fn anti_armour_round_flies_and_blows_up_on_the_droid() {
    let mut r = Range::new();
    let round = r.app.world().resource::<WeaponLibrary>().grenades["dc17m_antiarmor_round"].clone();
    for _ in 0..2 {
        r.run(1, &[Action::NextWeapon]);
        r.run(1, &[]);
    }
    assert_eq!(
        r.arsenal().weapons[r.arsenal().current].0,
        "dc17m_antiarmor"
    );
    r.seen();
    r.run(1, &[Action::Fire]);
    r.seconds(2.0, &[]);
    let seen = r.seen();
    assert_eq!(seen.blasts.len(), 1, "one blast");
    let b = &seen.blasts[0];
    let bx = b.point.x * 50.0;
    // It goes off on the droid's near side (x 460), not at the far wall.
    assert!((bx - 460.0).abs() < 60.0, "went off at x {bx}");
    let d = seen
        .damage
        .iter()
        .find(|d| d.target == r.droid)
        .expect("droid hurt");
    // A direct hit: the full 400, times the droid's x0.75 for explosions.
    let want = round.damage * r.droid_takes(&round.damage_type);
    assert!((d.amount - want).abs() < 1e-3, "{} vs {want}", d.amount);
}

#[test]
fn melee_hits_only_within_reach() {
    let mut r = Range::new();
    let reach = r
        .app
        .world()
        .resource::<WeaponLibrary>()
        .player
        .melee
        .clone();
    r.seen();
    r.run(1, &[Action::Melee]);
    r.run(1, &[]);
    assert!(r.seen().damage.is_empty(), "the droid is 1000 uu away");
    // Bring the droid within reach: 80 uu in front of the eye.
    r.app
        .world_mut()
        .entity_mut(r.droid)
        .insert(Transform::from_translation(b(
            -500.0 + 80.0 + 40.0,
            0.0,
            0.0,
        )));
    r.seconds(reach.delay + 0.1, &[]);
    r.run(1, &[Action::Melee]);
    r.run(1, &[]);
    let seen = r.seen();
    assert_eq!(seen.damage.len(), 1);
    assert_eq!(seen.damage[0].target, r.droid);
    // The DC-17m's own melee (220), times the droid's x3 for CTDamageMelee.
    let lib = r.app.world().resource::<WeaponLibrary>();
    let (_, dmg) = lib.weapons["dc17m_blaster"].melee.expect("rifle melee");
    assert_eq!(seen.damage[0].amount, dmg * r.droid_takes("CTDamageMelee"));
    assert_eq!(seen.damage[0].amount, 660.0);
    assert_eq!(seen.damage[0].kind, rc_combat::DamageKind::Melee);
}

#[test]
fn a_dropped_weapon_is_picked_up_with_use() {
    let mut r = Range::new();
    let p = r.app.world().resource::<WeaponLibrary>().weapons["e5_blaster_pickup"].clone();
    let mut commands = r.app.world_mut().commands();
    rc_combat::spawn_pickup(&mut commands, "e5_blaster_pickup", b(-200.0, 0.0, 0.0));
    r.run(2, &[Action::Use]);
    r.run(1, &[]);
    assert_eq!(r.arsenal().weapons.len(), 4, "not picked up from 300 uu");
    r.app
        .world_mut()
        .entity_mut(r.player)
        .insert(Transform::from_translation(b(-220.0, 0.0, 0.0)));
    r.run(3, &[]);
    assert_eq!(r.arsenal().weapons.len(), 4, "walking over it isn't enough");
    r.run(1, &[Action::Use]);
    r.run(1, &[]);
    let a = r.arsenal();
    let (id, w) = a.current().unwrap();
    assert_eq!(id, "e5_blaster_pickup", "in hand");
    assert_eq!(w.shots_in_clip() + w.reserve, p.start_ammo);
    let mut q = r.app.world_mut().query::<&rc_combat::WeaponPickup>();
    assert_eq!(
        q.iter(r.app.world()).count(),
        0,
        "it's gone from the ground"
    );
}

/// Put weapons in the "level" as the level part hands them over: name,
/// original class, Unreal position of the cylinder's centre.
fn place_weapons(r: &mut Range, list: &[(&str, &str, Vec3, u32)]) {
    use rc_game_core::pickups::{LevelWeaponPickups, PlacedWeapon};
    let weapons = list
        .iter()
        .enumerate()
        .map(|(i, (name, weapon, at, ammo))| PlacedWeapon {
            export: 1000 + i,
            name: name.to_string(),
            class: format!("{weapon}Pickup"),
            at: *at,
            weapon: weapon.to_string(),
            ammo: *ammo,
            message: format!("{weapon} message"),
            sound: Some(("GEN_Sound".into(), format!("{weapon}Sound"))),
            radius: 60.0,
            height: 15.0,
        })
        .collect();
    r.app
        .world_mut()
        .insert_resource(LevelWeaponPickups(weapons));
    r.run(3, &[]);
}

fn taken(r: &mut Range) -> Vec<rc_combat::WeaponTaken> {
    std::mem::take(&mut r.app.world_mut().resource_mut::<Seen>().taken)
}

#[test]
fn level_weapons_are_placed_and_one_enemy_weapon_is_carried_at_a_time() {
    let mut r = Range::new();
    r.app
        .init_resource::<PickedUpLog>()
        .add_systems(Update, keep_picked_up);
    place_weapons(
        &mut r,
        &[
            ("BowcasterPickup0", "Bowcaster", b(-480.0, 0.0, 15.0), 24),
            ("SMGPickup1", "SMG", b(-480.0, 600.0, 15.0), 160),
            ("MysteryPickup0", "NoSuchWeapon", b(0.0, 600.0, 15.0), 1),
        ],
    );
    let mut q = r.app.world_mut().query::<&rc_combat::WeaponPickup>();
    let placed: Vec<String> = q
        .iter(r.app.world())
        .filter_map(|p| p.placed.as_ref().map(|x| x.1.clone()))
        .collect();
    assert_eq!(placed.len(), 2, "the unknown one is left out: {placed:?}");
    // Standing on the bowcaster: the prompt, not the weapon.
    assert_eq!(r.arsenal().weapons.len(), 4);
    // Take it with Use: in hand, the pickup gone.
    r.run(1, &[Action::Use]);
    r.run(1, &[]);
    let a = r.arsenal();
    assert_eq!(a.current().unwrap().0, "bowcaster");
    let bow = &a.current().unwrap().1;
    assert_eq!(bow.shots_in_clip() + bow.reserve, 24);
    let t = taken(&mut r);
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].placed.as_ref().unwrap().1, "BowcasterPickup0");
    assert!(t[0].new_weapon);
    // Its words and its own pickup sound.
    let got = r.app.world().resource::<PickedUpLog>().0.clone();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].text, "Bowcaster message");
    assert_eq!(
        got[0].sound,
        Some(("GEN_Sound".to_string(), "BowcasterSound".to_string()))
    );
    // Fire one bolt, then go to the repeater and take it: the bowcaster
    // is thrown down with 23 shots, the repeater is in hand.
    r.run(1, &[Action::Fire]);
    r.seconds(1.0, &[]);
    r.app
        .world_mut()
        .entity_mut(r.player)
        .insert(Transform::from_translation(b(-480.0, 600.0, 0.0)));
    r.run(2, &[]);
    r.run(1, &[Action::Use]);
    r.run(1, &[]);
    let a = r.arsenal();
    let ids: Vec<&str> = a.weapons.iter().map(|w| w.0.as_str()).collect();
    assert_eq!(ids.iter().filter(|i| **i == "bowcaster").count(), 0, "{ids:?}");
    assert_eq!(a.current().unwrap().0, "trandoshan_smg");
    let t = taken(&mut r);
    assert_eq!(t[0].dropped.as_deref(), Some("bowcaster"));
    let mut q = r.app.world_mut().query::<&rc_combat::WeaponPickup>();
    let dropped: Vec<(String, Option<u32>)> = q
        .iter(r.app.world())
        .filter(|p| p.placed.is_none())
        .map(|p| (p.weapon.clone(), p.ammo))
        .collect();
    assert_eq!(dropped, [("bowcaster".to_string(), Some(23))]);
    // The save lists both map weapons as taken.
    let gone = r
        .app
        .world()
        .resource::<rc_combat::pickup::WeaponsGone>()
        .names();
    assert_eq!(gone, ["BowcasterPickup0", "SMGPickup1"]);
}

#[test]
fn walking_into_a_carried_weapon_takes_its_ammo() {
    let mut r = Range::new();
    // Use some sniper ammo first.
    {
        let mut a = r
            .app
            .world_mut()
            .get_mut::<Arsenal>(r.player)
            .unwrap();
        let s = a
            .weapons
            .iter_mut()
            .find(|w| w.0 == "dc17m_sniper")
            .unwrap();
        s.1.clip = 1.0;
        s.1.reserve = 0;
    }
    place_weapons(
        &mut r,
        &[("DC17mSniperWeaponPickup0", "DC17mSniper", b(-480.0, 0.0, 15.0), 20)],
    );
    let a = r.arsenal();
    let s = &a.weapons.iter().find(|w| w.0 == "dc17m_sniper").unwrap().1;
    assert_eq!(s.shots_in_clip() + s.reserve, 20, "1 + 20, capped at 20");
    let t = taken(&mut r);
    assert_eq!(t.len(), 1);
    assert!(!t[0].new_weapon);
    assert_eq!(t[0].added, 19);
}

#[test]
fn the_acp_array_gun_fires_eight_pellets() {
    let mut r = Range::new();
    place_weapons(
        &mut r,
        &[("ShotgunPickup0", "Shotgun", b(-480.0, 0.0, 15.0), 16)],
    );
    r.run(1, &[Action::Use]);
    r.run(1, &[]);
    assert_eq!(r.arsenal().current().unwrap().0, "acp_array_gun");
    r.seen();
    r.run(1, &[Action::Fire]);
    r.run(2, &[]);
    let seen = r.seen();
    assert_eq!(seen.landed.len(), 8, "one shot, eight pellets");
    let a = r.arsenal();
    let g = &a.current().unwrap().1;
    assert_eq!(g.shots_in_clip(), 7, "one shell used");
}

#[test]
fn rockets_and_concussion_shots_blast_the_droid() {
    for (class, id, wait) in [
        ("RocketLauncher", "rocket_launcher", 1.5),
        ("ConcussionRifle", "concussion_rifle", 0.5),
    ] {
        let mut r = Range::new();
        place_weapons(
            &mut r,
            &[(&format!("{class}Pickup0"), class, b(-480.0, 0.0, 15.0), 5)],
        );
        r.run(1, &[Action::Use]);
        r.run(1, &[]);
        assert_eq!(r.arsenal().current().unwrap().0, id);
        r.seen();
        // Aim at the droid's middle (1000 uu ahead, centre 84 uu up; the
        // eye is 144 uu up).
        r.run(1, &[Action::Fire]);
        r.seconds(wait, &[]);
        let seen = r.seen();
        assert_eq!(seen.blasts.len(), 1, "{id}: one blast");
        assert!(
            seen.damage.iter().any(|d| d.target == r.droid),
            "{id}: the droid is hurt"
        );
    }
}

#[test]
fn sonic_sticks_and_goes_off_when_a_droid_comes_close() {
    let mut r = Range::new();
    // Move the droid far away, drop the sonic on the floor: it sticks and
    // waits (its fuse is 10 s).
    r.app
        .world_mut()
        .entity_mut(r.droid)
        .insert(Transform::from_translation(b(2500.0, 0.0, 0.0)));
    r.seen();
    r.app.world_mut().write_message(ThrowGrenade {
        thrower: r.player,
        origin: b(300.0, 0.0, 10.0),
        velocity: Vec3::ZERO,
        grenade: "sonic_detonator".into(),
    });
    r.seconds(2.0, &[]);
    assert!(r.seen().blasts.is_empty(), "nobody within 400 uu yet");
    let mut q = r.app.world_mut().query::<&rc_combat::GrenadeFlight>();
    let f = q.single(r.app.world()).expect("waiting on the floor");
    assert!(f.flight.stuck, "stuck to the floor");
    // The droid steps within 400 uu: it goes off at the next check (0.5 s).
    r.app
        .world_mut()
        .entity_mut(r.droid)
        .insert(Transform::from_translation(b(600.0, 0.0, 0.0)));
    r.seconds(0.6, &[]);
    let seen = r.seen();
    assert_eq!(seen.blasts.len(), 1, "proximity blast");
    assert!(seen.damage.iter().any(|d| d.target == r.droid));
}

#[test]
fn a_flash_on_the_player_sends_no_damage_and_the_shield_recharges() {
    let mut r = Range::new();
    r.app
        .world_mut()
        .get_mut::<Shield>(r.player)
        .unwrap()
        .current = 50.0;
    // In front of the player (who looks along +X), so they see the flash.
    drop_grenade(&mut r, "flash_grenade", (-300.0, 0.0, 10.0));
    assert!(r.app.world().get::<Stunned>(r.player).is_some(), "flashed");
    r.seconds(5.0, &[]);
    let seen = r.seen();
    assert!(
        seen.damage.iter().all(|d| d.target != r.player),
        "a stun does no damage: {:?}",
        seen.damage.iter().filter(|d| d.target == r.player).count()
    );
    // 3 s delay after the last hit, then 20 a second: well above 50 by now.
    let s = r.app.world().get::<Shield>(r.player).unwrap().current;
    assert!(s > 70.0, "shield {s}");
}

/// Put a level's ammo boxes in: a DC-17m box 300 uu ahead and the
/// grenade crate that comes back 300 uu to the side, each with a drawn
/// model part to hide.
fn ammo_boxes(r: &mut Range) -> (Entity, Entity) {
    use rc_game_core::pickups::{LevelAmmoBoxes, PlacedAmmoBox};
    let rifle = PlacedAmmoBox {
        export: 7,
        name: "DC17mBlasterBox3".into(),
        class: "DC17mBlasterBox".into(),
        at: b(-200.0, 0.0, 15.0),
        ammo: "DC17mBlasterAmmo".into(),
        amount: 300,
        respawn: None,
        radius: 40.0,
        height: 15.0,
        message: "DC-17m Blaster Clips".into(),
        sound: Some(("GEN_Sound".into(), "GenericWeaponPickup".into())),
    };
    let crate_ = PlacedAmmoBox {
        export: 9,
        name: "ThermalDetonatorBox3".into(),
        class: "ThermalDetonatorBox".into(),
        at: b(-500.0, 300.0, 44.0),
        ammo: "ThermalDetonatorAmmo".into(),
        amount: 5,
        respawn: Some(0.1),
        radius: 45.0,
        height: 44.0,
        message: "Thermal Detonators".into(),
        sound: Some(("GEN_Sound".into(), "GenericWeaponPickup".into())),
    };
    r.app
        .world_mut()
        .insert_resource(LevelAmmoBoxes(vec![rifle, crate_]));
    let a = r
        .app
        .world_mut()
        .spawn((rc_level_viewer::draw::MoverPart(7), Visibility::default()))
        .id();
    let c = r
        .app
        .world_mut()
        .spawn((rc_level_viewer::draw::MoverPart(9), Visibility::default()))
        .id();
    r.run(2, &[]);
    (a, c)
}

fn move_player(r: &mut Range, x: f32, y: f32) {
    r.app
        .world_mut()
        .entity_mut(r.player)
        .insert(Transform::from_translation(b(x, y, 0.0)));
}

fn rifle_total(r: &Range) -> u32 {
    let a = r.arsenal();
    let (_, w) = a
        .weapons
        .iter()
        .find(|(id, _)| id == "dc17m_blaster")
        .unwrap();
    w.shots_in_clip() + w.reserve
}

fn set_rifle(r: &mut Range, clip: f32, reserve: u32) {
    let mut a = r.app.world_mut().get_mut::<Arsenal>(r.player).unwrap();
    let w = &mut a
        .weapons
        .iter_mut()
        .find(|(id, _)| id == "dc17m_blaster")
        .unwrap()
        .1;
    w.clip = clip;
    w.reserve = reserve;
}

#[test]
fn walking_into_an_ammo_box_refills_the_rifle() {
    let mut r = Range::new();
    let (part, _) = ammo_boxes(&mut r);
    // The player is given the rifle full, as the original does.
    assert_eq!(rifle_total(&r), 300);
    set_rifle(&mut r, 60.0, 60);
    // 300 uu away: nothing.
    r.run(2, &[]);
    assert_eq!(rifle_total(&r), 120);
    // Walk into it: topped up to the most carried, the box gone.
    move_player(&mut r, -260.0, 0.0);
    r.run(2, &[]);
    assert_eq!(rifle_total(&r), 300);
    let vis = *r.app.world().get::<Visibility>(part).unwrap();
    assert_eq!(vis, Visibility::Hidden, "the box's model is hidden");
    // The save lists it.
    use rc_game_core::save::SavePart;
    let s = rc_combat::ammo_box::PickupsSave::save(r.app.world_mut()).unwrap();
    assert_eq!(s.taken, vec!["DC17mBlasterBox3".to_string()]);
    // Walk off and back: it's gone for good.
    set_rifle(&mut r, 10.0, 0);
    move_player(&mut r, -500.0, 0.0);
    r.run(2, &[]);
    move_player(&mut r, -200.0, 0.0);
    r.run(2, &[]);
    assert_eq!(rifle_total(&r), 10);
}

#[test]
fn a_full_rifle_leaves_the_box_lying() {
    let mut r = Range::new();
    let (part, _) = ammo_boxes(&mut r);
    set_rifle(&mut r, 60.0, 240);
    move_player(&mut r, -200.0, 0.0);
    r.run(2, &[]);
    let vis = *r.app.world().get::<Visibility>(part).unwrap();
    assert_ne!(vis, Visibility::Hidden);
    // Using ammo while standing on it doesn't take it: the game tries a box
    // when touching starts.
    set_rifle(&mut r, 30.0, 240);
    r.run(2, &[]);
    assert_eq!(rifle_total(&r), 270);
    // Step off and back on: now it's taken.
    move_player(&mut r, -500.0, 0.0);
    r.run(2, &[]);
    move_player(&mut r, -200.0, 0.0);
    r.run(2, &[]);
    assert_eq!(rifle_total(&r), 300);
}

#[test]
fn the_grenade_crate_refills_and_comes_back() {
    let mut r = Range::new();
    let (_, part) = ammo_boxes(&mut r);
    let count = |r: &Range| {
        r.app
            .world()
            .get::<Grenades>(r.player)
            .unwrap()
            .carried
            .iter()
            .find(|(id, _)| id == "thermal_detonator")
            .unwrap()
            .1
    };
    assert_eq!(count(&r), 2);
    move_player(&mut r, -500.0, 300.0);
    r.run(2, &[]);
    assert_eq!(count(&r), 5);
    assert_eq!(
        *r.app.world().get::<Visibility>(part).unwrap(),
        Visibility::Hidden
    );
    // Back 0.1 s later; full now, so it stays.
    r.seconds(0.3, &[]);
    assert_ne!(
        *r.app.world().get::<Visibility>(part).unwrap(),
        Visibility::Hidden
    );
    use rc_game_core::save::SavePart;
    let s = rc_combat::ammo_box::PickupsSave::save(r.app.world_mut()).unwrap();
    assert!(s.taken.is_empty(), "boxes that come back aren't saved");
}

/// Every `PickedUp` sent, kept for the test to read.
#[derive(Resource, Default)]
struct PickedUpLog(Vec<rc_game_core::pickups::PickedUp>);

fn keep_picked_up(
    mut log: ResMut<PickedUpLog>,
    mut picked: MessageReader<rc_game_core::pickups::PickedUp>,
) {
    log.0.extend(picked.read().cloned());
}

#[test]
fn taking_a_box_says_what_it_was_and_plays_its_sound() {
    let mut r = Range::new();
    r.app
        .init_resource::<PickedUpLog>()
        .add_systems(Update, keep_picked_up);
    ammo_boxes(&mut r);
    // Full: touching it says nothing.
    set_rifle(&mut r, 60.0, 240);
    move_player(&mut r, -200.0, 0.0);
    r.run(2, &[]);
    assert!(r.app.world().resource::<PickedUpLog>().0.is_empty());
    // Not full: one message, with the box's words and sound.
    set_rifle(&mut r, 10.0, 0);
    move_player(&mut r, -500.0, 0.0);
    r.run(2, &[]);
    move_player(&mut r, -200.0, 0.0);
    r.run(2, &[]);
    let got = &r.app.world().resource::<PickedUpLog>().0;
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].text, "DC-17m Blaster Clips");
    assert_eq!(
        got[0].sound,
        Some(("GEN_Sound".to_string(), "GenericWeaponPickup".to_string()))
    );
}
