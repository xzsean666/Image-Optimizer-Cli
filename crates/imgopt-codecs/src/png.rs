use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::ImageEncoder;

use crate::interface::{
    CodecCapabilities, CodecError, CodecFormat, DecodedPixels, EncodeOptions, EncodedOutput,
    ImageCodec, PixelFormat,
};

/// Codec adapter for PNG image encoding and decoding.
#[derive(Debug, Default, Clone, Copy)]
pub struct PngCodec;

impl ImageCodec for PngCodec {
    fn capabilities(&self) -> CodecCapabilities {
        CodecCapabilities {
            format: CodecFormat::Png,
            supports_lossless: true,
            supports_lossy: false,
            supports_alpha: true,
        }
    }

    fn decode(&self, input_bytes: &[u8]) -> Result<DecodedPixels, CodecError> {
        if input_bytes.is_empty() {
            return Err(CodecError::DecodeError("empty input bytes".into()));
        }

        let img = image::load_from_memory_with_format(input_bytes, image::ImageFormat::Png)
            .map_err(|e| CodecError::DecodeError(format!("failed to decode PNG: {e}")))?;

        let width = img.width();
        let height = img.height();

        let (pixel_format, data) = match img.color() {
            image::ColorType::L8 => (PixelFormat::Grayscale8, img.into_luma8().into_raw()),
            image::ColorType::La8 => (
                PixelFormat::GrayscaleAlpha8,
                img.into_luma_alpha8().into_raw(),
            ),
            image::ColorType::Rgb8 => (PixelFormat::Rgb8, img.into_rgb8().into_raw()),
            image::ColorType::Rgba8 => (PixelFormat::Rgba8, img.into_rgba8().into_raw()),
            _ => (PixelFormat::Rgba8, img.into_rgba8().into_raw()),
        };

        Ok(DecodedPixels::new(width, height, pixel_format, data))
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

        let (compression, filter) = match options.effort {
            5..=6 => (CompressionType::Best, FilterType::Adaptive),
            3..=4 => (CompressionType::Default, FilterType::Adaptive),
            _ => (CompressionType::Fast, FilterType::NoFilter),
        };

        let mut encoded_bytes = Vec::new();
        let encoder = PngEncoder::new_with_quality(&mut encoded_bytes, compression, filter);

        let color_type = match pixels.pixel_format {
            PixelFormat::Grayscale8 => image::ExtendedColorType::L8,
            PixelFormat::GrayscaleAlpha8 => image::ExtendedColorType::La8,
            PixelFormat::Rgb8 => image::ExtendedColorType::Rgb8,
            PixelFormat::Rgba8 => image::ExtendedColorType::Rgba8,
        };

        encoder
            .write_image(&pixels.data, pixels.width, pixels.height, color_type)
            .map_err(|e| CodecError::EncodeError(format!("PNG encode error: {e}")))?;

        Ok(EncodedOutput {
            format: CodecFormat::Png,
            bytes: encoded_bytes,
            width: pixels.width,
            height: pixels.height,
            effective_options: options.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_png_rgb_lossless_roundtrip() {
        let codec = PngCodec;
        let width = 4;
        let height = 4;
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height {
            for x in 0..width {
                rgb.push((x * 50) as u8);
                rgb.push((y * 50) as u8);
                rgb.push(100);
            }
        }

        let pixels = DecodedPixels::new(width, height, PixelFormat::Rgb8, rgb.clone());
        let encoded = codec.encode(&pixels, &EncodeOptions::default()).unwrap();
        let decoded = codec.decode(&encoded.bytes).unwrap();

        assert_eq!(decoded.width, width);
        assert_eq!(decoded.height, height);
        assert_eq!(decoded.pixel_format, PixelFormat::Rgb8);
        assert_eq!(
            decoded.data, rgb,
            "Lossless PNG must preserve RGB pixels exactly"
        );
    }

    #[test]
    fn test_png_rgba_alpha_preservation() {
        let codec = PngCodec;
        let width = 2;
        let height = 2;
        let rgba = vec![
            255, 0, 0, 255, // opaque red
            0, 255, 0, 128, // semi-transparent green
            0, 0, 255, 0, // fully transparent blue
            128, 128, 128, 64, // semi-transparent gray
        ];

        let pixels = DecodedPixels::new(width, height, PixelFormat::Rgba8, rgba.clone());
        let encoded = codec.encode(&pixels, &EncodeOptions::default()).unwrap();
        let decoded = codec.decode(&encoded.bytes).unwrap();

        assert_eq!(decoded.pixel_format, PixelFormat::Rgba8);
        assert_eq!(
            decoded.data, rgba,
            "Lossless PNG must preserve RGBA alpha channel exactly"
        );
    }

    #[test]
    fn test_png_invalid_bytes() {
        let codec = PngCodec;
        assert!(matches!(
            codec.decode(b"not a valid png"),
            Err(CodecError::DecodeError(_))
        ));
    }
}
