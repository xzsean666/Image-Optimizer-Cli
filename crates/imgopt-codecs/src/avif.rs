use std::io::Cursor;
use std::mem::MaybeUninit;
use std::ptr::{self, NonNull};

use imgref::{Img, ImgRef};
use rav1d::include::dav1d::data::Dav1dData;
use rav1d::include::dav1d::dav1d::{Dav1dContext, Dav1dSettings};
use rav1d::include::dav1d::headers::*;
use rav1d::include::dav1d::picture::Dav1dPicture;
use rav1d::src::lib::{
    dav1d_close, dav1d_data_create, dav1d_data_unref, dav1d_default_settings, dav1d_get_picture,
    dav1d_open, dav1d_picture_unref, dav1d_send_data,
};
use rgb::{RGB8, RGBA8};
use yuv::{YuvPlanarImage, YuvRange, YuvStandardMatrix};

use crate::interface::{
    check_dimensions_against_limits, CodecCapabilities, CodecError, CodecFormat, DecodeLimits,
    DecodedPixels, EncodeOptions, EncodedOutput, ImageCodec, PixelFormat,
};

/// Codec adapter for AVIF image encoding (via `ravif`) and decoding (via `avif-parse` + pure-Rust `rav1d`).
#[derive(Debug, Default, Clone, Copy)]
pub struct AvifCodec;

impl ImageCodec for AvifCodec {
    fn capabilities(&self) -> CodecCapabilities {
        CodecCapabilities {
            format: CodecFormat::Avif,
            supports_lossless: false,
            supports_lossy: true,
            supports_alpha: true,
        }
    }

    fn decode(&self, input_bytes: &[u8]) -> Result<DecodedPixels, CodecError> {
        self.decode_with_limits(input_bytes, &DecodeLimits::default())
    }

    fn decode_with_limits(
        &self,
        input_bytes: &[u8],
        limits: &DecodeLimits,
    ) -> Result<DecodedPixels, CodecError> {
        if input_bytes.is_empty() {
            return Err(CodecError::DecodeError("empty AVIF input bytes".into()));
        }

        let avif = avif_parse::read_avif(&mut Cursor::new(input_bytes))
            .map_err(|e| CodecError::DecodeError(format!("failed to parse AVIF container: {e}")))?;

        let meta = avif.primary_item_metadata().map_err(|e| {
            CodecError::DecodeError(format!("failed to parse AVIF sequence header: {e}"))
        })?;
        let width = meta.max_frame_width.get();
        let height = meta.max_frame_height.get();
        check_dimensions_against_limits(width, height, limits)?;

        let has_alpha = avif.alpha_item.is_some();
        let channels = if has_alpha {
            4u64
        } else if meta.monochrome {
            1u64
        } else {
            3u64
        };
        if let Some(max_alloc) = limits.max_alloc_bytes {
            let needed = u64::from(width)
                .saturating_mul(u64::from(height))
                .saturating_mul(channels);
            if needed > max_alloc {
                return Err(CodecError::ResourceLimitExceeded(format!(
                    "AVIF decoded buffer size {needed} bytes exceeds limit of {max_alloc} bytes"
                )));
            }
        }

        let threads = std::thread::available_parallelism()
            .map_or(2, |n| n.get())
            .min(8);
        let mut decoder = Rav1dDecoder::new(threads)?;
        let color_pic = decoder.decode_av1(&avif.primary_item)?;
        let out_w = color_pic.p.w.max(0) as u32;
        let out_h = color_pic.p.h.max(0) as u32;
        check_dimensions_against_limits(out_w, out_h, limits)?;

        let (pixel_format, mut pixel_data) = convert_rav1d_picture_to_8bit(&color_pic, has_alpha)?;

        if let Some(ref alpha_av1) = avif.alpha_item {
            let alpha_pic = decoder.decode_av1(alpha_av1)?;
            apply_alpha_plane(
                &alpha_pic,
                out_w,
                out_h,
                avif.premultiplied_alpha,
                &mut pixel_data,
            )?;
        }

        Ok(DecodedPixels::new(out_w, out_h, pixel_format, pixel_data))
    }

    fn encode(
        &self,
        pixels: &DecodedPixels,
        options: &EncodeOptions,
    ) -> Result<EncodedOutput, CodecError> {
        if !pixels.is_valid() {
            return Err(CodecError::InvalidParameter(
                "invalid pixel buffer dimensions/data size".into(),
            ));
        }

        let width = pixels.width as usize;
        let height = pixels.height as usize;
        let quality = f32::from(options.quality.clamp(1, 100));
        let speed = map_effort_to_ravif_speed(options.effort);

        let encoder = ravif::Encoder::new()
            .with_quality(quality)
            .with_alpha_quality(quality)
            .with_speed(speed)
            .with_num_threads(Some(
                std::thread::available_parallelism()
                    .map_or(2, |n| n.get())
                    .min(4),
            ));

        let encoded = match pixels.pixel_format {
            PixelFormat::Rgb8 => {
                let rgb_pixels: Vec<RGB8> = pixels
                    .data
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|c| RGB8::new(c[0], c[1], c[2]))
                    .collect();
                encoder
                    .encode_rgb(Img::new(&rgb_pixels, width, height))
                    .map_err(|e| CodecError::EncodeError(format!("AVIF RGB encode error: {e}")))?
            }
            PixelFormat::Rgba8 => {
                let rgba_pixels: Vec<RGBA8> = pixels
                    .data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| RGBA8::new(c[0], c[1], c[2], c[3]))
                    .collect();
                encoder
                    .encode_rgba(Img::new(&rgba_pixels, width, height))
                    .map_err(|e| CodecError::EncodeError(format!("AVIF RGBA encode error: {e}")))?
            }
            PixelFormat::Grayscale8 => {
                let rgb_pixels: Vec<RGB8> =
                    pixels.data.iter().map(|&l| RGB8::new(l, l, l)).collect();
                encoder
                    .encode_rgb(Img::new(&rgb_pixels, width, height))
                    .map_err(|e| {
                        CodecError::EncodeError(format!("AVIF grayscale encode error: {e}"))
                    })?
            }
            PixelFormat::GrayscaleAlpha8 => {
                let rgba_pixels: Vec<RGBA8> = pixels
                    .data
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| RGBA8::new(c[0], c[0], c[0], c[1]))
                    .collect();
                encoder
                    .encode_rgba(Img::new(&rgba_pixels, width, height))
                    .map_err(|e| CodecError::EncodeError(format!("AVIF LA encode error: {e}")))?
            }
        };

        if encoded.avif_file.is_empty() {
            return Err(CodecError::EncodeError(
                "AVIF encoding produced empty output".into(),
            ));
        }

        Ok(EncodedOutput {
            format: CodecFormat::Avif,
            bytes: encoded.avif_file,
            width: pixels.width,
            height: pixels.height,
            effective_options: options.clone(),
        })
    }
}

fn map_effort_to_ravif_speed(effort: u8) -> u8 {
    match effort.clamp(1, 6) {
        1 => 10,
        2 => 8,
        3 => 7,
        4 => 6,
        5 => 4,
        _ => 3,
    }
}

struct Rav1dDecoder {
    ctx: Option<Dav1dContext>,
}

impl Rav1dDecoder {
    fn new(threads: usize) -> Result<Self, CodecError> {
        unsafe {
            let mut settings = MaybeUninit::<Dav1dSettings>::uninit();
            dav1d_default_settings(NonNull::from(&mut settings).cast());
            let settings = settings.assume_init_mut();
            settings.n_threads = threads.min(64) as _;
            settings.max_frame_delay = 1;
            settings.apply_grain = 1;

            let mut ctx = None;
            let res = dav1d_open(Some(NonNull::from(&mut ctx)), Some(NonNull::from(settings)));
            if res.0 != 0 {
                return Err(CodecError::DecodeError(format!(
                    "rav1d open failed ({})",
                    res.0.unsigned_abs()
                )));
            }
            Ok(Self { ctx })
        }
    }

    fn decode_av1(&mut self, av1_data: &[u8]) -> Result<OwnedDav1dPicture, CodecError> {
        let ctx = self
            .ctx
            .ok_or_else(|| CodecError::DecodeError("AV1 decoder closed".into()))?;
        unsafe {
            let mut data = Dav1dData::default();
            let mut last_pic: Option<OwnedDav1dPicture> = None;

            let send_res = (|| -> Result<(), CodecError> {
                let buf = dav1d_data_create(Some(NonNull::from(&mut data)), av1_data.len());
                let Some(buf) = NonNull::new(buf) else {
                    return Err(CodecError::ResourceLimitExceeded(
                        "failed to allocate AV1 input buffer".into(),
                    ));
                };
                ptr::copy_nonoverlapping(av1_data.as_ptr(), buf.as_ptr(), av1_data.len());

                loop {
                    let res = dav1d_send_data(Some(ctx), Some(NonNull::from(&mut data)));
                    if res.0 == 0 {
                        return Ok(());
                    }
                    let mut pending = Dav1dPicture::default();
                    let drain_res = dav1d_get_picture(Some(ctx), Some(NonNull::from(&mut pending)));
                    let owned = OwnedDav1dPicture(pending);
                    if drain_res.0 != 0 {
                        return Err(CodecError::DecodeError(format!(
                            "rav1d send_data failed ({})",
                            res.0.unsigned_abs()
                        )));
                    }
                    last_pic = Some(owned);
                }
            })();

            dav1d_data_unref(Some(NonNull::from(&mut data)));
            send_res?;

            loop {
                let mut pic = Dav1dPicture::default();
                let res = dav1d_get_picture(Some(ctx), Some(NonNull::from(&mut pic)));
                let owned = OwnedDav1dPicture(pic);
                if res.0 != 0 {
                    if let Some(final_pic) = last_pic {
                        return Ok(final_pic);
                    }
                    return Err(CodecError::DecodeError(format!(
                        "rav1d get_picture failed ({})",
                        res.0.unsigned_abs()
                    )));
                }
                last_pic = Some(owned);
            }
        }
    }
}

impl Drop for Rav1dDecoder {
    fn drop(&mut self) {
        unsafe { dav1d_close(Some(NonNull::from(&mut self.ctx))) };
    }
}

struct OwnedDav1dPicture(Dav1dPicture);

impl std::ops::Deref for OwnedDav1dPicture {
    type Target = Dav1dPicture;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for OwnedDav1dPicture {
    fn drop(&mut self) {
        unsafe { dav1d_picture_unref(Some(NonNull::from(&mut self.0))) };
    }
}

unsafe fn plane_slice<'a, T: Copy>(
    pic: &'a Dav1dPicture,
    plane_idx: usize,
) -> Result<ImgRef<'a, T>, CodecError> {
    let pixel_bytes = if pic.p.bpc <= 8 { 1 } else { 2 };
    if pixel_bytes != std::mem::size_of::<T>() {
        return Err(CodecError::DecodeError(
            "AVIF plane bitdepth mismatch".into(),
        ));
    }
    let plane = pic.data[plane_idx]
        .ok_or_else(|| CodecError::DecodeError("AVIF missing image plane".into()))?;
    let (ss_h, ss_v) = match (plane_idx != 0, pic.p.layout) {
        (false, _) | (true, DAV1D_PIXEL_LAYOUT_I444) => (0u8, 0u8),
        (true, DAV1D_PIXEL_LAYOUT_I420) => (1, 1),
        (true, DAV1D_PIXEL_LAYOUT_I422) => (1, 0),
        _ => {
            return Err(CodecError::UnsupportedFormat(
                "unsupported AVIF pixel layout".into(),
            ))
        }
    };
    let w = (pic.p.w as usize + usize::from(ss_h)) >> ss_h;
    let h = (pic.p.h as usize + usize::from(ss_v)) >> ss_v;
    let stride_bytes = pic.stride[usize::from(plane_idx != 0)];
    if stride_bytes <= 0 || !(stride_bytes as usize).is_multiple_of(pixel_bytes) {
        return Err(CodecError::DecodeError("invalid AVIF plane stride".into()));
    }
    let stride = stride_bytes as usize / pixel_bytes;
    if stride < w || w == 0 {
        return Err(CodecError::DecodeError(
            "invalid AVIF plane stride width".into(),
        ));
    }
    let buf = unsafe { std::slice::from_raw_parts(plane.as_ptr().cast::<T>(), stride * h) };
    Ok(ImgRef::new_stride(buf, w, h, stride))
}

fn rav1d_range(pic: &Dav1dPicture) -> YuvRange {
    let seq = pic.seq_hdr.map(|p| unsafe { p.as_ref() });
    if seq.is_some_and(|s| s.color_range != 0) {
        YuvRange::Full
    } else {
        YuvRange::Limited
    }
}

fn rav1d_matrix(pic: &Dav1dPicture) -> Result<YuvStandardMatrix, CodecError> {
    let seq = pic.seq_hdr.map(|p| unsafe { p.as_ref() });
    match seq.map(|s| s.mtrx).unwrap_or(DAV1D_MC_BT601) {
        DAV1D_MC_BT709 => Ok(YuvStandardMatrix::Bt709),
        DAV1D_MC_FCC => Ok(YuvStandardMatrix::Fcc),
        DAV1D_MC_BT470BG | DAV1D_MC_BT601 => Ok(YuvStandardMatrix::Bt601),
        DAV1D_MC_SMPTE240 => Ok(YuvStandardMatrix::Smpte240),
        DAV1D_MC_BT2020_NCL | DAV1D_MC_BT2020_CL => Ok(YuvStandardMatrix::Bt2020),
        _ => Ok(YuvStandardMatrix::Bt601),
    }
}

fn scale_luma_to_u8(val: u16, bits: u32, range: YuvRange) -> u8 {
    let (min, span) = match range {
        YuvRange::Full => (0u32, (1u32 << bits) - 1),
        YuvRange::Limited => (16u32 << (bits - 8), 219u32 << (bits - 8)),
    };
    let clamped = u32::from(val).saturating_sub(min).min(span);
    ((clamped * 255 + span / 2) / span) as u8
}

fn convert_rav1d_picture_to_8bit(
    pic: &Dav1dPicture,
    has_alpha: bool,
) -> Result<(PixelFormat, Vec<u8>), CodecError> {
    let w = pic.p.w as usize;
    let h = pic.p.h as usize;
    let range = rav1d_range(pic);

    if pic.p.layout == DAV1D_PIXEL_LAYOUT_I400 {
        let mut out = Vec::with_capacity(w * h * if has_alpha { 4 } else { 1 });
        if pic.p.bpc <= 8 {
            let y = unsafe { plane_slice::<u8>(pic, 0)? };
            for row in y.rows() {
                for &v in row {
                    let g = scale_luma_to_u8(u16::from(v), 8, range);
                    if has_alpha {
                        out.extend_from_slice(&[g, g, g, 255]);
                    } else {
                        out.push(g);
                    }
                }
            }
        } else {
            let bits = u32::from(pic.p.bpc as u8);
            let y = unsafe { plane_slice::<u16>(pic, 0)? };
            for row in y.rows() {
                for &v in row {
                    let g = scale_luma_to_u8(v, bits, range);
                    if has_alpha {
                        out.extend_from_slice(&[g, g, g, 255]);
                    } else {
                        out.push(g);
                    }
                }
            }
        }
        let fmt = if has_alpha {
            PixelFormat::Rgba8
        } else {
            PixelFormat::Grayscale8
        };
        return Ok((fmt, out));
    }

    let matrix = rav1d_matrix(pic)?;
    let seq = pic.seq_hdr.map(|p| unsafe { p.as_ref() });
    let is_identity = seq.is_some_and(|s| s.mtrx == DAV1D_MC_IDENTITY);

    if pic.p.bpc <= 8 {
        let y = unsafe { plane_slice::<u8>(pic, 0)? };
        let u = unsafe { plane_slice::<u8>(pic, 1)? };
        let v = unsafe { plane_slice::<u8>(pic, 2)? };
        let planar = YuvPlanarImage {
            y_plane: y.buf(),
            y_stride: y.stride() as u32,
            u_plane: u.buf(),
            u_stride: u.stride() as u32,
            v_plane: v.buf(),
            v_stride: v.stride() as u32,
            width: w as u32,
            height: h as u32,
        };
        let ch = if has_alpha { 4 } else { 3 };
        let mut out = vec![0u8; w * h * ch];
        let stride = (w * ch) as u32;
        let conv_res = match (is_identity, pic.p.layout, has_alpha) {
            (true, DAV1D_PIXEL_LAYOUT_I444, true) => {
                yuv::gbr_to_rgba(&planar, &mut out, stride, range)
            }
            (true, DAV1D_PIXEL_LAYOUT_I444, false) => {
                yuv::gbr_to_rgb(&planar, &mut out, stride, range)
            }
            (false, DAV1D_PIXEL_LAYOUT_I444, true) => {
                yuv::yuv444_to_rgba(&planar, &mut out, stride, range, matrix)
            }
            (false, DAV1D_PIXEL_LAYOUT_I444, false) => {
                yuv::yuv444_to_rgb(&planar, &mut out, stride, range, matrix)
            }
            (false, DAV1D_PIXEL_LAYOUT_I422, true) => {
                yuv::yuv422_to_rgba(&planar, &mut out, stride, range, matrix)
            }
            (false, DAV1D_PIXEL_LAYOUT_I422, false) => {
                yuv::yuv422_to_rgb(&planar, &mut out, stride, range, matrix)
            }
            (false, DAV1D_PIXEL_LAYOUT_I420, true) => {
                yuv::yuv420_to_rgba(&planar, &mut out, stride, range, matrix)
            }
            (false, DAV1D_PIXEL_LAYOUT_I420, false) => {
                yuv::yuv420_to_rgb(&planar, &mut out, stride, range, matrix)
            }
            _ => {
                return Err(CodecError::UnsupportedFormat(
                    "unsupported AVIF chroma subsampling".into(),
                ))
            }
        };
        conv_res
            .map_err(|e| CodecError::DecodeError(format!("AVIF YUV conversion failed: {e}")))?;
        return Ok((
            if has_alpha {
                PixelFormat::Rgba8
            } else {
                PixelFormat::Rgb8
            },
            out,
        ));
    }

    // 10/12/16-bit AVIF -> convert to u16 buffer then downscale cleanly to 8-bit
    let y = unsafe { plane_slice::<u16>(pic, 0)? };
    let u = unsafe { plane_slice::<u16>(pic, 1)? };
    let v = unsafe { plane_slice::<u16>(pic, 2)? };
    let planar = YuvPlanarImage {
        y_plane: y.buf(),
        y_stride: y.stride() as u32,
        u_plane: u.buf(),
        u_stride: u.stride() as u32,
        v_plane: v.buf(),
        v_stride: v.stride() as u32,
        width: w as u32,
        height: h as u32,
    };
    let ch = if has_alpha { 4 } else { 3 };
    let mut out16 = vec![0u16; w * h * ch];
    let stride = (w * ch) as u32;
    let bpc = pic.p.bpc;
    let conv_res = match (bpc, pic.p.layout, has_alpha) {
        (10, DAV1D_PIXEL_LAYOUT_I444, true) => {
            yuv::i410_to_rgba10(&planar, &mut out16, stride, range, matrix)
        }
        (10, DAV1D_PIXEL_LAYOUT_I444, false) => {
            yuv::i410_to_rgb10(&planar, &mut out16, stride, range, matrix)
        }
        (10, DAV1D_PIXEL_LAYOUT_I422, true) => {
            yuv::i210_to_rgba10(&planar, &mut out16, stride, range, matrix)
        }
        (10, DAV1D_PIXEL_LAYOUT_I422, false) => {
            yuv::i210_to_rgb10(&planar, &mut out16, stride, range, matrix)
        }
        (10, DAV1D_PIXEL_LAYOUT_I420, true) => {
            yuv::i010_to_rgba10(&planar, &mut out16, stride, range, matrix)
        }
        (10, DAV1D_PIXEL_LAYOUT_I420, false) => {
            yuv::i010_to_rgb10(&planar, &mut out16, stride, range, matrix)
        }
        (12, DAV1D_PIXEL_LAYOUT_I444, true) => {
            yuv::i412_to_rgba12(&planar, &mut out16, stride, range, matrix)
        }
        (12, DAV1D_PIXEL_LAYOUT_I444, false) => {
            yuv::i412_to_rgb12(&planar, &mut out16, stride, range, matrix)
        }
        (12, DAV1D_PIXEL_LAYOUT_I422, true) => {
            yuv::i212_to_rgba12(&planar, &mut out16, stride, range, matrix)
        }
        (12, DAV1D_PIXEL_LAYOUT_I422, false) => {
            yuv::i212_to_rgb12(&planar, &mut out16, stride, range, matrix)
        }
        (12, DAV1D_PIXEL_LAYOUT_I420, true) => {
            yuv::i012_to_rgba12(&planar, &mut out16, stride, range, matrix)
        }
        (12, DAV1D_PIXEL_LAYOUT_I420, false) => {
            yuv::i012_to_rgb12(&planar, &mut out16, stride, range, matrix)
        }
        _ => {
            return Err(CodecError::UnsupportedFormat(
                "unsupported AVIF high bit-depth layout".into(),
            ))
        }
    };
    conv_res
        .map_err(|e| CodecError::DecodeError(format!("AVIF 16-bit YUV conversion failed: {e}")))?;
    let shift = bpc.saturating_sub(8) as u32;
    let out8: Vec<u8> = out16
        .into_iter()
        .map(|v| (v >> shift).min(255) as u8)
        .collect();
    Ok((
        if has_alpha {
            PixelFormat::Rgba8
        } else {
            PixelFormat::Rgb8
        },
        out8,
    ))
}

fn apply_alpha_plane(
    alpha_pic: &Dav1dPicture,
    expected_w: u32,
    expected_h: u32,
    premultiplied: bool,
    rgba: &mut [u8],
) -> Result<(), CodecError> {
    if alpha_pic.p.w as u32 != expected_w || alpha_pic.p.h as u32 != expected_h {
        return Err(CodecError::DecodeError(
            "AVIF alpha plane dimensions do not match primary image".into(),
        ));
    }
    let range = rav1d_range(alpha_pic);
    if alpha_pic.p.bpc <= 8 {
        let y = unsafe { plane_slice::<u8>(alpha_pic, 0)? };
        for (row, dst_row) in y.rows().zip(rgba.chunks_exact_mut(expected_w as usize * 4)) {
            for (&a_raw, px) in row.iter().zip(dst_row.as_chunks_mut::<4>().0.iter_mut()) {
                let a = scale_luma_to_u8(u16::from(a_raw), 8, range);
                px[3] = a;
                if premultiplied && a > 0 && a < 255 {
                    let a32 = u32::from(a);
                    px[0] = ((u32::from(px[0]) * 255 + a32 / 2) / a32).min(255) as u8;
                    px[1] = ((u32::from(px[1]) * 255 + a32 / 2) / a32).min(255) as u8;
                    px[2] = ((u32::from(px[2]) * 255 + a32 / 2) / a32).min(255) as u8;
                }
            }
        }
    } else {
        let bits = u32::from(alpha_pic.p.bpc as u8);
        let y = unsafe { plane_slice::<u16>(alpha_pic, 0)? };
        for (row, dst_row) in y.rows().zip(rgba.chunks_exact_mut(expected_w as usize * 4)) {
            for (&a_raw, px) in row.iter().zip(dst_row.as_chunks_mut::<4>().0.iter_mut()) {
                let a = scale_luma_to_u8(a_raw, bits, range);
                px[3] = a;
                if premultiplied && a > 0 && a < 255 {
                    let a32 = u32::from(a);
                    px[0] = ((u32::from(px[0]) * 255 + a32 / 2) / a32).min(255) as u8;
                    px[1] = ((u32::from(px[1]) * 255 + a32 / 2) / a32).min(255) as u8;
                    px[2] = ((u32::from(px[2]) * 255 + a32 / 2) / a32).min(255) as u8;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_avif_encode_and_decode_roundtrip_rgb_and_rgba() {
        let codec = AvifCodec;
        let w = 16;
        let h = 16;
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                rgb.push((x * 15) as u8);
                rgb.push((y * 15) as u8);
                rgb.push(180);
            }
        }
        let pixels = DecodedPixels::new(w, h, PixelFormat::Rgb8, rgb);
        let enc = codec
            .encode(
                &pixels,
                &EncodeOptions {
                    quality: 85,
                    effort: 2,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(enc.format, CodecFormat::Avif);
        assert!(enc.bytes.len() > 20);

        let dec = codec.decode(&enc.bytes).unwrap();
        assert_eq!(dec.width, w);
        assert_eq!(dec.height, h);
        assert_eq!(dec.pixel_format, PixelFormat::Rgb8);

        let rgba_data: Vec<u8> = (0..(w * h))
            .flat_map(|i| [100u8, 150u8, 200u8, if i % 2 == 0 { 255 } else { 128 }])
            .collect();
        let rgba_pixels = DecodedPixels::new(w, h, PixelFormat::Rgba8, rgba_data);
        let enc_a = codec
            .encode(
                &rgba_pixels,
                &EncodeOptions {
                    quality: 85,
                    effort: 2,
                    ..Default::default()
                },
            )
            .unwrap();
        let dec_a = codec.decode(&enc_a.bytes).unwrap();
        assert_eq!(dec_a.pixel_format, PixelFormat::Rgba8);
        assert_eq!(dec_a.width, w);
        assert_eq!(dec_a.height, h);
    }
}
