//! Reads sound objects from `.uax` packages.
//!
//! Three classes hold sounds in Republic Commando:
//! - `Sound`: the sound itself, stored inside the package as a whole `.wav`
//!   file (RIFF). Every one checked so far is plain 16-bit PCM.
//! - `SoundStreamed`: no audio inside. It names a file that is played from
//!   disk while the game runs (music, and some ambient loops).
//! - `SoundMultiple`: a list of other sounds the game picks from.
//!
//! A `Sound` (and some groups) names its `Params`: an `AudioParams` object
//! in `Params_VOX.uax` (speech) or `Params_SFX.uax` (effects) that says how
//! the game plays it: heard from nowhere in particular (`NonPositional`,
//! radio chatter, briefings) or from its place in the level, how far it
//! carries (`Radius`), and whether it gets quieter with distance at all
//! (`bDisable3DFalloff`). Those property names and values were read from
//! the user's own game files; see [`AudioParams`].
//!
//! The `Sound` layout is UE2's `USound` (UE Viewer, github.com/gildor2/UEViewer,
//! MIT, Konstantin Nosov, `Unreal/UnrealSound/UnSound.cpp`): properties, the
//! file type as a name ("WAV"), then a lazy array with the file's bytes.
//! The `SoundStreamed` layout (properties, then a file name string) and the
//! `SoundMultiple` layout (properties, a list of sound references, then a
//! list of numbers) were worked out from the bytes of the user's own game
//! files, not taken from any source. Written from those descriptions; no code copied.

use crate::properties::{self, Property};
use crate::{Export, Package, ReadError, Reader, Result};

/// A `Sound` export: where its embedded file is.
#[derive(Debug, Clone)]
pub struct Sound {
    pub properties: Vec<Property>,
    /// Name of the embedded file's type, e.g. "WAV".
    pub file_type: String,
    /// Where the embedded file starts, counted from the start of the package.
    pub data_offset: usize,
    pub data_len: usize,
    /// The "skip position" stored before the bytes: the file offset just
    /// after them. Checked against what we actually read.
    pub skip_pos: i32,
    /// Bytes left in the object after the embedded file. Expected to be 0.
    pub leftover: usize,
}

/// A `SoundStreamed` export: the name of a file played from disk.
#[derive(Debug, Clone)]
pub struct SoundStreamed {
    pub properties: Vec<Property>,
    pub file_name: String,
    pub leftover: usize,
}

fn object_bytes<'a>(data: &'a [u8], export: &Export) -> Result<&'a [u8]> {
    let start = export.serial_offset as usize;
    data.get(start..start + export.serial_size as usize)
        .ok_or_else(|| ReadError::Invalid {
            offset: start,
            what: "export data is outside the file".into(),
        })
}

/// Read a `Sound` export. `data` is the whole package file.
pub fn read_sound(pkg: &Package, data: &[u8], export: &Export) -> Result<Sound> {
    let start = export.serial_offset as usize;
    let mut r = Reader::new(object_bytes(data, export)?);
    let properties = properties::read_properties(pkg, &mut r)?;
    let file_type = pkg.name(r.compact_index()?).to_string();
    // Lazy array: a skip position, then a byte array.
    let skip_pos = r.i32()?;
    let len = r.compact_index()?;
    if len < 0 {
        return r.invalid(format!("sound data length {len} is negative"));
    }
    let data_offset = start + r.pos();
    r.bytes(len as usize)?;
    Ok(Sound {
        properties,
        file_type,
        data_offset,
        data_len: len as usize,
        skip_pos,
        leftover: r.len() - r.pos(),
    })
}

impl Sound {
    /// The embedded file's bytes.
    pub fn bytes<'a>(&self, data: &'a [u8]) -> &'a [u8] {
        &data[self.data_offset..self.data_offset + self.data_len]
    }

    /// Problems that would show we read the object wrongly.
    pub fn check(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let end = self.data_offset + self.data_len;
        if self.skip_pos as i64 != end as i64 {
            problems.push(format!(
                "skip position {} but data ends at {end}",
                self.skip_pos
            ));
        }
        if self.leftover != 0 {
            problems.push(format!("{} bytes left over after the data", self.leftover));
        }
        problems
    }
}

/// Read a `SoundStreamed` export.
pub fn read_streamed(pkg: &Package, data: &[u8], export: &Export) -> Result<SoundStreamed> {
    let mut r = Reader::new(object_bytes(data, export)?);
    let properties = properties::read_properties(pkg, &mut r)?;
    let file_name = r.fstring()?;
    Ok(SoundStreamed {
        properties,
        file_name,
        leftover: r.len() - r.pos(),
    })
}

/// What an `AudioParams` object says about playing the sounds that name
/// it. Only the PC values are kept; the game also stores Xbox ones
/// (`RadiusXBox`, ...), which are left out.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioParams {
    /// `Radius`, Unreal units: how far the sound carries before it starts
    /// getting quieter (see `rc_scripts_plugin`'s sounds for how we use
    /// it). `None` when the object gives none.
    pub radius: Option<f32>,
    /// `NonPositional`: heard the same wherever the listener is (radio,
    /// briefings, the player's own lines).
    pub non_positional: bool,
    /// `bDisable3DFalloff`: from its side, but no quieter with distance
    /// (big explosions).
    pub no_falloff: bool,
    /// `Category` (`AG_Music` 0, `AG_Voice` 1, `AG_SFX` 2). Picks the
    /// volume slider the game plays the sound on, and music gets no room
    /// echo (`UAudioSubsystem::PlayUSound`). Unset is 0, music.
    pub category: u8,
    /// `MinVolume` / `MaxVolume`: each playing gets a random loudness
    /// between them (class default 1 and 1).
    pub min_volume: f32,
    pub max_volume: f32,
    /// `MinPitch` / `MaxPitch`: the same for pitch (class default 1 and 1).
    pub min_pitch: f32,
    pub max_pitch: f32,
}

impl Default for AudioParams {
    /// The `AudioParams` class defaults in `Engine.u`, which the game also
    /// uses for a sound that names no `Params` (`USound::GetParams`).
    fn default() -> Self {
        AudioParams {
            radius: None,
            non_positional: false,
            no_falloff: false,
            category: AG_MUSIC,
            min_volume: 1.0,
            max_volume: 1.0,
            min_pitch: 1.0,
            max_pitch: 1.0,
        }
    }
}

/// `AudioParams.Category` values (`Engine.u`, enum `EAudioGroup`).
pub const AG_MUSIC: u8 = 0;
pub const AG_VOICE: u8 = 1;
pub const AG_SFX: u8 = 2;

/// Read an `AudioParams` export (properties only).
pub fn read_audio_params(pkg: &Package, data: &[u8], export: &Export) -> Result<AudioParams> {
    let mut r = Reader::new(object_bytes(data, export)?);
    let props = properties::read_properties(pkg, &mut r)?;
    Ok(audio_params_from(pkg, &props))
}

fn audio_params_from(pkg: &Package, props: &[Property]) -> AudioParams {
    use properties::Value;
    let flag = |n: &str| {
        matches!(
            properties::find(pkg, props, n).map(|p| &p.value),
            Some(Value::Bool(true))
        )
    };
    let float = |n: &str, default: f32| match properties::find(pkg, props, n).map(|p| &p.value) {
        Some(Value::Float(f)) => *f,
        _ => default,
    };
    AudioParams {
        radius: match properties::find(pkg, props, "Radius").map(|p| &p.value) {
            Some(Value::Float(f)) => Some(*f),
            _ => None,
        },
        non_positional: flag("NonPositional"),
        no_falloff: flag("bDisable3DFalloff"),
        category: match properties::find(pkg, props, "Category").map(|p| &p.value) {
            Some(Value::Byte(b)) => *b,
            _ => AG_MUSIC,
        },
        min_volume: float("MinVolume", 1.0),
        max_volume: float("MaxVolume", 1.0),
        min_pitch: float("MinPitch", 1.0),
        max_pitch: float("MaxPitch", 1.0),
    }
}

/// Every `AudioParams` in a package (`Params_VOX.uax`, `Params_SFX.uax`),
/// by name as stored. Objects that don't read are left out.
pub fn read_all_audio_params(pkg: &Package, data: &[u8]) -> Vec<(String, AudioParams)> {
    pkg.exports
        .iter()
        .filter(|e| pkg.export_class_name(e) == "AudioParams")
        .filter_map(|e| {
            read_audio_params(pkg, data, e)
                .ok()
                .map(|p| (pkg.name(e.object_name).to_string(), p))
        })
        .collect()
}

/// The `Params` a sound (or group) names, as `package.name` in lower case
/// (`params_vox.voice_ca1_2d`), or `None` when it names none. `package` is
/// the outermost package of an imported object, or `this` for one in the
/// same package.
pub fn params_of(pkg: &Package, props: &[Property], this: &str) -> Option<String> {
    let r = match properties::find(pkg, props, "Params").map(|p| &p.value) {
        Some(properties::Value::Object(o)) if *o != 0 => *o,
        _ => return None,
    };
    let name = pkg.object_name(r);
    let mut package = this.to_string();
    if let crate::ObjectRef::Import(i) = crate::ObjectRef::from_index(r) {
        let mut outer = pkg.imports.get(i)?.package_index;
        for _ in 0..16 {
            match crate::ObjectRef::from_index(outer) {
                crate::ObjectRef::Import(j) if j < pkg.imports.len() => {
                    package = pkg.name(pkg.imports[j].object_name).to_string();
                    outer = pkg.imports[j].package_index;
                }
                _ => break,
            }
        }
    }
    Some(format!("{package}.{name}").to_ascii_lowercase())
}

/// A `SoundMultiple` export: a group of sounds.
#[derive(Debug, Clone)]
pub struct SoundMultiple {
    pub properties: Vec<Property>,
    /// Object references (see `ObjectRef`) to the sounds in the group.
    pub sounds: Vec<i32>,
    /// A second list stored after the sounds. Its meaning is not known yet;
    /// it is kept as 4-byte numbers (perhaps one weight per sound).
    pub extra: Vec<u32>,
    pub leftover: usize,
}

/// Read a `SoundMultiple` export.
pub fn read_multiple(pkg: &Package, data: &[u8], export: &Export) -> Result<SoundMultiple> {
    let mut r = Reader::new(object_bytes(data, export)?);
    let properties = properties::read_properties(pkg, &mut r)?;
    let count = r.compact_index()?;
    if !(0..=4096).contains(&count) {
        return r.invalid(format!("sound count {count} is implausible"));
    }
    let sounds = (0..count)
        .map(|_| r.compact_index())
        .collect::<Result<Vec<_>>>()?;
    let count = r.compact_index()?;
    if !(0..=4096).contains(&count) {
        return r.invalid(format!("second list count {count} is implausible"));
    }
    let extra = (0..count).map(|_| r.u32()).collect::<Result<Vec<_>>>()?;
    Ok(SoundMultiple {
        properties,
        sounds,
        extra,
        leftover: r.len() - r.pos(),
    })
}
