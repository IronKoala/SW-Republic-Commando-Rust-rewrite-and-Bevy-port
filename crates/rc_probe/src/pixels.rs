//! Decodes texture pixels into RGBA and measures whether the result makes sense.
//!
//! We can't look at the pictures, so we check them with numbers: a texture's
//! second mipmap is the first one shrunk to half size by the game's tools. If we
//! decode both correctly, shrinking our decoded mip 0 should give nearly the
//! same picture as our decoded mip 1. If the decoding were wrong, the two would
//! look like unrelated noise and differ a lot.

use rc_image::{PixelFormat, Rgba};
use rc_package::texture::{read_palette, Texture};
use rc_package::{Export, ObjectRef, Package};

pub(crate) struct Decoded {
    pub mip0: Rgba,
    /// Average per-channel difference (0-255) between mip 1 and mip 0 shrunk
    /// to half size. `None` if there's no usable mip 1.
    pub mip_difference: Option<f64>,
}

pub(crate) enum Outcome {
    Decoded(Decoded),
    /// Not decodable yet, with the reason.
    Skipped(String),
    Failed(String),
}

fn pixel_format(format: u8) -> Option<PixelFormat> {
    Some(match format {
        0 => PixelFormat::P8,
        3 => PixelFormat::Dxt1,
        5 => PixelFormat::Bgra8,
        7 => PixelFormat::Dxt3,
        8 => PixelFormat::Dxt5,
        10 => PixelFormat::G16,
        _ => return None,
    })
}

fn palette_for(pkg: &Package, data: &[u8], tex: &Texture) -> Result<Option<Vec<[u8; 4]>>, String> {
    let Some(index) = tex.palette_ref(pkg) else {
        return Ok(None);
    };
    let export: &Export = match ObjectRef::from_index(index) {
        ObjectRef::Export(i) => &pkg.exports[i],
        _ => return Err("palette is in another package (not followed yet)".into()),
    };
    let mut colors = read_palette(pkg, data, export).map_err(|e| format!("palette: {e}"))?;
    if tex.masked(pkg) {
        colors[0][3] = 0;
    }
    Ok(Some(colors))
}

pub(crate) fn decode(pkg: &Package, data: &[u8], tex: &Texture) -> Outcome {
    let Some(format) = pixel_format(tex.format) else {
        return Outcome::Skipped(format!("format {} not decoded yet", tex.format));
    };
    let palette = match palette_for(pkg, data, tex) {
        Ok(p) => p,
        Err(e) => return Outcome::Skipped(e),
    };
    let decode_mip = |i: usize| {
        let m = &tex.mips[i];
        let bytes = &data[m.data_offset..m.data_offset + m.data_len];
        rc_image::decode(
            format,
            m.width.max(1) as usize,
            m.height.max(1) as usize,
            bytes,
            palette.as_deref(),
        )
    };
    let mip0 = match decode_mip(0) {
        Ok(img) => img,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    let mip_difference = match tex.mips.get(1) {
        Some(m) if m.width >= 4 && m.height >= 4 => match decode_mip(1) {
            Ok(mip1) => mip0.half().mean_difference(&mip1),
            Err(e) => return Outcome::Failed(format!("mip 1: {e}")),
        },
        _ => None,
    };
    Outcome::Decoded(Decoded {
        mip0,
        mip_difference,
    })
}
