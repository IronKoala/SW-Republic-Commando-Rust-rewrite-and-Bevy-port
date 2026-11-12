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
use std::sync::Arc;

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
    /// Static meshes whose picture multiplies by a second one
    /// (`Picture::multiply`): where each corner falls on that second
    /// picture (its own texture coordinate set). Empty otherwise.
    pub layer_uvs: Vec<[f32; 2]>,
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
    /// Terrain only: which painted layer this is. Layer 0 is solid; later
    /// layers are drawn over it, showing as much as each corner's colour
    /// alpha says (from the layer's alpha map).
    pub terrain_layer: Option<usize>,
    /// Props drawn with their bump map: the material has one and the props
    /// store a light direction per corner. `light_dirs` and
    /// `light_colours` then have one entry per corner (see `BumpCorner`),
    /// directions in Bevy's axes.
    pub bump_lit: bool,
    pub light_dirs: Vec<[f32; 3]>,
    pub light_colours: Vec<[f32; 3]>,
}

/// The light a bump-mapped prop's corner gets from its main light, read
/// from what the map stores (`actors::read_baked_light`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BumpCorner {
    /// Unit direction towards the light, in the mesh's own axes (zero when
    /// the map stores no direction).
    pub dir: [f32; 3],
    /// The light's colour and strength on the game's 0..1 brightness scale,
    /// as it would light a surface facing it squarely.
    pub light: [f32; 3],
}

/// How bright the stored corner colours are, against the strength stored
/// with each light direction: the colour's brightest channel is 0.75 x
/// strength x how squarely the corner faces the light (middle half 0.69 to
/// 0.77 over 62,507 corners of yyy_06a; see docs/DESIGN.md "Lights in
/// maps").
pub const BUMP_LIGHT_PER_STRENGTH: f32 = 0.75;

/// Turn a prop's stored corner colours (before ambient is added) and light
/// directions into `BumpCorner`s. `None` if the prop has no directions or
/// the lists don't match.
///
/// The colour of the light comes from the corner's own colour where the
/// corner is lit enough to tell (brightest channel 16 or more), from the
/// prop's average lit colour elsewhere, and is white if no corner is lit.
/// Its strength comes from the direction's alpha.
pub fn bump_corners(colours: &[[u8; 4]], dirs: &[[u8; 4]]) -> Option<Vec<BumpCorner>> {
    if dirs.is_empty() || dirs.len() != colours.len() {
        return None;
    }
    const LIT: u8 = 16;
    let hue = |c: &[u8; 4]| {
        let m = c[0].max(c[1]).max(c[2]) as f32;
        [c[0] as f32 / m, c[1] as f32 / m, c[2] as f32 / m]
    };
    let mut sum = [0.0f32; 3];
    let mut n = 0;
    for c in colours.iter().filter(|c| c[0].max(c[1]).max(c[2]) >= LIT) {
        let h = hue(c);
        for k in 0..3 {
            sum[k] += h[k];
        }
        n += 1;
    }
    let prop_hue = if n > 0 {
        let m = sum[0].max(sum[1]).max(sum[2]).max(1e-6);
        sum.map(|v| v / m)
    } else {
        [1.0; 3]
    };
    Some(
        colours
            .iter()
            .zip(dirs)
            .map(|(c, d)| {
                let v = [0, 1, 2].map(|k| d[k] as f32 / 255.0 * 2.0 - 1.0);
                let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                let dir = if len > 0.1 {
                    v.map(|x| x / len)
                } else {
                    [0.0; 3]
                };
                let h = if c[0].max(c[1]).max(c[2]) >= LIT {
                    hue(c)
                } else {
                    prop_hue
                };
                let strength = BUMP_LIGHT_PER_STRENGTH * d[3] as f32 / 255.0;
                BumpCorner {
                    dir,
                    light: h.map(|x| x * strength),
                }
            })
            .collect(),
    )
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
    /// Materials drawn with see-through holes, and materials whose picture
    /// has low alpha that the material doesn't use for holes (usually a
    /// shine mask), so they are drawn solid.
    pub materials_see_through: usize,
    pub materials_alpha_ignored: usize,
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
    /// Of those, props we light ourselves from the map's lights
    /// (`unbaked_prop_light`) instead of drawing at full brightness.
    pub props_lit_from_lights: usize,
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
    /// Ammo and grenade boxes drawn (each on its own, so it can vanish
    /// when taken), and the furthest one was moved down onto the ground.
    pub ammo_boxes: usize,
    /// Weapons lying in the map, drawn on their own like the boxes.
    pub weapon_pickups: usize,
    pub ammo_box_drop_max: f32,
    pub sky_props: usize,
    /// Sky props whose stored light is all black, drawn at full brightness.
    pub sky_props_unlit: usize,
    /// Prop parts (one mesh section on one prop) whose material multiplies
    /// two pictures (`Combiner` with `CO_Multiply`, see `materials::Layer`).
    pub prop_parts_multiplied: usize,
    /// Prop parts drawn with the game's falloff shader (`HsFalloff`: light
    /// shafts and god rays, see `crate::falloff`).
    pub prop_parts_falloff: usize,
    /// Why `HsFalloff`'s class defaults couldn't be read, if they couldn't
    /// (then falloff materials are plain added light).
    pub falloff_error: Option<String>,
    /// Zones with an ambient light found (by where their ZoneInfo stands),
    /// and props that had it added to their stored light.
    pub zones_with_ambient: usize,
    pub props_with_ambient: usize,
    /// Physics props (`KarmaProp`) moved down onto the ground, and the
    /// furthest one moved (Unreal units).
    pub props_settled: usize,
    pub settle_drop_max: f32,
    /// Props whose own spot is inside solid rock or wall (BSP zone 0, which
    /// has no ambient light), placed in the zone their corners face instead
    /// (see `prop_zone`).
    pub props_zone_from_corners: usize,
    /// Props with bump-mapped light (a stored light direction per corner).
    pub props_bump_lit: usize,
    /// Of those, the ones drawn with a bump map on at least one part, and
    /// their parts whose material names no bump map (drawn flat).
    pub props_bump_drawn: usize,
    pub bump_parts_without_map: usize,
    /// Bump maps the drawn props use, and of those the ones stored in the
    /// game's two-byte bump formats (the rest are colour normal maps).
    pub bump_maps: usize,
    pub bump_maps_two_byte: usize,
    /// BSP surfaces whose material has a bump map. The map stores no light
    /// direction for them, so they are drawn flat.
    pub bsp_polygons_bump_flat: usize,
    /// Bump maps a material names that couldn't be read, with why.
    pub bump_failures: BTreeMap<String, usize>,
    pub backdrop_polygons: usize,
    /// Terrains (outdoor ground): read, triangles drawn, painted layers
    /// drawn, layers left out with why, terrains that couldn't be read.
    pub terrains: usize,
    pub terrain_triangles: usize,
    pub terrain_layers: usize,
    pub terrain_layer_failures: Vec<String>,
    pub terrain_failures: Vec<String>,
    /// Average baked light (0-255, red green blue) given to terrain corners,
    /// and of all prop corners in the map, to compare the two.
    pub terrain_light: Option<[f32; 3]>,
    pub prop_light: Option<[f32; 3]>,
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
    /// Each zone's distance fog (`crate::fog`).
    pub fog: crate::fog::ZoneFogs,
    /// Why the fog couldn't be read, if it couldn't (then there is none).
    pub fog_error: Option<String>,
}

#[derive(Default)]
struct Builder {
    batches: Vec<Batch>,
    by_key: HashMap<BatchKey, usize>,
}

/// Material, translucent, two-sided, from a mesh, lightmap page, has
/// colours, in the sky, mover, bump-lit.
type BatchKey = (
    String,
    bool,
    bool,
    bool,
    Option<usize>,
    bool,
    bool,
    Option<usize>,
    bool,
);

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
        bump_lit: bool,
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
            bump_lit,
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
                bump_lit,
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
            stats.materials_see_through += p.has_alpha as usize;
            stats.materials_alpha_ignored += (p.alpha_pixels && !p.has_alpha) as usize;
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
        // A surface marked "masked" cuts holes where its picture's alpha
        // is low, whatever the material itself says.
        let (key, pic) = if poly.poly_flags & poly_flags::MASKED != 0 {
            let pic = pic.map(|mut p| {
                p.has_alpha = p.alpha_pixels;
                p
            });
            (format!("{key} (masked)"), pic)
        } else {
            (key, pic)
        };
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
        ctx.stats.bsp_polygons_bump_flat += pic.as_ref().is_some_and(|p| p.bump.is_some()) as usize;
        let batch = b.batch(
            &key,
            &pic,
            translucent,
            two_sided,
            false,
            page,
            false,
            sky,
            None,
            false,
        );
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
        let mut mesh = static_mesh::read_static_mesh(&owner.0, &owner.1, &owner.0.exports[k])
            .map_err(|e| format!("mesh read error: {e}"))?;
        // Its simple collision shape, so physics props settle onto the
        // same things as in `rc_level::map_collision` (a shape that
        // can't be read leaves none).
        let _ = rc_level::collision::read_simple_shape(&owner.0, &owner.1, &mut mesh);
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
    bump_light: Option<&[BumpCorner]>,
    mover: Option<usize>,
) {
    let axes = rotation_axes(p.rotation);
    let s3 = p.draw_scale_3d;
    // Light directions only go with the colours they were baked with.
    let bump_light = bump_light.filter(|l| colours.is_some_and(|c| c.len() == l.len()));
    let mut bump_drawn = false;
    // A mirrored actor (an odd number of negative scales) turns its
    // triangles inside out, so flip them back.
    let mirrored = (s3[0] * s3[1] * s3[2] * p.draw_scale) < 0.0;
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
        let has_map = pic.as_ref().is_some_and(|p| p.bump.is_some());
        let bump_lit = bump_light.is_some() && has_map;
        if bump_light.is_some() && !has_map {
            ctx.stats.bump_parts_without_map += 1;
        }
        bump_drawn |= bump_lit;
        // The texture coordinate sets the picture (and its multiplying
        // layer, if any) are laid on; set 0 if the mesh lacks the one asked
        // for.
        let set = |n: usize| mesh.uv_sets.get(n).or(mesh.uv_sets.first());
        let uv0 = set(pic.as_ref().map_or(0, |p| p.uv_channel));
        let layer_set = pic
            .as_ref()
            .and_then(|p| p.multiply.as_ref())
            .map(|l| set(l.uv_channel));
        if layer_set.is_some() {
            ctx.stats.prop_parts_multiplied += 1;
        }
        // The falloff shader multiplies by the mesh's own painted colours,
        // not the baked light (see `crate::falloff`); white if it has none.
        let falloff = pic.as_ref().is_some_and(|p| p.falloff.is_some());
        let painted: Option<Vec<[u8; 4]>> = falloff.then(|| {
            (0..mesh.vertices.len())
                .map(|i| {
                    mesh.colors
                        .get(i)
                        .map_or([255; 4], |c| rc_image::unreal_color(*c))
                })
                .collect()
        });
        if falloff {
            ctx.stats.prop_parts_falloff += 1;
        }
        let colours = painted.as_deref().or(colours);
        let batch = b.batch(
            &key,
            &pic,
            false,
            false,
            true,
            None,
            colours.is_some(),
            sky,
            mover,
            bump_lit,
        );
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
                    if let Some(l) = layer_set {
                        batch.layer_uvs.push(
                            l.and_then(|u| u.get(vi as usize))
                                .copied()
                                .unwrap_or([0.0, 0.0]),
                        );
                    }
                    if let Some(c) = colours {
                        batch.colours.push(c[vi as usize]);
                    }
                    if let (true, Some(l)) = (bump_lit, bump_light) {
                        let corner = l[vi as usize];
                        // A direction (not a normal): it scales with the
                        // mesh, then turns with it.
                        let mut wd = [0.0f32; 3];
                        for (k, axis) in axes.iter().enumerate() {
                            for c in 0..3 {
                                wd[c] += corner.dir[k] * s3[k] * p.draw_scale * axis[c];
                            }
                        }
                        let len = bsp::dot(wd, wd).sqrt();
                        let wd = if len > 1e-6 {
                            wd.map(|x| x / len)
                        } else {
                            [0.0; 3]
                        };
                        batch.light_dirs.push(dir_to_bevy(wd));
                        batch.light_colours.push(corner.light);
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
    ctx.stats.props_bump_drawn += bump_drawn as usize;
}

/// A physics prop drawn once the ground is known: placement, mesh, corner
/// colours, bump light, in the sky, mover.
type LooseProp = (
    Placement,
    Rc<(StaticMesh, Loaded)>,
    Option<Vec<[u8; 4]>>,
    Option<Vec<BumpCorner>>,
    bool,
    Option<usize>,
);

/// A stored corner colour with extra light added (0-255 scale, alpha kept).
fn add_light(c: [u8; 4], extra: [f32; 3]) -> [u8; 4] {
    let mut out = c;
    for k in 0..3 {
        out[k] = (c[k] as f32 + extra[k]).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Each zone's ambient light (0-255 red, green, blue), by zone number.
///
/// A zone's settings sit on the `ZoneInfo` (or `SkyZoneInfo`) actor placed
/// inside it, so the zone is the one at that actor's spot. The map's
/// `LevelInfo` holds the settings for zones without one.
///
/// The ambient isn't part of the light stored on props: on geo_04a the
/// props' corners that face away from the sun store 0 (the middle value
/// over 6,859 such corners is 1), while the zone there has ambient
/// brightness 36. The sky zone's props store 0 everywhere and have 255.
/// So the game must add it when drawing, as Unreal's zones are described
/// to ("Legacy:ZoneInfo" on the Unreal Wiki).
///
/// The values returned are half the zone's ambient, because that is how
/// much of it the lightmaps hold: their darkest texels are half the
/// ambient (geo_04a: 35 for an ambient of 73 at its brightest; yyy_06a:
/// 21, 21, 27 for 36, 40, 50), with the sun on top at the same scale as on
/// the props (geo_04a: up to about 90 on both). Lightmaps and prop colours
/// are both drawn at double strength (`LightingSettings`), so the ambient
/// ends up at its full value on screen.
fn zone_ambients(
    pkg: &Package,
    data: &[u8],
    level: &actors::LevelInfo,
    bsp: &Bsp,
) -> HashMap<u8, [f32; 3]> {
    let mut by_zone = HashMap::new();
    let mut fallback = None;
    for &a in level.actors.iter().flatten() {
        let class = pkg.export_class_name(&pkg.exports[a]).to_ascii_lowercase();
        let is_level = class == "levelinfo";
        if !is_level && !class.contains("zoneinfo") {
            continue;
        }
        let Ok(props) = actors::read_actor_properties(pkg, data, a) else {
            continue;
        };
        let Some(z) = light::zone_ambient(pkg, &props).filter(|z| z.brightness > 0) else {
            continue;
        };
        let half = z.rgb().map(|c| c * 0.5);
        if is_level {
            fallback = Some(half);
        } else if let Some(zone) = bsp.point_zone(light::location(pkg, &props)) {
            by_zone.insert(zone, half);
        }
    }
    if let Some(f) = fallback {
        for zone in bsp.nodes.iter().flat_map(|n| n.zone) {
            by_zone.entry(zone).or_insert(f);
        }
    }
    by_zone
}

/// How much more of the zone's ambient light bump-mapped props get than
/// other props: the zone's whole ambient instead of half of it.
///
/// Props whose materials are bump-mapped (Republic Commando's `HsBump...`
/// materials) store a second list of light directions (see
/// `actors::read_baked_light`), so the original draws them with its bump
/// shader, not like other props. Measured on the user's original-game
/// screenshot of yyy_06a's start tunnel (1600x1200), against our view from
/// the same spot: with half the ambient the parts lit only by it came out
/// at about half the original's brightness (left wall 7 against 18, near
/// floor 13 against 40, ceiling 13 against 38), while the parts a lamp
/// lights were close (38 against 41). One screenshot, so a calibration,
/// not a proof. Not used on geo_04a-style props without bump maps, which
/// matched with half.
const BUMP_PROP_AMBIENT: f32 = 2.0;

/// Which zone a placed prop is drawn in, for its ambient light.
///
/// Usually the zone at the prop's own spot (`Location`). But level pieces
/// such as tunnel walls, floors and ceilings often have their spot inside
/// the solid rock behind them, which is BSP zone 0: no zone, no ambient
/// light. In yyy_06a that is 247 of 556 props, the whole start tunnel. The
/// original game shows them lit like the rest of the zone (its dark parts
/// match the zone's ambient), so for those props each corner is moved a
/// little off the surface, the way it faces, and the zone most corners land
/// in is used. If no corner lands in a zone, zone 0 stays.
fn prop_zone(bsp: &Bsp, p: &Placement, mesh: &StaticMesh, stats: &mut Stats) -> Option<u8> {
    let zone = bsp.point_zone(p.location);
    if zone != Some(0) || mesh.vertices.is_empty() {
        return zone;
    }
    /// How far off the surface to test, in Unreal units (a hand's width).
    const NUDGE: f32 = 8.0;
    let axes = rotation_axes(p.rotation);
    let mut counts = [0usize; 256];
    for v in &mesh.vertices {
        let w = p.transform_point(v.position);
        let mut n = [0.0f32; 3];
        for (k, axis) in axes.iter().enumerate() {
            for c in 0..3 {
                n[c] += v.normal[k] * axis[c];
            }
        }
        let len = bsp::dot(n, n).sqrt();
        if len < 1e-6 {
            continue;
        }
        let at = [0, 1, 2].map(|c| w[c] + n[c] / len * NUDGE);
        if let Some(z) = bsp.point_zone(at).filter(|&z| z != 0) {
            counts[z as usize] += 1;
        }
    }
    match (1..256)
        .max_by_key(|&z| counts[z])
        .filter(|&z| counts[z] > 0)
    {
        Some(z) => {
            stats.props_zone_from_corners += 1;
            Some(z as u8)
        }
        None => zone,
    }
}

/// Average of the batches' corner colours (red, green, blue, 0-255).
fn average_light<'a>(batches: impl Iterator<Item = &'a Batch>) -> Option<[f32; 3]> {
    let mut sum = [0.0f64; 3];
    let mut n = 0usize;
    for b in batches {
        for c in &b.colours {
            for k in 0..3 {
                sum[k] += c[k] as f64;
            }
            n += 1;
        }
    }
    (n > 0).then(|| sum.map(|v| (v / n as f64) as f32))
}

/// The light a terrain corner gets: its zone's ambient light (the halved value
/// from `zone_ambients`, as for props) plus each sun
/// (`Sunlight`) by how squarely it faces the corner. The game's own terrain
/// light isn't stored in the map's terrain data as far as we can tell, so
/// this is worked out the way the props' stored light behaves: on geo_04a
/// the props' light rises in a straight line with how squarely they face
/// the sun (half the sun's colour x brightness at half-facing, and so on;
/// measured 2026-11-01). It is about 20% brighter than the props there,
/// and has no shadows; other lights are left out.
fn terrain_light(normal: [f32; 3], suns: &[(Vec3f, [f32; 3])], ambient: [f32; 3]) -> [f32; 3] {
    let mut c = ambient;
    for (dir, colour) in suns {
        // `dir` is the way the light travels; a face lit head-on faces
        // the other way.
        let facing = -(normal[0] * dir[0] + normal[1] * dir[1] + normal[2] * dir[2]);
        if facing > 0.0 {
            for k in 0..3 {
                c[k] += colour[k] * facing;
            }
        }
    }
    c
}

type Vec3f = [f32; 3];

/// How a point or spot light fades with distance, for props we light
/// ourselves (`unbaked_prop_light`): full at the light, nothing at its
/// radius, in a straight line (see `POINT_LIGHT_SCALE`).
fn point_falloff(distance: f32, radius: f32) -> f32 {
    if radius <= 0.0 || distance >= radius {
        return 0.0;
    }
    1.0 - distance / radius
}

/// Point and spot lights on props we light ourselves: colour x brightness
/// x facing x `point_falloff` x this. Fitted 2026-11-10 by working out the
/// light for pro's props that do have stored light and comparing corner
/// by corner (corners lit in both): with a straight fade the stored light
/// was 1.58 times ours (middle half 1.46-1.61) and about as many corners
/// were lit (57,057 stored, 58,894 ours), so the shape fits and only the
/// strength was off. A fade squared, or a shorter reach, lit too few
/// corners and spread the ratio out (2.3-4.2). With 1.6: 0.99 (0.91-1.00).
const POINT_LIGHT_SCALE: f32 = 1.6;

/// Suns on props we light ourselves: colour x brightness x facing x this.
/// The same comparison on geo_01briefing (sun-lit) gave 0.81 (middle half
/// 0.80-0.81), which also matches the terrain being about 20% brighter
/// than the props with the full value (`terrain_light`). On geo_01a the
/// canyon walls shade much of it (39,519 corners lit there against our
/// 101,265, no shadows here), so it isn't usable for fitting.
const SUN_LIGHT_SCALE: f32 = 0.8;

/// Corner light for a prop the map stores no light for, worked out from the
/// map's lights the way the stored light behaves (0-255 scale, before the
/// draw gain).
///
/// The map only stores light for props that stay put (their
/// `StaticMeshInstance`). Props a level script moves or shows later, such
/// as the clone babies in the prologue's tanks (`bStatic` off), have none,
/// and Unreal lights those as it draws them, from the lights around them.
/// They used to be drawn at full brightness, which showed them as flat
/// shapes with no shading at all.
///
/// Each corner gets its zone's ambient (the halved value, as on other
/// props) plus, from each light that doesn't move (moving ones are added
/// live by `PropMaterial`), the light's colour x brightness x how squarely
/// the corner faces it x a fade to nothing at its radius. No shadows.
fn unbaked_prop_light(
    p: &Placement,
    mesh: &StaticMesh,
    lights: &[Light],
    ambient: [f32; 3],
) -> Vec<[u8; 4]> {
    let axes = rotation_axes(p.rotation);
    let s3 = p.draw_scale_3d;
    let world_points: Vec<Vec3f> = mesh
        .vertices
        .iter()
        .map(|v| p.transform_point(v.position))
        .collect();
    // Only lights that can reach the prop at all.
    let (lo, hi) = world_points
        .iter()
        .fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), w| {
            (
                [lo[0].min(w[0]), lo[1].min(w[1]), lo[2].min(w[2])],
                [hi[0].max(w[0]), hi[1].max(w[1]), hi[2].max(w[2])],
            )
        });
    let near: Vec<&Light> = lights
        .iter()
        .filter(|l| !l.dynamic)
        .filter(|l| {
            l.kind == light::LightKind::Directional || {
                let d2: f32 = (0..3)
                    .map(|k| {
                        let c = l.location[k].clamp(lo[k], hi[k]);
                        (l.location[k] - c).powi(2)
                    })
                    .sum();
                d2 < l.radius * l.radius
            }
        })
        .collect();
    mesh.vertices
        .iter()
        .zip(&world_points)
        .map(|(v, w)| {
            let mut wn = [0.0f32; 3];
            for (k, axis) in axes.iter().enumerate() {
                for c in 0..3 {
                    let sk = if s3[k].abs() < 1e-6 { 1e-6 } else { s3[k] };
                    wn[c] += v.normal[k] * axis[c] / sk;
                }
            }
            let len = bsp::dot(wn, wn).sqrt().max(1e-6);
            let n = wn.map(|x| x / len);
            let mut c = ambient;
            for l in &near {
                let colour = l.color().map(|x| x * l.brightness);
                let strength = match l.kind {
                    light::LightKind::Directional => {
                        let dir = rotation_axes(l.rotation)[0];
                        (-bsp::dot(n, dir)).max(0.0) * SUN_LIGHT_SCALE
                    }
                    light::LightKind::Point | light::LightKind::Spot => {
                        let to = [
                            l.location[0] - w[0],
                            l.location[1] - w[1],
                            l.location[2] - w[2],
                        ];
                        let d = bsp::dot(to, to).sqrt().max(1e-3);
                        let to = to.map(|x| x / d);
                        let mut k = bsp::dot(n, to).max(0.0)
                            * point_falloff(d, l.radius)
                            * POINT_LIGHT_SCALE;
                        if l.kind == light::LightKind::Spot {
                            // Same cone reading as the live spotlights
                            // (`lighting::spawn_lights`): 0-255 is 0-90 degrees.
                            let axis = rotation_axes(l.rotation)[0];
                            let outer = (l.cone as f32 / 255.0 * std::f32::consts::FRAC_PI_2)
                                .clamp(0.02, 1.55);
                            let along = -bsp::dot(to, axis);
                            if along < outer.cos() {
                                k = 0.0;
                            }
                        }
                        k
                    }
                };
                if strength > 0.0 {
                    for k in 0..3 {
                        c[k] += colour[k] * strength;
                    }
                }
            }
            [c[0], c[1], c[2], 255.0].map(|v| v.clamp(0.0, 255.0).round() as u8)
        })
        .collect()
}

/// A terrain's alpha (0-255) at grid point (x, y), read from its alpha map
/// picture stretched over the whole grid, blending the four nearest pixels
/// (the maps are often smaller than the grid: 16x16 over 64x64 points).
fn alpha_at(p: &Picture, x: usize, y: usize, size_x: usize, size_y: usize) -> u8 {
    let w = p.image.width.max(1);
    let h = p.image.height.max(1);
    let get = |px: usize, py: usize| {
        p.image
            .pixels
            .get((py.min(h - 1) * w + px.min(w - 1)) * 4 + 3)
            .copied()
            .unwrap_or(0) as f32
    };
    // Pixel centres sit at (i + 0.5) / w across the grid.
    let fx = ((x as f32 / (size_x.max(2) - 1) as f32) * w as f32 - 0.5).max(0.0);
    let fy = ((y as f32 / (size_y.max(2) - 1) as f32) * h as f32 - 0.5).max(0.0);
    let (x0, y0) = (fx as usize, fy as usize);
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let top = get(x0, y0) * (1.0 - tx) + get(x0 + 1, y0) * tx;
    let bottom = get(x0, y0 + 1) * (1.0 - tx) + get(x0 + 1, y0 + 1) * tx;
    (top * (1.0 - ty) + bottom * ty).round().clamp(0.0, 255.0) as u8
}

/// Layers painted with the editor's checkerboard texture (`Chex`) are
/// where the level designers placed bushes and other decorations, not
/// ground to draw: their alpha maps are named like `01DecoPalm` and
/// `BushyBush` (yyy_01b, yyy_35a). They are left out. Not compared with
/// the real game.
fn is_decoration_layer(texture_key: &str) -> bool {
    texture_key
        .rsplit('.')
        .next()
        .is_some_and(|n| n.eq_ignore_ascii_case("Chex"))
}

/// Draw one terrain: a batch per painted layer, all sharing the grid.
fn add_terrain(
    b: &mut Builder,
    ctx: &mut Ctx,
    t: &rc_level::terrain::Terrain,
    lights: &[Light],
    ambient: [f32; 3],
    sky: bool,
    props_light: Option<[f32; 3]>,
) {
    let tris = t.triangles();
    if tris.is_empty() {
        return;
    }
    let normals = t.normals();
    let suns: Vec<(Vec3f, [f32; 3])> = lights
        .iter()
        .filter(|l| l.kind == light::LightKind::Directional)
        .map(|l| {
            let dir = rotation_axes(l.rotation)[0];
            let c = l.color();
            (dir, c.map(|v| v * l.brightness))
        })
        .collect();
    // On the same 0-255 scale as the light baked into props, so the two
    // match: props lit by geo_04a's sun store about brightness x colour
    // x how squarely they face it (see `terrain_light`).
    // Without a sun (geo_03a, yyy_01b, yyy_35a) the ground is lit by
    // ordinary lights, which aren't worked out for terrain yet. Until they
    // are, it gets the props' average light, flat, so it isn't left darker
    // than everything around it.
    let flat = suns.is_empty().then_some(props_light).flatten();
    let light: Vec<[u8; 4]> = normals
        .iter()
        .map(|n| {
            let c = flat.unwrap_or_else(|| terrain_light(*n, &suns, ambient));
            [c[0], c[1], c[2], 255.0].map(|v| v.clamp(0.0, 255.0).round() as u8)
        })
        .collect();
    for (li, layer) in t.layers.iter().enumerate() {
        let (key, pic) = picture(
            ctx.mats,
            &mut ctx.stats,
            &mut ctx.seen,
            ctx.pkg,
            ctx.data,
            layer.texture,
        );
        if pic.is_none() {
            ctx.stats
                .terrain_layer_failures
                .push(format!("layer {li}: texture {key} not shown"));
            continue;
        }
        if is_decoration_layer(&key) {
            ctx.stats.terrain_layer_failures.push(format!(
                "layer {li}: {key} is a decoration placement layer, not drawn"
            ));
            continue;
        }
        // Layer 0 covers everything. The others need their alpha map.
        let alpha = if li == 0 {
            None
        } else {
            match ctx.mats.resolve(ctx.pkg, ctx.data, layer.alpha_map) {
                Ok(p) => Some(p),
                Err(e) => {
                    ctx.stats
                        .terrain_layer_failures
                        .push(format!("layer {li}: alpha map not read ({e})"));
                    continue;
                }
            }
        };
        let alphas: Vec<u8> = (0..t.points.len())
            .map(|i| match &alpha {
                Some(a) => alpha_at(a, i % t.size_x, i / t.size_x, t.size_x, t.size_y),
                None => 255,
            })
            .collect();
        // Leave out a layer that shows nowhere.
        if alphas.iter().all(|&a| a == 0) {
            ctx.stats
                .terrain_layer_failures
                .push(format!("layer {li}: {key} shows nowhere (alpha map all 0)"));
            continue;
        }
        // The texture repeats every UScale squares of the grid. Checked
        // against the layers' own stored `TerrainMatrix` on yyy_01b (UScale
        // 2 and 10 over 64-unit squares: 1/128 and 1/640 per unit).
        let us = layer.u_scale.abs().max(1e-3) * t.spacing[0];
        let vs = layer.v_scale.abs().max(1e-3) * t.spacing[1];
        let key = format!("terrain {} layer {li}: {key}", t.export);
        let batch = b.batch(
            &key,
            &pic,
            li > 0,
            true,
            false,
            None,
            true,
            sky,
            None,
            false,
        );
        batch.terrain_layer = Some(li);
        let base = batch.positions.len() as u32;
        for (i, p) in t.points.iter().enumerate() {
            batch.positions.push(to_bevy(*p));
            batch.normals.push(dir_to_bevy(normals[i]));
            batch.uvs.push([p[0] / us, p[1] / vs]);
            let mut c = light[i];
            c[3] = alphas[i];
            batch.colours.push(c);
        }
        for tri in &tris {
            batch.indices.extend(tri.map(|i| base + i));
        }
        ctx.stats.terrain_layers += 1;
        if li == 0 {
            ctx.stats.terrain_triangles += tris.len();
        }
    }
    ctx.stats.terrain_light = average_light(b.batches.iter().filter(|b| b.terrain_layer.is_some()));
}

pub fn build(map: Loaded, mats: &mut Materials) -> Result<Scene, String> {
    let (pkg, data) = (&map.0, &map.1);
    let level = actors::read_level(pkg, data).map_err(|e| format!("level: {e}"))?;
    let model = level.model.ok_or("the level has no BSP model")?;
    let bsp = bsp::read_model(pkg, data, &pkg.exports[model]).map_err(|e| format!("BSP: {e}"))?;

    let falloff_error = if mats.falloff_defaults.is_none() {
        match crate::falloff::FalloffDefaults::read(mats) {
            Ok(d) => {
                mats.falloff_defaults = Some(d);
                None
            }
            Err(e) => Some(e),
        }
    } else {
        None
    };
    let (fog, fog_error) = match crate::fog::FogDefaults::read(mats) {
        Ok(d) => (crate::fog::ZoneFogs::read(pkg, data, &level, &bsp, &d), None),
        Err(e) => (crate::fog::ZoneFogs::default(), Some(e)),
    };

    let mut b = Builder::default();
    let mut ctx = Ctx {
        mats,
        stats: Stats {
            falloff_error,
            ..Stats::default()
        },
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
    let mut ambients = Vec::new();
    let zone_ambient = zone_ambients(pkg, data, &level, &bsp);
    ctx.stats.zones_with_ambient = zone_ambient.len();

    let mut spawn = None;
    let mut floor_points = Vec::new();
    let mut mesh_cache = HashMap::new();
    let mut class_looks = rc_level::class_look::ClassLooks::new();
    // Doors and lifts get batches of their own, so they can move.
    let movers = {
        let mats = &mut *ctx.mats;
        rc_level::movers::read_movers(
            &mut class_looks,
            pkg,
            data,
            &level.actors,
            &mut |n: &str| mats.package(n),
        )
    };
    let mut mover_exports: std::collections::HashSet<usize> =
        movers.movers.iter().map(|m| m.place.export).collect();
    // So do props with a tag of their own, which level scripts may remove
    // (barricades, bunker walls). They are tagged `MoverPart` too.
    mover_exports.extend(rc_level::scripts::tagged_actors(pkg, data, &level).into_keys());
    // And props weapons can break (barrels, consoles, rock cover).
    {
        let mats = &mut *ctx.mats;
        mover_exports.extend(
            rc_level::breakables::read(&mut class_looks, pkg, data, &level, &mut |n: &str| {
                mats.package(n)
            })
            .into_iter()
            .map(|b| b.export),
        );
    }
    // And loose physics props (`rc_level::karma`): the physics part moves
    // them. Their models are drawn where the map puts them; the physics
    // part lets them fall from there, as the original does.
    let (karma_bodies, script_spawns) = {
        let mats = &mut *ctx.mats;
        let mut open = |n: &str| mats.package(n);
        let (bodies, _) = rc_level::karma::read(&mut class_looks, &map, &level, &mut open);
        let spawns = rc_level::karma::script_spawns(&mut class_looks, &map, &level, &mut open);
        (bodies, spawns)
    };
    mover_exports.extend(karma_bodies.iter().filter_map(|b| b.export));
    ctx.stats.movers = movers.movers.len();
    let mut placed: Vec<(Placement, Rc<(StaticMesh, Loaded)>)> = Vec::new();
    // Ammo boxes, dropped onto the ground like the game drops them
    // (`rc_level::pickups::rest_on_ground`).
    let mut boxes: Vec<LooseProp> = Vec::new();
    #[allow(clippy::type_complexity)]
    let mut unbaked: Vec<(
        Placement,
        Rc<(StaticMesh, Loaded)>,
        Option<u8>,
        Option<usize>,
    )> = Vec::new();
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
                } else if let Some(z) = light::zone_ambient(pkg, &props) {
                    ctx.stats.zone_ambients += 1;
                    ambients.push(z);
                }
            }
        }
        let mut p = p;
        // Ammo and grenade boxes, and weapons lying about: drawn on their
        // own so they can vanish when taken, with the model their
        // "...Defaults" class gives.
        let is_box = {
            let mats = &mut *ctx.mats;
            rc_level::pickups::is_takeable(&mut class_looks, pkg, p.export, &mut |n: &str| {
                mats.package(n)
            })
        };
        // Props and doors that take their model from their class.
        let mut owner = map.clone();
        if p.static_mesh == 0 {
            let mats = &mut *ctx.mats;
            let found = if is_box {
                rc_level::pickups::apply_box_look(&mut class_looks, pkg, &mut p, &mut |n: &str| {
                    mats.package(n)
                })
            } else {
                rc_level::class_look::apply_class_look(
                    &mut class_looks,
                    pkg,
                    data,
                    &mut p,
                    &mut |n: &str| mats.package(n),
                )
            };
            match found {
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
                let baked = if p.static_mesh_instance != 0 {
                    actors::read_baked_light(pkg, data, p.static_mesh_instance).ok()
                } else {
                    None
                };
                let bump_lit = baked.as_ref().is_some_and(|b| !b.directions.is_empty());
                ctx.stats.props_bump_lit += bump_lit as usize;
                // Worked out from the stored colours before ambient is
                // added: the light directions go with those.
                let bump_light = baked
                    .as_ref()
                    .and_then(|b| bump_corners(&b.colours, &b.directions));
                let colours = baked.map(|b| b.colours);
                let zone = prop_zone(&bsp, &p, &m.0, &mut ctx.stats);
                let in_sky = sky_zone.is_some() && zone == sky_zone;
                // The game adds the zone's ambient light when it draws a
                // prop: it isn't in the stored colours (corners facing away
                // from every light store 0,0,0; see `zone_ambients`). Half
                // of it, like the lightmaps (see `zone_ambients`), except on
                // bump-mapped props (see `BUMP_PROP_AMBIENT`).
                let colours = match (colours, zone.and_then(|z| zone_ambient.get(&z))) {
                    (Some(c), Some(a)) => {
                        ctx.stats.props_with_ambient += 1;
                        let k = if bump_lit { BUMP_PROP_AMBIENT } else { 1.0 };
                        let a = a.map(|x| x * k);
                        Some(c.iter().map(|v| add_light(*v, a)).collect())
                    }
                    (c, _) => c,
                };
                // Sky domes and backdrops store no light at all (every
                // corner 0,0,0 on geo_04a, yyy_04a, yyy_05a, yyy_06a), and
                // the game shows them lit, usually by the sky zone's bright
                // ambient (geo_04a: 255), added above. If the sky zone has
                // no ambient, draw them at full brightness instead of black.
                let colours = match colours {
                    Some(c) if in_sky && c.iter().all(|v| v[0] == 0 && v[1] == 0 && v[2] == 0) => {
                        ctx.stats.sky_props_unlit += 1;
                        None
                    }
                    other => other,
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
                ctx.stats.sky_props += in_sky as usize;
                let mover = (is_box || mover_exports.contains(&p.export)).then_some(p.export);
                if is_box {
                    boxes.push((p, m, colours, bump_light, in_sky, mover));
                    continue;
                }
                ctx.stats.movers_drawn += mover
                    .is_some_and(|m| movers.movers.iter().any(|x| x.place.export == m))
                    as usize;
                // No stored light (props that can move or appear later):
                // lit from the map's lights once they are all read.
                if colours.is_none() && !in_sky && !p.unlit && p.static_mesh_instance == 0 {
                    unbaked.push((p, m, zone, mover));
                    continue;
                }
                add_mesh_actor(
                    &mut b,
                    &mut ctx,
                    in_sky,
                    &p,
                    &m.0,
                    &m.1,
                    colours.as_deref(),
                    bump_light.as_deref(),
                    mover,
                );
                placed.push((p, m));
            }
            Err(e) => *ctx.stats.mesh_failures.entry(e).or_default() += 1,
        }
    }
    // Physics props the level's scripts make (geo_04's falling rocks):
    // drawn once each where its script puts it, as a hidden model the
    // physics part copies for every rock it makes (tagged `MoverPart`
    // with the action's export index). Unreal lights such actors from the
    // lights around them as it draws them; here that light is worked out
    // where the script puts them, as for other props with no stored light.
    for sp in &script_spawns {
        let Ok(body) = &sp.body else { continue };
        let found = (|| {
            let (first, rest) = body.mesh_path.split_first()?;
            let owner = ctx.mats.package(first)?;
            let k = crate::materials::find_export(&owner.0, rest)?;
            let mesh = static_mesh::read_static_mesh(&owner.0, &owner.1, &owner.0.exports[k]).ok()?;
            Some(Rc::new((mesh, owner)))
        })();
        let Some(m) = found else {
            ctx.stats.mesh_failures.entry(format!("script prop {}: model not read", sp.class)).and_modify(|n| *n += 1).or_insert(1);
            continue;
        };
        let p = Placement {
            export: sp.action,
            class: sp.class.clone(),
            name: sp.script.clone(),
            location: body.location,
            rotation: body.rotation,
            draw_scale: 1.0,
            draw_scale_3d: body.scale,
            pre_pivot: [0.0; 3],
            static_mesh: 0,
            skins: Vec::new(),
            hidden: false,
            unlit: false,
            static_mesh_instance: 0,
        };
        let zone = prop_zone(&bsp, &p, &m.0, &mut ctx.stats);
        unbaked.push((p, m, zone, Some(sp.action)));
    }
    for (p, m, zone, mover) in unbaked {
        let ambient = zone
            .and_then(|z| zone_ambient.get(&z))
            .copied()
            .unwrap_or([0.0; 3]);
        let colours = unbaked_prop_light(&p, &m.0, &lights, ambient);
        ctx.stats.props_lit_from_lights += 1;
        add_mesh_actor(
            &mut b,
            &mut ctx,
            false,
            &p,
            &m.0,
            &m.1,
            Some(&colours),
            None,
            mover,
        );
        // The script props' hidden models aren't level ground.
        if !script_spawns.iter().any(|sp| sp.action == p.export) {
            placed.push((p, m));
        }
    }
    let (terrains, terrain_failures) =
        rc_level::terrain::read_terrains(pkg, data, level.actors.iter().flatten().copied());
    ctx.stats.terrains = terrains.len();
    ctx.stats.terrain_failures = terrain_failures;
    {
        let pairs: Vec<_> = placed.iter().map(|(p, m)| (p, &m.0)).collect();
        let mut ground = rc_level::collision::collision_triangles(&bsp, &pairs);
        rc_level::collision::add_terrains(&mut ground, &terrains);
        let ground = rc_level::settle::Ground::new(&ground.triangles);
        for (mut p, m, colours, bump_light, in_sky, mover) in boxes {
            let found = {
                let mats = &mut *ctx.mats;
                rc_level::pickups::read_box(&mut class_looks, pkg, data, p.export, &mut |n: &str| {
                    mats.package(n)
                })
            };
            if let Some(mut bx) = found {
                let d = rc_level::pickups::rest_on_ground(&mut bx, |q| ground.below(q));
                p.location[2] -= d;
                ctx.stats.ammo_boxes += 1;
                ctx.stats.ammo_box_drop_max = ctx.stats.ammo_box_drop_max.max(d);
            } else {
                let found = {
                    let mats = &mut *ctx.mats;
                    rc_level::pickups::read_weapon(
                        &mut class_looks,
                        pkg,
                        data,
                        p.export,
                        &mut |n: &str| mats.package(n),
                    )
                };
                if let Some(mut w) = found {
                    let d = rc_level::pickups::rest_weapon_on_ground(&mut w, |q| ground.below(q));
                    p.location[2] -= d;
                    ctx.stats.weapon_pickups += 1;
                }
            }
            add_mesh_actor(
                &mut b,
                &mut ctx,
                in_sky,
                &p,
                &m.0,
                &m.1,
                colours.as_deref(),
                bump_light.as_deref(),
                mover,
            );
        }
    }
    ctx.stats.prop_light = average_light(b.batches.iter().filter(|b| b.from_mesh));
    {
        let mut seen = std::collections::HashSet::new();
        for p in b
            .batches
            .iter()
            .filter(|b| b.bump_lit)
            .filter_map(|b| b.picture.as_ref())
        {
            if let Some(m) = p.bump.as_ref().filter(|m| seen.insert(Arc::as_ptr(m))) {
                ctx.stats.bump_maps += 1;
                ctx.stats.bump_maps_two_byte += m.two_byte_format as usize;
            }
        }
        ctx.stats.bump_failures = ctx.mats.bump_failures.clone();
    }
    for t in &terrains {
        let zone = bsp.point_zone(t.point(t.size_x / 2, t.size_y / 2));
        let in_sky = sky_zone.is_some() && zone == sky_zone;
        let ambient = zone
            .and_then(|z| zone_ambient.get(&z))
            .copied()
            .unwrap_or([0.0; 3]);
        let props_light = ctx.stats.prop_light;
        add_terrain(&mut b, &mut ctx, t, &lights, ambient, in_sky, props_light);
    }
    let pairs: Vec<_> = placed.iter().map(|(p, m)| (p, &m.0)).collect();
    let mut collision = rc_level::collision::collision_triangles(&bsp, &pairs);
    rc_level::collision::add_terrains(&mut collision, &terrains);
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
        fog,
        fog_error,
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
