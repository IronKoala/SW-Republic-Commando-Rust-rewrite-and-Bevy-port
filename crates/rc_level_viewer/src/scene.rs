//! Builds everything the viewer draws from one map, without Bevy: the BSP
//! polygons and the placed static meshes, grouped by material into batches
//! of triangles in Bevy's coordinates, with the light the game baked into
//! them (lightmap pages for the BSP, a colour per corner for static meshes).

use crate::materials::{object_path, Loaded, Materials, Picture};
use rc_level::actors::{self, rotation_axes, Placement};
use rc_level::bsp::{self, poly_flags, Bsp};
use rc_level::lightmap::{self, LightMaps};
use rc_package::light::{self, Light};
use rc_package::static_mesh::{self, StaticMesh};
use rc_package::{ObjectRef, Package};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

/// Unreal units per Bevy unit (agreed with the other threads). This is the
/// one place the scale is set. Unreal's own rule of thumb is about 50 units
/// per metre, so one Bevy unit is roughly one metre; the exact real-world
/// scale (which matters for VR) is still to be measured against a character.
pub const UNREAL_UNITS_PER_BEVY_UNIT: f32 = 50.0;

/// Unreal (X forward, Y right, Z up, left-handed) to Bevy (Y up,
/// right-handed): swap Y and Z, then scale. Swapping two axes is a mirror,
/// which is exactly what turns left-handed into right-handed.
pub fn to_bevy(v: [f32; 3]) -> [f32; 3] {
    let s = 1.0 / UNREAL_UNITS_PER_BEVY_UNIT;
    [v[0] * s, v[2] * s, v[1] * s]
}

/// Directions (normals) convert the same way, without the scale.
pub fn dir_to_bevy(v: [f32; 3]) -> [f32; 3] {
    [v[0], v[2], v[1]]
}

/// Triangles that share one material.
#[derive(Default)]
pub struct Batch {
    pub picture: Option<Picture>,
    pub translucent: bool,
    pub two_sided: bool,
    /// True for static mesh triangles, false for BSP ones.
    pub from_mesh: bool,
    /// Part of the sky box (the sky zone), seen only through the sky camera.
    pub sky: bool,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// BSP only: which lightmap page lights these triangles, and where each
    /// corner falls on it (0..1). `None` for surfaces the game didn't light.
    pub lightmap_page: Option<usize>,
    pub lightmap_uvs: Vec<[f32; 2]>,
    /// Static meshes only: the baked light at each corner, red, green, blue,
    /// alpha as stored (0-255, the game's own brightness curve). Empty when
    /// the mesh has none.
    pub colours: Vec<[u8; 4]>,
    /// The mover (door, lift) these triangles belong to, by its export
    /// index, so they can be moved. `None` for everything that stays put.
    pub mover: Option<usize>,
}

#[derive(Default)]
pub struct Stats {
    pub bsp_polygons: usize,
    pub bsp_skipped: BTreeMap<&'static str, usize>,
    pub bsp_triangles: usize,
    pub bsp_checks: Vec<String>,
    pub bsp_winding: [usize; 3],
    pub actors: usize,
    pub actor_failures: usize,
    pub mesh_actors: usize,
    pub mesh_actors_drawn: usize,
    pub hidden_actors: usize,
    pub mesh_triangles: usize,
    pub unique_meshes: usize,
    pub mesh_failures: BTreeMap<String, usize>,
    pub materials_ok: usize,
    pub material_failures: BTreeMap<String, usize>,
    /// Baked lighting.
    pub lightmap_entries: usize,
    pub lightmap_pages: usize,
    pub lightmap_checks: Vec<String>,
    pub lightmap_error: Option<String>,
    pub bsp_polygons_lit: usize,
    pub bsp_polygons_unlit: usize,
    pub mesh_actors_with_colours: usize,
    pub mesh_actors_without_colours: usize,
    /// Colour lists whose length isn't the mesh's vertex count (not used).
    pub colour_count_mismatch: usize,
    pub lights: usize,
    pub dynamic_lights: usize,
    pub zone_ambients: usize,
    /// Floor test: PathNodes and PlayerStarts sit just above a walkable
    /// floor, so a ray straight down from each should hit level geometry
    /// soon, and never closer than about 80 units (they hover at a
    /// character's half height, so they are never inside a prop).
    pub floor_test: FloorTest,
    /// Triangles `rc_level::collision` would give a player to collide with.
    pub collision_triangles: usize,
    /// The same, counting only static mesh triangles. Turning props the
    /// wrong way shows up here as points with a prop much too close below.
    pub floor_test_meshes: FloorTest,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    /// Blocking volumes (invisible walls): read, stopping players, stopping
    /// the AI, failed to read, and with a scale we don't apply.
    pub blocking_volumes: usize,
    pub volumes_block_players: usize,
    pub volumes_block_actors: usize,
    pub volume_failures: Vec<String>,
    pub volumes_scaled: usize,
    /// PathNodes and PlayerStarts inside a volume that stops players. The AI
    /// walks between path points, so a correctly placed volume should hold
    /// none (or almost none) of them.
    pub path_points_in_volumes: usize,
    /// Volumes with a path point within 512 units of their bounding box.
    /// Invisible walls are put next to where people walk, so most should
    /// be (about 63% across the game; 9% if they were left unplaced).
    pub volumes_near_paths: usize,
    /// Sky: BSP polygons and props in the sky zone, and "fake backdrop"
    /// polygons left out so the sky shows through them.
    /// Props with no model in the map that take their class's default model
    /// (included in `mesh_actors`).
    pub class_mesh_actors: usize,
    pub sky_polygons: usize,
    /// Doors, lifts and other movers in the map, and how many have a model.
    pub movers: usize,
    pub movers_drawn: usize,
    pub sky_props: usize,
    pub backdrop_polygons: usize,
}

/// Where the sky camera stands: the map's `SkyZoneInfo`. Unreal 2 draws a
/// level's sky by looking out from this point, inside a small box (the sky
/// zone) somewhere far from the playing area, turned the same way as the
/// player's view but never moving with them. That makes the sky look
/// infinitely far away.
pub struct Sky {
    /// Bevy position.
    pub position: [f32; 3],
    /// The sky zone's number in the BSP.
    pub zone: u8,
    pub from: String,
}

pub struct Spawn {
    /// Bevy position of the eyes.
    pub position: [f32; 3],
    /// Unreal yaw (65536 = full turn).
    pub yaw: i32,
    pub from: String,
}

pub struct Scene {
    pub batches: Vec<Batch>,
    /// Lightmap pages, decoded to red, green, blue, alpha.
    pub lightmap_pages: Vec<rc_image::Rgba>,
    /// The map's lights and zone ambients (`rc_package::light`).
    pub lights: Vec<Light>,
    pub stats: Stats,
    pub spawn: Spawn,
    /// `None` when the map has no `SkyZoneInfo`.
    pub sky: Option<Sky>,
}

#[derive(Default)]
struct Builder {
    batches: Vec<Batch>,
    by_key: HashMap<BatchKey, usize>,
}

/// Material, translucent, two-sided, from a mesh, lightmap page, has
/// colours, in the sky, mover.
type BatchKey = (String, bool, bool, bool, Option<usize>, bool, bool, Option<usize>);

impl Builder {
    #[allow(clippy::too_many_arguments)]
    fn batch(
        &mut self,
        key: &str,
        picture: &Option<Picture>,
        translucent: bool,
        two_sided: bool,
        from_mesh: bool,
        lightmap_page: Option<usize>,
        coloured: bool,
        sky: bool,
        mover: Option<usize>,
    ) -> &mut Batch {
        let k = (
            key.to_string(),
            translucent,
            two_sided,
            from_mesh,
            lightmap_page,
            coloured,
            sky,
            mover,
        );
        let i = *self.by_key.entry(k).or_insert_with(|| {
            self.batches.push(Batch {
                picture: picture.clone(),
                translucent,
                two_sided,
                from_mesh,
                sky,
                lightmap_page,
                mover,
                ..Default::default()
            });
            self.batches.len() - 1
        });
        &mut self.batches[i]
    }
}

fn material_key(pkg: &Package, index: i32) -> String {
    if index == 0 {
        "(none)".into()
    } else {
        object_path(pkg, index).join(".")
    }
}

/// Look up a material, counting successes and failures by reason.
fn picture(
    mats: &mut Materials,
    stats: &mut Stats,
    seen: &mut HashMap<String, Option<Picture>>,
    pkg: &Package,
    data: &[u8],
    index: i32,
) -> (String, Option<Picture>) {
    let key = material_key(pkg, index);
    if let Some(p) = seen.get(&key) {
        return (key, p.clone());
    }
    let p = match mats.resolve(pkg, data, index) {
        Ok(p) => {
            stats.materials_ok += 1;
            Some(p)
        }
        Err(e) => {
            *stats.material_failures.entry(e).or_default() += 1;
            None
        }
    };
    seen.insert(key.clone(), p.clone());
    (key, p)
}

fn tex_size(p: &Option<Picture>) -> [f32; 2] {
    match p {
        Some(p) => [p.image.width as f32, p.image.height as f32],
        // Unknown textures: assume 256 so the pattern of UVs still makes sense.
        None => [256.0, 256.0],
    }
}

fn add_bsp(b: &mut Builder, bsp: &Bsp, lightmaps: &LightMaps, sky_zone: Option<u8>, ctx: &mut Ctx) {
    for poly in bsp.polygons() {
        ctx.stats.bsp_polygons += 1;
        let w = Bsp::winding(&poly);
        ctx.stats.bsp_winding[(w + 1) as usize] += 1;
        if poly.poly_flags & poly_flags::INVISIBLE != 0 {
            *ctx.stats.bsp_skipped.entry("invisible").or_default() += 1;
            continue;
        }
        if poly.poly_flags & poly_flags::PORTAL != 0 {
            *ctx.stats.bsp_skipped.entry("portal").or_default() += 1;
            continue;
        }
        // "Fake backdrop" surfaces are windows onto the sky: the game draws
        // the sky zone there instead of the surface.
        if poly.poly_flags & poly_flags::FAKE_BACKDROP != 0 {
            ctx.stats.backdrop_polygons += 1;
            *ctx.stats.bsp_skipped.entry("sky backdrop").or_default() += 1;
            continue;
        }
        // A polygon belongs to the zone on its front side.
        let sky = sky_zone.is_some_and(|z| bsp.nodes[poly.node].zone[1] == z);
        ctx.stats.sky_polygons += sky as usize;
        let (key, pic) = picture(
            ctx.mats,
            &mut ctx.stats,
            &mut ctx.seen,
            ctx.pkg,
            ctx.data,
            poly.material,
        );
        let size = tex_size(&pic);
        let translucent = poly.poly_flags & poly_flags::TRANSLUCENT != 0;
        let two_sided = poly.poly_flags & poly_flags::TWO_SIDED != 0;
        let node = &bsp.nodes[poly.node];
        let page = if poly.poly_flags & poly_flags::UNLIT != 0 {
            None
        } else {
            poly.points
                .first()
                .and_then(|p| lightmaps.page_uv(node, *p))
                .map(|(page, _)| page)
        };
        if page.is_some() {
            ctx.stats.bsp_polygons_lit += 1;
        } else {
            ctx.stats.bsp_polygons_unlit += 1;
        }
        let batch = b.batch(&key, &pic, translucent, two_sided, false, page, false, sky, None);
        let base = batch.positions.len() as u32;
        let n = dir_to_bevy(poly.normal);
        for (p, uv) in poly.points.iter().zip(&poly.uvs) {
            batch.positions.push(to_bevy(*p));
            batch.normals.push(n);
            batch.uvs.push([uv[0] / size[0], uv[1] / size[1]]);
            if page.is_some() {
                let lm = lightmaps.page_uv(node, *p).map(|(_, uv)| uv);
                batch.lightmap_uvs.push(lm.unwrap_or_default());
            }
        }
        // A fan of triangles from the first corner. The Y/Z swap mirrors the
        // polygon, so corners that went one way round now go the other; Bevy
        // draws the side where corners go anticlockwise. `winding` says which
        // way they went in the file.
        for k in 1..poly.points.len() as u32 - 1 {
            if w >= 0 {
                batch.indices.extend([base, base + k + 1, base + k]);
            } else {
                batch.indices.extend([base, base + k, base + k + 1]);
            }
            ctx.stats.bsp_triangles += 1;
        }
    }
}

struct Ctx<'a> {
    mats: &'a mut Materials,
    stats: Stats,
    seen: HashMap<String, Option<Picture>>,
    pkg: &'a Package,
    data: &'a [u8],
}

/// A static mesh and the package it came from (its materials are
/// references inside that package), or why it couldn't be read.
type MeshResult = Result<Rc<(StaticMesh, Loaded)>, String>;

/// Find and read a static mesh by its reference in the map.
fn load_mesh(
    mats: &mut Materials,
    cache: &mut HashMap<String, MeshResult>,
    map: &Loaded,
    index: i32,
) -> MeshResult {
    let pkg = &map.0;
    let path = object_path(pkg, index);
    let key = match ObjectRef::from_index(index) {
        // Objects inside the package itself: name the package too, as
        // class meshes come from several packages.
        ObjectRef::Export(_) => format!("({:p}){}", Rc::as_ptr(map), path.join(".")),
        _ => path.join("."),
    };
    if let Some(r) = cache.get(&key) {
        return r.clone();
    }
    let result = (|| {
        let (owner, k) = match ObjectRef::from_index(index) {
            ObjectRef::Export(k) => (map.clone(), k),
            ObjectRef::Import(_) => {
                let owner = mats
                    .package(&path[0])
                    .ok_or_else(|| format!("package {} not found", path[0]))?;
                let k = crate::materials::find_export(&owner.0, &path[1..])
                    .ok_or_else(|| format!("{key} not found"))?;
                (owner, k)
            }
            ObjectRef::None => return Err("no mesh".to_string()),
        };
        let mesh = static_mesh::read_static_mesh(&owner.0, &owner.1, &owner.0.exports[k])
            .map_err(|e| format!("mesh read error: {e}"))?;
        Ok(Rc::new((mesh, owner)))
    })();
    cache.insert(key, result.clone());
    result
}

#[allow(clippy::too_many_arguments)]
fn add_mesh_actor(
    b: &mut Builder,
    ctx: &mut Ctx,
    sky: bool,
    p: &Placement,
    mesh: &StaticMesh,
    owner: &(Package, Vec<u8>),
    colours: Option<&[[u8; 4]]>,
    mover: Option<usize>,
) {
    let axes = rotation_axes(p.rotation);
    let s3 = p.draw_scale_3d;
    // A mirrored actor (an odd number of negative scales) turns its
    // triangles inside out, so flip them back.
    let mirrored = (s3[0] * s3[1] * s3[2] * p.draw_scale) < 0.0;
    let uv0 = mesh.uv_sets.first();
    for (si, sec) in mesh.sections.iter().enumerate() {
        // The actor's Skins override the mesh's own materials.
        let (key, pic) = match p.skins.get(si).copied().filter(|&m| m != 0) {
            Some(m) => picture(
                ctx.mats,
                &mut ctx.stats,
                &mut ctx.seen,
                ctx.pkg,
                ctx.data,
                m,
            ),
            None => {
                let m = mesh.materials.get(si).copied().unwrap_or(0);
                picture(
                    ctx.mats,
                    &mut ctx.stats,
                    &mut ctx.seen,
                    &owner.0,
                    &owner.1,
                    m,
                )
            }
        };
        let batch = b.batch(&key, &pic, false, false, true, None, colours.is_some(), sky, mover);
        let first = sec.first_index as usize;
        let count = sec.num_faces as usize * 3;
        let Some(tris) = mesh.indices.get(first..first + count) else {
            continue;
        };
        // Copy just the vertices this section uses.
        let mut remap: HashMap<u16, u32> = HashMap::new();
        for chunk in tris.as_chunks::<3>().0 {
            let mut out = [0u32; 3];
            for (o, &vi) in out.iter_mut().zip(chunk) {
                *o = *remap.entry(vi).or_insert_with(|| {
                    let v = mesh.vertices[vi as usize];
                    let world = p.transform_point(v.position);
                    let n = v.normal;
                    let mut wn = [0.0f32; 3];
                    for (k, axis) in axes.iter().enumerate() {
                        for c in 0..3 {
                            // Normals scale by the inverse of the mesh's scale.
                            let sk = if s3[k].abs() < 1e-6 { 1e-6 } else { s3[k] };
                            wn[c] += n[k] * axis[c] / sk;
                        }
                    }
                    let len = bsp::dot(wn, wn).sqrt().max(1e-6);
                    batch.positions.push(to_bevy(world));
                    batch
                        .normals
                        .push(dir_to_bevy([wn[0] / len, wn[1] / len, wn[2] / len]));
                    batch.uvs.push(
                        uv0.and_then(|u| u.get(vi as usize))
                            .copied()
                            .unwrap_or([0.0, 0.0]),
                    );
                    if let Some(c) = colours {
                        batch.colours.push(c[vi as usize]);
                    }
                    batch.positions.len() as u32 - 1
                });
            }
            if mirrored {
                batch.indices.extend([out[0], out[2], out[1]]);
            } else {
                batch.indices.extend(out);
            }
            ctx.stats.mesh_triangles += 1;
        }
    }
}

pub fn build(map: Loaded, mats: &mut Materials) -> Result<Scene, String> {
    let (pkg, data) = (&map.0, &map.1);
    let level = actors::read_level(pkg, data).map_err(|e| format!("level: {e}"))?;
    let model = level.model.ok_or("the level has no BSP model")?;
    let bsp = bsp::read_model(pkg, data, &pkg.exports[model]).map_err(|e| format!("BSP: {e}"))?;

    let mut b = Builder::default();
    let mut ctx = Ctx {
        mats,
        stats: Stats::default(),
        seen: HashMap::new(),
        pkg,
        data,
    };
    ctx.stats.bsp_checks = bsp.check();
    let e = &pkg.exports[model];
    let start = e.serial_offset as usize;
    let lightmaps = data
        .get(start..start + e.serial_size as usize)
        .ok_or_else(|| "the BSP model is outside the file".to_string())
        .and_then(|object| lightmap::read_lightmaps(&bsp, object, start).map_err(|e| e.to_string()))
        .unwrap_or_else(|e| {
            // Without lightmaps the level still draws, just unlit.
            ctx.stats.lightmap_error = Some(e);
            LightMaps::default()
        });
    ctx.stats.lightmap_entries = lightmaps.entries.len();
    ctx.stats.lightmap_pages = lightmaps.pages.len();
    ctx.stats.lightmap_checks = lightmaps.check(&bsp);
    let mut lightmap_pages = Vec::new();
    for (i, page) in lightmaps.pages.iter().enumerate() {
        let decoded = crate::materials::pixel_format(page.format)
            .ok_or_else(|| format!("format {} not supported", page.format))
            .and_then(|f| {
                rc_image::decode(f, page.width, page.height, &page.pixels, None)
                    .map_err(|e| e.to_string())
            });
        match decoded {
            Ok(img) => lightmap_pages.push(img),
            Err(e) => {
                ctx.stats
                    .lightmap_checks
                    .push(format!("lightmap page {i} can't be decoded: {e}"));
                // A mid-grey page keeps the indices lined up.
                lightmap_pages.push(rc_image::Rgba {
                    width: 1,
                    height: 1,
                    pixels: vec![128, 128, 128, 255],
                });
            }
        }
    }
    // The sky: the first SkyZoneInfo, and the zone it stands in.
    let sky = level.actors.iter().flatten().find_map(|&a| {
        if pkg.export_class_name(&pkg.exports[a]) != "SkyZoneInfo" {
            return None;
        }
        let p = actors::read_placement(pkg, data, a).ok()?;
        Some(Sky {
            position: to_bevy(p.location),
            zone: bsp.point_zone(p.location)?,
            from: format!("{} at {:?}", p.name, p.location),
        })
    });
    let sky_zone = sky.as_ref().map(|s| s.zone);
    add_bsp(&mut b, &bsp, &lightmaps, sky_zone, &mut ctx);

    let mut lights = Vec::new();

    let mut spawn = None;
    let mut floor_points = Vec::new();
    let mut mesh_cache = HashMap::new();
    let mut class_looks = rc_level::class_look::ClassLooks::new();
    // Doors and lifts get batches of their own, so they can move.
    let movers = {
        let mats = &mut *ctx.mats;
        rc_level::movers::read_movers(&mut class_looks, pkg, data, &level.actors, &mut |n: &str| {
            mats.package(n)
        })
    };
    let mut mover_exports: std::collections::HashSet<usize> =
        movers.movers.iter().map(|m| m.place.export).collect();
    // So do props with a tag of their own, which level scripts may remove
    // (barricades, bunker walls). They are tagged `MoverPart` too.
    mover_exports.extend(rc_level::scripts::tagged_actors(pkg, data, &level).into_keys());
    ctx.stats.movers = movers.movers.len();
    let mut placed: Vec<(Placement, Rc<(StaticMesh, Loaded)>)> = Vec::new();
    for &a in level.actors.iter().flatten() {
        ctx.stats.actors += 1;
        let p = match actors::read_placement(pkg, data, a) {
            Ok(p) => p,
            Err(_) => {
                ctx.stats.actor_failures += 1;
                continue;
            }
        };
        if p.class == "PlayerStart" && spawn.is_none() {
            // Eye height: Unreal pawns stand with their centre at the
            // PlayerStart; eyes are a little above it.
            let eye = [p.location[0], p.location[1], p.location[2] + 64.0];
            spawn = Some(Spawn {
                position: to_bevy(eye),
                yaw: p.rotation[1],
                from: format!("{} at {:?}", p.name, p.location),
            });
        }
        if p.class == "PathNode" || p.class == "PlayerStart" {
            floor_points.push(p.location);
        }
        if light::is_light_class(&p.class) || p.class.to_ascii_lowercase().contains("zoneinfo") {
            if let Ok(props) = actors::read_actor_properties(pkg, data, a) {
                if let Some(l) = light::light_from_properties(pkg, &p.class, &props) {
                    ctx.stats.lights += 1;
                    ctx.stats.dynamic_lights += l.dynamic as usize;
                    lights.push(l);
                } else if light::zone_ambient(pkg, &props).is_some() {
                    ctx.stats.zone_ambients += 1;
                }
            }
        }
        let mut p = p;
        // Props and doors that take their model from their class.
        let mut owner = map.clone();
        if p.static_mesh == 0 {
            let mats = &mut *ctx.mats;
            match rc_level::class_look::apply_class_look(
                &mut class_looks,
                pkg,
                data,
                &mut p,
                &mut |n: &str| mats.package(n),
            ) {
                Some(o) => {
                    owner = o;
                    ctx.stats.class_mesh_actors += 1;
                }
                None => continue,
            }
        }
        ctx.stats.mesh_actors += 1;
        if p.hidden {
            ctx.stats.hidden_actors += 1;
            continue;
        }
        match load_mesh(ctx.mats, &mut mesh_cache, &owner, p.static_mesh) {
            Ok(m) => {
                ctx.stats.mesh_actors_drawn += 1;
                let colours = if p.static_mesh_instance != 0 {
                    actors::read_vertex_colours(pkg, data, p.static_mesh_instance).ok()
                } else {
                    None
                };
                let colours = match colours {
                    Some(c) if c.len() == m.0.vertices.len() => {
                        ctx.stats.mesh_actors_with_colours += 1;
                        Some(c)
                    }
                    Some(_) => {
                        ctx.stats.colour_count_mismatch += 1;
                        None
                    }
                    None => {
                        ctx.stats.mesh_actors_without_colours += 1;
                        None
                    }
                };
                let in_sky = sky_zone.is_some() && bsp.point_zone(p.location) == sky_zone;
                ctx.stats.sky_props += in_sky as usize;
                let mover = mover_exports.contains(&p.export).then_some(p.export);
                ctx.stats.movers_drawn += mover.is_some_and(|m| movers.movers.iter().any(|x| x.place.export == m)) as usize;
                add_mesh_actor(&mut b, &mut ctx, in_sky, &p, &m.0, &m.1, colours.as_deref(), mover);
                placed.push((p, m));
            }
            Err(e) => *ctx.stats.mesh_failures.entry(e).or_default() += 1,
        }
    }
    let pairs: Vec<_> = placed.iter().map(|(p, m)| (p, &m.0)).collect();
    let mut collision = rc_level::collision::collision_triangles(&bsp, &pairs);
    let volumes = rc_level::volumes::blocking_volumes(pkg, data, &level);
    ctx.stats.volumes_block_players =
        rc_level::collision::add_blocking_volumes(&mut collision, &volumes.volumes, true);
    ctx.stats.volumes_block_actors = volumes.volumes.iter().filter(|v| v.block_actors).count();
    ctx.stats.blocking_volumes = volumes.volumes.len();
    ctx.stats.volume_failures = volumes.failures;
    ctx.stats.volumes_scaled = volumes.scaled;
    ctx.stats.path_points_in_volumes = floor_points
        .iter()
        .filter(|p| {
            volumes
                .volumes
                .iter()
                .any(|v| v.block_players && v.contains(**p))
        })
        .count();
    ctx.stats.volumes_near_paths = volumes
        .volumes
        .iter()
        .filter(|v| {
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            for q in v.polygons.iter().flatten() {
                for c in 0..3 {
                    lo[c] = lo[c].min(q[c]);
                    hi[c] = hi[c].max(q[c]);
                }
            }
            floor_points
                .iter()
                .any(|p| (0..3).all(|c| p[c] > lo[c] - 512.0 && p[c] < hi[c] + 512.0))
        })
        .count();
    ctx.stats.collision_triangles = collision.triangles.len();
    ctx.stats.unique_meshes = mesh_cache.values().filter(|r| r.is_ok()).count();

    ctx.stats.floor_test = floor_test(b.batches.iter(), &floor_points);
    ctx.stats.floor_test_meshes =
        floor_test(b.batches.iter().filter(|b| b.from_mesh), &floor_points);

    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for batch in b.batches.iter().filter(|b| !b.sky) {
        for p in &batch.positions {
            for c in 0..3 {
                lo[c] = lo[c].min(p[c]);
                hi[c] = hi[c].max(p[c]);
            }
        }
    }
    ctx.stats.bounds_min = lo;
    ctx.stats.bounds_max = hi;
    let spawn = spawn.unwrap_or(Spawn {
        position: [
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ],
        yaw: 0,
        from: "no PlayerStart; centre of the level".into(),
    });
    Ok(Scene {
        batches: b.batches,
        lightmap_pages,
        lights,
        stats: ctx.stats,
        spawn,
        sky,
    })
}

#[derive(Default, Clone, Copy)]
pub struct FloorTest {
    pub tested: usize,
    /// Floor 0-150 units below.
    pub near: usize,
    /// Floor under 80 units below: the point would be inside something.
    pub too_close: usize,
    pub median: f32,
}

/// Distance straight down (in Unreal units) from `from` to the nearest
/// triangle facing up, for each point. Works in Bevy coordinates, where down
/// is -Y.
fn floor_test<'a>(batches: impl Iterator<Item = &'a Batch>, points: &[[f32; 3]]) -> FloorTest {
    let starts: Vec<[f32; 3]> = points.iter().map(|p| to_bevy(*p)).collect();
    let mut best = vec![f32::MAX; starts.len()];
    for b in batches {
        for t in b.indices.as_chunks::<3>().0 {
            let [a, bb, c] = [0, 1, 2].map(|k| b.positions[t[k] as usize]);
            let (minx, maxx) = (a[0].min(bb[0]).min(c[0]), a[0].max(bb[0]).max(c[0]));
            let (minz, maxz) = (a[2].min(bb[2]).min(c[2]), a[2].max(bb[2]).max(c[2]));
            for (i, s) in starts.iter().enumerate() {
                if s[0] < minx || s[0] > maxx || s[2] < minz || s[2] > maxz {
                    continue;
                }
                // Barycentric test in the X-Z plane, then height on the triangle.
                let d = (bb[2] - c[2]) * (a[0] - c[0]) + (c[0] - bb[0]) * (a[2] - c[2]);
                if d.abs() < 1e-9 {
                    continue;
                }
                let l1 = ((bb[2] - c[2]) * (s[0] - c[0]) + (c[0] - bb[0]) * (s[2] - c[2])) / d;
                let l2 = ((c[2] - a[2]) * (s[0] - c[0]) + (a[0] - c[0]) * (s[2] - c[2])) / d;
                let l3 = 1.0 - l1 - l2;
                if l1 < 0.0 || l2 < 0.0 || l3 < 0.0 {
                    continue;
                }
                let y = l1 * a[1] + l2 * bb[1] + l3 * c[1];
                let below = s[1] - y;
                if below >= -0.02 && below < best[i] {
                    best[i] = below;
                }
            }
        }
    }
    let mut dists: Vec<f32> = best
        .iter()
        .filter(|d| **d < f32::MAX)
        .map(|d| d * UNREAL_UNITS_PER_BEVY_UNIT)
        .collect();
    dists.sort_by(|a, b| a.total_cmp(b));
    FloorTest {
        tested: points.len(),
        near: dists.iter().filter(|d| **d <= 150.0).count(),
        too_close: dists.iter().filter(|d| **d < 80.0).count(),
        median: dists.get(dists.len() / 2).copied().unwrap_or(f32::NAN),
    }
}
