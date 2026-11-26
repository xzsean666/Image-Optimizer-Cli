use std::io::Cursor;

use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader, Limits};

pub mod avif;
pub mod gif;
pub mod interface;
pub mod jpeg;
pub mod png;
pub mod transform;
pub mod webp;

pub use avif::AvifCodec;
pub use gif::GifCodec;
pub use interface::{
    check_dimensions_against_limits, AnimationFrame, CodecCapabilities, CodecError, CodecFormat,
    DecodeLimits, DecodedAnimation, DecodedPixels, EncodeOptions, EncodedOutput, FakeCodec,
    ImageCodec, ImageMetadataChunks, MetadataPreservation, PixelFormat,
};
pub use jpeg::JpegCodec;
pub use png::PngCodec;
pub use transform::{
    compute_target_dimensions, convert_icc_to_srgb, decode_animation_with_limits,
    encode_animated_gif, encode_animated_webp, resize_pixels, ResizeMode,
};
pub use webp::WebpCodec;

/// Helper function to get default codec for a given format.
pub fn codec_for_format(format: CodecFormat) -> Box<dyn ImageCodec> {
    match format {
        CodecFormat::Jpeg => Box::new(JpegCodec),
        CodecFormat::Png => Box::new(PngCodec),
        CodecFormat::Webp => Box::new(WebpCodec),
        CodecFormat::Avif => Box::new(AvifCodec),
        CodecFormat::Gif => Box::new(GifCodec),
    }
}

fn map_image_error(codec_format: CodecFormat, err: image::ImageError) -> CodecError {
    match err {
        image::ImageError::Limits(lim) => CodecError::ResourceLimitExceeded(format!(
            "{codec_format} image exceeds configured limit: {lim}"
        )),
        other => CodecError::DecodeError(format!("failed to decode {codec_format}: {other}")),
    }
}

fn finish_decode<D: ImageDecoder>(
    mut decoder: D,
    codec_format: CodecFormat,
    limits: &DecodeLimits,
) -> Result<DecodedPixels, CodecError> {
    let (width, height) = decoder.dimensions();
    check_dimensions_against_limits(width, height, limits)?;

    if let Some(max_alloc) = limits.max_alloc_bytes {
        let decoded_bytes = decoder.total_bytes();
        if decoded_bytes > max_alloc {
            return Err(CodecError::ResourceLimitExceeded(format!(
                "decoded buffer size {decoded_bytes} bytes exceeds limit of {max_alloc} bytes"
            )));
        }
    }

    let icc_profile = decoder
        .icc_profile()
        .ok()
        .flatten()
        .filter(|b| !b.is_empty());
    let mut exif = decoder
        .exif_metadata()
        .ok()
        .flatten()
        .map(strip_exif_app1_prefix)
        .filter(|b| !b.is_empty());
    let xmp = decoder
        .xmp_metadata()
        .ok()
        .flatten()
        .filter(|b| !b.is_empty());

    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);

    let mut img =
        DynamicImage::from_decoder(decoder).map_err(|e| map_image_error(codec_format, e))?;

    if orientation != Orientation::NoTransforms {
        img.apply_orientation(orientation);
        if let Some(ref mut exif_bytes) = exif {
            let _ = Orientation::remove_from_exif_chunk(exif_bytes);
        }
    }

    let out_width = img.width();
    let out_height = img.height();

    let (pixel_format, data) = match codec_format {
        CodecFormat::Jpeg => match img.color() {
            image::ColorType::L8 => (PixelFormat::Grayscale8, img.into_luma8().into_raw()),
            _ => (PixelFormat::Rgb8, img.into_rgb8().into_raw()),
        },
        CodecFormat::Png | CodecFormat::Webp | CodecFormat::Avif | CodecFormat::Gif => {
            match img.color() {
                image::ColorType::L8 | image::ColorType::L16 => {
                    (PixelFormat::Grayscale8, img.into_luma8().into_raw())
                }
                image::ColorType::La8 | image::ColorType::La16 => (
                    PixelFormat::GrayscaleAlpha8,
                    img.into_luma_alpha8().into_raw(),
                ),
                image::ColorType::Rgb8 | image::ColorType::Rgb16 => {
                    (PixelFormat::Rgb8, img.into_rgb8().into_raw())
                }
                _ => (PixelFormat::Rgba8, img.into_rgba8().into_raw()),
            }
        }
    };

    Ok(DecodedPixels::with_metadata(
        out_width,
        out_height,
        pixel_format,
        data,
        ImageMetadataChunks {
            icc_profile,
            exif,
            xmp,
        },
    ))
}

/// Shared bounded decoder that checks dimensions/pixel limits before allocating pixel buffers,
/// rejects unsupported animated images, extracts metadata chunks, and normalizes EXIF orientation.
pub(crate) fn decode_with_image_reader(
    input_bytes: &[u8],
    format: image::ImageFormat,
    codec_format: CodecFormat,
    limits: &DecodeLimits,
) -> Result<DecodedPixels, CodecError> {
    if input_bytes.is_empty() {
        return Err(CodecError::DecodeError("empty input bytes".into()));
    }

    let mut img_limits = Limits::default();
    img_limits.max_image_width = limits.max_width;
    img_limits.max_image_height = limits.max_height;
    if let Some(max_alloc) = limits.max_alloc_bytes {
        img_limits.max_alloc = Some(max_alloc);
    }

    match format {
        image::ImageFormat::Png => {
            let decoder =
                image::codecs::png::PngDecoder::with_limits(Cursor::new(input_bytes), img_limits)
                    .map_err(|e| map_image_error(codec_format, e))?;
            if decoder.is_apng().unwrap_or(false) {
                return Err(CodecError::AnimatedImage(
                    "animated PNG (APNG) requires animation pipeline".into(),
                ));
            }
            finish_decode(decoder, codec_format, limits)
        }
        image::ImageFormat::WebP => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(input_bytes))
                .map_err(|e| map_image_error(codec_format, e))?;
            decoder
                .set_limits(img_limits)
                .map_err(|e| map_image_error(codec_format, e))?;
            if decoder.has_animation() {
                return Err(CodecError::AnimatedImage(
                    "animated WebP requires animation pipeline".into(),
                ));
            }
            finish_decode(decoder, codec_format, limits)
        }
        _ => {
            let mut reader = ImageReader::with_format(Cursor::new(input_bytes), format);
            reader.limits(img_limits);
            let decoder = reader
                .into_decoder()
                .map_err(|e| map_image_error(codec_format, e))?;
            finish_decode(decoder, codec_format, limits)
        }
    }
}

fn strip_exif_app1_prefix(bytes: Vec<u8>) -> Vec<u8> {
    if bytes.starts_with(b"Exif\0\0") {
        bytes[6..].to_vec()
    } else {
        bytes
    }
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert_eq!(VERSION, "0.1.0");
    }

    #[test]
    fn test_codec_for_format() {
        assert_eq!(
            codec_for_format(CodecFormat::Jpeg).capabilities().format,
            CodecFormat::Jpeg
        );
        assert_eq!(
            codec_for_format(CodecFormat::Png).capabilities().format,
            CodecFormat::Png
        );
        assert_eq!(
            codec_for_format(CodecFormat::Webp).capabilities().format,
            CodecFormat::Webp
        );
    }

    #[test]
    fn test_animated_webp_rejected_explicitly() {
        let mut config = ::webp::WebPConfig::new().unwrap();
        config.lossless = 1;
        let mut anim = ::webp::AnimEncoder::new(2, 2, &config);
        let frame1 = vec![255u8; 2 * 2 * 4];
        let frame2 = vec![0u8; 2 * 2 * 4];
        anim.add_frame(::webp::AnimFrame::new(
            &frame1,
            ::webp::PixelLayout::Rgba,
            2,
            2,
            0,
            None,
        ));
        anim.add_frame(::webp::AnimFrame::new(
            &frame2,
            ::webp::PixelLayout::Rgba,
            2,
            2,
            100,
            None,
        ));
        let webp_mem = anim.encode();

        let err = WebpCodec.decode(&webp_mem).unwrap_err();
        assert!(matches!(err, CodecError::AnimatedImage(_)));
    }
}
