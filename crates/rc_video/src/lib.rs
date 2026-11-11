//! Plays the original game's videos straight from the install: an AVI
//! file holding Microsoft MPEG-4 version 2 (`MP42`) pictures, such as
//! the menu background `Movies/menu_bg.avi`. No Bevy in here, so it can
//! be built and tested on its own; `rc_game` puts the pictures on screen.

pub mod avi;
pub mod msmpeg4v2;

pub use avi::AviVideo;
pub use msmpeg4v2::Stats;

/// A video read into memory, decoded one frame at a time on request.
pub struct VideoPlayer {
    data: Vec<u8>,
    pub info: AviVideo,
    decoder: msmpeg4v2::Decoder,
    /// Index of the next frame to decode.
    next: usize,
    /// Times the video wrapped back to its first frame.
    pub loops: u32,
}

impl VideoPlayer {
    /// Read an AVI file's bytes. Only `MP42` video is supported.
    pub fn open(data: Vec<u8>) -> Result<Self, String> {
        let info = avi::parse(&data).map_err(|e| e.to_string())?;
        if &info.codec != b"MP42" && &info.codec != b"mp42" {
            return Err(format!(
                "video codec {:?} is not supported (only MP42)",
                info.codec_name()
            ));
        }
        let decoder = msmpeg4v2::Decoder::new(info.width, info.height)?;
        Ok(Self {
            data,
            info,
            decoder,
            next: 0,
            loops: 0,
        })
    }

    pub fn stats(&self) -> Stats {
        self.decoder.stats
    }

    pub fn frame_count(&self) -> usize {
        self.info.frames.len()
    }

    /// Index of the frame most recently decoded.
    pub fn current(&self) -> Option<usize> {
        if self.decoder.picture().is_none() {
            None
        } else {
            Some((self.next + self.frame_count() - 1) % self.frame_count())
        }
    }

    /// Go back to before the first frame (the next [`Self::step`] decodes
    /// frame 0).
    pub fn restart(&mut self) {
        self.next = 0;
        self.loops = 0;
        self.decoder.reset();
    }

    /// Decode the next frame, going back to the first after the last.
    pub fn step(&mut self) -> Result<(), String> {
        if self.next >= self.frame_count() {
            self.next = 0;
            self.loops += 1;
            self.decoder.reset();
        }
        let (at, len) = self.info.frames[self.next];
        let n = self.next;
        self.next += 1;
        self.decoder
            .decode(&self.data[at..at + len])
            .map(|_| ())
            .map_err(|e| format!("frame {n}: {e}"))
    }

    /// Decode frames until `frame` (counted from the start, wrapping)
    /// is the current one. Used to keep the picture in step with time.
    pub fn advance_to(&mut self, frame: u64) -> Result<(), String> {
        let n = self.frame_count() as u64;
        let target = (frame % n) as usize;
        let mut guard = 0;
        while self.current() != Some(target) {
            self.step()?;
            guard += 1;
            if guard > 2 * n as usize {
                return Err("could not reach the wanted frame".into());
            }
        }
        Ok(())
    }

    /// The current picture as planar 4:2:0 (Y, Cb, Cr), cropped to the
    /// video size.
    pub fn yuv(&self) -> Option<Vec<u8>> {
        let p = self.decoder.picture()?;
        let (w, h) = (self.info.width as usize, self.info.height as usize);
        let mut out = Vec::with_capacity(w * h * 3 / 2);
        for r in 0..h {
            out.extend_from_slice(&p.y[r * p.y_stride..r * p.y_stride + w]);
        }
        for plane in [&p.cb, &p.cr] {
            for r in 0..h / 2 {
                out.extend_from_slice(&plane[r * p.c_stride..r * p.c_stride + w / 2]);
            }
        }
        Some(out)
    }

    /// The current picture as RGBA (sRGB, alpha 255), top row first.
    pub fn rgba(&self, out: &mut Vec<u8>) -> bool {
        let Some(p) = self.decoder.picture() else {
            return false;
        };
        let (w, h) = (self.info.width as usize, self.info.height as usize);
        out.clear();
        out.reserve(w * h * 4);
        for r in 0..h {
            for c in 0..w {
                let y = p.y[r * p.y_stride + c];
                let u = p.cb[(r / 2) * p.c_stride + c / 2];
                let v = p.cr[(r / 2) * p.c_stride + c / 2];
                let [rr, gg, bb] = yuv_to_rgb(y, u, v);
                out.extend_from_slice(&[rr, gg, bb, 255]);
            }
        }
        true
    }
}

/// ITU-R BT.601 "studio range" colour (Y 16..235, Cb/Cr 16..240) to
/// 8-bit RGB, the standard for MPEG-4 video of this age; ffmpeg converts
/// these files the same way. Fixed-point, rounded.
pub fn yuv_to_rgb(y: u8, u: u8, v: u8) -> [u8; 3] {
    let c = (y as i32 - 16) * 298;
    let d = u as i32 - 128;
    let e = v as i32 - 128;
    let r = (c + 409 * e + 128) >> 8;
    let g = (c - 100 * d - 208 * e + 128) >> 8;
    let b = (c + 516 * d + 128) >> 8;
    [
        r.clamp(0, 255) as u8,
        g.clamp(0, 255) as u8,
        b.clamp(0, 255) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grey_and_white_convert_as_expected() {
        assert_eq!(yuv_to_rgb(16, 128, 128), [0, 0, 0]);
        assert_eq!(yuv_to_rgb(235, 128, 128), [255, 255, 255]);
        assert_eq!(yuv_to_rgb(126, 128, 128), [128, 128, 128]);
    }
}
