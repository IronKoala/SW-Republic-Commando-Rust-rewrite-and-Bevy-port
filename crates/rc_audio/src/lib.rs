//! Reads `.wav` files (RIFF WAVE) into plain samples and measures them, so a
//! sound can be checked with numbers instead of by ear.
//!
//! Knows nothing about Unreal packages: it is given the bytes of a `.wav`
//! file. Only uncompressed PCM (format tag 1) at 8 or 16 bits is supported;
//! that is all Republic Commando's `.uax` packages were found to use.
//! Layout follows Microsoft's public RIFF/WAVE documentation ("Multimedia
//! Programming Interface and Data Specifications 1.0", 1991).

/// A decoded sound.
#[derive(Debug, Clone)]
pub struct Wav {
    /// Format tag from the `fmt ` chunk (1 = PCM).
    pub format_tag: u16,
    pub channels: u16,
    /// Samples per second, per channel.
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    /// Samples as signed 16-bit numbers, channels interleaved (left, right,
    /// left, right ... for stereo). 8-bit files are scaled up.
    pub samples: Vec<i16>,
    /// Bytes in the file after the chunks we understood. Expected to be 0.
    pub trailing_bytes: usize,
    /// Names of chunks other than `fmt ` and `data` (kept for the log).
    pub other_chunks: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum WavError {
    NotRiffWave,
    Truncated { what: &'static str },
    MissingChunk(&'static str),
    Unsupported { format_tag: u16, bits: u16 },
    BadFormat(String),
}

impl std::fmt::Display for WavError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WavError::NotRiffWave => write!(f, "not a RIFF WAVE file"),
            WavError::Truncated { what } => write!(f, "file ends in the middle of {what}"),
            WavError::MissingChunk(c) => write!(f, "no '{c}' chunk"),
            WavError::Unsupported { format_tag, bits } => {
                write!(f, "unsupported format: tag {format_tag}, {bits} bits")
            }
            WavError::BadFormat(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for WavError {}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

impl Wav {
    /// Parse a whole `.wav` file.
    pub fn parse(bytes: &[u8]) -> Result<Wav, WavError> {
        if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err(WavError::NotRiffWave);
        }
        // The RIFF size counts everything after the first 8 bytes.
        let riff_end = (8 + u32_at(bytes, 4) as usize).min(bytes.len());
        let mut fmt = None;
        let mut data = None;
        let mut other_chunks = Vec::new();
        let mut pos = 12;
        while pos + 8 <= riff_end {
            let id = &bytes[pos..pos + 4];
            let len = u32_at(bytes, pos + 4) as usize;
            let body_start = pos + 8;
            let body_end = body_start
                .checked_add(len)
                .filter(|&e| e <= bytes.len())
                .ok_or(WavError::Truncated { what: "a chunk" })?;
            let body = &bytes[body_start..body_end];
            match id {
                b"fmt " => fmt = Some(body),
                b"data" => data = Some(body),
                _ => other_chunks.push(String::from_utf8_lossy(id).into_owned()),
            }
            // Chunks are padded to an even length.
            pos = body_end + (len & 1);
        }
        let trailing_bytes = bytes.len().saturating_sub(pos.min(bytes.len()));
        let fmt = fmt.ok_or(WavError::MissingChunk("fmt "))?;
        let data = data.ok_or(WavError::MissingChunk("data"))?;
        if fmt.len() < 16 {
            return Err(WavError::Truncated {
                what: "the fmt chunk",
            });
        }
        let format_tag = u16_at(fmt, 0);
        let channels = u16_at(fmt, 2);
        let sample_rate = u32_at(fmt, 4);
        let block_align = u16_at(fmt, 12);
        let bits_per_sample = u16_at(fmt, 14);
        if format_tag != 1 || !(bits_per_sample == 8 || bits_per_sample == 16) {
            return Err(WavError::Unsupported {
                format_tag,
                bits: bits_per_sample,
            });
        }
        if channels == 0 || sample_rate == 0 {
            return Err(WavError::BadFormat(format!(
                "{channels} channels at {sample_rate} Hz"
            )));
        }
        let bytes_per_sample = bits_per_sample as usize / 8;
        if block_align as usize != bytes_per_sample * channels as usize {
            return Err(WavError::BadFormat(format!(
                "block align {block_align} doesn't match {channels} channels of {bits_per_sample} bits"
            )));
        }
        let samples = if bits_per_sample == 16 {
            data.as_chunks::<2>()
                .0
                .iter()
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect()
        } else {
            // 8-bit WAV samples are unsigned, centred on 128.
            data.iter().map(|&b| ((b as i16) - 128) << 8).collect()
        };
        Ok(Wav {
            format_tag,
            channels,
            sample_rate,
            bits_per_sample,
            samples,
            trailing_bytes,
            other_chunks,
        })
    }

    /// Number of sample frames (one sample per channel).
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels as usize
    }

    pub fn seconds(&self) -> f64 {
        self.frames() as f64 / self.sample_rate as f64
    }

    pub fn stats(&self) -> Stats {
        Stats::of(&self.samples)
    }
}

/// Numbers that describe a sound without listening to it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    /// Loudest single sample, 0.0 (silence) to 1.0 (full scale).
    pub peak: f64,
    /// Average loudness (root mean square), 0.0 to 1.0.
    pub rms: f64,
    /// How many samples sit at full scale (a sign of clipping).
    pub clipped: usize,
    /// FNV-1a hash of the samples: changes if any sample changes.
    pub checksum: u64,
}

impl Stats {
    pub fn of(samples: &[i16]) -> Stats {
        let mut peak = 0i32;
        let mut sum_sq = 0f64;
        let mut clipped = 0;
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for &s in samples {
            peak = peak.max((s as i32).abs());
            sum_sq += (s as f64) * (s as f64);
            if s == i16::MAX || s == i16::MIN {
                clipped += 1;
            }
            for b in s.to_le_bytes() {
                hash ^= b as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        let rms = if samples.is_empty() {
            0.0
        } else {
            (sum_sq / samples.len() as f64).sqrt() / 32768.0
        };
        Stats {
            peak: peak as f64 / 32768.0,
            rms,
            clipped,
            checksum: hash,
        }
    }

    /// Loudness in decibels relative to full scale (0 dB = loudest possible).
    pub fn peak_db(&self) -> f64 {
        to_db(self.peak)
    }

    pub fn rms_db(&self) -> f64 {
        to_db(self.rms)
    }
}

fn to_db(x: f64) -> f64 {
    if x <= 0.0 {
        f64::NEG_INFINITY
    } else {
        20.0 * x.log10()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a tiny made-up WAV file. No game data involved.
    fn wav(channels: u16, rate: u32, bits: u16, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        let align = channels * bits / 8;
        out.extend_from_slice(&(rate * align as u32).to_le_bytes());
        out.extend_from_slice(&align.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    #[test]
    fn reads_16_bit_stereo() {
        let samples: [i16; 4] = [100, -100, 16384, -32768];
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let w = Wav::parse(&wav(2, 22050, 16, &data)).unwrap();
        assert_eq!(
            (w.channels, w.sample_rate, w.bits_per_sample),
            (2, 22050, 16)
        );
        assert_eq!(w.samples, samples);
        assert_eq!(w.frames(), 2);
        assert_eq!(w.trailing_bytes, 0);
        let s = w.stats();
        assert_eq!(s.peak, 1.0);
        assert_eq!(s.clipped, 1);
    }

    #[test]
    fn reads_8_bit() {
        let w = Wav::parse(&wav(1, 11025, 8, &[128, 255, 0])).unwrap();
        assert_eq!(w.samples, [0, 127 << 8, -128 << 8]);
    }

    #[test]
    fn rejects_compressed_and_junk() {
        let mut f = wav(1, 22050, 16, &[0, 0]);
        f[20] = 2; // MS ADPCM
        assert_eq!(
            Wav::parse(&f).unwrap_err(),
            WavError::Unsupported {
                format_tag: 2,
                bits: 16
            }
        );
        assert_eq!(Wav::parse(b"hello").unwrap_err(), WavError::NotRiffWave);
        let f = wav(1, 22050, 16, &[0; 100]);
        assert!(Wav::parse(&f[..60]).is_err());
    }

    #[test]
    fn silence_and_checksum() {
        let s = Stats::of(&[0; 10]);
        assert_eq!((s.peak, s.rms), (0.0, 0.0));
        assert_eq!(s.peak_db(), f64::NEG_INFINITY);
        assert_ne!(Stats::of(&[0, 1]).checksum, Stats::of(&[1, 0]).checksum);
    }
}
