//! A check that Sev, Fixer and Scorch are drawn with their original
//! models and armour: opens a window, stands the three squad mates side by
//! side (built with the same code the game uses), captures the window, and
//! compares it with what we expect from our own posed points.
//!
//! For every material of every squad mate it checks
//! - the outline: the pixels covered on screen against the triangles we
//!   projected ourselves ("intersection over union", 1.0 is perfect);
//! - the colour: the balance of red, green and blue on screen against the
//!   material's picture sampled where the triangles are. Lighting changes
//!   brightness, not balance. A missing or wrong picture, or a renderer
//!   that lost its textures (everything magenta), fails here even when the
//!   outline is right.
//!
//! It also plays every animation in data/squad/looks.toml on the CPU and
//! checks the model stays in one piece (height in range).
//!
//! Run from the project folder, with RC_GAME_DIR set to the game:
//!   cargo run -p rc_squad_plugin --example rc_squad_look_check
//! Ends with "LOOK CHECK DONE: N checks passed, 0 failed". Log:
//! logs/rc_squad_look_check.log.

use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::WindowResolution;
use rc_game_core::GamePaths;
use rc_model::Model;
use rc_squad_plugin::look::{self, LooksFile, MateModel, Pose};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";
const LOG_PATH: &str = "logs/rc_squad_look_check.log";
const KEYS: [&str; 3] = ["sev", "fixer", "scorch"];
/// Background colour, 0-255 sRGB (blue-grey, unlike the armour).
const BACKGROUND: [u8; 3] = [40, 60, 90];
const BACKGROUND_TOLERANCE: i32 = 12;
const PASS_OUTLINE: f64 = 0.93;
/// Largest allowed difference in colour balance (each channel's share of
/// the total, 0 to 1). A white armour drawn magenta differs by about 0.3.
const MAX_HUE_DIFFERENCE: f64 = 0.04;
/// Materials covering fewer pixels than this are too small to judge.
const MIN_PIXELS: usize = 400;
const WAIT_FRAMES: u32 = 30;
const ATTEMPTS: u32 = 5;
const WINDOW: (u32, u32) = (1200, 700);

#[derive(Resource)]
struct Log(Mutex<std::fs::File>);

impl Log {
    fn line(&self, s: impl AsRef<str>) {
        println!("{}", s.as_ref());
        let mut f = self.0.lock().unwrap();
        let _ = writeln!(f, "{}", s.as_ref());
    }
}

#[derive(Resource, Default)]
struct Results {
    passed: u32,
    failed: u32,
}

impl Results {
    fn check(&mut self, log: &Log, ok: bool, what: String) {
        if ok {
            self.passed += 1;
        } else {
            self.failed += 1;
        }
        log.line(format!("{} {what}", if ok { "PASS" } else { "FAIL" }));
    }
}

#[derive(Resource)]
struct Models(Vec<(String, Arc<Model>)>, f32);

#[derive(Component)]
struct Mate(usize);

#[derive(Resource, Default)]
struct Run {
    frames: u32,
    attempts: u32,
    waiting: bool,
    done: bool,
    /// Per pixel: which (mate, material) is in front, and how far away.
    expected: Option<Vec<Option<(usize, usize)>>>,
}

fn main() -> AppExit {
    let game = std::env::var_os("RC_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_DIR));
    std::fs::create_dir_all("logs").expect("can't create the logs folder");
    let log = Log(Mutex::new(
        std::fs::File::create(LOG_PATH).expect("can't create the log"),
    ));
    log.line(format!("game folder: {}", game.display()));
    let paths = GamePaths::new(&game, vec![PathBuf::from("data")]);
    let mut results = Results::default();
    let looks = match LooksFile::read(Some(&paths)) {
        Ok((l, f)) => {
            log.line(format!("looks from {}", f.display()));
            l
        }
        Err(e) => {
            log.line(format!("FAIL {e}"));
            return AppExit::from_code(1);
        }
    };
    let mut models = Vec::new();
    for key in KEYS {
        let t = std::time::Instant::now();
        match look::load_model(&paths, &looks, key) {
            Ok((m, notes)) => {
                for n in notes {
                    log.line(format!("{key}: {n}"));
                }
                results.check(
                    &log,
                    true,
                    format!(
                        "{key}: model {} read in {:.0} ms",
                        m.name,
                        t.elapsed().as_secs_f64() * 1000.0
                    ),
                );
                cpu_checks(&log, &mut results, key, &m);
                skins_check(&log, &mut results, &paths, &looks, key);
                models.push((key.to_string(), Arc::new(m)));
            }
            Err(e) => results.check(&log, false, format!("{key}: model not read: {e}")),
        }
    }
    if models.len() != KEYS.len() {
        log.line(format!(
            "LOOK CHECK DONE: {} checks passed, {} failed",
            results.passed, results.failed
        ));
        return AppExit::from_code(1);
    }
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Delta Squad look check".into(),
                resolution: WindowResolution::new(WINDOW.0, WINDOW.1),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb_u8(
            BACKGROUND[0],
            BACKGROUND[1],
            BACKGROUND[2],
        )))
        .insert_resource(Models(models, looks.ambient))
        .insert_resource(log)
        .insert_resource(results)
        .init_resource::<Run>()
        .add_systems(Startup, setup)
        .add_systems(Update, step)
        .run()
}

/// Every animation on the CPU: all found, and the body keeps a sensible
/// height (a broken skeleton stretches it or folds it to nothing).
fn cpu_checks(log: &Log, results: &mut Results, key: &str, m: &Model) {
    let found = Pose::ALL
        .iter()
        .filter(|p| m.clip(p.key()).is_some())
        .count();
    results.check(
        log,
        found == Pose::ALL.len(),
        format!("{key}: {found} of {} animations found", Pose::ALL.len()),
    );
    results.check(
        log,
        m.sections.iter().all(|s| s.skin.is_some()),
        format!(
            "{key}: every material has a picture ({} materials)",
            m.sections.len()
        ),
    );
    for p in Pose::ALL {
        let Some(clip) = m.clip(p.key()) else {
            continue;
        };
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        let mut f = 0.0;
        while f < clip.num_frames {
            let pts = m.posed(p.key(), f);
            let (a, b) = pts
                .iter()
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), q| {
                    (a.min(q[1]), b.max(q[1]))
                });
            lo = lo.min((b - a) * 50.0);
            hi = hi.max((b - a) * 50.0);
            f += 1.0;
        }
        // Standing about 170 uu; lying down well under that. Anything over
        // 260 or under 20 means a broken pose.
        results.check(
            log,
            lo > 20.0 && hi < 260.0,
            format!(
                "{key}: {} ({}, {} frames) height {lo:.0} to {hi:.0} uu",
                p.key(),
                clip.name,
                clip.num_frames
            ),
        );
    }
}

/// The armour each squad mate wears in the original: `Skins` in its class
/// defaults (CommandoNNDeltaDefaults, in the Properties package), as full
/// names like `CloneTextures.CloneTextures.CloneCommando07_Shader`.
fn original_skins(paths: &GamePaths, class: &str) -> Result<Vec<String>, String> {
    use rc_package::class_defaults::{class_defaults, find_class, property_names};
    let read = |rel: &str| -> Result<(rc_package::Package, Vec<u8>), String> {
        let f = paths.game_file(rel);
        let d = std::fs::read(&f).map_err(|e| format!("can't read {}: {e}", f.display()))?;
        let p = rc_package::Package::parse(&d).map_err(|e| e.to_string())?;
        Ok((p, d))
    };
    let (pkg, data) = read("GameData/Properties/properties.u")?;
    let mut names: std::collections::HashSet<String> = property_names(&pkg).into_iter().collect();
    for u in ["core", "engine", "ctgame", "ctcharacters"] {
        let (p, _) = read(&format!("GameData/System/{u}.u"))?;
        names.extend(property_names(&p));
    }
    let wanted = format!("{class}Defaults");
    let export = find_class(&pkg, &wanted).ok_or_else(|| format!("no class {wanted}"))?;
    let defaults = class_defaults(&pkg, &data, export, &|n| {
        names.contains(&n.to_ascii_lowercase())
    })
    .ok_or_else(|| format!("{wanted}: defaults not readable"))?;
    let mut out = Vec::new();
    for p in &defaults.properties {
        if !pkg.name(p.name).eq_ignore_ascii_case("Skins") {
            continue;
        }
        // A dynamic array: how many, then each object reference.
        let rc_package::properties::Value::Raw(bytes) = &p.value else {
            return Err("Skins isn't stored as a list".into());
        };
        let mut r = rc_package::Reader::new(bytes);
        let n = r.compact_index().map_err(|e| e.to_string())?;
        for _ in 0..n {
            let i = r.compact_index().map_err(|e| e.to_string())?;
            out.push(rc_package::class::object_path(&pkg, i));
        }
    }
    Ok(out)
}

/// looks.toml must dress each squad mate as the original does.
fn skins_check(log: &Log, results: &mut Results, paths: &GamePaths, looks: &LooksFile, key: &str) {
    let class = rc_squad::SquadSettings::default()
        .members
        .iter()
        .find(|m| m.key == key)
        .map(|m| m.class.clone())
        .unwrap_or_default();
    let ours: Vec<String> = looks
        .member
        .get(key)
        .map(|m| m.skins.clone())
        .unwrap_or_default();
    match original_skins(paths, &class) {
        Ok(orig) => results.check(
            log,
            orig.iter()
                .zip(ours.iter().chain(std::iter::repeat(&String::new())))
                .all(|(o, us)| o.eq_ignore_ascii_case(us)),
            format!("{key}: armour {ours:?} is the original's ({class}Defaults Skins {orig:?})"),
        ),
        Err(e) => results.check(log, false, format!("{key}: original armour not read: {e}")),
    }
}

fn setup(
    mut commands: Commands,
    models: Res<Models>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    for (i, (_, model)) in models.0.iter().enumerate() {
        let x = (i as f32 - 1.0) * 2.6;
        // Facing the camera: the body's front is its local -Z (the game's
        // forward), the camera is on +Z... turn half way round.
        let mut e = commands.spawn((
            Mate(i),
            Transform::from_xyz(x, 0.0, 0.0)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
            Visibility::default(),
        ));
        let mm = look::spawn_pieces(
            model.clone(),
            models.1,
            &mut e,
            &mut meshes,
            &mut materials,
            &mut images,
        );
        e.insert(mm);
    }
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 2.0, 8.0).looking_at(Vec3::new(0.0, 1.7, 0.0), Vec3::Y),
        // As in rc_character: tonemapping needs lookup tables this build
        // leaves out (everything magenta otherwise).
        Tonemapping::None,
        DebandDither::Disabled,
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 6000.0,
            ..default()
        },
        Transform::from_xyz(2.0, 6.0, 6.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
    ));
}

/// Fill one triangle into the label grid, nearest wins.
#[allow(clippy::too_many_arguments)]
fn fill(
    labels: &mut [Option<(usize, usize)>],
    depth: &mut [f32],
    w: usize,
    h: usize,
    p: [Vec3; 3],
    label: (usize, usize),
) {
    let [a, b, c] = p.map(|v| v.truncate());
    let lo = a.min(b).min(c).max(Vec2::ZERO);
    let hi = a
        .max(b)
        .max(c)
        .min(Vec2::new(w as f32 - 1.0, h as f32 - 1.0));
    let area = (b - a).perp_dot(c - a);
    if area.abs() < 1e-6 || lo.x > hi.x || lo.y > hi.y {
        return;
    }
    for y in lo.y.floor() as usize..=hi.y.ceil() as usize {
        for x in lo.x.floor() as usize..=hi.x.ceil() as usize {
            if x >= w || y >= h {
                continue;
            }
            let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let w0 = (b - q).perp_dot(c - q) / area;
            let w1 = (c - q).perp_dot(a - q) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                let z = w0 * p[0].z + w1 * p[1].z + w2 * p[2].z;
                let i = y * w + x;
                if z < depth[i] {
                    depth[i] = z;
                    labels[i] = Some(label);
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn step(
    mut commands: Commands,
    mut run: ResMut<Run>,
    log: Res<Log>,
    results: ResMut<Results>,
    camera: Query<(&Camera, &GlobalTransform)>,
    mates: Query<(&Mate, &GlobalTransform, &MateModel)>,
    windows: Query<&Window>,
    mut exit: MessageWriter<AppExit>,
) {
    run.frames += 1;
    if run.done {
        log.line(format!(
            "LOOK CHECK DONE: {} checks passed, {} failed",
            results.passed, results.failed
        ));
        exit.write(if results.failed == 0 {
            AppExit::Success
        } else {
            AppExit::from_code(1)
        });
        return;
    }
    if run.waiting || run.frames < WAIT_FRAMES * (run.attempts + 1) {
        return;
    }
    let (Ok((cam, cam_t)), Ok(window)) = (camera.single(), windows.single()) else {
        return;
    };
    // What we expect: every triangle of every material, projected with
    // the camera, nearest in front.
    let (w, h) = (
        window.physical_width() as usize,
        window.physical_height() as usize,
    );
    let scale = window.scale_factor();
    let mut labels = vec![None; w * h];
    let mut depth = vec![f32::INFINITY; w * h];
    for (mate, t, mm) in &mates {
        let m = &mm.model;
        let points = m.posed(mm.pose.key(), mm.frame);
        let projected: Vec<Option<Vec3>> = points
            .iter()
            .map(|p| {
                let world = t.transform_point(look::model_turn() * Vec3::from_array(*p));
                let v = cam.world_to_viewport(cam_t, world).ok()?;
                let z = (world - cam_t.translation()).length();
                Some(Vec3::new(v.x * scale, v.y * scale, z))
            })
            .collect();
        for (si, s) in m.sections.iter().enumerate() {
            for tri in s.indices.chunks_exact(3) {
                let p = [tri[0], tri[1], tri[2]].map(|i| {
                    projected[m.mesh.wedges[s.wedges[i as usize] as usize].point as usize]
                });
                if let [Some(a), Some(b), Some(c)] = p {
                    fill(&mut labels, &mut depth, w, h, [a, b, c], (mate.0, si));
                }
            }
        }
    }
    run.expected = Some(labels);
    run.waiting = true;
    run.attempts += 1;
    log.line(format!(
        "frame {}: capturing the window ({w}x{h}), attempt {} of {ATTEMPTS}",
        run.frames, run.attempts
    ));
    commands
        .spawn(Screenshot::primary_window())
        .observe(check_capture);
}

fn check_capture(
    captured: On<ScreenshotCaptured>,
    mut run: ResMut<Run>,
    models: Res<Models>,
    mates: Query<(&Mate, &MateModel)>,
    log: Res<Log>,
    mut results: ResMut<Results>,
) {
    run.waiting = false;
    let shot = &captured.image;
    let (w, h) = (shot.width() as usize, shot.height() as usize);
    let format = shot.texture_descriptor.format;
    let bgra = matches!(
        format,
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
    );
    let Some(data) = shot.data.as_ref() else {
        return;
    };
    let pixel = |i: usize| -> [u8; 3] {
        let p = &data[i * 4..];
        if bgra {
            [p[2], p[1], p[0]]
        } else {
            [p[0], p[1], p[2]]
        }
    };
    let got = |i: usize| {
        let px = pixel(i);
        (0..3).any(|k| (px[k] as i32 - BACKGROUND[k] as i32).abs() > BACKGROUND_TOLERANCE)
    };
    let drawn = (0..w * h).filter(|&i| got(i)).count();
    let black = (0..w * h).all(|i| pixel(i) == [0, 0, 0]);
    let retry = (black || drawn < 1000) && run.attempts < ATTEMPTS;
    // The capture, for a person to look at (PPM: a plain picture format
    // most image viewers and GIMP open).
    let mut ppm = format!("P6 {w} {h} 255\n").into_bytes();
    for i in 0..w * h {
        ppm.extend_from_slice(&pixel(i));
    }
    let _ = std::fs::write("logs/rc_squad_look_check.ppm", ppm);
    if retry {
        log.line(format!(
            "capture {w}x{h}: {} (not drawn yet), trying again",
            if black {
                "all black".to_string()
            } else {
                format!("only {drawn} non-background pixels")
            }
        ));
        return;
    }
    let Some(labels) = run.expected.take() else {
        return;
    };
    if labels.len() != w * h {
        results.check(
            &log,
            false,
            format!("capture is {w}x{h}, not the window size we projected for"),
        );
        run.done = true;
        return;
    }
    let share = |v: [f64; 3]| {
        let t = (v[0] + v[1] + v[2]).max(1.0);
        v.map(|x| x / t)
    };
    let mut mates_sorted: Vec<_> = mates.iter().collect();
    mates_sorted.sort_by_key(|(m, _)| m.0);
    // Each mate's own screen columns: pixels outside every expected
    // outline still count against the mate whose outline is nearest.
    // Each mate's whole-body colour balance on screen, and what its
    // pictures say it should be (area-weighted over its materials).
    let mut measured: Vec<[f64; 3]> = Vec::new();
    let mut wanted: Vec<[f64; 3]> = Vec::new();
    for (mate, mm) in &mates_sorted {
        let key = &models.0[mate.0].0;
        let m = &mm.model;
        let mut cols: Vec<usize> = (0..w * h)
            .filter(|&i| labels[i].is_some_and(|l| l.0 == mate.0))
            .map(|i| i % w)
            .collect();
        cols.sort_unstable();
        let (Some(&x0), Some(&x1)) = (cols.first(), cols.last()) else {
            results.check(&log, false, format!("{key}: nothing projected on screen"));
            continue;
        };
        let (x0, x1) = (x0.saturating_sub(20), (x1 + 20).min(w - 1));
        let (mut both, mut either) = (0usize, 0usize);
        for y in 0..h {
            for x in x0..=x1 {
                let i = y * w + x;
                let want = labels[i].is_some_and(|l| l.0 == mate.0);
                let other = labels[i].is_some_and(|l| l.0 != mate.0);
                if other {
                    continue;
                }
                let g = got(i);
                both += (want && g) as usize;
                either += (want || g) as usize;
            }
        }
        let iou = both as f64 / either.max(1) as f64;
        results.check(
            &log,
            iou >= PASS_OUTLINE,
            format!("{key}: outline match {iou:.3} ({PASS_OUTLINE} or more passes)"),
        );
        let points = m.posed(mm.pose.key(), mm.frame);
        let mut all = [0f64; 3];
        let mut all_n = 0usize;
        for (si, s) in m.sections.iter().enumerate() {
            let mut sum = [0f64; 3];
            let mut n = 0usize;
            for i in 0..w * h {
                if labels[i] == Some((mate.0, si)) && got(i) {
                    let px = pixel(i);
                    for k in 0..3 {
                        sum[k] += px[k] as f64;
                    }
                    n += 1;
                }
            }
            for k in 0..3 {
                all[k] += sum[k];
            }
            all_n += n;
            let mean = sum.map(|c| c / n.max(1) as f64);
            let Some(want) = m.expected_colour(s, &points) else {
                results.check(
                    &log,
                    false,
                    format!("{key}: material {} has no picture", s.material),
                );
                continue;
            };
            if n < MIN_PIXELS {
                log.line(format!(
                    "{key}: material {} covers only {n} pixels, colour not judged",
                    s.material
                ));
                continue;
            }
            let (a, b) = (share(mean), share(want));
            let diff = (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0, f64::max);
            results.check(
                &log,
                diff <= MAX_HUE_DIFFERENCE,
                format!(
                    "{key}: material {} colour on screen R {:.0} G {:.0} B {:.0} over {n} pixels, picture R {:.0} G {:.0} B {:.0}; balance differs by {diff:.3} ({MAX_HUE_DIFFERENCE} or less passes)",
                    s.material, mean[0], mean[1], mean[2], want[0], want[1], want[2]
                ),
            );
        }
        measured.push(share(all.map(|c| c / all_n.max(1) as f64)));
        let mut want = [0f64; 3];
        for s in &m.sections {
            if let Some(c) = m.expected_colour(s, &points) {
                let weight = s.indices.len() as f64;
                for k in 0..3 {
                    want[k] += c[k] * weight;
                }
            }
        }
        wanted.push(share(want));
    }
    // Who is who: each mate's colour on screen must be nearest its own
    // armour's, not a squad mate's (Sev's red, Fixer's green, Scorch's
    // yellow markings shift the balance only a little, so this also
    // catches two mates swapping skins).
    let diff = |a: [f64; 3], b: [f64; 3]| (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0, f64::max);
    for (i, got) in measured.iter().enumerate() {
        let own = diff(*got, wanted[i]);
        let nearest_other = wanted
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, w)| diff(*got, *w))
            .fold(f64::INFINITY, f64::min);
        results.check(
            &log,
            own < nearest_other,
            format!(
                "{}: on screen it matches its own armour best (difference {own:.4}, nearest other squad mate's {nearest_other:.4})",
                models.0[i].0
            ),
        );
    }
    run.done = true;
}
