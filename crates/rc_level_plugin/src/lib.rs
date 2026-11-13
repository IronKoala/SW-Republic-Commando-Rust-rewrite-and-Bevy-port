//! The level part of the game. `LevelPlugin` loads the map the menus chose
//! (`CurrentLevel`) when the game enters `GameState::Loading`, and hands the
//! rest of the game what it needs:
//!
//! - `LevelCollision`: the map's BSP, every placed prop and door (also the
//!   ones whose model comes from their class) and the invisible walls
//!   (blocking volumes), loaded by `rc_level::map_collision`;
//! - `PlayerStart`: the feet and facing of the map's first `PlayerStart`;
//! - the map's look, drawn with the level viewer's code
//!   (`rc_level_viewer::scene` and `draw`), and its sky
//!   (`rc_level_viewer::sky`): a second camera inside the sky box that turns
//!   with the player's camera;
//! - `LevelInfo`: counts for the log and the self-tests;
//! - `LevelParts`: props and blocking volumes with a tag of their own
//!   (barricades, bunker walls), solid until a level script switches them
//!   off. They are part of `LevelCollision`;
//! - `LevelMoverData`: the map's doors, lifts and their triggers, as read
//!   from the map, for the movers part (`rc_movers_plugin`) to run. Their
//!   triangles are not in `LevelCollision`; that part adds them.
//!
//! Everything it spawns is removed when the game leaves `InGame`.
//!
//! It never reads the keyboard or moves the player, and it only touches the
//! player's camera to let the sky show through it (`SkyViewer` and no
//! clearing). A VR mod's camera gets the sky the same way: give it
//! `PlayerCamera`.

pub mod anim_props;

use bevy::camera::ClearColorConfig;
use bevy::prelude::*;
use rc_game_core::{
    ArrivalPortal, CurrentLevel, GameLog, GamePaths, GameState, LevelCollision, LoadingGate, PlayerCamera,
    PlayerStart,
};
use rc_level_viewer::bump::{BumpLitMaterial, BumpMapPlugin};
use rc_level_viewer::draw::{self, BACKGROUND};
use rc_level_viewer::lighting::LightingSettings;
use rc_level_viewer::sky::{self, SkyViewer};
use rc_level_viewer::{fog, materials, prop_light, scene};
use rc_movement::{Hit, MoveParams, TriangleWorld, Vec3, World as MoveWorld};
use std::collections::HashMap;
use std::sync::RwLock;
use rc_package::Package;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

/// For other parts and self-tests: the sky camera's marker, and the marker
/// the player's camera gets.
pub use rc_level_viewer::sky::{SkyCamera, SkyViewer as SkyFollows};

/// The name this part uses in the loading gate and the log.
pub const PART: &str = "level";

/// The lighting file a mod (or the player) can put in a data folder to
/// change brightness without touching code. Optional.
pub const LIGHTING_FILE: &str = "levels/lighting.cfg";

pub mod place_light;

pub struct LevelPlugin;

impl Plugin for LevelPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            sky::SkyPlugin,
            BumpMapPlugin,
            prop_light::PropLightPlugin,
            rc_hologram::HologramPlugin,
        ))
            // Zone fog on the player's camera, except while the visor's low
            // light mode has its own (`rc_game_core::OwnFog`).
            .add_systems(
                PostUpdate,
                fog::view::follow_zone_fog::<Without<rc_game_core::OwnFog>>
                    .after(bevy::transform::TransformSystems::Propagate),
            )
            .add_systems(OnEnter(GameState::Loading), begin)
            .add_systems(Update, load.run_if(in_state(GameState::Loading)))
            .add_message::<rc_game_core::PropAnim>()
            .add_systems(
                Update,
                (let_sky_through, fog_on_player_camera).run_if(in_state(GameState::InGame)),
            )
            .add_systems(
                Update,
                (
                    place_light::take_materials,
                    place_light::follow_light,
                    place_light::log_player_light,
                )
                    .chain()
                    .run_if(in_state(GameState::InGame)),
            )
            .add_systems(
                Update,
                anim_props::animate
                    .in_set(rc_game_core::GameSet::Simulate)
                    .run_if(rc_game_core::playing),
            )
            .add_systems(OnExit(GameState::InGame), forget);
    }
}

/// What was loaded, for the log and self-tests. Present from the end of
/// loading until the game leaves `InGame`.
#[derive(Resource, Debug, Clone, Default)]
pub struct LevelInfo {
    pub map: String,
    pub collision_triangles: usize,
    pub props: usize,
    /// Props and doors whose model comes from their class.
    pub class_props: usize,
    /// Props drawn with an animated model (bacta dispensers and the like),
    /// and the ones whose model couldn't be read.
    pub anim_props: usize,
    pub anim_prop_failures: usize,
    pub unreadable_props: usize,
    pub blocking_volumes: usize,
    /// Props that collide through their mesh's simple collision shape
    /// (collision off on every section, like the explosive barrels).
    pub simple_shape_props: usize,
    /// Props people walk through but shots stop at.
    pub shot_only_props: usize,
    pub drawn_triangles: usize,
    /// The sky box's position (Bevy units), if the map has one.
    pub sky: Option<[f32; 3]>,
    /// Whether the map has a PlayerStart (if not, the player starts above
    /// the middle of the map).
    pub has_player_start: bool,
    pub load_ms: f64,
    pub draw_ms: f64,
}

/// The map's doors, lifts and triggers as the map places them, and each
/// mover's collision triangles by its export index. Present from the end of
/// loading until the game leaves `InGame`.
#[derive(Resource, Debug, Clone, Default)]
pub struct LevelMoverData {
    pub movers: rc_level::movers::MapMovers,
    pub triangles: std::collections::HashMap<usize, Vec<[[f32; 3]; 3]>>,
}

/// Props weapons can break (`rc_level::breakables`): barrels, consoles,
/// rock cover, clamps. Each is also a part in `LevelParts`, by its export
/// index. Present from the end of loading until the game leaves `InGame`.
#[derive(Resource, Debug, Clone, Default)]
pub struct LevelBreakables(pub Vec<rc_level::breakables::Breakable>);

/// Sounds that loop where the map puts them (`rc_level::ambient`): the
/// battle in the distance, fires, steam. Present from the end of loading
/// until the game leaves `InGame`.
#[derive(Resource, Debug, Clone, Default)]
pub struct LevelAmbientSounds(pub Vec<rc_level::ambient::AmbientSound>);

/// The level's walls and rooms as the game's sound code sees them
/// (`rc_level::hearing`): whether a wall stands between a sound and the
/// listener, and how many rooms apart they are. Present from the end of
/// loading until the game leaves `InGame`, when the map's zones read.
#[derive(Resource, Debug, Clone)]
pub struct LevelHearing(pub Arc<rc_level::hearing::Hearing>);

/// The faces of blocking volumes that a line with no thickness passes
/// through in the original (`bBlockZeroExtentTraces` off). They are part
/// of `LevelCollision` too; a sight check that must see through them, as
/// the original's do (the objective tracker), treats a hit on one of these
/// as no hit. Present while in the game.
#[derive(Resource, Clone)]
pub struct LineThroughVolumes(pub Arc<TriangleWorld>);

/// One part of the level a script can switch off (see `LevelParts`).
pub struct Part {
    pub export: usize,
    pub class: String,
    pub tag: String,
    pub solid: bool,
    /// Stops shots but not people (`rc_level::class_look::Blocks`), like
    /// the shield wall pieces of geo_04c.
    pub shots_only: bool,
    world: Option<TriangleWorld>,
}

struct PartsShared {
    fixed: Arc<dyn MoveWorld + Send + Sync>,
    /// Props that stop shots but not people, other than parts.
    shots_only: Option<TriangleWorld>,
    parts: RwLock<Vec<Part>>,
}

/// Props and blocking volumes with a `Tag` of their own (barricades, bunker
/// walls). They start solid; level scripts switch them off by tag
/// (`ACTION_SetCollision`, a prop blown up). `LevelCollision` is the level
/// plus the solid parts. Present while in the game. Cheap to clone.
#[derive(Resource, Clone)]
pub struct LevelParts(Arc<PartsShared>);

impl LevelParts {
    pub fn new(
        fixed: Arc<dyn MoveWorld + Send + Sync>,
        tagged: &HashMap<usize, (String, String)>,
        triangles: HashMap<usize, Vec<[[f32; 3]; 3]>>,
    ) -> Self {
        Self::with_shots_only(fixed, tagged, triangles, Vec::new(), HashMap::new())
    }

    /// As `new`, plus what stops shots but not people: `shot_triangles`
    /// (props that aren't parts) and `shot_parts` (parts, by export index;
    /// their triangles are not in `triangles`). In the `world` these only
    /// stop sweeps with no size (shots, looks), never a walking body. See
    /// `rc_level::map_collision::MapCollision::shot_triangles`.
    pub fn with_shots_only(
        fixed: Arc<dyn MoveWorld + Send + Sync>,
        tagged: &HashMap<usize, (String, String)>,
        triangles: HashMap<usize, Vec<[[f32; 3]; 3]>>,
        shot_triangles: Vec<[[f32; 3]; 3]>,
        shot_parts: HashMap<usize, Vec<[[f32; 3]; 3]>>,
    ) -> Self {
        let v = |p: [f32; 3]| Vec3::new(p[0], p[1], p[2]);
        let world_of = |ts: &Vec<[[f32; 3]; 3]>| {
            TriangleWorld::new(ts.iter().map(|t| [v(t[0]), v(t[1]), v(t[2])]).collect())
        };
        let mut parts: Vec<Part> = tagged
            .iter()
            .map(|(&export, (class, tag))| {
                let shot = shot_parts.get(&export);
                Part {
                    export,
                    class: class.clone(),
                    tag: tag.clone(),
                    solid: true,
                    shots_only: shot.is_some(),
                    world: shot.or(triangles.get(&export)).map(world_of),
                }
            })
            .collect();
        parts.sort_by_key(|p| p.export);
        LevelParts(Arc::new(PartsShared {
            fixed,
            shots_only: (!shot_triangles.is_empty()).then(|| world_of(&shot_triangles)),
            parts: RwLock::new(parts),
        }))
    }

    /// Make every part with this tag solid or not. Returns how many parts
    /// have the tag.
    pub fn set_solid(&self, tag: &str, solid: bool) -> usize {
        let mut parts = self.0.parts.write().unwrap_or_else(|e| e.into_inner());
        let mut n = 0;
        for p in parts.iter_mut().filter(|p| p.tag.eq_ignore_ascii_case(tag)) {
            p.solid = solid;
            n += 1;
        }
        n
    }

    /// Make one part (by export index) solid or not.
    pub fn set_solid_export(&self, export: usize, solid: bool) {
        let mut parts = self.0.parts.write().unwrap_or_else(|e| e.into_inner());
        for p in parts.iter_mut().filter(|p| p.export == export) {
            p.solid = solid;
        }
    }

    /// (export, class, tag, solid, has collision) for every part.
    pub fn list(&self) -> Vec<(usize, String, String, bool, bool)> {
        let parts = self.0.parts.read().unwrap_or_else(|e| e.into_inner());
        parts
            .iter()
            .map(|p| (p.export, p.class.clone(), p.tag.clone(), p.solid, p.world.is_some()))
            .collect()
    }

    /// The box around each part with this tag that has collision
    /// (lowest corner, highest corner), Unreal units.
    pub fn bounds(&self, tag: &str) -> Vec<(Vec3, Vec3)> {
        let parts = self.0.parts.read().unwrap_or_else(|e| e.into_inner());
        parts
            .iter()
            .filter(|p| p.tag.eq_ignore_ascii_case(tag))
            .filter_map(|p| p.world.as_ref().map(|w| w.bounds()))
            .collect()
    }

    /// The box around one part (by export index), if it has collision
    /// (lowest corner, highest corner), Unreal units.
    pub fn bounds_of(&self, export: usize) -> Option<(Vec3, Vec3)> {
        let parts = self.0.parts.read().unwrap_or_else(|e| e.into_inner());
        parts
            .iter()
            .find(|p| p.export == export)
            .and_then(|p| p.world.as_ref().map(|w| w.bounds()))
    }

    /// The parts that stop shots but not people (export indices).
    pub fn shots_only(&self) -> Vec<usize> {
        let parts = self.0.parts.read().unwrap_or_else(|e| e.into_inner());
        parts
            .iter()
            .filter(|p| p.shots_only && p.world.is_some())
            .map(|p| p.export)
            .collect()
    }

    /// The level plus the parts that are solid now.
    pub fn world(&self) -> Arc<dyn MoveWorld + Send + Sync> {
        Arc::new(WithParts(self.0.clone()))
    }
}

struct WithParts(Arc<PartsShared>);

impl MoveWorld for WithParts {
    fn sweep(&self, extent: Vec3, start: Vec3, end: Vec3) -> Option<Hit> {
        let mut best = self.0.fixed.sweep(extent, start, end);
        let mut take = |h: Option<Hit>| {
            if let Some(h) = h {
                if best.is_none_or(|b| h.time < b.time || (h.start_solid && !b.start_solid)) {
                    best = Some(h);
                }
            }
        };
        // A sweep with no size is a shot or a look; anything with a size
        // is a body moving.
        let line = extent == Vec3::ZERO;
        if line {
            if let Some(w) = &self.0.shots_only {
                take(w.sweep(extent, start, end));
            }
        }
        let parts = self.0.parts.read().unwrap_or_else(|e| e.into_inner());
        for p in parts.iter().filter(|p| p.solid && (line || !p.shots_only)) {
            let Some(w) = &p.world else { continue };
            take(w.sweep(extent, start, end));
        }
        best
    }
}

/// Frames to wait in the loading screen before the slow load, so the
/// loading screen is drawn first.
#[derive(Resource)]
struct Countdown(u32);

fn begin(
    mut commands: Commands,
    mut gate: ResMut<LoadingGate>,
    paths: Option<Res<GamePaths>>,
) {
    gate.wait(PART);
    // Where rc_hologram reads the game's own hologram glow and noise
    // pictures from.
    if let Some(paths) = paths {
        commands.insert_resource(rc_hologram::HologramGameData(paths.game_data()));
    }
    commands.insert_resource(Countdown(3));
}

fn forget(
    mut commands: Commands,
    views: Query<Entity, With<fog::view::ZoneFogView>>,
) {
    // The fog belongs to the level: the menus and the next level start
    // without it.
    commands.remove_resource::<fog::view::LevelFog>();
    for e in &views {
        commands
            .entity(e)
            .remove::<(fog::view::ZoneFogView, bevy::pbr::DistanceFog)>();
    }
    commands.remove_resource::<LevelInfo>();
    commands.remove_resource::<LevelMoverData>();
    commands.remove_resource::<LevelParts>();
    commands.remove_resource::<LineThroughVolumes>();
    commands.remove_resource::<rc_game_core::PlaceLight>();
    commands.remove_resource::<LevelBreakables>();
    commands.remove_resource::<LevelAmbientSounds>();
    commands.remove_resource::<LevelHearing>();
    commands.remove_resource::<rc_game_core::pickups::LevelAmmoBoxes>();
    commands.remove_resource::<rc_game_core::pickups::LevelWeaponPickups>();
}

/// The loading itself. It blocks for as long as the map takes (about 2 s for
/// geo_01a in a debug build); the loading screen stays up meanwhile.
fn load(world: &mut World) {
    {
        let Some(mut c) = world.get_resource_mut::<Countdown>() else {
            return;
        };
        if c.0 > 0 {
            c.0 -= 1;
            return;
        }
    }
    world.remove_resource::<Countdown>();
    let log = world.resource::<GameLog>().clone();
    let paths = world.resource::<GamePaths>().clone();
    let Some(map) = world.resource::<CurrentLevel>().map.clone() else {
        world.resource_mut::<LoadingGate>().fail(PART, "no map chosen");
        return;
    };
    let path = paths.map_path(&map);
    let game_data = paths.game_data();

    // Collision and the player start, in Unreal units. The pawn's centre
    // stands at the PlayerStart, so the feet are one collision height lower.
    let height = MoveParams::player_commando().collision_height;
    let t = Instant::now();
    let mut lvl = match rc_level::map_collision::load(&path, &game_data, height) {
        Ok(l) => l,
        Err(e) => {
            log.line(format!("{PART}: ERROR loading {}: {e}", path.display()));
            world.resource_mut::<LoadingGate>().fail(PART, e);
            return;
        }
    };
    let load_ms = t.elapsed().as_secs_f64() * 1000.0;
    log.line(format!(
        "{PART}: {} loaded: {} collision triangles, {} solid props ({} with their class's model, {} unreadable, {} you can walk through left out), {} blocking volumes ({} unreadable), {:.0} ms",
        path.display(),
        lvl.triangles.len(),
        lvl.mesh_actors,
        lvl.class_mesh_actors,
        lvl.mesh_failures,
        lvl.not_solid_props,
        lvl.blocking_volumes,
        lvl.volume_failures,
        load_ms
    ));
    log.line(format!(
        "{PART}: {} terrains (outdoor ground) solid with {} triangles{}",
        lvl.terrains,
        lvl.terrain_triangles,
        if lvl.terrain_failures.is_empty() {
            String::new()
        } else {
            format!("; NOT read: {}", lvl.terrain_failures.join("; "))
        }
    ));
    let mut info = LevelInfo {
        map: map.clone(),
        collision_triangles: lvl.triangles.len(),
        props: lvl.mesh_actors,
        class_props: lvl.class_mesh_actors,
        unreadable_props: lvl.mesh_failures,
        blocking_volumes: lvl.blocking_volumes,
        simple_shape_props: lvl.simple_shape_props,
        shot_only_props: lvl.shot_only_props,
        has_player_start: lvl.start.is_some(),
        load_ms,
        ..default()
    };
    let v = |p: [f32; 3]| Vec3::new(p[0], p[1], p[2]);
    let tw = TriangleWorld::new(lvl.triangles.iter().map(|t| [v(t[0]), v(t[1]), v(t[2])]).collect());
    // Coming from another map's exit: arrive at the teleporter it names,
    // if this map has one (`#playerstart` and the like mean the start).
    let portal = world.remove_resource::<ArrivalPortal>();
    let arrival = portal.as_ref().and_then(|p| {
        let found = lvl.arrival(&p.0);
        log.line(format!(
            "{PART}: arriving through {:?}: {}",
            p.0,
            if found.is_some() {
                "a teleporter with that tag"
            } else {
                "no teleporter has that tag; the player start"
            }
        ));
        found
    });
    let (feet, yaw) = match arrival.or(lvl.start) {
        Some((f, yaw)) => (v(f), yaw),
        None => {
            let (lo, hi) = tw.bounds();
            log.line(format!("{PART}: no PlayerStart, starting above the middle"));
            (Vec3::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0, hi.z), 0.0)
        }
    };
    log.line(format!(
        "{PART}: player start feet ({:.1}, {:.1}, {:.1}) facing {:.0} deg",
        feet.x,
        feet.y,
        feet.z,
        yaw.to_degrees()
    ));
    world.insert_resource(PlayerStart { feet, yaw });
    let anim = anim_props::spawn(world, &paths, &lvl.anim_props);
    log.line(format!(
        "{PART}: {} props with an animated model drawn ({}), {} animation sounds{}",
        anim.drawn,
        anim.models.join(", "),
        anim.sounds,
        if anim.failures.is_empty() {
            String::new()
        } else {
            format!("; NOT drawn: {}", anim.failures.join("; "))
        }
    ));
    info.anim_props = anim.drawn;
    info.anim_prop_failures = lvl.anim_props.len() - anim.drawn;
    world.insert_resource(LevelMoverData {
        movers: lvl.movers,
        triangles: lvl.mover_triangles,
    });
    let parts = LevelParts::with_shots_only(
        Arc::new(tw),
        &lvl.parts,
        std::mem::take(&mut lvl.part_triangles),
        std::mem::take(&mut lvl.shot_triangles),
        std::mem::take(&mut lvl.shot_part_triangles),
    );
    log.line(format!(
        "{PART}: {} props collide through their simple collision shape; {} props stop shots but not people",
        lvl.simple_shape_props, lvl.shot_only_props
    ));
    log.line(format!(
        "{PART}: {} parts a script may switch off ({} solid ones)",
        lvl.parts.len(),
        parts.list().iter().filter(|p| p.4).count()
    ));
    log.line(format!(
        "{PART}: {} props weapons can break ({} explode, {} tell the level scripts when they break)",
        lvl.breakables.len(),
        lvl.breakables.iter().filter(|b| b.explosion.is_some()).count(),
        lvl.breakables.iter().filter(|b| b.broken_event().is_some()).count()
    ));
    world.insert_resource(LevelBreakables(std::mem::take(&mut lvl.breakables)));
    log.line(format!(
        "{PART}: {} looping sounds placed in the map",
        lvl.ambient_sounds.len()
    ));
    world.insert_resource(LevelAmbientSounds(std::mem::take(&mut lvl.ambient_sounds)));
    match lvl.hearing.take() {
        Some(h) => {
            log.line(format!(
                "{PART}: {} zones for sound (walls and rooms between a sound and the listener)",
                h.zones()
            ));
            world.insert_resource(LevelHearing(Arc::new(h)));
        }
        None => log.line(format!(
            "{PART}: the map's zones couldn't be read; walls won't muffle sounds"
        )),
    }
    world.insert_resource(LevelCollision(parts.world()));
    world.insert_resource(LineThroughVolumes(Arc::new(TriangleWorld::new(
        lvl.line_through_triangles
            .iter()
            .map(|t| [v(t[0]), v(t[1]), v(t[2])])
            .collect(),
    ))));
    world.insert_resource(parts);
    // Ammo and grenade boxes, for the combat part to hand out.
    let boxes: Vec<_> = lvl
        .ammo_boxes
        .iter()
        .map(|b| rc_game_core::pickups::PlacedAmmoBox {
            export: b.export,
            name: b.name.clone(),
            class: b.class.clone(),
            at: bevy::math::Vec3::new(b.location[0], b.location[2], b.location[1])
                / rc_game_core::units::UU_PER_UNIT,
            ammo: b.ammo.clone(),
            amount: b.amount,
            respawn: b.respawn,
            radius: b.radius,
            height: b.height,
            message: b.message.clone(),
            sound: b.sound.clone(),
        })
        .collect();
    log.line(format!(
        "{PART}: {} ammo and grenade boxes ({} moved down onto the ground, the furthest {:.0} uu); {} pickups of all kinds left out of collision",
        boxes.len(),
        lvl.ammo_boxes.iter().filter(|b| b.dropped > 0.0).count(),
        lvl.ammo_boxes.iter().map(|b| b.dropped).fold(0.0f32, f32::max),
        lvl.pickups_not_solid
    ));
    world.insert_resource(rc_game_core::pickups::LevelAmmoBoxes(boxes));
    // Weapons lying in the map, the same way.
    let weapons: Vec<_> = lvl
        .weapons
        .iter()
        .map(|w| rc_game_core::pickups::PlacedWeapon {
            export: w.export,
            name: w.name.clone(),
            class: w.class.clone(),
            at: bevy::math::Vec3::new(w.location[0], w.location[2], w.location[1])
                / rc_game_core::units::UU_PER_UNIT,
            weapon: w.weapon.clone(),
            ammo: w.ammo,
            message: w.message.clone(),
            sound: w.sound.clone(),
            radius: w.radius,
            height: w.height,
        })
        .collect();
    log.line(format!(
        "{PART}: {} weapons lying in the map ({} moved down onto the ground, the furthest {:.0} uu)",
        weapons.len(),
        lvl.weapons.iter().filter(|w| w.dropped > 0.0).count(),
        lvl.weapons.iter().map(|w| w.dropped).fold(0.0f32, f32::max),
    ));
    world.insert_resource(rc_game_core::pickups::LevelWeaponPickups(weapons));

    // The look. If it can't be built the level is still solid, just
    // invisible, so the game goes on and says so in the log.
    let t = Instant::now();
    match build_look(&path, &game_data, &paths) {
        Ok((sc, settings, notes)) => {
            for n in notes {
                log.line(format!("{PART}: lighting: {n}"));
            }
            // The light probe: how much light reaches any point, from the
            // light baked into the level's surfaces, for the characters,
            // the gun and the helmet (see `place_light`).
            let probe_t = Instant::now();
            let (probe, ps) = rc_level_viewer::probe::LightProbe::build(
                &sc,
                settings.vertex_colour_gain,
                settings.lightmap_gain,
                place_light::PROBE_RADIUS,
            );
            log.line(format!(
                "{PART}: light probe: {} samples ({} from props, {} terrain, {} walls with lightmaps) in {:.0} ms",
                probe.samples(),
                ps.prop_samples,
                ps.terrain_samples,
                ps.bsp_samples,
                probe_t.elapsed().as_secs_f64() * 1000.0
            ));
            world.insert_resource(rc_game_core::PlaceLight(Arc::new(place_light::Probe(probe))));
            let report = world.resource_scope(|world, mut meshes: Mut<Assets<Mesh>>| {
                world.resource_scope(|world, mut images: Mut<Assets<Image>>| {
                    world.resource_scope(|world, mut mats: Mut<Assets<StandardMaterial>>| {
                        world.resource_scope(|world, mut bumps: Mut<Assets<BumpLitMaterial>>| {
                          world.resource_scope(|world, mut props: Mut<Assets<prop_light::PropMaterial>>| {
                            let mut commands = world.commands();
                            draw::spawn_level(
                                &mut commands,
                                &mut meshes,
                                &mut images,
                                &mut mats,
                                Some(&mut bumps),
                                &mut props,
                                &sc,
                                &settings,
                            )
                          })
                        })
                    })
                })
            });
            let mut spawned = report.entities.clone();
            if let Some(s) = &sc.sky {
                let bg = Color::srgb_u8(BACKGROUND[0], BACKGROUND[1], BACKGROUND[2]);
                let mut commands = world.commands();
                let cam = sky::spawn_sky_camera(&mut commands, s, bg);
                // Under the player's camera (order 0), the first-person
                // arms' (1) and the menus' and display's camera (2).
                commands.entity(cam).insert(Camera {
                    order: -1,
                    clear_color: ClearColorConfig::Custom(bg),
                    ..default()
                });
                spawned.push(cam);
                info.sky = Some(s.position);
                log.line(format!(
                    "{PART}: sky camera inside the sky box at {} ({} sky batches)",
                    s.from, report.sky_batches
                ));
            } else {
                log.line(format!("{PART}: this map has no SkyZoneInfo, so no sky"));
            }
            world.flush();
            for e in spawned {
                if let Ok(mut e) = world.get_entity_mut(e) {
                    e.insert(DespawnOnExit(GameState::InGame));
                }
            }
            info.drawn_triangles = report.triangles;
            info.draw_ms = t.elapsed().as_secs_f64() * 1000.0;
            log.line(format!(
                "{PART}: drawn in {:.0} ms: {} triangles, {} textured batches, {} lightmapped, {} props/terrain lit by real-time lights too (strength {}), {} lights",
                info.draw_ms,
                report.triangles,
                report.textured,
                report.lightmapped,
                report.coloured_live,
                settings.live_light_on_props,
                report.lights.point + report.lights.spot + report.lights.directional
            ));
            log.line(match &sc.stats.falloff_error {
                None => format!(
                    "{PART}: light shafts with the game's falloff shader (HsFalloff): {} prop parts, {} draw batches",
                    sc.stats.prop_parts_falloff, report.falloff
                ),
                Some(e) => format!(
                    "{PART}: light shafts drawn as plain added light: HsFalloff's settings couldn't be read ({e})"
                ),
            });
        }
        Err(e) => log.line(format!("{PART}: map look not built ({e}); it will be invisible")),
    }
    world.insert_resource(info);
    world.resource_mut::<LoadingGate>().done(PART);
}

fn build_look(
    map_path: &Path,
    game_data: &Path,
    paths: &GamePaths,
) -> Result<(scene::Scene, LightingSettings, Vec<String>), String> {
    let (settings, notes) = match paths.data_file(LIGHTING_FILE) {
        Some(p) => LightingSettings::load(&p),
        None => (LightingSettings::default(), Vec::new()),
    };
    let data = std::fs::read(map_path).map_err(|e| e.to_string())?;
    let pkg = Package::parse(&data).map_err(|e| e.to_string())?;
    let mut mats = materials::Materials::new(game_data);
    let sc = scene::build(Rc::new((pkg, data)), &mut mats)?;
    Ok((sc, settings, notes))
}

/// The player's camera shows the distance fog of the zone it is in, as
/// the map sets it (`rc_level_viewer::fog`).
fn fog_on_player_camera(
    mut commands: Commands,
    fogs: Option<Res<fog::view::LevelFog>>,
    views: Query<(Entity, &GlobalTransform), (With<PlayerCamera>, Without<fog::view::ZoneFogView>)>,
    log: Res<GameLog>,
) {
    let Some(fogs) = fogs else {
        return;
    };
    for (e, at) in &views {
        commands.entity(e).insert(fog::view::ZoneFogView::default());
        let b = at.translation() * 50.0;
        let (zone, f) = fogs.0.at([b.x, b.z, b.y]);
        log.line(format!(
            "{PART}: distance fog on the player's camera: {} of {} zones turn it on; the camera is in zone {:?}: {}",
            fogs.0.zones_with_fog,
            fogs.0.zones_read,
            zone,
            match f {
                Some(f) => format!(
                    "colour {:?}, from {} to {} Unreal units",
                    f.colour, f.start, f.end
                ),
                None => "no fog".to_string(),
            }
        ));
    }
}

/// When the map has a sky, the player's camera must not clear the picture
/// the sky camera drew, and the sky camera must turn with it.
fn let_sky_through(
    mut commands: Commands,
    info: Option<Res<LevelInfo>>,
    mut views: Query<(Entity, &mut Camera), (With<PlayerCamera>, Without<SkyViewer>)>,
    log: Res<GameLog>,
) {
    if info.is_none_or(|i| i.sky.is_none()) {
        return;
    }
    for (e, mut cam) in &mut views {
        cam.clear_color = ClearColorConfig::None;
        commands.entity(e).insert(SkyViewer);
        log.line(format!(
            "{PART}: the player's camera now shows the sky behind the level"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wall (x = 10) that stops shots only, as a part and as a plain
    /// prop: a line stops at it, a body walks through, and a broken part
    /// stops nothing.
    #[test]
    fn shots_only_walls_stop_lines_not_bodies() {
        let wall = |x: f32| {
            vec![
                [[x, -100.0, -100.0], [x, 100.0, -100.0], [x, 0.0, 100.0]],
                [[x, -100.0, 100.0], [x, 100.0, 100.0], [x, 0.0, -100.0]],
            ]
        };
        let tagged = HashMap::from([(
            7usize,
            ("K_GEO04_WallComponent".to_string(), "W".to_string()),
        )]);
        let body = Vec3::new(20.0, 20.0, 40.0);
        let (a, b) = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(40.0, 0.0, 0.0));
        for (loose, part) in [
            (wall(10.0), HashMap::new()),
            (Vec::new(), HashMap::from([(7usize, wall(10.0))])),
        ] {
            let parts = LevelParts::with_shots_only(
                Arc::new(TriangleWorld::new(vec![])),
                &tagged,
                HashMap::new(),
                loose,
                part,
            );
            let w = parts.world();
            let hit = w.sweep(Vec3::ZERO, a, b).expect("a shot stops at the wall");
            assert!((hit.time - 0.25).abs() < 0.01, "hit at {}", hit.time);
            assert!(w.sweep(body, a, b).is_none(), "a body walks through");
        }
        let parts = LevelParts::with_shots_only(
            Arc::new(TriangleWorld::new(vec![])),
            &tagged,
            HashMap::new(),
            Vec::new(),
            HashMap::from([(7usize, wall(10.0))]),
        );
        parts.set_solid_export(7, false);
        assert!(
            parts.world().sweep(Vec3::ZERO, a, b).is_none(),
            "a broken piece stops nothing"
        );
    }
}
