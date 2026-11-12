//! The game's music, as its own packages describe it.
//!
//! Every piece of music is a `SoundStreamed` object in one of the
//! `GameData/Sounds/*_music.uax` packages (`geo_music`, `ras_music`,
//! `yyy_music`, `gen_music`, `ui_music`). Each names an Ogg Vorbis file in
//! `GameData/Music` and says how it fades in and out, whether it loops, and
//! (through its `Params`, kept in `params_mus.uax`) how loud it plays.
//!
//! Pieces are grouped with `SoundMultiple` objects:
//! - overlay groups (`GEO_03A_eggroom_ols`): short pieces played now and
//!   then over the room music, with an optional `FirstSound` to start with;
//! - battle groups (`GEO_battle02multi`): battle loops to pick from, with a
//!   `BattleWonMusic` flourish for the end of the fight and a
//!   `BattleLostMusic` sting for the player's death;
//! - "megamulti" groups (`GEO_battle_megamulti`): battle groups to pick
//!   from, whose own `BattleLostMusic` is the music while the player is
//!   down (`musGEN_playerincap_lp`).
//!
//! Groups can hold other groups. Members are picked at random, weighted by
//! their `Likelihood` (1 by default), as the engine's `SoundMultiple` does.
//!
//! Room music lives in `AudioEnvironment` classes (`GameData/Properties/
//! audioenv.u`): `iMuseState` (the loop), `iMuseOverlays` (an overlay
//! group), `OverlayStartupDelay`, `OverlayMinSpacing`, `OverlayMaxSpacing`
//! (seconds) and `bDoMusicOnceOnly`. Volumes in a map name one in their
//! `AudioClass`.

use rc_package::class_library::ClassLibrary;
use rc_package::properties::{self, Property, Value};
use rc_package::Package;
use std::collections::HashMap;
use std::path::Path;

/// How a fade's loudness changes over its time (the engine's
/// `EMusicFadeType`: `MFT_Linear`, `MFT_Sine`, `MFT_Log`, `MFT_LogInverse`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FadeShape {
    #[default]
    Linear,
    Sine,
    Log,
    LogInverse,
}

impl FadeShape {
    pub fn from_byte(b: u8) -> FadeShape {
        match b {
            1 => FadeShape::Sine,
            2 => FadeShape::Log,
            3 => FadeShape::LogInverse,
            _ => FadeShape::Linear,
        }
    }

    /// The engine's curve from a fader position to a loudness (Engine.dll
    /// 0x103d1dc0, which calls 0x103d1b70, 0x103d1c50 and 0x103d1ca0).
    /// Positions at or past the ends give 0 or 1 (except linear, which
    /// passes the position on unchanged).
    pub fn curve(self, x: f32) -> f32 {
        match self {
            FadeShape::Linear => x,
            _ if x >= 1.0 => 1.0,
            _ if x <= 0.0 => 0.0,
            // sin(x * pi/2)
            FadeShape::Sine => (x * std::f32::consts::FRAC_PI_2).sin(),
            // 10 ^ ((x * 100 - 100) * 0.05): a fade of 100 decibels.
            FadeShape::Log => db_curve(x),
            // 2x - the "log" curve; it goes over 1 past x = 0.5, and the
            // fader then holds it at its target (see [`Fader`]).
            FadeShape::LogInverse => 2.0 * x - db_curve(x),
        }
    }

    /// The engine's way back from a loudness to a fader position, used
    /// when a fade starts (Engine.dll 0x103d1cf0, which calls 0x103d1b30,
    /// 0x103d1bb0 and 0x103d1c00). For "sine" it is the true inverse
    /// (`asin(v) * 2/pi`); for "log" and "inverse log" the engine calls the
    /// same functions as [`FadeShape::curve`], so a fade between 0 and 1
    /// runs the position from 0 to 1 (or 1 to 0).
    pub fn position(self, v: f32) -> f32 {
        match self {
            FadeShape::Linear => v,
            _ if v >= 1.0 => 1.0,
            _ if v <= 0.0 => 0.0,
            FadeShape::Sine => v.asin() * std::f32::consts::FRAC_2_PI,
            FadeShape::Log => db_curve(v),
            FadeShape::LogInverse => 2.0 * v - db_curve(v),
        }
    }

    /// Loudness a fraction `t` (0 to 1) of the way into a fade in from
    /// silence to full: what a [`Fader`] gives.
    pub fn gain(self, t: f32) -> f32 {
        let mut f = Fader::at(0.0);
        f.start(1.0, 1.0, self);
        f.advance(t.clamp(0.0, 1.0));
        f.value()
    }
}

fn db_curve(x: f32) -> f32 {
    10f32.powf((x * 100.0 - 100.0) * 0.05)
}

/// The engine's music fader (Engine.dll: set up at 0x103d1cf0, moved on and
/// read at 0x103d2851 and 0x103d1dc0). A fade moves a position at a steady
/// rate from `position(current)` to `position(target)` over its time; the
/// loudness is `curve(position)`, never past the target on the side it is
/// heading (going up it stops at the target; going down it stops at it).
/// Pieces start at 0 and fade to 1; their own loudness multiplies this.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fader {
    pos: f32,
    end: f32,
    rate: f32,
    value: f32,
    target: f32,
    shape: FadeShape,
}

impl Fader {
    /// At `value`, not fading.
    pub fn at(value: f32) -> Fader {
        Fader {
            pos: -1.0,
            end: -1.0,
            rate: 0.0,
            value,
            target: value,
            shape: FadeShape::Linear,
        }
    }

    /// Fade from where it is to `target` over `seconds` (0: at once).
    pub fn start(&mut self, target: f32, seconds: f32, shape: FadeShape) {
        self.target = target;
        self.shape = shape;
        if seconds == 0.0 {
            self.pos = -1.0;
            self.end = -1.0;
            self.rate = 0.0;
            self.value = target;
            return;
        }
        self.end = shape.position(target);
        self.pos = shape.position(self.value);
        self.rate = (self.end - self.pos) / seconds;
    }

    /// Move on by `dt` seconds.
    pub fn advance(&mut self, dt: f32) {
        if self.value == self.target {
            return;
        }
        self.pos += self.rate * dt;
        if self.rate > 0.0 {
            if self.pos > self.end {
                self.pos = self.end;
            }
        } else if self.end > self.pos {
            self.pos = self.end;
        }
        let mut v = self.shape.curve(self.pos);
        if self.rate > 0.0 {
            if v > self.target {
                v = self.target;
            }
        } else if self.target > v {
            v = self.target;
        }
        self.value = v;
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    pub fn target(&self) -> f32 {
        self.target
    }
}

/// A fade: how long, and its shape. Zero seconds means at once.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Fade {
    pub seconds: f32,
    pub shape: FadeShape,
}

impl Fade {
    pub const NONE: Fade = Fade {
        seconds: 0.0,
        shape: FadeShape::Linear,
    };
}

/// One piece of music: a `SoundStreamed`.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    /// Its object name, as maps and scripts write it (`musGEO_battle02_lp`).
    pub name: String,
    /// The file in `GameData/Music`, without `.ogg`.
    pub file: String,
    pub looping: bool,
    pub fade_in: Fade,
    pub fade_out: Fade,
    /// Loudness range from its `Params` (`MinVolume`, `MaxVolume`); each
    /// play picks a value in between. 1 when it has none.
    pub volume: (f32, f32),
    pub likelihood: f32,
}

/// A group to pick from: a `SoundMultiple`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Group {
    pub name: String,
    /// Member names (pieces or other groups).
    pub members: Vec<String>,
    pub first: Option<String>,
    pub won: Option<String>,
    pub lost: Option<String>,
    pub likelihood: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Piece(Piece),
    Group(Group),
}

/// Room music from an `AudioEnvironment` class.
#[derive(Debug, Clone, PartialEq)]
pub struct RoomMusic {
    pub class: String,
    pub state: Option<String>,
    pub overlays: Option<String>,
    pub startup_delay: f32,
    pub min_spacing: f32,
    pub max_spacing: f32,
    pub once_only: bool,
}

/// Every piece and group, by lower-case name.
#[derive(Debug, Clone, Default)]
pub struct MusicLibrary {
    pub entries: HashMap<String, Entry>,
    /// Room music by lower-case `AudioEnvironment` class name. Only classes
    /// that have music.
    pub rooms: HashMap<String, RoomMusic>,
    /// Problems met while reading (a package missing, an object unreadable).
    pub notes: Vec<String>,
}

/// Small deterministic random numbers (the music doesn't need more).
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }
    /// A number from 0 up to (not including) 1.
    pub fn unit(&mut self) -> f32 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        ((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32) / (1u64 << 24) as f32
    }
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo).max(0.0) * self.unit()
    }
}

impl MusicLibrary {
    /// Read every `*_music.uax` in `GameData/Sounds` and the room music in
    /// `GameData/Properties/audioenv.u` (with `GameData/System/engine.u`
    /// for the defaults). `game_data` is the `GameData` folder.
    pub fn load(game_data: &Path) -> MusicLibrary {
        let mut lib = MusicLibrary::default();
        let sounds = find_dir(game_data, "Sounds");
        let params = sounds
            .as_ref()
            .and_then(|d| find_file(d, "params_mus.uax"))
            .map(|p| read_params(&p, &mut lib.notes))
            .unwrap_or_default();
        let mut files: Vec<_> = sounds
            .as_ref()
            .and_then(|d| std::fs::read_dir(d).ok())
            .map(|r| {
                r.flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.to_ascii_lowercase().ends_with("_music.uax"))
                    })
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        if files.is_empty() {
            lib.notes
                .push(format!("no *_music.uax in {}/Sounds", game_data.display()));
        }
        for f in &files {
            match std::fs::read(f) {
                Ok(data) => lib.add_package(&data, &params, &f.display().to_string()),
                Err(e) => lib.notes.push(format!("{}: {e}", f.display())),
            }
        }
        lib.load_rooms(game_data);
        lib
    }

    /// Add one music package's pieces and groups.
    pub fn add_package(&mut self, data: &[u8], params: &HashMap<String, (f32, f32)>, label: &str) {
        let pkg = match Package::parse(data) {
            Ok(p) => p,
            Err(e) => {
                self.notes.push(format!("{label}: {e}"));
                return;
            }
        };
        for e in &pkg.exports {
            let class = pkg.export_class_name(e);
            let name = pkg.name(e.object_name).to_string();
            let entry = match class {
                "SoundStreamed" => match rc_package::sound::read_streamed(&pkg, data, e) {
                    Ok(s) => {
                        let p = &s.properties;
                        let file = s
                            .file_name
                            .trim_end_matches(".ogg")
                            .trim_end_matches(".OGG")
                            .to_string();
                        let volume = object_name(&pkg, p, "Params")
                            .and_then(|n| params.get(&n.to_ascii_lowercase()).copied())
                            .unwrap_or((1.0, 1.0));
                        Entry::Piece(Piece {
                            name: name.clone(),
                            file,
                            looping: bool_prop(&pkg, p, "Looping"),
                            fade_in: Fade {
                                seconds: float_prop(&pkg, p, "FadeInTime").unwrap_or(0.0),
                                shape: FadeShape::from_byte(byte_prop(&pkg, p, "FadeInType")),
                            },
                            fade_out: Fade {
                                seconds: float_prop(&pkg, p, "FadeOutTime").unwrap_or(0.0),
                                shape: FadeShape::from_byte(byte_prop(&pkg, p, "FadeOutType")),
                            },
                            volume,
                            likelihood: float_prop(&pkg, p, "Likelihood").unwrap_or(1.0),
                        })
                    }
                    Err(err) => {
                        self.notes.push(format!("{label} {name}: {err}"));
                        continue;
                    }
                },
                "SoundMultiple" => match rc_package::sound::read_multiple(&pkg, data, e) {
                    Ok(m) => {
                        let p = &m.properties;
                        Entry::Group(Group {
                            name: name.clone(),
                            members: m
                                .sounds
                                .iter()
                                .map(|&i| pkg.object_name(i).to_string())
                                .filter(|n| !n.eq_ignore_ascii_case("None"))
                                .collect(),
                            first: object_name(&pkg, p, "FirstSound"),
                            won: object_name(&pkg, p, "BattleWonMusic"),
                            lost: object_name(&pkg, p, "BattleLostMusic"),
                            likelihood: float_prop(&pkg, p, "Likelihood").unwrap_or(1.0),
                        })
                    }
                    Err(err) => {
                        self.notes.push(format!("{label} {name}: {err}"));
                        continue;
                    }
                },
                _ => continue,
            };
            self.entries.insert(name.to_ascii_lowercase(), entry);
        }
    }

    fn load_rooms(&mut self, game_data: &Path) {
        let mut classes = ClassLibrary::new();
        for (dir, file) in [("System", "engine.u"), ("Properties", "audioenv.u")] {
            let path = find_dir(game_data, dir).and_then(|d| find_file(&d, file));
            match path.map(std::fs::read) {
                Some(Ok(data)) => {
                    if let Err(e) = classes.add_package(file.trim_end_matches(".u"), data) {
                        self.notes.push(format!("{dir}/{file}: {e}"));
                    }
                }
                _ => self
                    .notes
                    .push(format!("can't read {dir}/{file}: no room music")),
            }
        }
        for class in classes.subclasses_of("AudioEnvironment") {
            let d = classes.defaults(&class);
            let object = |k: &str| {
                d.object(k)
                    .filter(|n| !n.eq_ignore_ascii_case("None"))
                    .map(str::to_string)
            };
            let state = object("iMuseState");
            let overlays = object("iMuseOverlays");
            if state.is_none() && overlays.is_none() {
                continue;
            }
            self.rooms.insert(
                class.to_ascii_lowercase(),
                RoomMusic {
                    class: class.clone(),
                    state,
                    overlays,
                    startup_delay: d.number("OverlayStartupDelay").unwrap_or(15.0),
                    min_spacing: d.number("OverlayMinSpacing").unwrap_or(30.0),
                    max_spacing: d.number("OverlayMaxSpacing").unwrap_or(60.0),
                    once_only: d.bool("bDoMusicOnceOnly").unwrap_or(true),
                },
            );
        }
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.get(&name.to_ascii_lowercase())
    }

    pub fn piece(&self, name: &str) -> Option<&Piece> {
        match self.get(name) {
            Some(Entry::Piece(p)) => Some(p),
            _ => None,
        }
    }

    pub fn group(&self, name: &str) -> Option<&Group> {
        match self.get(name) {
            Some(Entry::Group(g)) => Some(g),
            _ => None,
        }
    }

    pub fn room(&self, class: &str) -> Option<&RoomMusic> {
        self.rooms.get(&class.to_ascii_lowercase())
    }

    fn likelihood(&self, name: &str) -> f32 {
        match self.get(name) {
            Some(Entry::Piece(p)) => p.likelihood,
            Some(Entry::Group(g)) => g.likelihood,
            None => 0.0,
        }
    }

    /// Pick one member of a group, weighted by likelihood. Members the
    /// library doesn't know are skipped.
    pub fn pick_member(&self, group: &Group, rng: &mut Rng) -> Option<String> {
        let known: Vec<(&String, f32)> = group
            .members
            .iter()
            .filter(|m| self.get(m).is_some())
            .map(|m| (m, self.likelihood(m).max(0.0)))
            .collect();
        let total: f32 = known.iter().map(|(_, w)| w).sum();
        if known.is_empty() {
            return None;
        }
        if total <= 0.0 {
            return Some(known[0].0.clone());
        }
        let mut x = rng.unit() * total;
        for (m, w) in &known {
            if x < *w {
                return Some((*m).clone());
            }
            x -= w;
        }
        Some(known[known.len() - 1].0.clone())
    }

    /// The piece to play for a name: the piece itself, or for a group a
    /// member picked at random (following groups inside groups).
    pub fn resolve(&self, name: &str, rng: &mut Rng) -> Option<Piece> {
        let mut name = name.to_string();
        for _ in 0..8 {
            match self.get(&name)? {
                Entry::Piece(p) => return Some(p.clone()),
                Entry::Group(g) => name = self.pick_member(g, rng)?,
            }
        }
        None
    }

    /// Count of pieces and groups (for the log).
    pub fn counts(&self) -> (usize, usize) {
        let pieces = self
            .entries
            .values()
            .filter(|e| matches!(e, Entry::Piece(_)))
            .count();
        (pieces, self.entries.len() - pieces)
    }
}

/// `params_mus.uax`: each `AudioParams` object's (MinVolume, MaxVolume).
fn read_params(path: &Path, notes: &mut Vec<String>) -> HashMap<String, (f32, f32)> {
    let mut out = HashMap::new();
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            notes.push(format!("{}: {e}", path.display()));
            return out;
        }
    };
    let Ok(pkg) = Package::parse(&data) else {
        notes.push(format!("{}: not a package", path.display()));
        return out;
    };
    for e in &pkg.exports {
        if pkg.export_class_name(e) != "AudioParams" {
            continue;
        }
        let start = e.serial_offset as usize;
        let Some(body) = data.get(start..start + e.serial_size as usize) else {
            continue;
        };
        let mut r = rc_package::Reader::new(body);
        let Ok(p) = properties::read_properties(&pkg, &mut r) else {
            continue;
        };
        let max = float_prop(&pkg, &p, "MaxVolume").unwrap_or(1.0);
        let min = float_prop(&pkg, &p, "MinVolume").unwrap_or(max);
        out.insert(
            pkg.name(e.object_name).to_ascii_lowercase(),
            (min.min(max), max.max(min)),
        );
    }
    out
}

fn float_prop(pkg: &Package, p: &[Property], name: &str) -> Option<f32> {
    match properties::find(pkg, p, name).map(|p| &p.value) {
        Some(Value::Float(f)) => Some(*f),
        Some(Value::Int(i)) => Some(*i as f32),
        _ => None,
    }
}

fn byte_prop(pkg: &Package, p: &[Property], name: &str) -> u8 {
    match properties::find(pkg, p, name).map(|p| &p.value) {
        Some(Value::Byte(b)) => *b,
        _ => 0,
    }
}

fn bool_prop(pkg: &Package, p: &[Property], name: &str) -> bool {
    matches!(
        properties::find(pkg, p, name).map(|p| &p.value),
        Some(Value::Bool(true))
    )
}

fn object_name(pkg: &Package, p: &[Property], name: &str) -> Option<String> {
    match properties::find(pkg, p, name).map(|p| &p.value) {
        Some(Value::Object(o)) if *o != 0 => Some(pkg.object_name(*o).to_string()),
        _ => None,
    }
}

/// A folder inside `base`, any case.
fn find_dir(base: &Path, name: &str) -> Option<std::path::PathBuf> {
    let exact = base.join(name);
    if exact.is_dir() {
        return Some(exact);
    }
    std::fs::read_dir(base).ok()?.flatten().find_map(|e| {
        let p = e.path();
        (p.is_dir() && e.file_name().to_string_lossy().eq_ignore_ascii_case(name)).then_some(p)
    })
}

/// A file inside `dir`, any case.
fn find_file(dir: &Path, name: &str) -> Option<std::path::PathBuf> {
    let exact = dir.join(name);
    if exact.is_file() {
        return Some(exact);
    }
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        e.file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
            .then(|| e.path())
    })
}
