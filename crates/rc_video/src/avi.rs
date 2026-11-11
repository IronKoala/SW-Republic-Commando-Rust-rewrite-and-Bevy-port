//! A small reader for the AVI files the game ships (RIFF "AVI "). It
//! only finds what playing a silent video needs: the picture size, the
//! frame rate, the codec's four-letter code and each frame's bytes.
//!
//! Layout (Microsoft's published AVI RIFF format): `RIFF size "AVI "`,
//! then a `LIST "hdrl"` holding `avih` (main header) and one
//! `LIST "strl"` per stream (`strh` stream header + `strf` format), then
//! `LIST "movi"` holding the frame chunks (`00dc`, `00db`, ...), maybe
//! grouped in `LIST "rec "`, then an optional `idx1` index.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct AviVideo {
    pub width: u32,
    pub height: u32,
    /// Four-letter codec code from the stream format (`biCompression`),
    /// e.g. `MP42`.
    pub codec: [u8; 4],
    /// Frames per second = rate / scale from the video stream header.
    pub rate: u32,
    pub scale: u32,
    /// Byte range of each video frame inside the file, in play order.
    /// An empty chunk (a "drop" frame) repeats the previous picture.
    pub frames: Vec<(usize, usize)>,
}

impl AviVideo {
    pub fn fps(&self) -> f64 {
        if self.scale == 0 {
            0.0
        } else {
            self.rate as f64 / self.scale as f64
        }
    }

    pub fn seconds(&self) -> f64 {
        let fps = self.fps();
        if fps <= 0.0 {
            0.0
        } else {
            self.frames.len() as f64 / fps
        }
    }

    pub fn codec_name(&self) -> String {
        String::from_utf8_lossy(&self.codec).into_owned()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AviError(pub String);

impl fmt::Display for AviError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "avi: {}", self.0)
    }
}

impl std::error::Error for AviError {}

fn err<T>(msg: impl Into<String>) -> Result<T, AviError> {
    Err(AviError(msg.into()))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Walk the chunks in `d[start..end]`, calling `f(id, body_start, body_len)`.
fn chunks(d: &[u8], start: usize, end: usize, f: &mut dyn FnMut(&[u8], usize, usize)) {
    let mut at = start;
    while at + 8 <= end {
        let id = &d[at..at + 4];
        let Some(len) = u32_at(d, at + 4) else { break };
        let body = at + 8;
        let len = len as usize;
        let len = len.min(end.saturating_sub(body));
        f(id, body, len);
        // Chunks are padded to an even size.
        at = body + len + (len & 1);
    }
}

/// Read the first video stream of an AVI file held in memory.
pub fn parse(d: &[u8]) -> Result<AviVideo, AviError> {
    if d.len() < 12 || &d[0..4] != b"RIFF" || &d[8..12] != b"AVI " {
        return err("not a RIFF AVI file");
    }
    let riff_end = (8 + u32_at(d, 4).unwrap_or(0) as usize).min(d.len());

    let mut hdrl: Option<(usize, usize)> = None;
    let mut movi: Option<(usize, usize)> = None;
    let mut idx1: Option<(usize, usize)> = None;
    chunks(d, 12, riff_end, &mut |id, body, len| {
        if id == b"LIST" && len >= 4 {
            match &d[body..body + 4] {
                b"hdrl" => hdrl = Some((body + 4, body + len)),
                b"movi" => movi = Some((body + 4, body + len)),
                _ => {}
            }
        } else if id == b"idx1" {
            idx1 = Some((body, body + len));
        }
    });
    let Some((h0, h1)) = hdrl else {
        return err("no hdrl list");
    };
    let Some((m0, m1)) = movi else {
        return err("no movi list");
    };

    // Find the first stream whose strh type is "vids".
    let mut stream_no = 0usize;
    let mut video: Option<(usize, AviVideo)> = None;
    chunks(d, h0, h1, &mut |id, body, len| {
        if video.is_some() || id != b"LIST" || len < 4 || &d[body..body + 4] != b"strl" {
            return;
        }
        let this = stream_no;
        stream_no += 1;
        let mut strh: Option<(usize, usize)> = None;
        let mut strf: Option<(usize, usize)> = None;
        chunks(d, body + 4, body + len, &mut |id, b, l| match id {
            b"strh" => strh = Some((b, l)),
            b"strf" => strf = Some((b, l)),
            _ => {}
        });
        let (Some((sh, shl)), Some((sf, sfl))) = (strh, strf) else {
            return;
        };
        if shl < 28 || &d[sh..sh + 4] != b"vids" || sfl < 20 {
            return;
        }
        let scale = u32_at(d, sh + 20).unwrap_or(0);
        let rate = u32_at(d, sh + 24).unwrap_or(0);
        // BITMAPINFOHEADER: size, width, height (may be negative for
        // top-down), planes+bits, compression.
        let width = u32_at(d, sf + 4).unwrap_or(0);
        let height = (u32_at(d, sf + 8).unwrap_or(0) as i32).unsigned_abs();
        let mut codec = [0u8; 4];
        codec.copy_from_slice(&d[sf + 16..sf + 20]);
        video = Some((
            this,
            AviVideo {
                width,
                height,
                codec,
                rate,
                scale,
                frames: Vec::new(),
            },
        ));
    });
    let Some((stream, mut video)) = video else {
        return err("no video stream");
    };
    let tag = format!("{stream:02}");
    let wanted =
        |id: &[u8]| id[0..2] == *tag.as_bytes() && (id[2..4] == *b"dc" || id[2..4] == *b"db");

    // Frames in file order. Frames sit directly in movi or inside
    // `LIST "rec "` groups.
    let mut frames = Vec::new();
    fn walk(
        d: &[u8],
        a: usize,
        b: usize,
        wanted: &dyn Fn(&[u8]) -> bool,
        frames: &mut Vec<(usize, usize)>,
    ) {
        chunks(d, a, b, &mut |id, body, len| {
            if id == b"LIST" && len >= 4 && &d[body..body + 4] == b"rec " {
                walk(d, body + 4, body + len, wanted, frames);
            } else if wanted(id) {
                frames.push((body, len));
            }
        });
    }
    walk(d, m0, m1, &wanted, &mut frames);
    let _ = idx1; // File order is play order for these files.
    if frames.is_empty() {
        return err("no video frames in movi");
    }
    video.frames = frames;
    Ok(video)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend((body.len() as u32).to_le_bytes());
        v.extend(body);
        if body.len() & 1 == 1 {
            v.push(0);
        }
        v
    }

    fn list(kind: &[u8; 4], parts: &[Vec<u8>]) -> Vec<u8> {
        let mut body = kind.to_vec();
        for p in parts {
            body.extend(p);
        }
        chunk(b"LIST", &body)
    }

    #[test]
    fn reads_size_rate_codec_and_frames() {
        let mut strh = b"vidsMP42".to_vec();
        strh.extend([0u8; 12]);
        strh.extend(1000u32.to_le_bytes()); // scale
        strh.extend(15000u32.to_le_bytes()); // rate
        strh.extend([0u8; 28]);
        let mut strf = 40u32.to_le_bytes().to_vec();
        strf.extend(512u32.to_le_bytes());
        strf.extend(512u32.to_le_bytes());
        strf.extend(1u16.to_le_bytes());
        strf.extend(24u16.to_le_bytes());
        strf.extend(b"MP42");
        strf.extend([0u8; 20]);
        let hdrl = list(
            b"hdrl",
            &[
                chunk(b"avih", &[0; 56]),
                list(b"strl", &[chunk(b"strh", &strh), chunk(b"strf", &strf)]),
            ],
        );
        let movi = list(
            b"movi",
            &[
                chunk(b"00dc", &[1, 2, 3]),
                chunk(b"01wb", &[9; 4]),
                chunk(b"00dc", &[]),
            ],
        );
        let mut body = b"AVI ".to_vec();
        body.extend(hdrl);
        body.extend(movi);
        let file = chunk(b"RIFF", &body);
        let v = parse(&file).unwrap();
        assert_eq!((v.width, v.height), (512, 512));
        assert_eq!(v.codec_name(), "MP42");
        assert_eq!(v.fps(), 15.0);
        assert_eq!(v.frames.len(), 2);
        assert_eq!(
            &file[v.frames[0].0..v.frames[0].0 + v.frames[0].1],
            &[1, 2, 3]
        );
        assert_eq!(v.frames[1].1, 0);
    }
}
