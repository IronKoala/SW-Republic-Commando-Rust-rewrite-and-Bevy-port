//! Turns texture pixel data into plain RGBA images (4 bytes per pixel:
//! red, green, blue, alpha) and writes them as PNG files.
//!
//! Knows nothing about Unreal packages: it is given raw bytes, a format and a
//! size. DXT ("S3TC") block layouts follow Microsoft's public Direct3D
//! documentation for BC1, BC2 and BC3. PNG follows the W3C PNG specification.

pub mod png;

/// A decoded picture, `width * height * 4` bytes, rows top to bottom.
#[derive(Debug, Clone)]
pub struct Rgba {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// One byte per pixel, an index into a 256-colour palette.
    P8,
    /// Four bytes per pixel, stored blue, green, red, alpha.
    Bgra8,
    /// 4x4 blocks of 8 bytes, colour with optional 1-bit transparency.
    Dxt1,
    /// 4x4 blocks of 16 bytes, explicit 4-bit alpha then a DXT1 colour block.
    Dxt3,
    /// 4x4 blocks of 16 bytes, interpolated alpha then a DXT1 colour block.
    Dxt5,
    /// Two bytes per pixel, grayscale (terrain heightmaps).
    G16,
}

#[derive(Debug)]
pub enum DecodeError {
    TooShort { needed: usize, got: usize },
    NoPalette,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::TooShort { needed, got } => {
                write!(f, "pixel data too short: needed {needed} bytes, got {got}")
            }
            DecodeError::NoPalette => write!(f, "paletted texture without a palette"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decode `data` into RGBA. `palette` is 256 RGBA colours, needed for P8.
pub fn decode(
    format: PixelFormat,
    width: usize,
    height: usize,
    data: &[u8],
    palette: Option<&[[u8; 4]]>,
) -> Result<Rgba, DecodeError> {
    let (w, h) = (width.max(1), height.max(1));
    let blocks = w.div_ceil(4) * h.div_ceil(4);
    let needed = match format {
        PixelFormat::P8 => w * h,
        PixelFormat::Bgra8 => w * h * 4,
        PixelFormat::G16 => w * h * 2,
        PixelFormat::Dxt1 => blocks * 8,
        PixelFormat::Dxt3 | PixelFormat::Dxt5 => blocks * 16,
    };
    if data.len() < needed {
        return Err(DecodeError::TooShort {
            needed,
            got: data.len(),
        });
    }
    let mut out = vec![0u8; w * h * 4];
    match format {
        PixelFormat::P8 => {
            let palette = palette.ok_or(DecodeError::NoPalette)?;
            for (i, &index) in data[..w * h].iter().enumerate() {
                let c = palette
                    .get(index as usize)
                    .copied()
                    .unwrap_or([0, 0, 0, 255]);
                out[i * 4..i * 4 + 4].copy_from_slice(&c);
            }
        }
        PixelFormat::Bgra8 => {
            for (o, i) in out.chunks_exact_mut(4).zip(data.chunks_exact(4)) {
                o.copy_from_slice(&[i[2], i[1], i[0], i[3]]);
            }
        }
        PixelFormat::G16 => {
            for (o, i) in out.chunks_exact_mut(4).zip(data.chunks_exact(2)) {
                // Keep the high byte of the little-endian 16-bit value.
                let g = i[1];
                o.copy_from_slice(&[g, g, g, 255]);
            }
        }
        PixelFormat::Dxt1 | PixelFormat::Dxt3 | PixelFormat::Dxt5 => {
            let block_size = if format == PixelFormat::Dxt1 { 8 } else { 16 };
            let blocks_wide = w.div_ceil(4);
            for (b, block) in data[..needed].chunks_exact(block_size).enumerate() {
                let (bx, by) = ((b % blocks_wide) * 4, (b / blocks_wide) * 4);
                let mut texels = [[0u8; 4]; 16];
                match format {
                    PixelFormat::Dxt1 => color_block(block, true, &mut texels),
                    PixelFormat::Dxt3 => {
                        color_block(&block[8..], false, &mut texels);
                        for (i, t) in texels.iter_mut().enumerate() {
                            let nibble = (block[i / 2] >> ((i % 2) * 4)) & 0x0F;
                            t[3] = nibble * 17;
                        }
                    }
                    _ => {
                        color_block(&block[8..], false, &mut texels);
                        alpha_block(&block[..8], &mut texels);
                    }
                }
                for (i, t) in texels.iter().enumerate() {
                    let (x, y) = (bx + i % 4, by + i / 4);
                    if x < w && y < h {
                        let o = (y * w + x) * 4;
                        out[o..o + 4].copy_from_slice(t);
                    }
                }
            }
        }
    }
    Ok(Rgba {
        width: w,
        height: h,
        pixels: out,
    })
}

fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 31) as u32;
    let g = ((c >> 5) & 63) as u32;
    let b = (c & 31) as u32;
    [
        (r * 255 / 31) as u8,
        (g * 255 / 63) as u8,
        (b * 255 / 31) as u8,
    ]
}

/// An 8-byte colour block: two 565 colours, then 2 bits per texel.
/// `dxt1_mode` allows the 3-colour + transparent variant (when c0 <= c1).
fn color_block(block: &[u8], dxt1_mode: bool, out: &mut [[u8; 4]; 16]) {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let (a, b) = (rgb565(c0), rgb565(c1));
    let mix = |wa: u32, wb: u32, d: u32| -> [u8; 4] {
        let m = |i: usize| ((a[i] as u32 * wa + b[i] as u32 * wb) / d) as u8;
        [m(0), m(1), m(2), 255]
    };
    let palette = if c0 > c1 || !dxt1_mode {
        [
            [a[0], a[1], a[2], 255],
            [b[0], b[1], b[2], 255],
            mix(2, 1, 3),
            mix(1, 2, 3),
        ]
    } else {
        [
            [a[0], a[1], a[2], 255],
            [b[0], b[1], b[2], 255],
            mix(1, 1, 2),
            [0, 0, 0, 0],
        ]
    };
    let bits = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    for (i, t) in out.iter_mut().enumerate() {
        *t = palette[((bits >> (i * 2)) & 3) as usize];
    }
}

/// DXT5's 8-byte alpha block: two alpha values, then 3 bits per texel.
fn alpha_block(block: &[u8], out: &mut [[u8; 4]; 16]) {
    let (a0, a1) = (block[0] as u32, block[1] as u32);
    let mut table = [0u32; 8];
    table[0] = a0;
    table[1] = a1;
    if a0 > a1 {
        for i in 1..7 {
            table[i + 1] = (a0 * (7 - i as u32) + a1 * i as u32) / 7;
        }
    } else {
        for i in 1..5 {
            table[i + 1] = (a0 * (5 - i as u32) + a1 * i as u32) / 5;
        }
        table[6] = 0;
        table[7] = 255;
    }
    let mut bits = 0u64;
    for (i, &byte) in block[2..8].iter().enumerate() {
        bits |= (byte as u64) << (8 * i);
    }
    for (i, t) in out.iter_mut().enumerate() {
        t[3] = table[((bits >> (i * 3)) & 7) as usize] as u8;
    }
}

impl Rgba {
    /// Halve the picture by averaging each 2x2 square of pixels.
    pub fn half(&self) -> Rgba {
        let (w, h) = ((self.width / 2).max(1), (self.height / 2).max(1));
        let mut pixels = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(self.width - 1);
                        let sy = (y * 2 + dy).min(self.height - 1);
                        sum += self.pixels[(sy * self.width + sx) * 4 + c] as u32;
                    }
                    pixels[(y * w + x) * 4 + c] = (sum / 4) as u8;
                }
            }
        }
        Rgba {
            width: w,
            height: h,
            pixels,
        }
    }

    /// Average difference per colour channel (0-255) between two pictures of
    /// the same size. Alpha is ignored. `None` if the sizes differ.
    pub fn mean_difference(&self, other: &Rgba) -> Option<f64> {
        if self.width != other.width || self.height != other.height {
            return None;
        }
        let mut sum = 0u64;
        for (a, b) in self
            .pixels
            .chunks_exact(4)
            .zip(other.pixels.chunks_exact(4))
        {
            for c in 0..3 {
                sum += (a[c] as i32 - b[c] as i32).unsigned_abs() as u64;
            }
        }
        Some(sum as f64 / (self.width * self.height * 3) as f64)
    }

    /// Average red, green, blue and alpha.
    pub fn mean(&self) -> [f64; 4] {
        let mut sum = [0u64; 4];
        for p in self.pixels.chunks_exact(4) {
            for c in 0..4 {
                sum[c] += p[c] as u64;
            }
        }
        let n = (self.width * self.height).max(1) as f64;
        sum.map(|s| s as f64 / n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dxt1_solid_red_block() {
        // c0 = pure red (0xF800), c1 = black, every texel index 0.
        let block = [0x00, 0xF8, 0x00, 0x00, 0, 0, 0, 0];
        let img = decode(PixelFormat::Dxt1, 4, 4, &block, None).unwrap();
        assert!(img.pixels.chunks(4).all(|p| p == [255, 0, 0, 255]));
    }

    #[test]
    fn dxt1_transparent_mode() {
        // c0 <= c1 selects 3 colours + transparent; index 3 is transparent.
        let block = [0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        let img = decode(PixelFormat::Dxt1, 4, 4, &block, None).unwrap();
        assert!(img.pixels.chunks(4).all(|p| p[3] == 0));
    }

    #[test]
    fn dxt5_alpha_endpoints() {
        // a0 = 200, a1 = 100, all indices 1 -> alpha 100. Colour white.
        let mut block = [0u8; 16];
        block[0] = 200;
        block[1] = 100;
        let bits: u64 = (0..16).map(|i| 1u64 << (3 * i)).sum();
        block[2..8].copy_from_slice(&bits.to_le_bytes()[..6]);
        block[8..10].copy_from_slice(&0xFFFFu16.to_le_bytes());
        let img = decode(PixelFormat::Dxt5, 4, 4, &block, None).unwrap();
        assert!(
            img.pixels.chunks(4).all(|p| p == [255, 255, 255, 100]),
            "{:?}",
            &img.pixels[..8]
        );
    }

    #[test]
    fn bgra_is_swapped_to_rgba() {
        let img = decode(PixelFormat::Bgra8, 1, 1, &[1, 2, 3, 4], None).unwrap();
        assert_eq!(img.pixels, [3, 2, 1, 4]);
    }

    #[test]
    fn palette_lookup() {
        let mut pal = vec![[0, 0, 0, 255]; 256];
        pal[7] = [10, 20, 30, 255];
        let img = decode(PixelFormat::P8, 2, 1, &[7, 0], Some(&pal)).unwrap();
        assert_eq!(img.pixels, [10, 20, 30, 255, 0, 0, 0, 255]);
    }

    #[test]
    fn half_averages() {
        let img = Rgba {
            width: 2,
            height: 2,
            pixels: vec![
                0, 0, 0, 0, 100, 100, 100, 100, 0, 0, 0, 0, 100, 100, 100, 100,
            ],
        };
        assert_eq!(img.half().pixels, [50, 50, 50, 50]);
    }
}
