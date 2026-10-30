//! What combat looks like: the weapon in the player's view, and glowing
//! bolts. Only runs when the game has a renderer; tests without one skip it.
//!
//! Weapons with first-person arms (`[arms]` in their file) are drawn by
//! `arms_view`. **Our choice, not the game's:** for a weapon without them
//! (or whose arms can't be read) we show its plain 3D model (the static
//! mesh the game uses when the weapon lies on the ground) at a fixed spot
//! in view. The game draws bolts as particle effects (`DC_17_Bolt_Proj`);
//! we draw each as a thin glowing box in the weapon's colour. No muzzle
//! flash yet.

use crate::library::WeaponLibrary;
use crate::Arsenal;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use rc_game_core::{GameLog, GamePaths, Player, PlayerCamera};
use rc_level_viewer::draw::to_image;
use rc_level_viewer::materials::Materials;
use rc_package::static_mesh::read_static_mesh;
use std::collections::HashMap;

/// A bolt's look per weapon id.
#[derive(Resource)]
pub(crate) struct BoltLooks {
    mesh: Handle<Mesh>,
    materials: HashMap<String, Handle<StandardMaterial>>,
    fallback: Handle<StandardMaterial>,
}

impl BoltLooks {
    pub(crate) fn attach(&self, e: &mut EntityCommands, weapon: &str) {
        let m = self.materials.get(weapon).unwrap_or(&self.fallback);
        e.insert((Mesh3d(self.mesh.clone()), MeshMaterial3d(m.clone())));
    }
}

fn glow(materials: &mut Assets<StandardMaterial>, c: [f32; 3]) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: Color::linear_rgb(c[0], c[1], c[2]),
        emissive: LinearRgba::rgb(c[0] * 4.0, c[1] * 4.0, c[2] * 4.0),
        unlit: true,
        ..default()
    })
}

/// A thrown grenade's look per grenade id. **Our choice:** a small
/// glowing ball in the grenade's colour, until the thrown models
/// (`ThermalDetonator` and friends in weapons.usx) are drawn.
#[derive(Resource)]
pub(crate) struct GrenadeLooks {
    mesh: Handle<Mesh>,
    materials: HashMap<String, Handle<StandardMaterial>>,
}

impl GrenadeLooks {
    pub(crate) fn attach(&self, e: &mut EntityCommands, grenade: &str) {
        if let Some(m) = self.materials.get(grenade) {
            e.insert((Mesh3d(self.mesh.clone()), MeshMaterial3d(m.clone())));
        }
    }
}

pub(crate) fn make_grenade_looks(
    mut commands: Commands,
    lib: Res<WeaponLibrary>,
    looks: Option<Res<GrenadeLooks>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    if looks.is_some() {
        return;
    }
    let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
        return;
    };
    // 8 uu across.
    let mesh = meshes.add(Sphere::new(0.08));
    let map = lib
        .grenades
        .iter()
        .map(|(id, p)| (id.clone(), glow(&mut materials, p.blast_colour)))
        .collect();
    commands.insert_resource(GrenadeLooks {
        mesh,
        materials: map,
    });
}

/// Make the bolt looks once (only if the game can draw meshes).
pub(crate) fn make_bolt_looks(
    mut commands: Commands,
    lib: Res<WeaponLibrary>,
    looks: Option<Res<BoltLooks>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    if looks.is_some() {
        return;
    }
    {
        let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
            return;
        };
        // 1 uu wide, 60 uu long, along the bolt's flight (-Z in Bevy is
        // "forward" for `looking_to`).
        let mesh = meshes.add(Cuboid::new(0.04, 0.04, 1.2));
        let fallback = glow(&mut materials, [1.0, 0.3, 0.1]);
        let map = lib
            .weapons
            .iter()
            .map(|(id, p)| (id.clone(), glow(&mut materials, p.bolt_colour)))
            .collect();
        commands.insert_resource(BoltLooks {
            mesh,
            materials: map,
            fallback,
        });
    }
}

/// The model in hand, as a child of the player's camera.
#[derive(Component)]
pub(crate) struct HeldModel(String);

/// Meshes and materials made from game files, per weapon id.
#[derive(Resource, Default)]
pub(crate) struct ModelCache(HashMap<String, Vec<(Handle<Mesh>, Handle<StandardMaterial>)>>);

#[allow(clippy::too_many_arguments)]
pub(crate) fn show_weapon_model(
    mut commands: Commands,
    lib: Res<WeaponLibrary>,
    paths: Option<Res<GamePaths>>,
    players: Query<&Arsenal, With<Player>>,
    cameras: Query<(Entity, Option<&Children>), With<PlayerCamera>>,
    held: Query<&HeldModel>,
    arms: Option<Res<crate::arms_view::ArmsModels>>,
    mut cache: Local<Option<ModelCache>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    images: Option<ResMut<Assets<Image>>>,
    log: Option<Res<GameLog>>,
) {
    let (Some(mut meshes), Some(mut materials), Some(mut images), Some(paths)) =
        (meshes, materials, images, paths)
    else {
        return;
    };
    let Ok(arsenal) = players.single() else {
        return;
    };
    let Ok((camera, children)) = cameras.single() else {
        return;
    };
    // Weapons with first-person arms show those instead (`arms_view`),
    // unless the arms couldn't be read.
    let want = arsenal.current().map(|w| w.0.clone()).filter(|id| {
        let has_arms = lib.weapons.get(id).is_some_and(|p| p.arms.is_some());
        !(has_arms && arms.as_ref().is_none_or(|a| a.covers(id)))
    });
    let mut showing = None;
    for c in children.into_iter().flatten() {
        if let Ok(h) = held.get(*c) {
            if Some(&h.0) == want.as_ref() {
                showing = Some(*c);
            } else {
                commands.entity(*c).despawn();
            }
        }
    }
    let Some(id) = want else {
        return;
    };
    if showing.is_some() {
        return;
    }
    let Some(p) = lib.weapons.get(&id) else {
        return;
    };
    let Some(mesh_name) = &p.model else {
        return;
    };
    let cache = cache.get_or_insert_with(ModelCache::default);
    if !cache.0.contains_key(&id) {
        let made = load_model(
            &paths,
            &p.model_package,
            mesh_name,
            &mut meshes,
            &mut materials,
            &mut images,
        );
        let line = match &made {
            Ok((parts, untextured)) if untextured.is_empty() => {
                format!("weapons: model {mesh_name} for {id}: {} parts, all textured", parts.len())
            }
            Ok((parts, untextured)) => format!(
                "weapons: model {mesh_name} for {id}: {} parts, {} drawn grey (no texture: {untextured:?})",
                parts.len(),
                untextured.len()
            ),
            Err(e) => format!("weapons: model {mesh_name} for {id} not shown: {e}"),
        };
        match &log {
            Some(l) => l.line(line),
            None => println!("{line}"),
        }
        cache
            .0
            .insert(id.clone(), made.map(|m| m.0).unwrap_or_default());
    }
    let parts = &cache.0[&id];
    // Forward, right, up in uu to camera space (forward is -Z, right +X,
    // up +Y), in Bevy units.
    let o = p.model_offset;
    let at = Vec3::new(o[1], o[2], -o[0]) / rc_game_core::units::UU_PER_UNIT;
    commands.entity(camera).with_children(|c| {
        c.spawn((
            HeldModel(id.clone()),
            Transform::from_translation(at),
            Visibility::default(),
        ))
        .with_children(|m| {
            for (mesh, mat) in parts {
                m.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone())));
            }
        });
    });
}

/// Read a static mesh from the game and make one Bevy mesh per section,
/// in camera space: Unreal (forward x, right y, up z) to (y, z, -x) / 50.
fn load_model(
    paths: &GamePaths,
    package: &str,
    name: &str,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> Result<(Vec<(Handle<Mesh>, Handle<StandardMaterial>)>, Vec<String>), String> {
    let mut mats = Materials::new(&paths.game_data());
    let loaded = mats
        .package(package)
        .ok_or_else(|| format!("package {package} not found"))?;
    let (pkg, data) = (&loaded.0, &loaded.1);
    let export = pkg
        .exports
        .iter()
        .find(|e| {
            pkg.export_class_name(e) == "StaticMesh"
                && pkg.name(e.object_name).eq_ignore_ascii_case(name)
        })
        .ok_or_else(|| format!("no static mesh {name} in {package}"))?;
    let mesh = read_static_mesh(pkg, data, export).map_err(|e| e.to_string())?;
    let s = 1.0 / rc_game_core::units::UU_PER_UNIT;
    let to_view = |v: [f32; 3]| [v[1] * s, v[2] * s, -v[0] * s];
    let to_view_dir = |v: [f32; 3]| [v[1], v[2], -v[0]];
    let uvs = mesh.uv_sets.first().cloned().unwrap_or_default();
    let mut out = vec![];
    let mut untextured = vec![];
    for (si, sec) in mesh.sections.iter().enumerate() {
        let first = sec.first_index as usize;
        let count = sec.num_faces as usize * 3;
        let Some(tris) = mesh.indices.get(first..first + count) else {
            continue;
        };
        let mut positions = vec![];
        let mut normals = vec![];
        let mut tex = vec![];
        let mut index = vec![];
        let mut remap = HashMap::new();
        for &vi in tris {
            let n = *remap.entry(vi).or_insert_with(|| {
                let v = mesh.vertices[vi as usize];
                positions.push(to_view(v.position));
                normals.push(to_view_dir(v.normal));
                tex.push(uvs.get(vi as usize).copied().unwrap_or([0.0, 0.0]));
                positions.len() as u32 - 1
            });
            index.push(n);
        }
        let m = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, tex)
        .with_inserted_indices(Indices::U32(index));
        let material = match mats.resolve(pkg, data, mesh.materials.get(si).copied().unwrap_or(0)) {
            Ok(pic) => StandardMaterial {
                base_color: Color::WHITE,
                base_color_texture: Some(images.add(to_image(&pic))),
                unlit: true,
                ..default()
            },
            Err(e) => {
                untextured.push(e);
                StandardMaterial {
                    base_color: Color::srgb(0.6, 0.6, 0.6),
                    unlit: true,
                    ..default()
                }
            }
        };
        out.push((meshes.add(m), materials.add(material)));
    }
    Ok((out, untextured))
}
