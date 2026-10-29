//! `rc_viewer --mesh`: shows one static mesh (a fixed 3D model) from a `.usx`
//! package, with the texture its material points at.
//!
//! The model is drawn "unlit" (no lights or shading, every pixel is just the
//! texture colour) through an orthographic camera (no perspective: far parts
//! are drawn the same size as near ones). Both make the on-screen check exact:
//! `check_screen` draws the same model itself, in plain Rust, and compares its
//! picture with what Bevy put on screen.

use crate::{
    load_texture, read_capture, Capture, FrameStats, LoadedTexture, RunLog, ScreenCheck,
    WINDOW_SIZE,
};
use bevy::asset::RenderAssetUsages;
use bevy::camera::ScalingMode;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::ScreenshotCaptured;
use bevy::render::view::{Msaa, Tonemapping};
use rc_image::Rgba;
use rc_package::properties::{self, Value};
use rc_package::static_mesh::{self, StaticMesh};
use rc_package::{ObjectRef, Package, Reader};
use std::fs;
use std::path::Path;
use std::time::Instant;

/// The model fills this share of the window.
const FILL: f32 = 0.9;
/// How fast the model turns once the check is done, in turns per minute.
const SPIN_PER_MINUTE: f32 = 6.0;

/// The mesh, already in Bevy's coordinates, handed from `main` to `setup`.
#[derive(Resource)]
pub(crate) struct LoadedMesh {
    name: String,
    /// Positions with the model's centre moved to (0, 0, 0).
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u16>,
    /// Size of the model along Bevy's x, y, z.
    extent: [f32; 3],
    texture: LoadedTexture,
}

/// Marks the model's entity, so the spin system can find it.
#[derive(Component)]
pub(crate) struct Model;

/// Unreal's axes are x forward, y right, z up ("left-handed"). Bevy's are
/// x right, y up, z toward the viewer ("right-handed"). Swapping y and z
/// turns one into the other without mirroring the model.
fn to_bevy(v: [f32; 3]) -> [f32; 3] {
    [v[0], v[2], v[1]]
}

/// Which way each triangle faces, worked out from its corner order, compared
/// with the normals stored in the file. Returns (agree, disagree) for
/// counter-clockwise front faces, the order Bevy draws.
fn winding_agreement(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    indices: &[u16],
) -> (usize, usize) {
    let (mut agree, mut disagree) = (0, 0);
    for t in indices.as_chunks::<3>().0 {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(positions[i as usize]));
        let face = (b - a).cross(c - a);
        let stored: Vec3 = t.iter().map(|&i| Vec3::from(normals[i as usize])).sum();
        if face.dot(stored) > 0.0 {
            agree += 1;
        } else {
            disagree += 1;
        }
    }
    (agree, disagree)
}

/// Find what a material reference points at: a `Texture` or `Shader` import
/// whose outermost package is a `.utx` file in `GameData/Textures`. Returns
/// its full name, that file's name, and the class.
fn material_import(pkg: &Package, material: i32) -> Result<(String, String, &'static str), String> {
    let ObjectRef::Import(i) = ObjectRef::from_index(material) else {
        return Err("material is not an import (not supported yet)".into());
    };
    let import = &pkg.imports[i];
    let class = pkg.name(import.class_name);
    let name = pkg.name(import.object_name).to_string();
    let class = match class {
        "Texture" => "Texture",
        "Shader" => "Shader",
        _ => {
            return Err(format!(
                "material {name} is a {class}, only textures and shaders are supported yet"
            ))
        }
    };
    // Walk up the chain of groups to the outermost package.
    let mut outer = import.package_index;
    let mut chain = Vec::new();
    while let ObjectRef::Import(o) = ObjectRef::from_index(outer) {
        chain.push(pkg.name(pkg.imports[o].object_name).to_string());
        outer = pkg.imports[o].package_index;
    }
    let file = format!(
        "{}.utx",
        chain
            .last()
            .ok_or("material's package is unknown")?
            .to_lowercase()
    );
    chain.reverse();
    Ok((format!("{}.{name}", chain.join(".")), file, class))
}

/// A `Shader` combines several textures. Its `Diffuse` property is the main
/// colour texture, which is the one we draw for now. Returns that texture's
/// name, read from the shader's property list in the `.utx` file.
fn shader_diffuse(path: &Path, shader: &str) -> Result<String, String> {
    let data = fs::read(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    let pkg = Package::parse(&data).map_err(|e| e.to_string())?;
    let export = pkg
        .exports
        .iter()
        .find(|e| pkg.export_class_name(e) == "Shader" && pkg.name(e.object_name) == shader)
        .ok_or_else(|| format!("no shader called {shader} in {}", path.display()))?;
    let start = export.serial_offset as usize;
    let object = data
        .get(start..start + export.serial_size as usize)
        .ok_or("shader data is outside the file")?;
    let props =
        properties::read_properties(&pkg, &mut Reader::new(object)).map_err(|e| e.to_string())?;
    let diffuse = match properties::find(&pkg, &props, "Diffuse").map(|p| &p.value) {
        Some(Value::Object(i)) if *i != 0 => *i,
        _ => return Err(format!("shader {shader} has no Diffuse texture")),
    };
    match ObjectRef::from_index(diffuse) {
        ObjectRef::Export(i) if pkg.export_class_name(&pkg.exports[i]) == "Texture" => {
            Ok(pkg.name(pkg.exports[i].object_name).to_string())
        }
        _ => Err(format!(
            "shader {shader}'s Diffuse ({}) is not a texture in the same file (not supported yet)",
            pkg.object_name(diffuse)
        )),
    }
}

/// Read the mesh and its texture from the game files and log the numbers.
pub(crate) fn load(
    log: &RunLog,
    game: &Path,
    path: &Path,
    wanted: &str,
) -> Result<LoadedMesh, String> {
    let t = Instant::now();
    let data = fs::read(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    let pkg = Package::parse(&data).map_err(|e| e.to_string())?;
    log.line(format!(
        "package read: {} bytes, {} exports, {:.1} ms",
        data.len(),
        pkg.exports.len(),
        t.elapsed().as_secs_f64() * 1000.0
    ));
    let export = pkg
        .exports
        .iter()
        .find(|e| {
            pkg.export_class_name(e) == "StaticMesh"
                && pkg.name(e.object_name).eq_ignore_ascii_case(wanted)
        })
        .ok_or_else(|| format!("no static mesh called {wanted} in this package"))?;
    let name = pkg.name(export.object_name).to_string();
    let mesh: StaticMesh =
        static_mesh::read_static_mesh(&pkg, &data, export).map_err(|e| e.to_string())?;
    log.line(format!(
        "mesh read: {name}, {} vertices, {} triangles, {} sections, {} UV sets, {} vertex colours, {} bytes of collision data not read",
        mesh.vertices.len(),
        mesh.triangle_count(),
        mesh.sections.len(),
        mesh.uv_sets.len(),
        mesh.colors.len(),
        mesh.unread
    ));
    let b = &mesh.bounding_box;
    let size: [f32; 3] = std::array::from_fn(|k| b.max[k] - b.min[k]);
    log.line(format!(
        "mesh bounding box (Unreal units, x forward, y right, z up): min {:?} max {:?}, size {:.2} x {:.2} x {:.2}",
        b.min, b.max, size[0], size[1], size[2]
    ));
    if let Some((lo, hi)) = mesh.uv_range(0) {
        log.line(format!(
            "mesh UV range: U {:.4} to {:.4}, V {:.4} to {:.4}",
            lo[0], hi[0], lo[1], hi[1]
        ));
    }
    let problems = mesh.check();
    if problems.is_empty() {
        log.line("mesh checks: all passed");
    } else {
        for p in &problems {
            log.line(format!("mesh check problem: {p}"));
        }
        return Err(format!("{} mesh check(s) failed", problems.len()));
    }

    // The texture. Without one, draw the model plain grey.
    let texture = match mesh.materials.first() {
        None => Err("mesh has no material".to_string()),
        Some(&m) => material_import(&pkg, m),
    }
    .and_then(|(full_name, file, class)| {
        log.line(format!(
            "material: {class} {full_name}, looked up in GameData/Textures/{file}"
        ));
        let path = game.join("GameData").join("Textures").join(file);
        let mut tex_name = full_name.rsplit('.').next().unwrap_or_default().to_string();
        if class == "Shader" {
            tex_name = shader_diffuse(&path, &tex_name)?;
            log.line(format!("shader's Diffuse texture: {tex_name}"));
        }
        load_texture(log, &path, &tex_name)
    });
    let texture = texture.unwrap_or_else(|e| {
        log.line(format!("no texture, drawing the model grey: {e}"));
        LoadedTexture {
            name: "grey".into(),
            image: Rgba {
                width: 1,
                height: 1,
                pixels: vec![160, 160, 160, 255],
            },
        }
    });

    let centre = to_bevy(std::array::from_fn(|k| (b.min[k] + b.max[k]) / 2.0));
    let positions: Vec<[f32; 3]> = mesh
        .vertices
        .iter()
        .map(|v| {
            let p = to_bevy(v.position);
            std::array::from_fn(|k| p[k] - centre[k])
        })
        .collect();
    let normals: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| to_bevy(v.normal)).collect();
    let (agree, disagree) = winding_agreement(&positions, &normals, &mesh.indices);
    log.line(format!(
        "triangle facing: {agree} of {} triangles face the same way as their stored normals after the axis swap ({disagree} don't)",
        agree + disagree
    ));
    Ok(LoadedMesh {
        name,
        positions,
        normals,
        uvs: mesh.uv_sets[0].clone(),
        indices: mesh.indices,
        extent: to_bevy(size),
        texture,
    })
}

/// How many pixels one world unit covers in a capture of this size. Matches
/// the camera's `AutoMin` scaling: the model's box plus a margin always fits.
fn pixels_per_unit(loaded: &LoadedMesh, w: f32, h: f32) -> f32 {
    let (min_w, min_h) = (loaded.extent[0] / FILL, loaded.extent[1] / FILL);
    (w / min_w).min(h / min_h)
}

pub(crate) fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    loaded: Res<LoadedMesh>,
    log: Res<RunLog>,
) {
    let img = &loaded.texture.image;
    let image = images.add(Image::new(
        Extent3d {
            width: img.width as u32,
            height: img.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        img.pixels.clone(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ));
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, loaded.positions.clone())
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, loaded.normals.clone())
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, loaded.uvs.clone())
    .with_inserted_indices(Indices::U16(loaded.indices.clone()));
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(image),
        unlit: true,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(material),
        Transform::IDENTITY,
        Model,
    ));
    // Camera in front of the model (Bevy +z), looking at its centre. That
    // shows the side of the gun: Unreal's x (its length) across, z up.
    let depth = loaded.extent.iter().fold(0.0f32, |a, &b| a.max(b));
    commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::AutoMin {
                min_width: loaded.extent[0] / FILL,
                min_height: loaded.extent[1] / FILL,
            },
            near: 0.0,
            far: depth * 4.0,
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(0.0, 0.0, depth * 2.0).looking_at(Vec3::ZERO, Vec3::Y),
        // No colour adjustments and no edge smoothing, so screen pixels are
        // exactly the texture's colours and the check can compare them.
        Tonemapping::None,
        Msaa::Off,
    ));
    let ppu = pixels_per_unit(&loaded, WINDOW_SIZE as f32, WINDOW_SIZE as f32);
    log.line(format!(
        "scene set up: orthographic camera + mesh {} with texture {}, about {:.1} pixels per Unreal unit, model about {:.0}x{:.0} pixels on screen",
        loaded.name,
        loaded.texture.name,
        ppu,
        loaded.extent[0] * ppu,
        loaded.extent[1] * ppu
    ));
}

/// Once the on-screen check is done, turn the model slowly so you can see
/// all of it. It stays still before that, so the check sees a known view.
pub(crate) fn spin(
    time: Res<Time>,
    check: Res<ScreenCheck>,
    mut models: Query<&mut Transform, With<Model>>,
) {
    if check.passed.is_none() {
        return;
    }
    let angle = time.delta_secs() * SPIN_PER_MINUTE / 60.0 * std::f32::consts::TAU;
    for mut t in &mut models {
        t.rotate_y(angle);
    }
}

/// Our own drawing of the model, one entry per screen pixel: the texture
/// coordinate seen there, or `None` for background.
fn reference_uvs(loaded: &LoadedMesh, w: usize, h: usize) -> Vec<Option<[f32; 2]>> {
    let ppu = pixels_per_unit(loaded, w as f32, h as f32);
    let screen: Vec<[f32; 3]> = loaded
        .positions
        .iter()
        .map(|p| {
            [
                w as f32 / 2.0 + p[0] * ppu,
                h as f32 / 2.0 - p[1] * ppu,
                p[2],
            ]
        })
        .collect();
    let mut out = vec![None; w * h];
    let mut depth = vec![f32::NEG_INFINITY; w * h];
    for t in loaded.indices.as_chunks::<3>().0 {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| i as usize);
        let (pa, pb, pc) = (screen[a], screen[b], screen[c]);
        // Signed area on screen. Screen y points down, so a triangle that is
        // counter-clockwise in the world (facing the camera) is negative here.
        let area = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pb[1] - pa[1]) * (pc[0] - pa[0]);
        if area >= 0.0 {
            continue; // facing away; Bevy skips these too
        }
        let x0 = pa[0].min(pb[0]).min(pc[0]).floor().max(0.0) as usize;
        let x1 = (pa[0].max(pb[0]).max(pc[0]).ceil() as usize).min(w);
        let y0 = pa[1].min(pb[1]).min(pc[1]).floor().max(0.0) as usize;
        let y1 = (pa[1].max(pb[1]).max(pc[1]).ceil() as usize).min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                // How much of each corner this pixel centre has (barycentric
                // weights); all three are 0 or more inside the triangle.
                let wa = ((pb[0] - px) * (pc[1] - py) - (pb[1] - py) * (pc[0] - px)) / area;
                let wb = ((pc[0] - px) * (pa[1] - py) - (pc[1] - py) * (pa[0] - px)) / area;
                let wc = 1.0 - wa - wb;
                if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                    continue;
                }
                let z = wa * pa[2] + wb * pb[2] + wc * pc[2];
                let i = y * w + x;
                if z > depth[i] {
                    depth[i] = z;
                    let (ua, ub, uc) = (loaded.uvs[a], loaded.uvs[b], loaded.uvs[c]);
                    out[i] = Some(std::array::from_fn(|k| {
                        wa * ua[k] + wb * ub[k] + wc * uc[k]
                    }));
                }
            }
        }
    }
    out
}

/// Texture colour at a texture coordinate, blending the four nearest pixels
/// as the graphics card does.
fn sample(tex: &Rgba, uv: [f32; 2]) -> [f32; 3] {
    let fx = (uv[0] * tex.width as f32 - 0.5).clamp(0.0, (tex.width - 1) as f32);
    let fy = (uv[1] * tex.height as f32 - 0.5).clamp(0.0, (tex.height - 1) as f32);
    let (x0, y0) = (fx as usize, fy as usize);
    let (x1, y1) = ((x0 + 1).min(tex.width - 1), (y0 + 1).min(tex.height - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let px = |x: usize, y: usize, c: usize| tex.pixels[(y * tex.width + x) * 4 + c] as f32;
    std::array::from_fn(|c| {
        let top = px(x0, y0, c) * (1.0 - tx) + px(x1, y0, c) * tx;
        let bottom = px(x0, y1, c) * (1.0 - tx) + px(x1, y1, c) * tx;
        top * (1.0 - ty) + bottom * ty
    })
}

/// Compares the captured window with our own drawing of the model:
/// 1. Shape: which pixels show the model at all. Measured as overlap divided
///    by the area either drawing covers (1.0 means identical outlines).
/// 2. Colour: on pixels well inside the model, the average difference from
///    the texture colour our drawing expects there.
///
/// For comparison it also scores the texture flipped upside down, which
/// should score clearly worse if the check means anything.
pub(crate) fn check_screen(
    captured: On<ScreenshotCaptured>,
    loaded: Res<LoadedMesh>,
    log: Res<RunLog>,
    frames: Res<FrameStats>,
    mut check: ResMut<ScreenCheck>,
) {
    let Some(Capture {
        w,
        h,
        pixels,
        format,
    }) = read_capture(&captured.image, &log, &frames, &mut check)
    else {
        return;
    };
    let t = Instant::now();
    let reference = reference_uvs(&loaded, w, h);
    let background = crate::BACKGROUND;
    let on_screen: Vec<bool> = pixels.iter().map(|p| *p != background).collect();
    let (mut both, mut either, mut ours, mut theirs) = (0u64, 0u64, 0u64, 0u64);
    for (r, &s) in reference.iter().zip(&on_screen) {
        let r = r.is_some();
        both += (r && s) as u64;
        either += (r || s) as u64;
        ours += r as u64;
        theirs += s as u64;
    }
    let overlap = both as f64 / either.max(1) as f64;
    // Only background: the window was drawn before the model and its texture
    // reached the graphics card (seen once in the cloud test machine). Try
    // again a little later, like an all-black capture.
    if theirs == 0 && check.attempts < crate::CHECK_ATTEMPTS {
        log.line("on-screen check: only background on screen (model not drawn yet), trying again");
        check.next_frame = Some(frames.frames + crate::CHECK_FRAME);
        return;
    }

    // Colour, on pixels whose eight neighbours are also model pixels in our
    // drawing, so edges (where rounding decides) don't count.
    let tex = &loaded.texture.image;
    let (mut diff, mut flipped_diff, mut count) = (0f64, 0f64, 0u64);
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let Some(uv) = reference[y * w + x] else {
                continue;
            };
            let inside = (0..9).all(|k| reference[(y + k / 3 - 1) * w + x + k % 3 - 1].is_some());
            if !inside {
                continue;
            }
            let s = pixels[y * w + x];
            let expected = sample(tex, uv);
            let flipped = sample(tex, [uv[0], 1.0 - uv[1]]);
            for c in 0..3 {
                diff += (s[c] as f64 - expected[c] as f64).abs();
                flipped_diff += (s[c] as f64 - flipped[c] as f64).abs();
            }
            count += 1;
        }
    }
    let count = count.max(1);
    let avg = diff / (count * 3) as f64;
    let flipped_avg = flipped_diff / (count * 3) as f64;
    log.line(format!(
        "on-screen check: capture {w}x{h} {format:?}, our drawing took {:.1} ms",
        t.elapsed().as_secs_f64() * 1000.0
    ));
    log.line(format!(
        "on-screen check: model pixels on screen {theirs}, in our drawing {ours}, in both {both}; outline match {overlap:.3} (1.0 is identical, over 0.97 passes)"
    ));
    log.line(format!(
        "on-screen check: colour difference on {count} inner pixels {avg:.2} of 255 per channel (under 10 passes); with the texture flipped upside down it would be {flipped_avg:.2}"
    ));
    let pass = overlap > 0.97 && avg < 10.0;
    check.passed = Some(pass);
    log.line(format!(
        "on-screen check {}",
        if pass { "PASS" } else { "FAIL" }
    ));
}
