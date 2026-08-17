use crate::interface::{
    CodecCapabilities, CodecError, CodecFormat, DecodedPixels, EncodeOptions, EncodedOutput,
    ImageCodec, PixelFormat,
};

/// Codec adapter for JPEG image encoding and decoding.
#[derive(Debug, Default, Clone, Copy)]
pub struct JpegCodec;

impl ImageCodec for JpegCodec {
    fn capabilities(&self) -> CodecCapabilities {
        CodecCapabilities {
            format: CodecFormat::Jpeg,
            supports_lossless: false,
            supports_lossy: true,
            supports_alpha: false,
        }
    }

    fn decode(&self, input_bytes: &[u8]) -> Result<DecodedPixels, CodecError> {
        if input_bytes.is_empty() {
            return Err(CodecError::DecodeError("empty input bytes".into()));
        }

        let img = image::load_from_memory_with_format(input_bytes, image::ImageFormat::Jpeg)
            .map_err(|e| CodecError::DecodeError(format!("failed to decode JPEG: {e}")))?;

        let width = img.width();
        let height = img.height();

        let (pixel_format, data) = match img.color() {
            image::ColorType::L8 => (PixelFormat::Grayscale8, img.into_luma8().into_raw()),
            image::ColorType::La8 => {
                let rgb = img.into_rgb8();
                (PixelFormat::Rgb8, rgb.into_raw())
            }
            image::ColorType::Rgb8 => (PixelFormat::Rgb8, img.into_rgb8().into_raw()),
            image::ColorType::Rgba8 => {
                let rgb = img.into_rgb8();
                (PixelFormat::Rgb8, rgb.into_raw())
            }
            _ => {
                let rgb = img.into_rgb8();
                (PixelFormat::Rgb8, rgb.into_raw())
            }
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

        let quality = options.quality.clamp(1, 100);
        let mut encoded_bytes = Vec::new();
        let mut encoder = jpeg_encoder::Encoder::new(&mut encoded_bytes, quality);
        encoder.set_optimized_huffman_tables(true);
        if options.effort >= 4 {
            encoder.set_progressive(true);
        }
        if options.effort >= 2 {
            encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_2_0);
        } else {
            encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_4_4);
        }

        match pixels.pixel_format {
            PixelFormat::Grayscale8 => {
                encoder
                    .encode(
                        &pixels.data,
                        pixels.width as u16,
                        pixels.height as u16,
                        jpeg_encoder::ColorType::Luma,
                    )
                    .map_err(|e| CodecError::EncodeError(format!("JPEG encode error: {e}")))?;
            }
            PixelFormat::Rgb8 => {
                encoder
                    .encode(
                        &pixels.data,
                        pixels.width as u16,
                        pixels.height as u16,
                        jpeg_encoder::ColorType::Rgb,
                    )
                    .map_err(|e| CodecError::EncodeError(format!("JPEG encode error: {e}")))?;
            }
            PixelFormat::Rgba8 => {
                // Flatten RGBA to RGB with white background
                let num_pixels = (pixels.width * pixels.height) as usize;
                let mut rgb_data = Vec::with_capacity(num_pixels * 3);
                for chunk in pixels.data.chunks_exact(4) {
                    let r = chunk[0];
                    let g = chunk[1];
                    let b = chunk[2];
                    let a = chunk[3] as u32;

                    let out_r = ((r as u32 * a + 255 * (255 - a)) / 255) as u8;
                    let out_g = ((g as u32 * a + 255 * (255 - a)) / 255) as u8;
                    let out_b = ((b as u32 * a + 255 * (255 - a)) / 255) as u8;
                    rgb_data.push(out_r);
                    rgb_data.push(out_g);
                    rgb_data.push(out_b);
                }

                encoder
                    .encode(
                        &rgb_data,
                        pixels.width as u16,
                        pixels.height as u16,
                        jpeg_encoder::ColorType::Rgb,
                    )
                    .map_err(|e| CodecError::EncodeError(format!("JPEG encode error: {e}")))?;
            }
            PixelFormat::GrayscaleAlpha8 => {
                let num_pixels = (pixels.width * pixels.height) as usize;
                let mut gray_data = Vec::with_capacity(num_pixels);
                for chunk in pixels.data.chunks_exact(2) {
                    let l = chunk[0];
                    let a = chunk[1] as u32;
                    let out_l = ((l as u32 * a + 255 * (255 - a)) / 255) as u8;
                    gray_data.push(out_l);
                }

                encoder
                    .encode(
                        &gray_data,
                        pixels.width as u16,
                        pixels.height as u16,
                        jpeg_encoder::ColorType::Luma,
                    )
                    .map_err(|e| CodecError::EncodeError(format!("JPEG encode error: {e}")))?;
            }
        }

        Ok(EncodedOutput {
            format: CodecFormat::Jpeg,
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
    fn test_jpeg_encode_and_decode_roundtrip() {
        let codec = JpegCodec;
        let width = 8;
        let height = 8;
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height {
            for x in 0..width {
                rgb.push((x * 30) as u8);
                rgb.push((y * 30) as u8);
                rgb.push(128);
            }
        }

        let pixels = DecodedPixels::new(width, height, PixelFormat::Rgb8, rgb);
        let options = EncodeOptions {
            quality: 90,
            effort: 4,
            lossless: false,
            preserve_metadata: false,
        };

        let encoded = codec.encode(&pixels, &options).unwrap();
        assert_eq!(encoded.format, CodecFormat::Jpeg);
        assert!(!encoded.bytes.is_empty());

        let decoded = codec.decode(&encoded.bytes).unwrap();
        assert_eq!(decoded.width, width);
        assert_eq!(decoded.height, height);
        assert_eq!(decoded.pixel_format, PixelFormat::Rgb8);
    }

    #[test]
    fn test_jpeg_rgba_alpha_flattening() {
        let codec = JpegCodec;
        let width = 2;
        let height = 2;
        let rgba = vec![
            255, 0, 0, 255, // solid red
            0, 255, 0, 128, // half transparent green
            0, 0, 255, 0, // fully transparent blue
            0, 0, 0, 255, // solid black
        ];

        let pixels = DecodedPixels::new(width, height, PixelFormat::Rgba8, rgba);
        let encoded = codec.encode(&pixels, &EncodeOptions::default()).unwrap();
        let decoded = codec.decode(&encoded.bytes).unwrap();

        assert_eq!(decoded.width, width);
        assert_eq!(decoded.height, height);
        assert_eq!(decoded.pixel_format, PixelFormat::Rgb8);
    }

    #[test]
    fn test_jpeg_invalid_bytes() {
        let codec = JpegCodec;
        assert!(matches!(
            codec.decode(b"not a valid jpeg"),
            Err(CodecError::DecodeError(_))
        ));
    }
}
