//! rc_enemies: battle droids and the other enemies in the game.
//!
//! [`EnemiesPlugin`] follows `docs/PLUGINS.md`. When a map loads it reads the
//! map's spawners and path network (`rc_ai`, `rc_nav`) and every enemy
//! type's settings (`data/enemies/*.toml`, which mods can replace or add
//! to; each file names the game classes it is for).
//! During play it runs the fight (`rc_ai::Battle`): spawners switch on,
//! droids appear, see the player, take cover, keep their distance and
//! shoot. Their shots go through the weapons part (`rc_combat::FireShot`),
//! which flies the bolts and applies the damage; droids get `Health`,
//! `Team::Separatist` and `Hittable`, so the player's shots hurt them, and
//! they react to `Damage` and `Died`. It sets `MusicMood::Battle` while
//! droids are fighting. Each droid is the game's animated `BattleDroid`
//! mesh, posed on the CPU every frame.
//!
//! Everything it does is logged to the game log with the prefix `enemies:`.

pub mod look;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use look::{DroidLook, Pose};
use rc_ai::{AiEvent, AiParams, Battle, BattleEvent, Mode, Target};
use rc_combat::{
    Blast, Body, CombatSet, Damage, Dead, Died, FireShot, Health, Hittable, ShotLanded, Stunned,
    Team,
};
use rc_game_core::units::{to_bevy, to_unreal};
use rc_game_core::{
    playing, CurrentLevel, GameLog, GamePaths, GameSet, GameState, LevelCollision, MusicMood,
    Player, PlayerCamera,
};
use rc_nav::NavGraph;
use rc_package::Package;
use std::collections::HashMap;
use std::sync::Arc;

const PART: &str = "enemies";
/// Seconds between the summary lines in the log.
const SUMMARY_EVERY: f32 = 5.0;
/// A droid counts as "just fired" (for its animation) this long, seconds.
const FIRE_POSE_TIME: f32 = 0.35;

pub struct EnemiesPlugin;

impl Plugin for EnemiesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Looks>()
            .add_systems(OnEnter(GameState::Loading), load_fight)
            .add_systems(OnExit(GameState::InGame), end_fight)
            .add_systems(
                Update,
                (think, place_droids)
                    .chain()
                    .in_set(GameSet::Simulate)
                    .before(CombatSet::Fire)
                    .run_if(playing)
                    .run_if(resource_exists::<Fight>),
            )
            .add_systems(
                Update,
                react_to_combat
                    .in_set(GameSet::Simulate)
                    .after(CombatSet::ApplyDamage)
                    .run_if(playing)
                    .run_if(resource_exists::<Fight>),
            )
            .add_systems(
                Update,
                animate
                    .in_set(GameSet::View)
                    .run_if(playing)
                    .run_if(resource_exists::<Fight>),
            );
    }
}

/// The fight on the current map. Inserted while loading; tests can insert
/// their own with [`Fight::new`].
#[derive(Resource)]
pub struct Fight {
    pub battle: Battle,
    /// Each droid's entity, by its id in the battle.
    pub entities: HashMap<u32, Entity>,
    pub stats: FightStats,
    next_summary: f32,
}

/// Counts for the log and for tests.
#[derive(Debug, Default, Clone)]
pub struct FightStats {
    pub spawned: u32,
    pub shots: u32,
    /// Droid shots that hurt the player.
    pub hits_on_player: u32,
    pub damage_to_player: f32,
    /// Times a droid was hurt, and destroyed.
    pub droids_hurt: u32,
    pub droids_destroyed: u32,
    pub first_spotted: Option<f32>,
    /// Where droid bolts ended: on another droid, on the level, or nowhere
    /// (out of range). Hits on the player are `hits_on_player`.
    pub bolts_on_droids: u32,
    pub bolts_on_level: u32,
    pub bolts_nowhere: u32,
}

impl Fight {
    pub fn new(battle: Battle) -> Self {
        Fight {
            battle,
            entities: HashMap::new(),
            stats: FightStats::default(),
            next_summary: 0.0,
        }
    }
}

/// One enemy on screen (a battle droid or any other type).
#[derive(Component, Debug)]
pub struct Droid {
    pub id: u32,
    /// Its enemy type: the settings file's id, like `battle_droid`.
    pub kind: String,
    pub pose: Pose,
    /// Frame of the current animation.
    pub frame: f32,
    /// Battle time of its last shot.
    last_fire: f32,
    mesh: Option<Handle<Mesh>>,
}

/// Each enemy type's mesh, read once from the game and kept between maps.
#[derive(Resource, Default)]
pub struct Looks {
    tried: std::collections::HashSet<String>,
    pub by_type: HashMap<String, TypeLook>,
}

pub struct TypeLook {
    pub look: Arc<DroidLook>,
    material: Option<Handle<StandardMaterial>>,
}

/// Every enemy type: each `enemies/*.toml` across the data folders (a
/// mod's file replaces the base file with the same name), each read on top
/// of the battle droid's built-in values. The battle droid comes first.
fn enemy_types(paths: &GamePaths, log: &GameLog) -> Vec<Arc<AiParams>> {
    let mut files = std::collections::BTreeMap::new();
    for dir in paths.data_dirs.iter().rev() {
        let folder = GamePaths::new(dir.clone(), vec![]).game_file("enemies");
        let Ok(entries) = std::fs::read_dir(folder) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("toml"))
            {
                let id = p
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .to_ascii_lowercase();
                files.insert(id, p);
            }
        }
    }
    let mut out = Vec::new();
    for (id, path) in files {
        let read = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| AiParams::battle_droid().with_overrides(&t));
        match read {
            Ok(mut p) => {
                p.kind.id = id;
                log.line(format!(
                    "{PART}: enemy type {} ({}, {}) for classes {:?} from {}: health {}, speed {}, weapon {}, accuracy {}, cover {}",
                    p.kind.id,
                    p.kind.name,
                    p.kind.body,
                    p.kind.classes,
                    path.display(),
                    p.combat.health,
                    p.movement.ground_speed,
                    p.combat.weapon,
                    p.combat.accuracy,
                    p.combat.uses_cover
                ));
                out.push(Arc::new(p));
            }
            Err(e) => log.line(format!(
                "{PART}: ERROR in {}: {e}; that enemy type is left out",
                path.display()
            )),
        }
    }
    if !out.iter().any(|p| p.kind.id == "battle_droid") {
        log.line(format!(
            "{PART}: no enemies/battle_droid.toml, using built-in battle droid values"
        ));
        out.push(Arc::new(AiParams::battle_droid()));
    }
    out.sort_by_key(|p| p.kind.id != "battle_droid");
    out
}

/// Read the map's spawners and paths. A map that can't be read gives no
/// droids, not a failed load: the level part reports map problems.
#[allow(clippy::too_many_arguments)]
fn load_fight(
    mut commands: Commands,
    level: Res<CurrentLevel>,
    paths: Res<GamePaths>,
    log: Res<GameLog>,
    mut looks: ResMut<Looks>,
    images: Option<ResMut<Assets<Image>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    commands.remove_resource::<Fight>();
    let Some(map) = level.map.clone() else { return };
    let t = std::time::Instant::now();
    let types = enemy_types(&paths, &log);
    let params = types[0].clone();
    let path = paths.map_path(&map);
    let read = std::fs::read(&path)
        .map_err(|e| format!("can't read {}: {e}", path.display()))
        .and_then(|data| {
            let pkg = Package::parse(&data).map_err(|e| e.to_string())?;
            let nav = NavGraph::from_map(&pkg, &data)?;
            let factories = rc_ai::read_factories(&pkg, &data)?;
            Ok((nav, factories))
        });
    let (nav, factories) = match read {
        Ok(r) => r,
        Err(e) => {
            log.line(format!("{PART}: no droids on this map: {e}"));
            return;
        }
    };
    let all = factories.len();
    // Each spawner makes the enemy type whose file claims its class.
    // Spawners of classes no file claims (squad mates, turrets, ...) are
    // left to other parts; each such class is logged once.
    let mut unclaimed: std::collections::BTreeMap<String, u32> = Default::default();
    let mut per_type: std::collections::BTreeMap<String, u32> = Default::default();
    let mut droids: Vec<(rc_ai::Factory, Arc<AiParams>)> = Vec::new();
    for f in factories {
        match types.iter().find(|t| t.kind.matches(&f.prototype)) {
            Some(t) => {
                *per_type.entry(t.kind.id.clone()).or_default() += 1;
                droids.push((f, t.clone()));
            }
            None => *unclaimed.entry(f.prototype.clone()).or_default() += 1,
        }
    }
    let triggered = droids.iter().filter(|f| !f.0.triggers.is_empty()).count();
    log.line(format!(
        "{PART}: {map}: spawners by enemy type {per_type:?}; spawners no enemy file claims {unclaimed:?}"
    ));
    log.line(format!(
        "{PART}: {map}: {} path points, {} links ({} blocked until scripts open them), {all} spawners, {} make enemies ({triggered} with walk-in triggers, the rest start when the player comes within {:.0} uu), {:.0} ms",
        nav.nodes.len(),
        nav.link_count(),
        nav.blocked.len(),
        droids.len(),
        params.spawning.activate_unscripted_within,
        t.elapsed().as_secs_f64() * 1000.0
    ));

    let mut images = images;
    let mut materials = materials;
    // Only the types this map uses, each once per run.
    for t in &types {
        if !per_type.contains_key(&t.kind.id) || looks.tried.contains(&t.kind.id) {
            continue;
        }
        looks.tried.insert(t.kind.id.clone());
        let started = std::time::Instant::now();
        match DroidLook::load(&paths, &t.look) {
            Ok((look, notes)) => {
                for n in notes {
                    log.line(format!("{PART}: {}: {n}", t.kind.id));
                }
                log.line(format!(
                    "{PART}: {} mesh read in {:.0} ms",
                    t.kind.id,
                    started.elapsed().as_secs_f64() * 1000.0
                ));
                let mut material = None;
                if let (Some(images), Some(materials)) = (images.as_mut(), materials.as_mut()) {
                    let texture = look.skin.as_ref().map(|(_, img)| {
                        images.add(Image::new(
                            Extent3d {
                                width: img.width as u32,
                                height: img.height as u32,
                                depth_or_array_layers: 1,
                            },
                            TextureDimension::D2,
                            img.pixels.clone(),
                            TextureFormat::Rgba8UnormSrgb,
                            RenderAssetUsages::RENDER_WORLD,
                        ))
                    });
                    // The level's lit surfaces carry their light baked in
                    // and the scene has no ambient light, so a character
                    // would be black away from live lights. Give it a
                    // steady share of its own colour ("ambient" in the
                    // look settings), as light it gives off itself.
                    let a = t.look.ambient.max(0.0);
                    material = Some(materials.add(StandardMaterial {
                        base_color: if texture.is_some() {
                            Color::WHITE
                        } else {
                            Color::srgb(0.6, 0.6, 0.6)
                        },
                        emissive: LinearRgba::rgb(a, a, a)
                            * if texture.is_some() { 1.0 } else { 0.3 },
                        emissive_texture: texture.clone(),
                        base_color_texture: texture,
                        perceptual_roughness: 0.7,
                        ..default()
                    }));
                }
                looks.by_type.insert(
                    t.kind.id.clone(),
                    TypeLook {
                        look: Arc::new(look),
                        material,
                    },
                );
            }
            Err(e) => log.line(format!(
                "{PART}: {} mesh not loaded, they will be invisible: {e}",
                t.kind.id
            )),
        }
    }
    commands.insert_resource(Fight::new(Battle::with_types(params, nav, droids)));
}

fn end_fight(
    mut commands: Commands,
    fight: Option<Res<Fight>>,
    mut mood: ResMut<MusicMood>,
    log: Res<GameLog>,
) {
    if let Some(f) = fight {
        summary(&f, &log);
    }
    commands.remove_resource::<Fight>();
    if *mood != MusicMood::Calm {
        *mood = MusicMood::Calm;
    }
}

fn summary(f: &Fight, log: &GameLog) {
    let s = &f.stats;
    log.line(format!(
        "{PART}: t={:.1}s {} droids alive of {} spawned, {} destroyed; {} droid shots, {} hit the player for {:.0} damage ({} hit other droids, {} the level, {} nothing); droids hurt {} times; first spotted at {}",
        f.battle.time,
        f.battle.alive().count(),
        s.spawned,
        s.droids_destroyed,
        s.shots,
        s.hits_on_player,
        s.damage_to_player,
        s.bolts_on_droids,
        s.bolts_on_level,
        s.bolts_nowhere,
        s.droids_hurt,
        s.first_spotted.map_or("never".into(), |t| format!("{t:.1}s"))
    ));
}

/// Where the player is, for droids to look at (Unreal units).
fn player_target(
    player: &Query<(&Transform, Option<&Hittable>, Has<Dead>), With<Player>>,
    camera: &Query<&Transform, (With<PlayerCamera>, Without<Player>)>,
) -> Option<Target> {
    let (t, hit, dead) = player.single().ok()?;
    if dead {
        return None;
    }
    let feet = to_unreal(t.translation);
    let up = hit.map_or(84.0, |h| h.center_up);
    let centre = feet + rc_movement::Vec3::new(0.0, 0.0, up);
    let eye = camera
        .single()
        .map(|c| to_unreal(c.translation))
        .unwrap_or(centre + rc_movement::Vec3::new(0.0, 0.0, 60.0));
    Some(Target { centre, eye })
}

/// Run the fight for one frame: spawn, think, move, shoot.
#[allow(clippy::too_many_arguments)]
fn think(
    mut commands: Commands,
    time: Res<Time>,
    mut fight: ResMut<Fight>,
    collision: Option<Res<LevelCollision>>,
    player: Query<(&Transform, Option<&Hittable>, Has<Dead>), With<Player>>,
    camera: Query<&Transform, (With<PlayerCamera>, Without<Player>)>,
    allies: Query<(&Transform, &Hittable, &Team), (Without<Player>, Without<Dead>, Without<Droid>)>,
    mut droids: Query<&mut Droid>,
    mut fire: MessageWriter<FireShot>,
    mut mood: ResMut<MusicMood>,
    looks: Res<Looks>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    log: Res<GameLog>,
) {
    let Some(collision) = collision else { return };
    let dt = time.delta_secs().min(0.1);
    let target = player_target(&player, &camera);
    // The player's side besides the player (squad mates): droids fight
    // them too.
    let others: Vec<Target> = allies
        .iter()
        .filter(|(_, _, team)| **team == Team::Republic)
        .map(|(t, hit, _)| {
            let centre = to_unreal(t.translation) + rc_movement::Vec3::new(0.0, 0.0, hit.center_up);
            Target {
                centre,
                eye: centre + rc_movement::Vec3::new(0.0, 0.0, hit.half_height * 0.7),
            }
        })
        .collect();
    let fight = &mut *fight;
    let events = fight.battle.step_with(dt, &*collision.0, target, &others);
    let now = fight.battle.time;
    let mut meshes = meshes;
    for e in events {
        match e {
            BattleEvent::SpawnerOn {
                spawner,
                by_trigger,
            } => {
                let f = &fight.battle.spawners[spawner].factory;
                log.line(format!(
                    "{PART}: t={now:.1}s spawner {} ({}) switched on by {}",
                    f.name,
                    f.tag,
                    if by_trigger {
                        "its walk-in trigger"
                    } else {
                        "the player coming near (scripted in the original)"
                    }
                ));
            }
            BattleEvent::Spawned {
                id,
                spawner,
                feet,
                yaw,
            } => {
                let p = fight.battle.spawners[spawner].params.clone();
                let tl = looks.by_type.get(&p.kind.id);
                let mesh = match (tl.map(|t| &t.look), meshes.as_mut()) {
                    (Some(look), Some(meshes)) => {
                        let points = look.posed(Pose::Idle, 0.0);
                        let (pos, normals) = look.wedge_attributes(&points);
                        let mut m = Mesh::new(
                            PrimitiveTopology::TriangleList,
                            RenderAssetUsages::default(),
                        );
                        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
                        m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
                        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, look.uvs());
                        m.insert_indices(Indices::U32(look.indices.clone()));
                        Some(meshes.add(m))
                    }
                    _ => None,
                };
                let mut e = commands.spawn((
                    Name::new(format!("{} {id}", p.kind.name)),
                    Droid {
                        id,
                        kind: p.kind.id.clone(),
                        pose: Pose::Idle,
                        frame: 0.0,
                        last_fire: f32::NEG_INFINITY,
                        mesh: mesh.clone(),
                    },
                    Health::new(p.combat.health),
                    Team::Separatist,
                    if p.kind.body == "organic" {
                        Body::Organic
                    } else {
                        Body::Droid
                    },
                    Hittable {
                        radius: p.movement.collision_radius,
                        half_height: p.movement.collision_height,
                        center_up: p.movement.collision_height,
                    },
                    Transform::from_translation(to_bevy(feet))
                        .with_rotation(Quat::from_rotation_y(-yaw)),
                    Visibility::default(),
                    DespawnOnExit(GameState::InGame),
                ));
                if let (Some(mesh), Some(material)) = (mesh, tl.and_then(|t| t.material.as_ref())) {
                    // The points move every frame, so Bevy's stored outline
                    // would be wrong: don't let it skip drawing by it.
                    e.insert((
                        Mesh3d(mesh),
                        MeshMaterial3d(material.clone()),
                        NoFrustumCulling,
                    ));
                }
                fight.entities.insert(id, e.id());
                fight.stats.spawned += 1;
                log.line(format!(
                    "{PART}: t={now:.1}s droid {id} ({}, health {}) spawned by {} at ({:.0}, {:.0}, {:.0}) facing {:.0} deg",
                    p.kind.id,
                    p.combat.health,
                    fight.battle.spawners[spawner].factory.name,
                    feet.x,
                    feet.y,
                    feet.z,
                    yaw.to_degrees()
                ));
            }
            BattleEvent::Ai { id, event } => {
                let Some(&entity) = fight.entities.get(&id) else {
                    continue;
                };
                match event {
                    AiEvent::Fire {
                        origin, direction, ..
                    } => {
                        fight.stats.shots += 1;
                        if let Ok(mut d) = droids.get_mut(entity) {
                            d.last_fire = now;
                        }
                        fire.write(FireShot {
                            shooter: entity,
                            origin: to_bevy(origin),
                            direction: to_bevy(direction),
                            weapon: fight
                                .battle
                                .fighters
                                .iter()
                                .find(|f| f.id == id)
                                .map_or_else(
                                    || fight.battle.params.combat.weapon.clone(),
                                    |f| f.params.combat.weapon.clone(),
                                ),
                        });
                    }
                    AiEvent::Spotted { distance } => {
                        fight.stats.first_spotted.get_or_insert(now);
                        log.line(format!("{PART}: t={now:.1}s droid {id} spotted the player {distance:.0} uu away"));
                    }
                    AiEvent::ModeChanged { from, to } => log.line(format!(
                        "{PART}: t={now:.1}s droid {id}: {} -> {}",
                        from.name(),
                        to.name()
                    )),
                    AiEvent::ManeuverChanged { from, to } => log.line(format!(
                        "{PART}: t={now:.1}s droid {id}: maneuver {from} -> {to}"
                    )),
                    AiEvent::Route { nodes, length } => log.line(format!(
                        "{PART}: t={now:.1}s droid {id}: route of {nodes} nodes, {length:.0} uu"
                    )),
                    other => log.line(format!("{PART}: t={now:.1}s droid {id}: {other:?}")),
                }
            }
        }
    }

    let want = if fight.battle.in_fight() {
        MusicMood::Battle
    } else {
        MusicMood::Calm
    };
    if *mood != want {
        *mood = want;
        log.line(format!("{PART}: t={now:.1}s music mood {want:?}"));
    }
    if now >= fight.next_summary {
        fight.next_summary = now + SUMMARY_EVERY;
        summary(fight, &log);
    }
}

/// Put each droid's entity where its body is.
fn place_droids(fight: Res<Fight>, mut droids: Query<(&Droid, &mut Transform, &mut Hittable)>) {
    for f in &fight.battle.fighters {
        let Some(&e) = fight.entities.get(&f.id) else {
            continue;
        };
        let Ok((_, mut t, mut hit)) = droids.get_mut(e) else {
            continue;
        };
        t.translation = to_bevy(f.pawn.feet());
        t.rotation = Quat::from_rotation_y(-f.facing);
        let h = if f.pawn.crouched {
            f.params.movement.crouch_height
        } else {
            f.params.movement.collision_height
        };
        if hit.half_height != h {
            hit.half_height = h;
            hit.center_up = h;
        }
    }
}

/// Damage and deaths from the weapons part, and the player's shots heard.
#[allow(clippy::too_many_arguments)]
fn react_to_combat(
    mut fight: ResMut<Fight>,
    mut damage: MessageReader<Damage>,
    mut died: MessageReader<Died>,
    mut shots: MessageReader<FireShot>,
    mut landed: MessageReader<ShotLanded>,
    mut blasts: MessageReader<Blast>,
    stunned: Query<(&Droid, &Stunned), Added<Stunned>>,
    droids: Query<&Droid>,
    players: Query<(), With<Player>>,
    transforms: Query<&Transform>,
    log: Res<GameLog>,
) {
    let fight = &mut *fight;
    let now = fight.battle.time;
    for d in damage.read() {
        if players.contains(d.target) {
            if d.instigator.is_some_and(|i| droids.contains(i)) {
                fight.stats.hits_on_player += 1;
                fight.stats.damage_to_player += d.amount;
            }
            continue;
        }
        let Ok(droid) = droids.get(d.target) else {
            continue;
        };
        fight.stats.droids_hurt += 1;
        let from = d
            .instigator
            .and_then(|i| transforms.get(i).ok())
            .map(|t| to_unreal(t.translation) + rc_movement::Vec3::new(0.0, 0.0, 84.0));
        for e in fight.battle.damaged(droid.id, from) {
            if let BattleEvent::Ai { event, .. } = e {
                log.line(format!(
                    "{PART}: t={now:.1}s droid {} hurt ({:.0} damage): {event:?}",
                    droid.id, d.amount
                ));
            }
        }
    }
    for d in died.read() {
        let Ok(droid) = droids.get(d.entity) else {
            continue;
        };
        fight.battle.killed(droid.id);
        fight.stats.droids_destroyed += 1;
        let f = fight.battle.fighters.iter().find(|f| f.id == droid.id);
        log.line(format!(
            "{PART}: t={now:.1}s droid {} destroyed (fired {} shots, walked {:.0} uu)",
            droid.id,
            f.map_or(0, |f| f.shots_fired),
            f.map_or(0.0, |f| f.distance_walked)
        ));
    }
    for s in shots.read() {
        if players.contains(s.shooter) {
            fight.battle.hear(to_unreal(s.origin));
        }
    }
    // Grenade blasts are heard by every droid in earshot.
    for b in blasts.read() {
        fight.battle.hear(to_unreal(b.point));
    }
    for (droid, s) in &stunned {
        fight.battle.stun(droid.id, s.left);
        log.line(format!(
            "{PART}: t={now:.1}s droid {} stunned for {:.1} s",
            droid.id, s.left
        ));
    }
    for l in landed.read() {
        if !droids.contains(l.shooter) {
            continue;
        }
        let st = &mut fight.stats;
        match l.target {
            Some(t) if droids.contains(t) => {
                st.bolts_on_droids += 1;
                if let (Ok(a), Ok(b)) = (droids.get(l.shooter), droids.get(t)) {
                    let pos = |e: Entity| transforms.get(e).map(|t| to_unreal(t.translation)).ok();
                    let gap = pos(l.shooter)
                        .zip(pos(t))
                        .map_or(-1.0, |(p, q)| (p - q).length());
                    log.line(format!(
                        "{PART}: t={now:.1}s droid {}'s bolt hit droid {} ({:.0} uu along its flight; the two stand {gap:.0} uu apart)",
                        a.id, b.id, l.distance
                    ));
                }
            }
            Some(_) => {}
            None if l.on_level => st.bolts_on_level += 1,
            None => st.bolts_nowhere += 1,
        }
    }
}

/// Which animation a droid should be playing.
fn choose_pose(f: &rc_ai::Fighter, just_fired: bool) -> Pose {
    if f.mode == Mode::Dead {
        return Pose::Death;
    }
    let v = f.pawn.velocity;
    let speed = (v.x * v.x + v.y * v.y).sqrt();
    if f.pawn.crouched {
        return if just_fired {
            Pose::CrouchFire
        } else {
            Pose::Crouch
        };
    }
    if speed > 30.0 {
        let rel = rc_ai::brain::angle_diff(v.y.atan2(v.x), f.facing).to_degrees();
        return if rel.abs() <= 45.0 {
            if speed > 0.75 * f.params.movement.ground_speed {
                Pose::Run
            } else {
                Pose::Walk
            }
        } else if rel.abs() >= 135.0 {
            Pose::WalkBack
        } else if rel > 0.0 {
            // Unreal's yaw grows clockwise seen from above: to the right.
            Pose::StepRight
        } else {
            Pose::StepLeft
        };
    }
    if just_fired {
        Pose::Fire
    } else {
        Pose::Idle
    }
}

/// Advance each droid's animation and pose its mesh.
fn animate(
    time: Res<Time>,
    fight: Res<Fight>,
    looks: Res<Looks>,
    mut droids: Query<&mut Droid>,
    meshes: Option<ResMut<Assets<Mesh>>>,
) {
    let dt = time.delta_secs();
    let now = fight.battle.time;
    let mut meshes = meshes;
    for f in &fight.battle.fighters {
        let Some(&e) = fight.entities.get(&f.id) else {
            continue;
        };
        let Ok(mut d) = droids.get_mut(e) else {
            continue;
        };
        let pose = choose_pose(f, now - d.last_fire < FIRE_POSE_TIME);
        if pose != d.pose {
            d.pose = pose;
            d.frame = 0.0;
        }
        let Some(look) = looks.by_type.get(&d.kind).map(|t| &t.look) else {
            continue;
        };
        let clip = look.clip(d.pose);
        let last = (clip.num_frames - 1.0).max(0.0);
        let before = d.frame;
        d.frame = if d.pose.once() {
            (d.frame + dt * clip.rate).min(last)
        } else {
            (d.frame + dt * clip.rate).rem_euclid(clip.num_frames.max(1.0))
        };
        // A body that finished falling doesn't need posing again.
        if d.pose.once() && before == last && d.frame == last {
            continue;
        }
        let (Some(handle), Some(meshes)) = (&d.mesh, meshes.as_mut()) else {
            continue;
        };
        let points = look.posed(d.pose, d.frame);
        let (pos, normals) = look.wedge_attributes(&points);
        if let Some(mut m) = meshes.get_mut(handle) {
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        }
    }
}
