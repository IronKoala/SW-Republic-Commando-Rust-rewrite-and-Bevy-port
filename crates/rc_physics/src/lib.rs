//! Loose physics props in the game: the rocks, crates, barrels and debris
//! the original simulates with its Karma physics engine, the `MegaForce`
//! actors that push them when a level script triggers them, pushes from
//! shots and blasts, and the rock slides level scripts make
//! (`ACTION_SpawnActor`, geo_04's `K_GEO4_FallingRock1`-`3`).
//!
//! The props and their settings come from the map and the game's class
//! defaults (`rc_level::karma`, handed over by the level part as
//! `LevelKarma`). The rules they follow are the original's, read from its
//! scripts (`KarmaProp`, `KActor`, `MegaForce`, `HurtRadius`) and its
//! `Engine.dll` (units, gravity, damping, the speed cap, when a landing
//! sound plays); each is written down in docs/DESIGN.md ("Physics props")
//! with where it was found, and so are the places this differs.
//!
//! Karma itself (the MathEngine library) is replaced by Avian
//! (https://github.com/avianphysics/avian). One Bevy unit is 50 Unreal
//! units, which is also one Karma unit (`K_U2MEScale` 0.02).

use avian3d::prelude::*;
use bevy::prelude::*;
use rc_combat::{Blast, ShotLanded};
use rc_effects_plugin::SpawnEffect;
use rc_game_core::units::UU_PER_UNIT;
use rc_game_core::{GameLog, ScriptEvent, ScriptSpawnActor};
use rc_game_core::{GameState, Play};
use rc_level::karma::{Impulse, KarmaBody, Shape};
use rc_level_plugin::{LevelKarma, LevelParts};
use rc_level_viewer::draw::MoverPart;
use std::collections::{HashMap, HashSet};

const PART: &str = "physics";

/// The level's gravity (`PhysicsVolume.Gravity`, -1100 uu/s² down; the
/// maps keep the default), in Bevy units.
pub const GRAVITY: f32 = -1100.0 / UU_PER_UNIT;
/// `Actor.MaxKarmaSpeed` (800 Karma units/s, checked after each step in
/// `Engine.dll`), in Bevy units (= Karma units).
pub const MAX_SPEED: f32 = 800.0;
/// Karma's body mass limits around the world's default mass (1):
/// `Engine.dll` clamps every mass to 0.01..100 times it.
pub const MASS_RANGE: (f32, f32) = (0.01, 100.0);

/// What collides with what.
#[derive(PhysicsLayer, Clone, Copy, Debug, Default)]
pub enum Layer {
    /// The level: BSP, terrain, props that don't move, blocking volumes.
    #[default]
    Level,
    /// Physics props that block other physics props (`bBlockKarma`).
    Body,
    /// Physics props that don't (only the level stops them).
    Ghost,
}

pub struct PhysicsPropsPlugin;

impl Plugin for PhysicsPropsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsPlugins::default())
            .insert_resource(Gravity(Vec3::new(0.0, GRAVITY, 0.0)))
            .init_resource::<Built>()
            .add_message::<ScriptEvent>()
            .add_message::<ScriptSpawnActor>()
            .add_message::<rc_game_core::RaiseEvent>()
            .add_message::<ShotLanded>()
            .add_message::<Blast>()
            .add_message::<SpawnEffect>()
            .add_systems(OnExit(GameState::InGame), forget)
            .add_systems(
                Update,
                (
                    pause,
                    build,
                    attach_drawing,
                    triggers_and_spawns,
                    shots_and_blasts,
                    apply_pushes,
                    leave_parts,
                    follow,
                    impacts,
                    life_spans,
                    level_parts,
                    came_to_rest,
                    summary,
                )
                    .chain(),
            );
    }
}

/// The physics as the level started, set up once a level is loaded.
#[derive(Resource, Default)]
struct Built {
    done: bool,
    /// Body entity by its map export.
    by_export: HashMap<usize, Entity>,
    /// Hidden models of script-made props (`MoverPart` with the action's
    /// export index), by action.
    templates: HashMap<usize, Vec<Entity>>,
    /// Static collision of each part a script may switch off.
    parts: HashMap<usize, Entity>,
    /// Impulses to give at the next step: body, impulse (Bevy units x
    /// mass / s), where (Bevy).
    pushes: Vec<(Entity, Vec3, Vec3)>,
    /// Count of props made by scripts, for the log.
    made: usize,
    /// Parts switched off (their physics collision is off too).
    parts_off: HashSet<usize>,
}

/// One physics prop and its game rules state.
#[derive(Component, Debug)]
pub struct Prop {
    pub body: KarmaBody,
    /// Where it started (Bevy), so its drawn model (placed at the start)
    /// can follow it.
    pub start: Transform,
    /// `DamageThreshold`, counting down.
    pub threshold: i32,
    pub take_shot: bool,
    pub health: i32,
    /// Simulated (`Physics` is `PHYS_Karma`).
    pub karma: bool,
    /// Karma physics but asleep (not `KStartEnabled`, not woken yet):
    /// held still. Karma wakes a resting body when it is shot, pushed or
    /// triggered, or when a moving body touches it.
    pub asleep: bool,
    /// When it goes (`KActor` `LifeSpan`), or its `LifeTime` timer
    /// (`KarmaProp`), level time.
    pub life_end: Option<f32>,
    pub timing_out: bool,
    /// `LastImpactTime`.
    pub last_impact: f32,
    /// Its triangles in the level's collision are off (it moved).
    pub part_off: bool,
    /// Its coming to rest was logged.
    pub rested: bool,
}

/// A drawn model following a physics prop: its transform is the prop's
/// motion since it started.
#[derive(Component, Debug, Clone, Copy)]
struct Follow {
    body: Entity,
    start_inv: Mat4,
}

/// Marks hidden templates (copied for each prop a script makes).
#[derive(Component)]
struct Template;

fn sw(v: [f32; 3]) -> Vec3 {
    Vec3::new(v[0], v[2], v[1])
}

fn to_bevy(v: [f32; 3]) -> Vec3 {
    sw(v) / UU_PER_UNIT
}

/// Unreal rotation to a Bevy turn (the swap of Y and Z on both sides).
pub fn turn(rotation: [i32; 3]) -> Quat {
    let ax = rc_level::actors::rotation_axes(rotation);
    Quat::from_mat3(&Mat3::from_cols(sw(ax[0]), sw(ax[2]), sw(ax[1]))).normalize()
}

/// A Bevy turn taking the Y axis onto `axis` (Avian capsules and cylinders
/// run along Y).
fn y_onto(axis: Vec3) -> Quat {
    Quat::from_rotation_arc(Vec3::Y, axis.normalize_or(Vec3::Y))
}

/// The body's collision shape, in its own frame (Bevy units).
pub fn collider(shapes: &[Shape]) -> Option<Collider> {
    let u = UU_PER_UNIT;
    let mut parts: Vec<(Vec3, Quat, Collider)> = Vec::new();
    for s in shapes {
        match s {
            Shape::Convex(points) => {
                let pts: Vec<Vec3> = points.iter().map(|p| sw(*p) / u).collect();
                parts.push((Vec3::ZERO, Quat::IDENTITY, Collider::convex_hull(&pts)?));
            }
            Shape::Sphere { centre, radius } => parts.push((
                sw(*centre) / u,
                Quat::IDENTITY,
                Collider::sphere(radius / u),
            )),
            Shape::Box { centre, axes, size } => {
                let rot = Quat::from_mat3(&Mat3::from_cols(sw(axes[0]), sw(axes[2]), sw(axes[1])));
                parts.push((
                    sw(*centre) / u,
                    rot.normalize(),
                    Collider::cuboid(size[0] / u, size[2] / u, size[1] / u),
                ))
            }
            Shape::Capsule {
                centre,
                axis,
                radius,
                length,
            } => parts.push((
                sw(*centre) / u,
                y_onto(sw(*axis)),
                Collider::capsule(radius / u, length / u),
            )),
            Shape::Cylinder {
                centre,
                axis,
                radius,
                height,
            } => parts.push((
                sw(*centre) / u,
                y_onto(sw(*axis)),
                Collider::cylinder(radius / u, height / u),
            )),
        }
    }
    match parts.len() {
        0 => None,
        1 if parts[0].0 == Vec3::ZERO && parts[0].1 == Quat::IDENTITY => Some(parts.remove(0).2),
        _ => Some(Collider::compound(parts)),
    }
}

/// Karma's stored inertia (`KMeshProps`, for a mass of 1, Karma units,
/// xx xy xz yy yz zz) as a Bevy tensor for `mass`, scaled by the draw scale
/// squared.
pub fn inertia(i: [f32; 6], mass: f32, scale: f32) -> Mat3 {
    let k = mass * scale * scale;
    let (xx, xy, xz, yy, yz, zz) = (i[0], i[1], i[2], i[3], i[4], i[5]);
    // Swapping Y and Z swaps those rows and columns.
    Mat3::from_cols(
        Vec3::new(xx, xz, xy),
        Vec3::new(xz, zz, yz),
        Vec3::new(xy, yz, yy),
    ) * k
}

/// The friction Avian uses for a prop's `KFriction`: Karma gives no
/// friction below 0.01 (`Engine.dll`).
pub fn friction(k: f32) -> f32 {
    if k < 0.01 {
        0.0
    } else {
        k
    }
}

fn spawn_body(commands: &mut Commands, body: &KarmaBody, log: &GameLog) -> Option<Entity> {
    let Some(shape) = collider(&body.shapes) else {
        log.line(format!(
            "{PART}: {} ({}): its Karma shape can't be made; left out",
            body.name, body.class
        ));
        return None;
    };
    let p = &body.params;
    let mass = p.mass.clamp(MASS_RANGE.0, MASS_RANGE.1);
    let scale = (body.scale[0].abs() + body.scale[1].abs() + body.scale[2].abs()) / 3.0;
    let start =
        Transform::from_translation(to_bevy(body.location)).with_rotation(turn(body.rotation));
    let layers = if body.blocks_karma {
        CollisionLayers::new(Layer::Body, [Layer::Level, Layer::Body])
    } else {
        CollisionLayers::new(Layer::Ghost, [Layer::Level])
    };
    let mut e = commands.spawn((
        Name::new(format!("physics prop {}", body.name)),
        start,
        // Asleep (`KStartEnabled` off) or not simulated: held still until
        // woken (see `wake`).
        if body.karma_now && body.params.start_enabled {
            RigidBody::Dynamic
        } else {
            RigidBody::Static
        },
        shape,
        layers,
        Mass(mass),
        NoAutoMass,
        LinearDamping(p.linear_damping),
        AngularDamping(p.angular_damping),
        GravityScale(p.gravity_scale),
        MaxLinearSpeed(MAX_SPEED),
        Friction {
            dynamic_coefficient: friction(p.friction),
            static_coefficient: friction(p.friction),
            combine_rule: CoefficientCombine::Multiply,
        },
        Restitution {
            coefficient: p.restitution.clamp(0.0, 1.0),
            combine_rule: CoefficientCombine::Multiply,
        },
        DespawnOnExit(GameState::InGame),
    ));
    e.insert((
        CenterOfMass(sw([
            body.com_nudge[0] * body.scale[0],
            body.com_nudge[1] * body.scale[1],
            body.com_nudge[2] * body.scale[2],
        ])),
        NoAutoCenterOfMass,
    ));
    if let Ok(i) = AngularInertia::try_from_mat3(inertia(body.inertia, mass, scale)) {
        e.insert((i, NoAutoAngularInertia));
    }
    if body.karma_now && p.start_enabled {
        e.insert(LinearVelocity(to_bevy(p.start_lin_vel)));
    }
    e.insert(Prop {
        body: body.clone(),
        start,
        threshold: body.damage_threshold,
        take_shot: body.take_shot,
        health: body.health,
        karma: body.karma_now,
        asleep: body.karma_now && !body.params.start_enabled,
        life_end: None,
        timing_out: false,
        last_impact: f32::MIN,
        part_off: false,
        rested: false,
    });
    Some(e.id())
}

fn triangles_collider(tris: &[[[f32; 3]; 3]]) -> Option<Collider> {
    if tris.is_empty() {
        return None;
    }
    let mut v = Vec::with_capacity(tris.len() * 3);
    let mut idx = Vec::with_capacity(tris.len());
    for t in tris {
        let n = v.len() as u32;
        v.extend(t.iter().map(|p| to_bevy(*p)));
        idx.push([n, n + 1, n + 2]);
    }
    Some(Collider::trimesh(v, idx))
}

fn level_body(commands: &mut Commands, name: String, c: Collider) -> Entity {
    commands
        .spawn((
            Name::new(name),
            Transform::IDENTITY,
            RigidBody::Static,
            c,
            CollisionLayers::new(Layer::Level, [Layer::Body, Layer::Ghost]),
            // Bodies' own values count against the level (`Engine.dll`):
            // a product with 1 leaves them.
            Friction {
                dynamic_coefficient: 1.0,
                static_coefficient: 1.0,
                combine_rule: CoefficientCombine::Multiply,
            },
            Restitution {
                coefficient: 1.0,
                combine_rule: CoefficientCombine::Multiply,
            },
            DespawnOnExit(GameState::InGame),
        ))
        .id()
}

fn forget(mut built: ResMut<Built>) {
    *built = Built::default();
}

fn pause(state: Option<Res<State<Play>>>, mut time: ResMut<Time<Physics>>) {
    // Play only exists in the game; nothing moves elsewhere.
    let running = state.is_none_or(|s| *s.get() == Play::Running);
    if running && time.is_paused() {
        time.unpause();
    } else if !running && !time.is_paused() {
        time.pause();
    }
}

fn build(
    mut commands: Commands,
    karma: Option<Res<LevelKarma>>,
    mut built: ResMut<Built>,
    log: Res<GameLog>,
) {
    let Some(karma) = karma else { return };
    if built.done {
        return;
    }
    built.done = true;
    if let Some(c) = triangles_collider(&karma.ground) {
        level_body(&mut commands, "level collision".into(), c);
    }
    for (export, tris) in &karma.parts {
        if let Some(c) = triangles_collider(tris) {
            let e = level_body(&mut commands, format!("level part {export}"), c);
            built.parts.insert(*export, e);
        }
    }
    let (mut moving, mut resting, mut still) = (0, 0, 0);
    for b in &karma.bodies {
        let Some(e) = spawn_body(&mut commands, b, &log) else {
            continue;
        };
        if let Some(x) = b.export {
            built.by_export.insert(x, e);
        }
        match (b.karma_now, b.params.start_enabled) {
            (true, true) => moving += 1,
            (true, false) => resting += 1,
            _ => still += 1,
        }
    }
    log.line(format!(
        "{PART}: {} level triangles and {} switchable parts solid for physics props; {} props: {moving} moving from the start, {resting} resting until woken, {still} standing still until triggered",
        karma.ground.len(),
        built.parts.len(),
        moving + resting + still
    ));
    for b in &karma.bodies {
        if b.params.start_ang_vel != [0.0; 3] {
            log.line(format!(
                "{PART}: {}: its starting spin (KStartAngVel {:?}) isn't given (not done yet)",
                b.name, b.params.start_ang_vel
            ));
        }
    }
}

/// Drawn models of props follow them; script props' models are hidden
/// templates.
#[allow(clippy::type_complexity)]
fn attach_drawing(
    mut commands: Commands,
    karma: Option<Res<LevelKarma>>,
    mut built: ResMut<Built>,
    parts: Query<(Entity, &MoverPart), (Without<Follow>, Without<Template>)>,
    props: Query<&Prop>,
) {
    let Some(karma) = karma else { return };
    if !built.done {
        return;
    }
    for (e, part) in &parts {
        if let Some(&body) = built.by_export.get(&part.0) {
            let Ok(p) = props.get(body) else { continue };
            commands.entity(e).insert(Follow {
                body,
                start_inv: p.start.to_matrix().inverse(),
            });
        } else if karma.spawns.iter().any(|s| s.action == part.0) {
            commands.entity(e).insert((Template, Visibility::Hidden));
            built.templates.entry(part.0).or_default().push(e);
        }
    }
}

/// `KarmaProp.Trigger` / `KActor.Trigger`.
fn trigger(p: &mut Prop, e: Entity, commands: &mut Commands, now: f32, log: &GameLog, by: &str) {
    if p.body.is_karma_prop {
        p.take_shot = true;
        if p.body.life_time > 0.0 {
            p.life_end = Some(now + p.body.life_time);
        }
    } else {
        if !p.timing_out && p.body.life_time > 0.0 {
            p.timing_out = true;
            p.life_end = Some(now + p.body.life_time);
        }
        p.take_shot = true;
    }
    if !p.karma && p.body.trigger_makes_karma {
        p.karma = true;
        p.asleep = true;
        log.line(format!(
            "{PART}: t={now:.2} {} triggered by {by}: physics on",
            p.body.name
        ));
    } else {
        log.line(format!(
            "{PART}: t={now:.2} {} triggered by {by}: woken",
            p.body.name
        ));
    }
    wake(p, e, commands);
}

/// `KWake`: a held prop starts moving.
fn wake(p: &mut Prop, e: Entity, commands: &mut Commands) {
    if p.karma && p.asleep {
        p.asleep = false;
        commands.entity(e).insert(RigidBody::Dynamic);
    }
    commands.entity(e).remove::<Sleeping>();
}

/// `TakeDamage` on a physics prop (`KarmaProp.AffectKarma`,
/// `KActor.TakeDamage`). Returns the push to give, if any.
#[allow(clippy::too_many_arguments)]
fn take_damage(
    p: &mut Prop,
    e: Entity,
    damage: f32,
    at: Vec3,
    momentum_uu: Vec3,
    now: f32,
    events: &mut Vec<String>,
    commands: &mut Commands,
) -> Option<(Entity, Vec3, Vec3)> {
    let damage = damage as i32;
    if p.body.is_karma_prop {
        // TriggerEvent first, every time.
        events.extend(p.body.event.clone());
        p.threshold -= damage;
        if p.take_shot && p.threshold <= 0 {
            if p.body.life_time > 0.0 {
                p.life_end = Some(now + p.body.life_time);
            }
            return wake_and_push(p, e, at, momentum_uu, commands);
        }
    } else {
        if !p.timing_out && p.body.life_time > 0.0 {
            p.timing_out = true;
            p.life_end = Some(now + p.body.life_time);
        }
        if p.health != 0 {
            p.health -= damage;
            if p.health <= 0 {
                p.life_end = Some(now);
            }
        }
        p.threshold -= damage;
        if p.take_shot && p.threshold <= 0 {
            let push = wake_and_push(p, e, at, momentum_uu, commands);
            events.extend(p.body.event.clone());
            return push;
        }
    }
    None
}

/// `KWake` and `KAddImpulse(momentum, at)`: nothing happens to a prop that
/// isn't simulated. Karma scales the impulse to Karma units (0.02), so
/// the speed change is momentum / mass in Unreal units per second.
fn wake_and_push(
    p: &mut Prop,
    e: Entity,
    at: Vec3,
    momentum_uu: Vec3,
    commands: &mut Commands,
) -> Option<(Entity, Vec3, Vec3)> {
    if !p.karma {
        return None;
    }
    wake(p, e, commands);
    let mass = p.body.params.mass.clamp(MASS_RANGE.0, MASS_RANGE.1);
    // Avian's impulse is mass x speed change; Karma's is in the prop's
    // own KMass.
    Some((
        e,
        sw(momentum_uu.to_array()) / UU_PER_UNIT * (mass / p.body.params.mass.max(1e-6)),
        at,
    ))
}

fn send_events(names: Vec<String>, by: &str, raise: &mut MessageWriter<rc_game_core::RaiseEvent>) {
    for n in names {
        if n.is_empty() || n.eq_ignore_ascii_case("None") {
            continue;
        }
        raise.write(rc_game_core::RaiseEvent {
            name: n,
            by: by.to_string(),
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn triggers_and_spawns(
    mut commands: Commands,
    karma: Option<Res<LevelKarma>>,
    mut built: ResMut<Built>,
    mut events: MessageReader<ScriptEvent>,
    mut spawns: MessageReader<ScriptSpawnActor>,
    mut raise: MessageWriter<rc_game_core::RaiseEvent>,
    mut props: Query<(Entity, &mut Prop, &Transform)>,
    spatial: SpatialQuery,
    time: Res<Time>,
    log: Res<GameLog>,
) {
    let Some(karma) = karma else {
        events.clear();
        spawns.clear();
        return;
    };
    if !built.done {
        return;
    }
    let now = time.elapsed_secs();
    for s in spawns.read() {
        let Some(sp) = karma.spawns.iter().find(|x| x.action == s.action) else {
            log.line(format!(
                "{PART}: t={now:.2} {} makes {}: not a physics prop (not done yet)",
                s.by, s.class
            ));
            continue;
        };
        let Ok(body) = &sp.body else { continue };
        let Some(e) = spawn_body(&mut commands, body, &log) else {
            continue;
        };
        built.made += 1;
        let start =
            Transform::from_translation(to_bevy(body.location)).with_rotation(turn(body.rotation));
        let n = built.templates.get(&s.action).map_or(0, |t| t.len());
        for &t in built.templates.get(&s.action).into_iter().flatten() {
            commands
                .entity(t)
                .clone_and_spawn()
                .remove::<(MoverPart, Template)>()
                .insert((
                    Visibility::Inherited,
                    Follow {
                        body: e,
                        start_inv: start.to_matrix().inverse(),
                    },
                    DespawnOnExit(GameState::InGame),
                ));
        }
        log.line(format!(
            "{PART}: t={now:.2} {} makes {} at ({:.0}, {:.0}, {:.0}) ({} so far){}",
            s.by,
            sp.class,
            body.location[0],
            body.location[1],
            body.location[2],
            built.made,
            if n == 0 {
                "; its model wasn't drawn"
            } else {
                ""
            }
        ));
    }
    for ev in events.read() {
        let name = &ev.name;
        // Props with that tag: `Trigger`.
        for (e, mut p, _) in &mut props {
            if p.body.tag.eq_ignore_ascii_case(name) {
                trigger(&mut p, e, &mut commands, now, &log, name);
            }
        }
        // MegaForces with that tag.
        for m in karma
            .megaforces
            .iter()
            .filter(|m| m.tag.eq_ignore_ascii_case(name))
        {
            let origin = to_bevy(m.location);
            let dir_u = rc_level::karma::facing(m.rotation);
            let dir = sw(dir_u).normalize_or_zero();
            let mut reached = 0;
            let mut fired = Vec::new();
            // Who it reaches, and where: IT_Line traces through every
            // actor on the line (`TraceActors`), IT_Sphere takes every
            // actor within `ForceDistance` (`CollidingActors`).
            let mut hits: Vec<(Entity, Vec3)> = Vec::new();
            match m.kind {
                Impulse::Line => {
                    if let Ok(d) = Dir3::new(dir) {
                        for h in spatial.ray_hits(
                            origin,
                            d,
                            m.distance / UU_PER_UNIT,
                            u32::MAX,
                            true,
                            &SpatialQueryFilter::from_mask([Layer::Body, Layer::Ghost]),
                        ) {
                            hits.push((h.entity, origin + dir * h.distance));
                        }
                    }
                }
                Impulse::Sphere => {
                    for (e, p, t) in &props {
                        let d = (t.translation - origin).length() * UU_PER_UNIT;
                        if d <= m.distance + p.body.collision_radius {
                            hits.push((e, t.translation));
                        }
                    }
                }
            }
            for (e, hit) in hits {
                let Ok((_, mut p, t)) = props.get_mut(e) else {
                    continue;
                };
                reached += 1;
                if m.auto_activate {
                    trigger(&mut p, e, &mut commands, now, &log, &m.name);
                }
                if !p.karma {
                    continue;
                }
                let (amount, at) = match m.kind {
                    Impulse::Line => (
                        Vec3::from_array(dir_u) * m.amount,
                        if m.at_centre { t.translation } else { hit },
                    ),
                    Impulse::Sphere => {
                        let away = if m.emitter_direction {
                            Vec3::from_array(dir_u)
                        } else {
                            let d = (t.translation - origin).normalize_or_zero();
                            // Back to Unreal axes.
                            Vec3::new(d.x, d.z, d.y)
                        };
                        (away * m.amount, t.translation)
                    }
                };
                if let Some(push) =
                    take_damage(&mut p, e, 0.0, at, amount, now, &mut fired, &mut commands)
                {
                    built.pushes.push(push);
                }
            }
            log.line(format!(
                "{PART}: t={now:.2} {} ({:?}, {} uu, force {}) on {name}: {reached} props reached",
                m.name, m.kind, m.distance, m.amount
            ));
            send_events(fired, &m.name, &mut raise);
        }
    }
}

/// `KDamageImpulse` and `KDeathUpKick` of a damage type (the class's own,
/// else its `...Defaults` class, as `Properties.u` stores the base ones).
fn impulse_of(karma: &LevelKarma, damage_type: &str) -> (f32, f32) {
    let k = damage_type.to_ascii_lowercase();
    karma
        .impulses
        .get(&k)
        .or_else(|| karma.impulses.get(&format!("{k}defaults")))
        .copied()
        .unwrap_or((0.0, 0.0))
}

#[allow(clippy::too_many_arguments)]
fn shots_and_blasts(
    mut commands: Commands,
    karma: Option<Res<LevelKarma>>,
    mut built: ResMut<Built>,
    mut shots: MessageReader<ShotLanded>,
    mut blasts: MessageReader<Blast>,
    mut raise: MessageWriter<rc_game_core::RaiseEvent>,
    mut props: Query<(Entity, &mut Prop, &Transform)>,
    shooters: Query<&GlobalTransform>,
    spatial: SpatialQuery,
    time: Res<Time>,
    log: Res<GameLog>,
) {
    let Some(karma) = karma else {
        shots.clear();
        blasts.clear();
        return;
    };
    let now = time.elapsed_secs();
    let mut fired = Vec::new();
    for s in shots.read() {
        if s.target.is_some() {
            continue;
        }
        let Ok(from) = shooters.get(s.shooter) else {
            continue;
        };
        let along = s.point - from.translation();
        let Ok(dir) = Dir3::new(along) else { continue };
        // From where the shot started (its length), to just past where it
        // landed: the first prop on the way.
        let len = s.distance / UU_PER_UNIT;
        let origin = s.point - *dir * len;
        let Some(hit) = spatial.cast_ray(
            origin,
            dir,
            len + 0.2,
            true,
            &SpatialQueryFilter::from_mask([Layer::Body, Layer::Ghost]),
        ) else {
            continue;
        };
        let Ok((e, mut p, _)) = props.get_mut(hit.entity) else {
            continue;
        };
        let (k, _) = impulse_of(&karma, &s.damage_type);
        // Weapon traces: momentum = Dir * KDamageImpulse, at the hit.
        let d = Vec3::new(dir.x, dir.z, dir.y);
        let at = origin + *dir * hit.distance;
        let push = take_damage(
            &mut p,
            e,
            s.damage,
            at,
            d * k,
            now,
            &mut fired,
            &mut commands,
        );
        log.line(format!(
            "{PART}: t={now:.2} shot ({}, {}) hit {}: {}",
            s.weapon,
            s.damage_type,
            p.body.name,
            if push.is_some() {
                format!("pushed ({k} momentum)")
            } else {
                format!(
                    "not moved (threshold {}, takes shots {})",
                    p.threshold, p.take_shot
                )
            }
        ));
        built.pushes.extend(push);
    }
    for b in blasts.read() {
        let (k, up) = impulse_of(&karma, &b.damage_type);
        let mut reached = 0;
        for (e, mut p, t) in &mut props {
            // HurtRadius: VisibleCollidingActors within DamageRadius.
            let to = t.translation - b.point;
            let dist = to.length() * UU_PER_UNIT;
            if dist > b.radius + p.body.collision_radius {
                continue;
            }
            if let Ok(d) = Dir3::new(to) {
                let blocked = spatial
                    .cast_ray(
                        b.point,
                        d,
                        to.length(),
                        true,
                        &SpatialQueryFilter::from_mask(Layer::Level),
                    )
                    .is_some_and(|h| h.distance < to.length() - 0.05);
                if blocked {
                    continue;
                }
            }
            let dir_u = {
                let n = to.normalize_or_zero();
                Vec3::new(n.x, n.z, n.y)
            };
            let scale = if dist < p.body.collision_radius {
                1.0
            } else {
                1.0 - (dist - p.body.collision_radius) / b.radius
            };
            let mut momentum = k * dir_u;
            momentum.z += up;
            momentum *= scale;
            reached += 1;
            let push = take_damage(
                &mut p,
                e,
                scale * b.damage,
                t.translation,
                momentum,
                now,
                &mut fired,
                &mut commands,
            );
            built.pushes.extend(push);
        }
        if reached > 0 {
            log.line(format!(
                "{PART}: t={now:.2} {} ({}) reached {reached} physics props",
                b.grenade, b.damage_type
            ));
        }
    }
    send_events(fired, "a physics prop", &mut raise);
}

fn apply_pushes(mut built: ResMut<Built>, mut forces: Query<(Forces, &RigidBody)>) {
    let mut later = Vec::new();
    for (e, impulse, at) in std::mem::take(&mut built.pushes) {
        match forces.get_mut(e) {
            Ok((mut f, rb)) if rb.is_dynamic() => f.apply_linear_impulse_at_point(impulse, at),
            // Just switched on: next frame.
            Ok(_) => later.push((e, impulse, at)),
            Err(_) => {}
        }
    }
    built.pushes = later;
}

/// A prop that moves takes its triangles out of the level's collision:
/// people walk through moving props (see docs/DESIGN.md).
fn leave_parts(
    parts: Option<Res<LevelParts>>,
    mut props: Query<(&mut Prop, &RigidBody), Without<Sleeping>>,
) {
    let Some(parts) = parts else { return };
    for (mut p, rb) in &mut props {
        if p.part_off || !rb.is_dynamic() {
            continue;
        }
        if let Some(x) = p.body.export {
            parts.set_solid_export(x, false);
        }
        p.part_off = true;
    }
}

fn follow(
    bodies: Query<&Transform, (With<Prop>, Without<Follow>)>,
    mut drawn: Query<(&Follow, &mut Transform)>,
) {
    for (f, mut t) in &mut drawn {
        let Ok(b) = bodies.get(f.body) else { continue };
        let m = b.to_matrix() * f.start_inv;
        let n = Transform::from_matrix(m);
        if *t != n {
            *t = n;
        }
    }
}

/// `KImpact`: the engine calls it when two bodies (or a body and the
/// level) touch faster than the prop's `KImpactThreshold` (Unreal units per
/// second, the speed of the touching points against each other,
/// `Engine.dll` 0x1058054d); the prop plays one of its `ImpactSounds` at
/// most every `ImpactInterval`.
fn impacts(
    mut commands: Commands,
    collisions: Collisions,
    mut props: Query<(
        &mut Prop,
        &Transform,
        &LinearVelocity,
        &AngularVelocity,
        &ComputedCenterOfMass,
    )>,
    mut effects: MessageWriter<SpawnEffect>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs();
    let mut hits: Vec<(Entity, Vec3, f32)> = Vec::new();
    let mut touching = Vec::new();
    for pair in collisions.iter() {
        if !pair.is_touching() {
            continue;
        }
        let (Some(a), Some(b)) = (pair.body1, pair.body2) else {
            continue;
        };
        touching.push((a, b));
        let vel = |e: Entity, at: Vec3| -> Vec3 {
            props
                .get(e)
                .map(|(_, t, v, w, c)| {
                    let com = t.translation + t.rotation * c.0;
                    v.0 + w.0.cross(at - com)
                })
                .unwrap_or(Vec3::ZERO)
        };
        for m in &pair.manifolds {
            for pt in &m.points {
                let at = pt.point;
                let speed = (vel(a, at) - vel(b, at)).length() * UU_PER_UNIT;
                hits.push((a, at, speed));
                hits.push((b, at, speed));
            }
        }
    }
    // A moving prop touching a resting one wakes it (Karma enables a
    // disabled body that an enabled one touches).
    for (a, b) in touching {
        for (x, y) in [(a, b), (b, a)] {
            let moving = props.get(y).is_ok_and(|(p, ..)| p.karma && !p.asleep);
            if !moving {
                continue;
            }
            if let Ok((mut p, ..)) = props.get_mut(x) {
                if p.asleep {
                    wake(&mut p, x, &mut commands);
                }
            }
        }
    }
    for (e, at, speed) in hits {
        let Ok((mut p, ..)) = props.get_mut(e) else {
            continue;
        };
        if speed <= p.body.params.impact_threshold {
            continue;
        }
        if now <= p.last_impact + p.body.impact_interval {
            continue;
        }
        p.last_impact = now;
        if p.body.impact_sounds.is_empty() {
            continue;
        }
        // `Rand(numSounds)`.
        let i = ((now * 7919.0) as usize) % p.body.impact_sounds.len();
        effects.write(SpawnEffect {
            effect: String::new(),
            at,
            floor: None,
            sound: Some(p.body.impact_sounds[i].clone()),
            by: format!("{} landing", p.body.name),
            ..default()
        });
    }
}

/// A `KActor`'s `LifeSpan` ends: it goes. A `KarmaProp`'s `LifeTime`
/// timer runs its `Timer` (its broken state), not done yet.
fn life_spans(
    mut commands: Commands,
    mut props: Query<(Entity, &mut Prop)>,
    drawn: Query<(Entity, &Follow)>,
    time: Res<Time>,
    log: Res<GameLog>,
) {
    let now = time.elapsed_secs();
    for (e, mut p) in &mut props {
        let Some(end) = p.life_end else { continue };
        if now < end {
            continue;
        }
        p.life_end = None;
        if p.body.is_karma_prop {
            log.line(format!(
                "{PART}: t={now:.2} {}: its LifeTime is up, which breaks it (the prop's broken state, not done yet)",
                p.body.name
            ));
            continue;
        }
        log.line(format!(
            "{PART}: t={now:.2} {} removed (its LifeSpan is up or its Health is gone)",
            p.body.name
        ));
        for (d, f) in &drawn {
            if f.body == e {
                commands.entity(d).despawn();
            }
        }
        commands.entity(e).despawn();
    }
}

/// Parts a script switches off stop being solid for physics props too.
fn level_parts(
    mut commands: Commands,
    parts: Option<Res<LevelParts>>,
    mut built: ResMut<Built>,
    time: Res<Time>,
    mut last: Local<f32>,
) {
    let Some(parts) = parts else { return };
    let now = time.elapsed_secs();
    if now - *last < 0.25 {
        return;
    }
    *last = now;
    let built = &mut *built;
    for (export, _, _, solid, _) in parts.list() {
        let Some(&e) = built.parts.get(&export) else {
            continue;
        };
        if solid && built.parts_off.remove(&export) {
            commands.entity(e).remove::<ColliderDisabled>();
        } else if !solid && built.parts_off.insert(export) {
            commands.entity(e).insert(ColliderDisabled);
        }
    }
}

/// Log where each prop comes to rest (Avian puts it to sleep), and how
/// far it fell, once.
fn came_to_rest(
    mut props: Query<(&mut Prop, &Transform), Added<Sleeping>>,
    time: Res<Time>,
    log: Res<GameLog>,
) {
    let now = time.elapsed_secs();
    for (mut p, t) in &mut props {
        if !p.karma
            || p.rested
            || (t.translation - p.start.translation).length() * UU_PER_UNIT < 1.0
        {
            continue;
        }
        p.rested = true;
        let at = t.translation * UU_PER_UNIT;
        log.line(format!(
            "{PART}: t={now:.2} {} came to rest at ({:.0}, {:.0}, {:.0}), {:.0} uu below where it started",
            p.body.name,
            at.x,
            at.z,
            at.y,
            (p.start.translation.y - t.translation.y) * UU_PER_UNIT
        ));
    }
}

/// Every 10 seconds while props move: how many, how fast, and how far
/// they have fallen, for the log.
fn summary(
    props: Query<(&Prop, &Transform, &LinearVelocity), Without<Sleeping>>,
    time: Res<Time>,
    mut next: Local<f32>,
    log: Res<GameLog>,
) {
    let now = time.elapsed_secs();
    if now < *next {
        return;
    }
    *next = now + 10.0;
    let moving: Vec<_> = props
        .iter()
        .filter(|(p, ..)| p.karma && !p.asleep)
        .collect();
    if moving.is_empty() {
        return;
    }
    let fastest = moving
        .iter()
        .map(|(_, _, v)| v.0.length() * UU_PER_UNIT)
        .fold(0.0, f32::max);
    let (name, drop) = moving
        .iter()
        .map(|(p, t, _)| {
            (
                p.body.name.as_str(),
                (p.start.translation.y - t.translation.y) * UU_PER_UNIT,
            )
        })
        .fold(("", f32::MIN), |a, b| if b.1 > a.1 { b } else { a });
    log.line(format!(
        "{PART}: t={now:.2} {} props moving, fastest {fastest:.0} uu/s; furthest fallen {name} {drop:.0} uu",
        moving.len()
    ));
}

#[cfg(test)]
mod tests;
