use std::io::Cursor;

use fast_image_resize::images::{Image as FirImage, ImageRef as FirImageRef};
use fast_image_resize::{PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::codecs::gif::{GifDecoder, GifEncoder};
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, Delay, Frame, RgbaImage};
use qcms::{DataType, Intent, Profile, Transform};

use crate::interface::{
    check_dimensions_against_limits, AnimationFrame, CodecError, CodecFormat, DecodeLimits,
    DecodedAnimation, DecodedPixels, EncodeOptions, PixelFormat,
};

/// Image resizing behavior when target dimensions are provided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ResizeMode {
    /// Fit inside target bounds preserving aspect ratio; never upscale beyond original dimensions.
    #[default]
    ScaleDown,
    /// Fit inside target bounds preserving aspect ratio (allows upscaling).
    Contain,
    /// Fill target bounds completely by center-cropping excess edges.
    Cover,
    /// Stretch/resize to exact target width and height.
    Exact,
}

/// Compute target `(width, height)` given optional width/height constraints and a `ResizeMode`.
pub fn compute_target_dimensions(
    src_w: u32,
    src_h: u32,
    target_w: Option<u32>,
    target_h: Option<u32>,
    mode: ResizeMode,
) -> (u32, u32) {
    if src_w == 0 || src_h == 0 {
        return (src_w, src_h);
    }
    match (target_w, target_h) {
        (None, None) => (src_w, src_h),
        (Some(w), None) => {
            if matches!(mode, ResizeMode::ScaleDown) && src_w <= w {
                return (src_w, src_h);
            }
            let new_h =
                ((u64::from(src_h) * u64::from(w)) + u64::from(src_w) / 2) / u64::from(src_w);
            (w.max(1), (new_h as u32).max(1))
        }
        (None, Some(h)) => {
            if matches!(mode, ResizeMode::ScaleDown) && src_h <= h {
                return (src_w, src_h);
            }
            let new_w =
                ((u64::from(src_w) * u64::from(h)) + u64::from(src_h) / 2) / u64::from(src_h);
            ((new_w as u32).max(1), h.max(1))
        }
        (Some(w), Some(h)) => match mode {
            ResizeMode::Exact | ResizeMode::Cover => (w.max(1), h.max(1)),
            ResizeMode::ScaleDown if src_w <= w && src_h <= h => (src_w, src_h),
            ResizeMode::ScaleDown | ResizeMode::Contain => {
                let scale_w = f64::from(w) / f64::from(src_w);
                let scale_h = f64::from(h) / f64::from(src_h);
                let scale = scale_w.min(scale_h);
                let new_w = (f64::from(src_w) * scale).round().max(1.0) as u32;
                let new_h = (f64::from(src_h) * scale).round().max(1.0) as u32;
                (new_w.min(w.max(1)), new_h.min(h.max(1)))
            }
        },
    }
}

/// Resize decoded pixels with SIMD-accelerated Lanczos3 resampling via `fast_image_resize`.
pub fn resize_pixels(
    pixels: &DecodedPixels,
    target_w: u32,
    target_h: u32,
    mode: ResizeMode,
) -> Result<DecodedPixels, CodecError> {
    if !pixels.is_valid() {
        return Err(CodecError::InvalidParameter(
            "invalid pixel buffer for resize".into(),
        ));
    }
    if target_w == 0 || target_h == 0 {
        return Err(CodecError::InvalidParameter(
            "resize dimensions must be greater than 0".into(),
        ));
    }
    if pixels.width == target_w && pixels.height == target_h {
        return Ok(pixels.clone());
    }

    let pixel_type = match pixels.pixel_format {
        PixelFormat::Grayscale8 => PixelType::U8,
        PixelFormat::GrayscaleAlpha8 => PixelType::U8x2,
        PixelFormat::Rgb8 => PixelType::U8x3,
        PixelFormat::Rgba8 => PixelType::U8x4,
    };

    let src_image = FirImageRef::new(pixels.width, pixels.height, &pixels.data, pixel_type)
        .map_err(|e| {
            CodecError::EncodeError(format!("failed to create resize source view: {e}"))
        })?;

    let mut dst_image = FirImage::new(target_w, target_h, pixel_type);
    let mut options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(
        fast_image_resize::FilterType::Lanczos3,
    ));
    if mode == ResizeMode::Cover {
        options = options.fit_into_destination(Some((0.5, 0.5)));
    }

    let mut resizer = Resizer::new();
    resizer
        .resize(&src_image, &mut dst_image, &options)
        .map_err(|e| CodecError::EncodeError(format!("image resize failed: {e}")))?;

    Ok(DecodedPixels::with_metadata(
        target_w,
        target_h,
        pixels.pixel_format,
        dst_image.into_vec(),
        pixels.metadata.clone(),
    ))
}

/// Convert non-sRGB ICC-profiled pixels into sRGB via `qcms` so stripping ICC metadata
/// does not cause wide-gamut color shifts in browsers.
/// Also returns whether the image pixels or ICC profile were changed.
pub fn convert_icc_to_srgb_with_status(pixels: &DecodedPixels) -> (DecodedPixels, bool) {
    let Some(ref icc_bytes) = pixels.metadata.icc_profile else {
        return (pixels.clone(), false);
    };
    if icc_bytes.is_empty() {
        return (pixels.clone(), false);
    }

    let Some(src_profile) = Profile::new_from_slice(icc_bytes, false) else {
        return (pixels.clone(), false);
    };
    if src_profile.is_sRGB() {
        let mut out = pixels.clone();
        out.metadata.icc_profile = None;
        return (out, true);
    }

    let dst_profile = Profile::new_sRGB();
    let data_type = match pixels.pixel_format {
        PixelFormat::Rgb8 => DataType::RGB8,
        PixelFormat::Rgba8 => DataType::RGBA8,
        _ => return (pixels.clone(), false),
    };

    let Some(transform) = Transform::new(&src_profile, &dst_profile, data_type, Intent::Perceptual)
    else {
        return (pixels.clone(), false);
    };

    let mut out = pixels.clone();
    transform.apply(&mut out.data);
    out.metadata.icc_profile = None;
    (out, true)
}

/// Convert non-sRGB ICC-profiled pixels into sRGB via `qcms` so stripping ICC metadata
/// does not cause wide-gamut color shifts in browsers.
pub fn convert_icc_to_srgb(pixels: &DecodedPixels) -> DecodedPixels {
    convert_icc_to_srgb_with_status(pixels).0
}

/// Decode an animated GIF, animated WebP, or APNG into RGBA8 frames with streaming per-frame memory bounds.
pub fn decode_animation_with_limits(
    bytes: &[u8],
    format: CodecFormat,
    limits: &DecodeLimits,
) -> Result<Option<DecodedAnimation>, CodecError> {
    match format {
        CodecFormat::Gif => {
            let dec = GifDecoder::new(Cursor::new(bytes))
                .map_err(|e| CodecError::DecodeError(format!("failed to decode GIF: {e}")))?;
            collect_animation_frames(dec.into_frames(), "GIF", limits)
        }
        CodecFormat::Webp => {
            let dec = WebPDecoder::new(Cursor::new(bytes))
                .map_err(|e| CodecError::DecodeError(format!("failed to decode WebP: {e}")))?;
            if !dec.has_animation() {
                return Ok(None);
            }
            collect_animation_frames(dec.into_frames(), "animated WebP", limits)
        }
        CodecFormat::Png => {
            let dec = PngDecoder::new(Cursor::new(bytes))
                .map_err(|e| CodecError::DecodeError(format!("failed to decode PNG: {e}")))?;
            if !dec.is_apng().unwrap_or(false) {
                return Ok(None);
            }
            let apng = dec
                .apng()
                .map_err(|e| CodecError::DecodeError(format!("failed to open APNG: {e}")))?;
            collect_animation_frames(apng.into_frames(), "APNG", limits)
        }
        _ => Ok(None),
    }
}

fn collect_animation_frames<'a>(
    frames: image::Frames<'a>,
    label: &str,
    limits: &DecodeLimits,
) -> Result<Option<DecodedAnimation>, CodecError> {
    let mut out_frames = Vec::new();
    let mut canvas_dims: Option<(u32, u32)> = None;
    let mut total_alloc = 0u64;

    for frame_res in frames {
        let f = frame_res.map_err(|e| {
            CodecError::DecodeError(format!("failed to decode {label} frames: {e}"))
        })?;
        let buf = f.buffer();
        let fw = buf.width();
        let fh = buf.height();
        check_dimensions_against_limits(fw, fh, limits)?;

        let (cw, ch) = *canvas_dims.get_or_insert((fw, fh));
        if fw != cw || fh != ch {
            return Err(CodecError::DecodeError(format!(
                "{label} frame dimensions {fw}x{fh} do not match canvas {cw}x{ch}"
            )));
        }

        let frame_bytes = u64::from(fw)
            .saturating_mul(u64::from(fh))
            .saturating_mul(4);
        total_alloc = total_alloc.saturating_add(frame_bytes);
        if let Some(max_alloc) = limits.max_alloc_bytes {
            if total_alloc > max_alloc {
                return Err(CodecError::ResourceLimitExceeded(format!(
                    "animation frames total {total_alloc} bytes exceed limit of {max_alloc} bytes"
                )));
            }
        }

        let (num, den) = f.delay().numer_denom_ms();
        let delay_millis = num.checked_div(den).map_or(100, |d| d.max(10));
        let rgba = f.into_buffer().into_raw();
        out_frames.push(AnimationFrame {
            width: fw,
            height: fh,
            rgba,
            delay_millis,
        });
    }

    if out_frames.len() <= 1 {
        return Ok(None);
    }

    let (width, height) = canvas_dims.unwrap_or((0, 0));
    Ok(Some(DecodedAnimation {
        width,
        height,
        frames: out_frames,
    }))
}

/// Encode an animation sequence as animated WebP using `libwebp` (`webp::AnimEncoder`).
#[cfg(feature = "webp")]
pub fn encode_animated_webp(
    anim: &DecodedAnimation,
    options: &EncodeOptions,
) -> Result<Vec<u8>, CodecError> {
    if anim.frames.is_empty() {
        return Err(CodecError::InvalidParameter(
            "cannot encode animation with 0 frames".into(),
        ));
    }
    if anim.width == 0 || anim.height == 0 {
        return Err(CodecError::InvalidParameter(
            "animation dimensions must be greater than 0".into(),
        ));
    }
    let mut config = webp::WebPConfig::new().map_err(|()| {
        CodecError::EncodeError("failed to initialize animated WebP config".into())
    })?;
    config.lossless = if options.lossless { 1 } else { 0 };
    config.quality = f32::from(options.quality.clamp(1, 100));
    config.method = i32::from(options.effort.clamp(1, 6));

    let mut encoder = webp::AnimEncoder::new(anim.width, anim.height, &config);
    let mut timestamp_ms: i32 = 0;
    for frame in &anim.frames {
        let expected_len = (frame.width as usize)
            .checked_mul(frame.height as usize)
            .and_then(|px| px.checked_mul(4))
            .ok_or_else(|| {
                CodecError::InvalidParameter("animation frame dimensions overflow".into())
            })?;
        if frame.width != anim.width
            || frame.height != anim.height
            || frame.rgba.len() != expected_len
        {
            return Err(CodecError::InvalidParameter(
                "invalid animation frame dimensions or RGBA buffer length".into(),
            ));
        }
        encoder.add_frame(webp::AnimFrame::from_rgba(
            &frame.rgba,
            frame.width,
            frame.height,
            timestamp_ms,
        ));
        timestamp_ms =
            timestamp_ms.saturating_add(i32::try_from(frame.delay_millis).unwrap_or(100));
    }
    let mem = encoder.encode();
    if mem.is_empty() {
        return Err(CodecError::EncodeError(
            "animated WebP encoding produced empty output".into(),
        ));
    }
    Ok(mem.to_vec())
}

#[cfg(not(feature = "webp"))]
pub fn encode_animated_webp(
    _anim: &DecodedAnimation,
    _options: &EncodeOptions,
) -> Result<Vec<u8>, CodecError> {
    Err(CodecError::UnsupportedFormat(
        "animated WebP encoding requires the `webp` feature".into(),
    ))
}

/// Encode an animation sequence as animated GIF.
pub fn encode_animated_gif(
    anim: &DecodedAnimation,
    options: &EncodeOptions,
) -> Result<Vec<u8>, CodecError> {
    if anim.frames.is_empty() {
        return Err(CodecError::InvalidParameter(
            "cannot encode animation with 0 frames".into(),
        ));
    }
    let speed = match options.effort.clamp(1, 6) {
        1 => 30,
        2 => 20,
        3 => 15,
        4 => 10,
        5 => 5,
        _ => 1,
    };
    let mut out = Vec::new();
    {
        let mut encoder = GifEncoder::new_with_speed(&mut out, speed);
        encoder
            .set_repeat(image::codecs::gif::Repeat::Infinite)
            .map_err(|e| CodecError::EncodeError(format!("GIF repeat header error: {e}")))?;
        for f in &anim.frames {
            let img = RgbaImage::from_raw(f.width, f.height, f.rgba.clone()).ok_or_else(|| {
                CodecError::InvalidParameter("invalid animation frame buffer".into())
            })?;
            let delay = Delay::from_numer_denom_ms(f.delay_millis.max(10), 1);
            encoder
                .encode_frame(Frame::from_parts(img, 0, 0, delay))
                .map_err(|e| CodecError::EncodeError(format!("GIF frame encode error: {e}")))?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_target_dimensions_modes() {
        assert_eq!(
            compute_target_dimensions(2000, 1000, Some(1000), None, ResizeMode::ScaleDown),
            (1000, 500)
        );
        // Never upscales in ScaleDown
        assert_eq!(
            compute_target_dimensions(800, 600, Some(1000), None, ResizeMode::ScaleDown),
            (800, 600)
        );
        // Allows upscaling in Contain
        assert_eq!(
            compute_target_dimensions(800, 600, Some(1600), Some(1600), ResizeMode::Contain),
            (1600, 1200)
        );
        assert_eq!(
            compute_target_dimensions(800, 600, Some(400), Some(400), ResizeMode::Exact),
            (400, 400)
        );
    }

    #[test]
    fn test_resize_pixels_rgb_and_rgba() {
        let pixels = DecodedPixels::new(16, 8, PixelFormat::Rgba8, vec![128; 16 * 8 * 4]);
        let resized = resize_pixels(&pixels, 8, 4, ResizeMode::ScaleDown).unwrap();
        assert_eq!(resized.width, 8);
        assert_eq!(resized.height, 4);
        assert!(resized.is_valid());
    }

    #[test]
    fn test_animation_gif_and_webp_roundtrip() {
        let anim = DecodedAnimation {
            width: 4,
            height: 4,
            frames: vec![
                AnimationFrame {
                    width: 4,
                    height: 4,
                    rgba: [255, 0, 0, 255].repeat(16),
                    delay_millis: 50,
                },
                AnimationFrame {
                    width: 4,
                    height: 4,
                    rgba: [0, 0, 255, 255].repeat(16),
                    delay_millis: 50,
                },
            ],
        };
        let opts = EncodeOptions::default();
        let gif_bytes = encode_animated_gif(&anim, &opts).unwrap();
        let dec_gif =
            decode_animation_with_limits(&gif_bytes, CodecFormat::Gif, &DecodeLimits::default())
                .unwrap()
                .unwrap();
        assert_eq!(dec_gif.frames.len(), 2);

        #[cfg(feature = "webp")]
        {
            let webp_bytes = encode_animated_webp(&dec_gif, &opts).unwrap();
            let dec_webp = decode_animation_with_limits(
                &webp_bytes,
                CodecFormat::Webp,
                &DecodeLimits::default(),
            )
            .unwrap()
            .unwrap();
            assert_eq!(dec_webp.frames.len(), 2);
        }
    }

    #[test]
    fn test_convert_icc_to_srgb_preserves_valid_pixels_when_no_icc() {
        let pixels = DecodedPixels::new(4, 4, PixelFormat::Rgb8, vec![100; 48]);
        let converted = convert_icc_to_srgb(&pixels);
        assert_eq!(converted.data, pixels.data);
    }

    #[test]
    fn test_animation_decode_enforces_streaming_alloc_limit() {
        let anim = DecodedAnimation {
            width: 16,
            height: 16,
            frames: vec![
                AnimationFrame {
                    width: 16,
                    height: 16,
                    rgba: [255, 0, 0, 255].repeat(256),
                    delay_millis: 40,
                },
                AnimationFrame {
                    width: 16,
                    height: 16,
                    rgba: [0, 255, 0, 255].repeat(256),
                    delay_millis: 40,
                },
            ],
        };
        let gif_bytes = encode_animated_gif(&anim, &EncodeOptions::default()).unwrap();
        let tight = DecodeLimits {
            max_alloc_bytes: Some(1500), // 1 frame = 1024 B ok, 2nd frame = 2048 B exceeds
            ..Default::default()
        };
        let err = decode_animation_with_limits(&gif_bytes, CodecFormat::Gif, &tight).unwrap_err();
        assert!(matches!(err, CodecError::ResourceLimitExceeded(_)));
    }
}
