//! What a map says about its music: the battle music settings on its
//! `LevelInfo`, and the volumes whose `AudioClass` (an `AudioEnvironment`
//! class) may hold room music. Which classes really have music is decided
//! by `rc_music`, which reads the classes; this only finds the volumes and
//! their shapes.

use crate::actors::{self, LevelInfo};
use crate::volumes::{self, BlockingVolume};
use rc_package::properties::{self, Value};
use rc_package::Package;

/// `LevelInfo`'s battle engine values, as the map stores them. `None`
/// means the map leaves the engine default.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LevelBattle {
    pub music: Option<String>,
    pub threshold: Option<i32>,
    pub inner: Option<f32>,
    pub outer: Option<f32>,
    pub criteria: Option<u8>,
}

/// A volume with an `AudioClass`.
#[derive(Debug, Clone)]
pub struct AudioVolume {
    pub audio_class: String,
    /// Its shape and name (the `BlockingVolume` reader handles any brush
    /// volume; the blocking flags mean nothing here).
    pub shape: BlockingVolume,
}

#[derive(Debug, Clone, Default)]
pub struct MapMusic {
    pub battle: LevelBattle,
    pub volumes: Vec<AudioVolume>,
    /// Volumes that couldn't be read, and why.
    pub failures: Vec<String>,
}

/// Read the map's music settings.
pub fn read(pkg: &Package, data: &[u8], level: &LevelInfo) -> MapMusic {
    let mut out = MapMusic::default();
    for &a in level.actors.iter().flatten() {
        let class = pkg.export_class_name(&pkg.exports[a]);
        let is_level = class.eq_ignore_ascii_case("LevelInfo");
        if !is_level && !class.to_ascii_lowercase().ends_with("volume") {
            continue;
        }
        let Ok(props) = actors::read_actor_properties(pkg, data, a) else {
            continue;
        };
        if is_level {
            let b = &mut out.battle;
            for p in &props {
                if p.array_index != 0 {
                    continue;
                }
                match (pkg.name(p.name).to_ascii_lowercase().as_str(), &p.value) {
                    ("battlemusic", Value::Object(o)) if *o != 0 => {
                        b.music = Some(pkg.object_name(*o).to_string())
                    }
                    ("battlebaddiethreshold", Value::Int(i)) => b.threshold = Some(*i),
                    ("battleradiusinner", Value::Float(f)) => b.inner = Some(*f),
                    ("battleradiusouter", Value::Float(f)) => b.outer = Some(*f),
                    ("battlecriteria", Value::Byte(c)) => b.criteria = Some(*c),
                    _ => {}
                }
            }
            continue;
        }
        let audio = match properties::find(pkg, &props, "AudioClass").map(|p| &p.value) {
            Some(Value::Object(o)) if *o != 0 => pkg.object_name(*o).to_string(),
            _ => continue,
        };
        match volumes::read_volume(pkg, data, a) {
            Ok((shape, _)) => out.volumes.push(AudioVolume {
                audio_class: audio,
                shape,
            }),
            Err(e) => out
                .failures
                .push(format!("{}: {e}", pkg.name(pkg.exports[a].object_name))),
        }
    }
    out
}
