//! rc_soundtest: plays one weapon sound and one ambient loop from the
//! installed game through Bevy's audio, and logs numbers that prove it.
//!
//! Usage:
//!   rc_soundtest [package] [--weapon NAME] [--ambient NAME] [--seconds N]
//!
//! Defaults: `wep_GEN_dc17Fire_02` (the DC-17 rifle firing) and
//! `amb_KAM_siege_lp_01` (a looping battle ambience), both from
//! `GameData/Sounds/gen_sound.uax` in the game folder (RC_GAME_DIR or the
//! default Steam path). The ambience loops the whole time; the rifle fires
//! every two seconds. The program closes itself after `--seconds` (default 8).
//!
//! It has no window. Everything is logged to `logs/rc_soundtest.log`:
//! 1. Each sound's numbers from our own decoder (`rc_audio`): sample rate,
//!    channels, length, peak, loudness, checksum.
//! 2. The same sound decoded by Bevy's own decoder, compared sample by
//!    sample with ours. They must match.
//! 3. While running: when each sound actually starts on the sound card, and
//!    how far the ambience has played. If there is no sound card, it says so.

use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::asset::AssetPlugin;
use bevy::audio::{AudioPlugin, Decodable, Source};
use bevy::prelude::*;
use rc_audio::{Stats, Wav};
use rc_package::{sound, Package};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const DEFAULT_GAME_DIR: &str = r"C:\Games\Steam\steamapps\common\Star Wars Republic Commando";
const DEFAULT_WEAPON: &str = "wep_GEN_dc17Fire_02";
const DEFAULT_AMBIENT: &str = "amb_KAM_siege_lp_01";
const LOG_PATH: &str = "logs/rc_soundtest.log";
/// Seconds between rifle shots, and before the first one.
const SHOT_INTERVAL: f64 = 2.0;
const FIRST_SHOT: f64 = 0.5;
/// Bevy's decoder gives samples as decimals from -1 to 1; ours are whole
/// numbers from -32768 to 32767. After scaling, they may differ by at most
/// this much (rounding only).
const MAX_SAMPLE_DIFFERENCE: f64 = 1.0;

/// Our own log file, shared with Bevy's systems.
#[derive(Resource)]
struct RunLog {
    file: Mutex<fs::File>,
    start: Instant,
}

impl RunLog {
    fn line(&self, s: impl AsRef<str>) {
        let s = format!(
            "[{:>8.3}s] {}",
            self.start.elapsed().as_secs_f64(),
            s.as_ref()
        );
        println!("{s}");
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "{s}");
        }
    }
}

/// A sound read from the package, ready to hand to Bevy.
struct LoadedSound {
    name: String,
    wav: Wav,
    bytes: Vec<u8>,
}

#[derive(Resource)]
struct Sounds {
    weapon: Handle<AudioSource>,
    weapon_seconds: f64,
    ambient_seconds: f64,
}

#[derive(Component)]
struct Ambient;

#[derive(Component)]
struct Shot(u32);

#[derive(Resource)]
struct RunState {
    started: Instant,
    seconds: f64,
    shots_fired: u32,
    shots_started: u32,
    ambient_started: Option<f64>,
    /// Latest (time, playback position) of the ambience, in seconds.
    ambient_position: Option<(f64, f64)>,
    next_report: f64,
    numbers_ok: bool,
}

fn game_dir() -> PathBuf {
    std::env::var_os("RC_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_DIR))
}

fn find_sound(pkg: &Package, data: &[u8], wanted: &str) -> Result<LoadedSound, String> {
    let export = pkg
        .exports
        .iter()
        .find(|e| {
            pkg.export_class_name(e) == "Sound"
                && pkg.name(e.object_name).eq_ignore_ascii_case(wanted)
        })
        .ok_or_else(|| format!("no sound called {wanted} in this package"))?;
    let s = sound::read_sound(pkg, data, export).map_err(|e| e.to_string())?;
    let problems = s.check();
    if !problems.is_empty() {
        return Err(format!("{wanted}: {}", problems.join("; ")));
    }
    let bytes = s.bytes(data).to_vec();
    let wav = Wav::parse(&bytes).map_err(|e| format!("{wanted}: {e}"))?;
    Ok(LoadedSound {
        name: pkg.name(export.object_name).to_string(),
        wav,
        bytes,
    })
}

fn log_numbers(log: &RunLog, what: &str, s: &LoadedSound) {
    let w = &s.wav;
    let st = w.stats();
    log.line(format!(
        "{what} {}: {} Hz, {} channel(s), {} bit, {} frames, {:.3} s, peak {:.3} ({:.1} dB), rms {:.1} dB, checksum {:016x}",
        s.name,
        w.sample_rate,
        w.channels,
        w.bits_per_sample,
        w.frames(),
        w.seconds(),
        st.peak,
        st.peak_db(),
        st.rms_db(),
        st.checksum
    ));
}

/// Decode the sound again with Bevy's own decoder and compare every sample
/// with ours. Returns true if they match.
fn cross_check(log: &RunLog, s: &LoadedSound) -> bool {
    let source = AudioSource {
        bytes: s.bytes.clone().into(),
    };
    let decoder = source.decoder();
    let rate = decoder.sample_rate().get();
    let channels = decoder.channels().get();
    let theirs: Vec<f64> = decoder.map(|x| x as f64).collect();
    let mut max_diff = 0f64;
    for (a, b) in s.wav.samples.iter().zip(&theirs) {
        max_diff = max_diff.max((*a as f64 - b * 32768.0).abs());
    }
    let as_i16: Vec<i16> = theirs
        .iter()
        .map(|x| (x * 32768.0).round().clamp(-32768.0, 32767.0) as i16)
        .collect();
    let st = Stats::of(&as_i16);
    let ok = rate == s.wav.sample_rate
        && channels == s.wav.channels
        && theirs.len() == s.wav.samples.len()
        && max_diff <= MAX_SAMPLE_DIFFERENCE;
    log.line(format!(
        "  Bevy's decoder: {rate} Hz, {channels} channel(s), {} samples (ours {}), largest difference {max_diff:.3}, peak {:.3}, checksum {:016x}  -> {}",
        theirs.len(),
        s.wav.samples.len(),
        st.peak,
        st.checksum,
        if ok { "MATCH" } else { "MISMATCH" }
    ));
    ok
}

fn main() {
    let game = game_dir();
    let mut package = game.join("GameData").join("Sounds").join("gen_sound.uax");
    let mut weapon = DEFAULT_WEAPON.to_string();
    let mut ambient = DEFAULT_AMBIENT.to_string();
    let mut seconds = 8.0f64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--weapon" => weapon = args.next().expect("--weapon needs a sound name"),
            "--ambient" => ambient = args.next().expect("--ambient needs a sound name"),
            "--seconds" => {
                seconds = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .expect("--seconds needs a number")
            }
            _ => package = PathBuf::from(a),
        }
    }

    fs::create_dir_all("logs").expect("can't create the logs folder");
    let log = RunLog {
        file: Mutex::new(fs::File::create(LOG_PATH).expect("can't create the log file")),
        start: Instant::now(),
    };
    log.line(format!("rc_soundtest {}", env!("CARGO_PKG_VERSION")));
    log.line(format!("package: {}", package.display()));

    let t = Instant::now();
    let loaded = fs::read(&package)
        .map_err(|e| format!("can't read {}: {e}", package.display()))
        .and_then(|data| {
            let pkg = Package::parse(&data).map_err(|e| e.to_string())?;
            log.line(format!(
                "package read: {} bytes, {} exports, {:.1} ms",
                data.len(),
                pkg.exports.len(),
                t.elapsed().as_secs_f64() * 1000.0
            ));
            Ok((
                find_sound(&pkg, &data, &weapon)?,
                find_sound(&pkg, &data, &ambient)?,
            ))
        });
    let (weapon, ambient) = match loaded {
        Ok(v) => v,
        Err(e) => {
            log.line(format!("FAILED: {e}"));
            std::process::exit(1);
        }
    };

    log.line("--- our decoder vs. Bevy's decoder ---");
    log_numbers(&log, "weapon ", &weapon);
    let weapon_ok = cross_check(&log, &weapon);
    log_numbers(&log, "ambient", &ambient);
    let ambient_ok = cross_check(&log, &ambient);

    log.line(format!(
        "--- playing for {seconds} s: ambience on loop, rifle every {SHOT_INTERVAL} s ---"
    ));
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1.0 / 60.0,
        ))),
        AssetPlugin::default(),
        AudioPlugin::default(),
    ))
    .insert_resource(RunState {
        started: Instant::now(),
        seconds,
        shots_fired: 0,
        shots_started: 0,
        ambient_started: None,
        ambient_position: None,
        next_report: 1.0,
        numbers_ok: weapon_ok && ambient_ok,
    })
    .insert_resource(PendingSounds(Mutex::new(Some((weapon, ambient)))))
    .insert_resource(log)
    .add_systems(Startup, start_ambient)
    .add_systems(Update, (fire_weapon, watch_playback, finish).chain());
    let exit = app.run();
    std::process::exit(if exit.is_success() { 0 } else { 1 });
}

/// The decoded sounds, handed from `main` to the startup system.
#[derive(Resource)]
struct PendingSounds(Mutex<Option<(LoadedSound, LoadedSound)>>);

fn start_ambient(
    mut commands: Commands,
    pending: Res<PendingSounds>,
    mut sources: ResMut<Assets<AudioSource>>,
    log: Res<RunLog>,
) {
    let (weapon, ambient) = pending.0.lock().unwrap().take().unwrap();
    let ambient_handle = sources.add(AudioSource {
        bytes: ambient.bytes.into(),
    });
    let weapon_handle = sources.add(AudioSource {
        bytes: weapon.bytes.into(),
    });
    commands.spawn((Ambient, AudioPlayer(ambient_handle), PlaybackSettings::LOOP));
    log.line(format!("ambient {} queued (looping)", ambient.name));
    commands.insert_resource(Sounds {
        weapon: weapon_handle,
        weapon_seconds: weapon.wav.seconds(),
        ambient_seconds: ambient.wav.seconds(),
    });
}

fn fire_weapon(
    mut commands: Commands,
    sounds: Res<Sounds>,
    mut state: ResMut<RunState>,
    log: Res<RunLog>,
) {
    let t = state.started.elapsed().as_secs_f64();
    let due = FIRST_SHOT + state.shots_fired as f64 * SHOT_INTERVAL;
    if t >= due && due + sounds.weapon_seconds <= state.seconds {
        state.shots_fired += 1;
        commands.spawn((
            Shot(state.shots_fired),
            AudioPlayer(sounds.weapon.clone()),
            PlaybackSettings::DESPAWN,
        ));
        log.line(format!(
            "shot {} queued ({t:.3} s after playback began)",
            state.shots_fired
        ));
    }
}

fn watch_playback(
    new_shots: Query<&Shot, Added<AudioSink>>,
    new_ambient: Query<(), (With<Ambient>, Added<AudioSink>)>,
    ambient: Query<&AudioSink, With<Ambient>>,
    sounds: Res<Sounds>,
    mut state: ResMut<RunState>,
    log: Res<RunLog>,
) {
    let t = state.started.elapsed().as_secs_f64();
    for shot in &new_shots {
        state.shots_started += 1;
        log.line(format!(
            "shot {} started on the sound card ({t:.3} s)",
            shot.0
        ));
    }
    if !new_ambient.is_empty() {
        state.ambient_started = Some(t);
        log.line(format!("ambience started on the sound card ({t:.3} s)"));
    }
    if t >= state.next_report {
        state.next_report += 1.0;
        match ambient.single() {
            Ok(sink) => {
                let position = sink.position().as_secs_f64();
                state.ambient_position = Some((t, position));
                log.line(format!(
                    "ambience playing: position {position:.3} s (the sound is {:.3} s long and loops), paused {}",
                    sounds.ambient_seconds,
                    sink.is_paused()
                ))
            }
            Err(_) => log.line("ambience is not playing (no sound card found?)"),
        }
    }
}

fn finish(state: Res<RunState>, log: Res<RunLog>, mut exit: MessageWriter<AppExit>) {
    if state.started.elapsed().as_secs_f64() < state.seconds {
        return;
    }
    log.line("--- result ---");
    log.line(format!(
        "numbers check (our decoder matches Bevy's): {}",
        if state.numbers_ok { "PASS" } else { "FAIL" }
    ));
    let played = state.ambient_started.is_some() && state.shots_started == state.shots_fired;
    log.line(format!(
        "playback: ambience started {}, shots queued {}, shots started {} -> {}",
        state.ambient_started.is_some(),
        state.shots_fired,
        state.shots_started,
        if played {
            "PLAYED"
        } else {
            "NOT PLAYED (Bevy found no sound card)"
        }
    ));
    // On a real sound card the ambience plays in real time: its position
    // grows by about one second per second. A fake ("null") sound card, as in
    // the cloud test machine, swallows the sound as fast as it can.
    if let (Some(start), Some((t, position))) = (state.ambient_started, state.ambient_position) {
        if t > start {
            log.line(format!(
                "ambience speed: {:.2}x real time (1.00 expected on a real sound card)",
                position / (t - start)
            ));
        }
    }
    log.line(format!("log written to {LOG_PATH}"));
    exit.write(if state.numbers_ok && played {
        AppExit::Success
    } else {
        AppExit::error()
    });
}
