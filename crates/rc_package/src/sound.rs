//! Reads sound objects from `.uax` packages.
//!
//! Three classes hold sounds in Republic Commando:
//! - `Sound`: the sound itself, stored inside the package as a whole `.wav`
//!   file (RIFF). Every one checked so far is plain 16-bit PCM.
//! - `SoundStreamed`: no audio inside. It names a file that is played from
//!   disk while the game runs (music, and some ambient loops).
//! - `SoundMultiple`: a list of other sounds the game picks from.
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
