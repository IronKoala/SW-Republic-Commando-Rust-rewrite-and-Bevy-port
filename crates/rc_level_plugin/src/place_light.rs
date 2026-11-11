//! Lights squad mates, droids, the gun and the helmet by where they are
//! (see `rc_game_core::place_light`): the level's light probe goes into
//! `PlaceLight`, and every entity tagged `LitByPlace` gets its materials
//! matched to the light around it a few times a second.

use bevy::prelude::*;
use rc_game_core::{GameLog, LightTarget, LitByPlace, PlaceLight};
use rc_level_viewer::probe::LightProbe;
use std::collections::HashMap;

/// How far (Bevy units, about metres) a point looks for lit surfaces.
pub const PROBE_RADIUS: f32 = 6.0;

/// Seconds between light checks for each tagged entity.
const CHECK_EVERY: f32 = 0.1;
/// How quickly the drawn light follows the place (share per second), so
/// walking from shade into sun eases rather than jumps.
const EASE_PER_SECOND: f32 = 4.0;

pub struct Probe(pub LightProbe);

impl rc_game_core::place_light::LightAt for Probe {
    fn light_at(&self, p: Vec3) -> Option<Vec3> {
        self.0.light_at([p.x, p.y, p.z]).map(Vec3::from_array)
    }
}

/// Each tagged entity's own materials and what they were made with, and
/// the light it is drawn at now.
#[derive(Component)]
pub struct PlaceLit {
    /// Material, its colour as made (base colour or glow, linear).
    materials: Vec<(Handle<StandardMaterial>, LinearRgba)>,
    now: Option<Vec3>,
    since: f32,
}

impl PlaceLit {
    /// The place's light this entity is drawn by now (eased), once it has
    /// been measured.
    pub fn light(&self) -> Option<Vec3> {
        self.now
    }
}

/// Gives each newly tagged entity its own copies of its materials (they
/// are often shared between all droids of a kind).
#[allow(clippy::type_complexity)]
pub fn take_materials(
    mut commands: Commands,
    tagged: Query<(Entity, &LitByPlace), Without<PlaceLit>>,
    children: Query<&Children>,
    mut meshes: Query<&mut MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (e, lit) in &tagged {
        let mut own = Vec::new();
        let mut copies: HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>> =
            HashMap::new();
        let mut list = vec![e];
        list.extend(children.iter_descendants(e));
        for d in list {
            let Ok(mut m) = meshes.get_mut(d) else {
                continue;
            };
            let id = m.0.id();
            let handle = match copies.get(&id) {
                Some(h) => h.clone(),
                None => {
                    let Some(made) = materials.get(&m.0).cloned() else {
                        continue;
                    };
                    let colour = match lit.target {
                        LightTarget::Emissive => made.emissive,
                        LightTarget::BaseColor => made.base_color.to_linear(),
                    };
                    let h = materials.add(made);
                    own.push((h.clone(), colour));
                    copies.insert(id, h.clone());
                    h
                }
            };
            m.0 = handle;
        }
        // Models posed or loaded a little later have no meshes yet: try
        // again next frame.
        if own.is_empty() {
            continue;
        }
        commands.entity(e).insert(PlaceLit {
            materials: own,
            now: None,
            since: CHECK_EVERY,
        });
    }
}

/// Matches each tagged entity's materials to the light where it is.
pub fn follow_light(
    time: Res<Time>,
    light: Option<Res<PlaceLight>>,
    mut tagged: Query<(&LitByPlace, &mut PlaceLit, &GlobalTransform)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(light) = light else {
        return;
    };
    let dt = time.delta_secs();
    for (lit, mut state, at) in &mut tagged {
        state.since += dt;
        if state.since < CHECK_EVERY {
            continue;
        }
        let step = state.since;
        state.since = 0.0;
        let Some(here) = light.at(at.translation() + Vec3::Y * lit.height) else {
            continue;
        };
        let now = match state.now {
            Some(was) => was + (here - was) * (EASE_PER_SECOND * step).min(1.0),
            None => here,
        };
        state.now = Some(now);
        let f = lit.factor(lit.brightness(now));
        for (h, made) in &state.materials {
            let Some(mut m) = materials.get_mut(h) else {
                continue;
            };
            let c = LinearRgba::new(
                made.red * f.x,
                made.green * f.y,
                made.blue * f.z,
                made.alpha,
            );
            match lit.target {
                LightTarget::Emissive => m.emissive = c,
                LightTarget::BaseColor => m.base_color = c.into(),
            }
        }
    }
}

/// Writes the light the player stands in to the log every few seconds, so
/// a playtest can be compared with the original's screenshots by number.
pub fn log_player_light(
    time: Res<Time>,
    light: Option<Res<PlaceLight>>,
    eyes: Query<&GlobalTransform, With<rc_game_core::PlayerCamera>>,
    log: Res<GameLog>,
    mut next: Local<f32>,
) {
    let (Some(light), Ok(eye)) = (light, eyes.single()) else {
        return;
    };
    if time.elapsed_secs() < *next {
        return;
    }
    *next = time.elapsed_secs() + 5.0;
    let p = eye.translation();
    match light.at(p) {
        Some(l) => log.line(format!(
            "light: at the player's eyes ({:.1}, {:.1}, {:.1}) {:.2} {:.2} {:.2}",
            p.x, p.y, p.z, l.x, l.y, l.z
        )),
        None => log.line(format!(
            "light: no lit surface near the player's eyes ({:.1}, {:.1}, {:.1})",
            p.x, p.y, p.z
        )),
    }
}
