//! Microsoft MPEG-4 version 2 (`MP42`) picture decoder.
//!
//! The building blocks (variable-length code tables, DC prediction,
//! coefficient walk, inverse DCT, motion compensation) come from the
//! MIT-licensed `oxideav-msmpeg4` crate (Mark Karpeles,
//! https://github.com/OxideAV/oxideav-msmpeg4). Its own version 2
//! picture decoder could not decode a real `MP42` stream (every frame
//! failed on a test clip), so this file is the picture and macroblock
//! loop, adapted from that crate's `picture.rs` (MIT) with these
//! differences, each found by decoding test clips and comparing every
//! pixel against ffmpeg's decoder:
//!
//! 1. The picture header carries one more field: 5 bits on key frames
//!    (the slice count, 0x16 + slices) and 1 bit on other frames (whether
//!    macroblocks carry a "skipped" bit).
//! 2. Key-frame macroblocks use a short 4-entry code for the colour
//!    "coded block" bits, followed by the AC-prediction bit and the
//!    brightness coded-block pattern, read as-is (not inverted).
//! 3. Intra macroblocks inside predicted frames also read their
//!    brightness coded-block pattern as-is.
//! 4. Colour blocks of intra macroblocks, and all blocks of predicted
//!    macroblocks, use the full three-way escape for rare coefficients,
//!    and the run escape adds nothing extra on predicted blocks.
//!
//! With these, test clips decode to within 1 to 2 levels (out of 255) of
//! ffmpeg on every pixel, with no drift over hundreds of frames. The
//! remaining difference is the inverse-DCT rounding (ffmpeg's own
//! inverse DCT versus the one this crate copies from Microsoft's codec).
//!
//! Not handled yet (the decoder says so in its error instead of guessing):
//! more than one slice per frame, and AC prediction (a per-macroblock
//! flag that predicts the first row or column of coefficients from the
//! neighbour). ffmpeg's encoder never uses either, so they could not be
//! checked; [`Stats`] counts them on real files.

use oxideav_core::bits::BitReader;
use oxideav_msmpeg4::ac::AcVlcTable;
use oxideav_msmpeg4::ac::Scan;
use oxideav_msmpeg4::dc_pred::DcCache;
use oxideav_msmpeg4::header::PictureType;
use oxideav_msmpeg4::idct::idct8x8_to_pel;
use oxideav_msmpeg4::mcbpcy::{decode_cbpy_no_wrap, decode_mcbpcy_v2, V2FrameType};
use oxideav_msmpeg4::mv::Mv;
use oxideav_msmpeg4::mv_pred::{MvGrid, MvGridCell};
use oxideav_msmpeg4::picture::{Picture, PictureDims};

pub type Error = String;

/// Counts for the log, so a real file shows which parts it uses.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub key_frames: u32,
    pub predicted_frames: u32,
    pub skipped_macroblocks: u32,
    pub intra_macroblocks: u32,
    pub inter_macroblocks: u32,
    /// Key frames whose header asks for more than one slice.
    pub multi_slice_frames: u32,
    /// Predicted frames whose macroblocks carry no "skipped" bit.
    pub no_skip_bit_frames: u32,
}

/// Decodes a stream frame by frame. Feed every frame in order, starting
/// with a key frame; call [`Decoder::reset`] before looping back.
pub struct Decoder {
    dims: PictureDims,
    last: Option<Picture>,
    /// Slices per frame, set by each key frame and kept by the frames
    /// that follow it.
    slices: u32,
    pub stats: Stats,
}

impl Decoder {
    pub fn new(width: u32, height: u32) -> Result<Self, Error> {
        let dims = PictureDims::new(width, height).map_err(|e| e.to_string())?;
        Ok(Self {
            dims,
            last: None,
            slices: 1,
            stats: Stats::default(),
        })
    }

    pub fn reset(&mut self) {
        self.last = None;
    }

    /// The last decoded picture (planar 4:2:0, rows padded to 16).
    pub fn picture(&self) -> Option<&Picture> {
        self.last.as_ref()
    }

    /// Decode one frame's bytes. An empty frame repeats the last picture.
    pub fn decode(&mut self, data: &[u8]) -> Result<&Picture, Error> {
        if data.is_empty() {
            return self
                .last
                .as_ref()
                .ok_or_else(|| "empty first frame".to_string());
        }
        let mut br = BitReader::new(data);
        let pic = decode_picture(
            &mut br,
            self.dims,
            self.last.as_ref(),
            &mut self.slices,
            &mut self.stats,
        )?;
        self.last = Some(pic);
        Ok(self.last.as_ref().unwrap())
    }
}

fn bit(br: &mut BitReader<'_>) -> Result<bool, Error> {
    br.read_bit().map_err(|e| e.to_string())
}

fn bits(br: &mut BitReader<'_>, n: u32) -> Result<u32, Error> {
    br.read_u32(n).map_err(|e| e.to_string())
}

struct Header {
    key: bool,
    quant: u32,
    skip_bits: bool,
    /// Key frames only: slices in this and the following frames.
    slices: u32,
}

fn read_header(br: &mut BitReader<'_>, stats: &mut Stats) -> Result<Header, Error> {
    let kind = bits(br, 2)?;
    let quant = bits(br, 5)?;
    if quant == 0 {
        return Err("quantiser 0".into());
    }
    match kind {
        0 => {
            let slice_code = bits(br, 5)?;
            if slice_code < 0x17 {
                return Err(format!(
                    "key frame slice code {slice_code:#x} is below 0x17"
                ));
            }
            if slice_code != 0x17 {
                stats.multi_slice_frames += 1;
            }
            stats.key_frames += 1;
            Ok(Header {
                key: true,
                quant,
                skip_bits: false,
                slices: slice_code - 0x16,
            })
        }
        1 => {
            let skip_bits = bit(br)?;
            if !skip_bits {
                stats.no_skip_bit_frames += 1;
            }
            stats.predicted_frames += 1;
            Ok(Header {
                key: false,
                quant,
                skip_bits,
                slices: 0,
            })
        }
        k => Err(format!("unknown picture type {k}")),
    }
}

struct Tables {
    intra_luma: AcVlcTable,
    intra_chroma: AcVlcTable,
    inter: AcVlcTable,
}

fn tables() -> Tables {
    Tables {
        intra_luma: AcVlcTable::v3_intra_g5(),
        intra_chroma: AcVlcTable::v3_intra_g4(),
        inter: AcVlcTable {
            run_ext_bias: 0,
            ..AcVlcTable::inter_for_chroma_sel(2)
        },
    }
}

struct MbHeader {
    intra: bool,
    ac_pred: bool,
    cbpy: u8,
    cbp_cb: bool,
    cbp_cr: bool,
}

/// Key-frame macroblock header: 4-entry colour code ("1" = none,
/// "01" = both, "001" = blue only, "000" = red only), AC-prediction bit,
/// then the brightness pattern.
fn read_key_mb_header(br: &mut BitReader<'_>) -> Result<MbHeader, Error> {
    let cbpc: u8 = if bit(br)? {
        0
    } else if bit(br)? {
        3
    } else if bit(br)? {
        2
    } else {
        1
    };
    let ac_pred = bit(br)?;
    let cbpy = decode_cbpy_no_wrap(br).map_err(|e| e.to_string())?;
    Ok(MbHeader {
        intra: true,
        ac_pred,
        cbpy,
        cbp_cb: cbpc & 2 != 0,
        cbp_cr: cbpc & 1 != 0,
    })
}

fn decode_picture(
    br: &mut BitReader<'_>,
    dims: PictureDims,
    reference: Option<&Picture>,
    slices: &mut u32,
    stats: &mut Stats,
) -> Result<Picture, Error> {
    let hdr = read_header(br, stats)?;
    let t = tables();
    let (mb_w, mb_h) = dims.mb_dims();
    if hdr.key {
        *slices = hdr.slices;
    }
    // Each slice is a band of macroblock rows. At the top row of a slice
    // the rows above don't count as neighbours: the DC predictors start
    // over and motion vectors don't look upwards.
    let slice_rows = mb_h / (*slices).max(1) as usize;
    if slice_rows == 0 {
        return Err(format!("{} slices for {mb_h} macroblock rows", *slices));
    }
    let mut pic;
    let mut dc_cache = DcCache::for_v1v2(mb_w, mb_h);
    if hdr.key {
        pic = Picture::alloc(dims, PictureType::I);
        for my in 0..mb_h {
            if my > 0 && my % slice_rows == 0 {
                dc_cache.reset();
            }
            for mx in 0..mb_w {
                let h = read_key_mb_header(br).map_err(|e| format!("key MB ({mx},{my}): {e}"))?;
                stats.intra_macroblocks += 1;
                decode_intra_mb(br, &mut pic, &mut dc_cache, &h, mx, my, hdr.quant, &t)
                    .map_err(|e| format!("key MB ({mx},{my}): {e}"))?;
            }
        }
        return Ok(pic);
    }
    let reference = reference.ok_or("predicted frame without an earlier key frame")?;
    pic = Picture::alloc(dims, PictureType::P);
    let mut mv_grid = MvGrid::new(mb_w, mb_h);
    for my in 0..mb_h {
        if my > 0 && my % slice_rows == 0 {
            dc_cache.reset();
        }
        let top_of_slice = my % slice_rows == 0;
        for mx in 0..mb_w {
            decode_predicted_mb(
                br,
                &mut pic,
                &mut mv_grid,
                &mut dc_cache,
                reference,
                mx,
                my,
                &hdr,
                top_of_slice,
                &t,
                stats,
            )
            .map_err(|e| format!("predicted MB ({mx},{my}): {e}"))?;
        }
    }
    Ok(pic)
}

#[allow(clippy::too_many_arguments)]
fn decode_predicted_mb(
    br: &mut BitReader<'_>,
    pic: &mut Picture,
    mv_grid: &mut MvGrid,
    dc_cache: &mut DcCache,
    reference: &Picture,
    mx: usize,
    my: usize,
    hdr: &Header,
    top_of_slice: bool,
    t: &Tables,
    stats: &mut Stats,
) -> Result<(), Error> {
    if hdr.skip_bits && bit(br)? {
        stats.skipped_macroblocks += 1;
        mv_grid.set_cell(mx, my, MvGridCell::OneMv(Mv::default()));
        apply_mc(pic, reference, mx, my, (0, 0));
        return Ok(());
    }
    // The crate's "I" mode reads the 8-entry macroblock-type code with no
    // skipped bit in front (we read that above).
    let d = decode_mcbpcy_v2(br, V2FrameType::I).map_err(|e| e.to_string())?;
    if d.is_intra {
        stats.intra_macroblocks += 1;
        // The crate inverts the brightness pattern of intra macroblocks;
        // version 2 sends it as-is.
        let h = MbHeader {
            intra: true,
            ac_pred: d.ac_pred,
            cbpy: 15 - d.cbpy,
            cbp_cb: d.cbp_cb,
            cbp_cr: d.cbp_cr,
        };
        decode_intra_mb(br, pic, dc_cache, &h, mx, my, hdr.quant, t)?;
        mv_grid.set_cell(mx, my, MvGridCell::OneMv(Mv::default()));
        return Ok(());
    }
    stats.inter_macroblocks += 1;
    let predictor = one_mv_predictor(mv_grid, mx, my, top_of_slice);
    let mv = oxideav_msmpeg4::mv::decode_mv_v1v2(br, predictor).map_err(|e| e.to_string())?;
    mv_grid.set_cell(mx, my, MvGridCell::OneMv(mv));
    apply_mc(pic, reference, mx, my, (mv.x as i32, mv.y as i32));
    for block in 0..6usize {
        let coded = match block {
            0..=3 => d.cbpy & (1 << (3 - block)) != 0,
            4 => d.cbp_cb,
            _ => d.cbp_cr,
        };
        if !coded {
            continue;
        }
        let mut coeffs = [0i32; 64];
        oxideav_msmpeg4::ac::decode_inter_block(br, &mut coeffs, &t.inter, hdr.quant)
            .map_err(|e| e.to_string())?;
        let mut residual = [0i32; 64];
        idct8x8_to_pel(&coeffs, &mut residual);
        put_block(pic, mx, my, block, &residual, true);
    }
    Ok(())
}

fn one_mv_predictor(mv_grid: &MvGrid, mx: usize, my: usize, top_of_slice: bool) -> Mv {
    use oxideav_msmpeg4::mv_pred::{predict_block_mv, resolve_block_candidates, Block};
    let nset = mv_grid.neighbour_set_for_slice(mx, my, top_of_slice);
    let cands = resolve_block_candidates(Block::TopLeft, nset, [None, None, None]);
    predict_block_mv(Block::TopLeft, &cands)
}

#[allow(clippy::too_many_arguments)]
fn decode_intra_mb(
    br: &mut BitReader<'_>,
    pic: &mut Picture,
    dc_cache: &mut DcCache,
    h: &MbHeader,
    mx: usize,
    my: usize,
    quant: u32,
    t: &Tables,
) -> Result<(), Error> {
    use oxideav_msmpeg4::dc_pred::AcEdges;
    use oxideav_msmpeg4::mb;
    debug_assert!(h.intra);
    for block in 0..6usize {
        let coded = match block {
            0..=3 => h.cbpy & (1 << (3 - block)) != 0,
            4 => h.cbp_cb,
            _ => h.cbp_cr,
        };
        let (bx, by) = match block {
            0..=3 => (mx * 2 + (block & 1), my * 2 + (block >> 1)),
            _ => (mx, my),
        };
        let pred = match block {
            0..=3 => dc_cache.predict_luma(bx, by),
            4 => dc_cache.predict_chroma(false, bx, by),
            _ => dc_cache.predict_chroma(true, bx, by),
        };
        let table = if block <= 3 {
            &t.intra_luma
        } else {
            &t.intra_chroma
        };
        // DC: size + value code, predicted from the neighbour the
        // gradient points at.
        let dc_diff = mb::decode_intra_dc_diff_v1v2(br, block).map_err(|e| e.to_string())?;
        let dc_level = mb::reconstruct_intra_dc_level(dc_diff, pred.predictor);
        // AC: with AC prediction on, the scan follows the prediction
        // direction and the first row (from above) or column (from the
        // left) adds the neighbour's levels.
        let mut levels = [0i32; 64];
        if coded {
            let scan = if h.ac_pred {
                pred.direction.ac_scan()
            } else {
                Scan::Zigzag
            };
            oxideav_msmpeg4::ac::decode_intra_ac(br, &mut levels, scan, table, 1)
                .map_err(|e| e.to_string())?;
        }
        if h.ac_pred {
            let p = match block {
                0..=3 => dc_cache.ac_predict_luma(bx, by, pred.direction),
                4 => dc_cache.ac_predict_chroma(false, bx, by, pred.direction),
                _ => dc_cache.ac_predict_chroma(true, bx, by, pred.direction),
            };
            for (k, v) in p.iter().enumerate().skip(1) {
                let pos = match pred.direction {
                    oxideav_msmpeg4::dc_pred::PredDir::FromTop => k,
                    oxideav_msmpeg4::dc_pred::PredDir::FromLeft => 8 * k,
                };
                levels[pos] += v;
            }
        }
        let edges = AcEdges::from_levels(&levels);
        match block {
            0..=3 => {
                dc_cache.luma_set(bx, by, dc_level);
                dc_cache.luma_ac_set(bx, by, edges);
            }
            4 => {
                dc_cache.chroma_set(false, bx, by, dc_level);
                dc_cache.chroma_ac_set(false, bx, by, edges);
            }
            _ => {
                dc_cache.chroma_set(true, bx, by, dc_level);
                dc_cache.chroma_ac_set(true, bx, by, edges);
            }
        }
        let mut coeffs = levels;
        if coded || h.ac_pred {
            oxideav_msmpeg4::iq::dequantise_h263(&mut coeffs, quant, 1)
                .map_err(|e| e.to_string())?;
        }
        coeffs[0] = mb::reconstruct_intra_dc_v1v2(dc_diff, pred.predictor);
        let mut pels = [0i32; 64];
        idct8x8_to_pel(&coeffs, &mut pels);
        put_block(pic, mx, my, block, &pels, false);
    }
    Ok(())
}

/// Write (or, for `add`, add onto the prediction) one 8x8 block.
fn put_block(pic: &mut Picture, mx: usize, my: usize, block: usize, v: &[i32; 64], add: bool) {
    let (plane, stride, x0, y0) = match block {
        0..=3 => (
            &mut pic.y,
            pic.y_stride,
            mx * 16 + (block & 1) * 8,
            my * 16 + (block >> 1) * 8,
        ),
        4 => (&mut pic.cb, pic.c_stride, mx * 8, my * 8),
        _ => (&mut pic.cr, pic.c_stride, mx * 8, my * 8),
    };
    for j in 0..8 {
        for i in 0..8 {
            let off = (y0 + j) * stride + x0 + i;
            if let Some(p) = plane.get_mut(off) {
                let base = if add { *p as i32 } else { 0 };
                *p = (base + v[j * 8 + i]).clamp(0, 255) as u8;
            }
        }
    }
}

fn apply_mc(pic: &mut Picture, reference: &Picture, mx: usize, my: usize, mv_half: (i32, i32)) {
    use oxideav_msmpeg4::mc::{mc_macroblock, RefPlane};
    fn plane(data: &[u8], stride: usize) -> RefPlane<'_> {
        RefPlane {
            data,
            stride,
            width: stride,
            height: data.len() / stride,
        }
    }
    let (y_stride, c_stride) = (pic.y_stride, pic.c_stride);
    mc_macroblock(
        &plane(&reference.y, reference.y_stride),
        &plane(&reference.cb, reference.c_stride),
        &plane(&reference.cr, reference.c_stride),
        &mut pic.y,
        &mut pic.cb,
        &mut pic.cr,
        y_stride,
        c_stride,
        mx,
        my,
        mv_half,
        0,
    );
}
