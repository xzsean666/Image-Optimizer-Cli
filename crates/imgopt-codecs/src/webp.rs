use crate::interface::{
    CodecCapabilities, CodecError, CodecFormat, DecodedPixels, EncodeOptions, EncodedOutput,
    ImageCodec, PixelFormat,
};

/// Codec adapter for WebP image encoding and decoding.
#[derive(Debug, Default, Clone, Copy)]
pub struct WebpCodec;

impl ImageCodec for WebpCodec {
    fn capabilities(&self) -> CodecCapabilities {
        CodecCapabilities {
            format: CodecFormat::Webp,
            supports_lossless: true,
            supports_lossy: true,
            supports_alpha: true,
        }
    }

    fn decode(&self, input_bytes: &[u8]) -> Result<DecodedPixels, CodecError> {
        if input_bytes.is_empty() {
            return Err(CodecError::DecodeError("empty input bytes".into()));
        }

        let img = image::load_from_memory_with_format(input_bytes, image::ImageFormat::WebP)
            .map_err(|e| CodecError::DecodeError(format!("failed to decode WebP: {e}")))?;

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

        let quality = options.quality.clamp(1, 100) as f32;

        let encoded_bytes = match pixels.pixel_format {
            PixelFormat::Rgb8 => {
                let encoder = webp::Encoder::from_rgb(&pixels.data, pixels.width, pixels.height);
                if options.lossless {
                    encoder.encode_lossless().to_vec()
                } else {
                    encoder.encode(quality).to_vec()
                }
            }
            PixelFormat::Rgba8 => {
                let encoder = webp::Encoder::from_rgba(&pixels.data, pixels.width, pixels.height);
                if options.lossless {
                    encoder.encode_lossless().to_vec()
                } else {
                    encoder.encode(quality).to_vec()
                }
            }
            PixelFormat::Grayscale8 => {
                let num_pixels = (pixels.width * pixels.height) as usize;
                let mut rgb_data = Vec::with_capacity(num_pixels * 3);
                for &l in &pixels.data {
                    rgb_data.push(l);
                    rgb_data.push(l);
                    rgb_data.push(l);
                }
                let encoder = webp::Encoder::from_rgb(&rgb_data, pixels.width, pixels.height);
                if options.lossless {
                    encoder.encode_lossless().to_vec()
                } else {
                    encoder.encode(quality).to_vec()
                }
            }
            PixelFormat::GrayscaleAlpha8 => {
                let num_pixels = (pixels.width * pixels.height) as usize;
                let mut rgba_data = Vec::with_capacity(num_pixels * 4);
                for chunk in pixels.data.chunks_exact(2) {
                    let l = chunk[0];
                    let a = chunk[1];
                    rgba_data.push(l);
                    rgba_data.push(l);
                    rgba_data.push(l);
                    rgba_data.push(a);
                }
                let encoder = webp::Encoder::from_rgba(&rgba_data, pixels.width, pixels.height);
                if options.lossless {
                    encoder.encode_lossless().to_vec()
                } else {
                    encoder.encode(quality).to_vec()
                }
            }
        };

        if encoded_bytes.is_empty() {
            return Err(CodecError::EncodeError(
                "WebP encoding produced empty output".into(),
            ));
        }

        Ok(EncodedOutput {
            format: CodecFormat::Webp,
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
    fn test_webp_rgb_lossless_roundtrip() {
        let codec = WebpCodec;
        let width = 4;
        let height = 4;
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height {
            for x in 0..width {
                rgb.push((x * 40) as u8);
                rgb.push((y * 40) as u8);
                rgb.push(150);
            }
        }

        let pixels = DecodedPixels::new(width, height, PixelFormat::Rgb8, rgb.clone());
        let options = EncodeOptions {
            quality: 100,
            effort: 4,
            lossless: true,
            preserve_metadata: false,
        };

        let encoded = codec.encode(&pixels, &options).unwrap();
        assert_eq!(encoded.format, CodecFormat::Webp);

        let decoded = codec.decode(&encoded.bytes).unwrap();
        assert_eq!(decoded.width, width);
        assert_eq!(decoded.height, height);
    }

    #[test]
    fn test_webp_rgba_alpha_lossy() {
        let codec = WebpCodec;
        let width = 4;
        let height = 4;
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                rgba.push((x * 50) as u8);
                rgba.push((y * 50) as u8);
                rgba.push(200);
                rgba.push(if (x + y) % 2 == 0 { 255 } else { 128 });
            }
        }

        let pixels = DecodedPixels::new(width, height, PixelFormat::Rgba8, rgba);
        let options = EncodeOptions {
            quality: 80,
            effort: 4,
            lossless: false,
            preserve_metadata: false,
        };

        let encoded = codec.encode(&pixels, &options).unwrap();
        assert_eq!(encoded.format, CodecFormat::Webp);

        let decoded = codec.decode(&encoded.bytes).unwrap();
        assert_eq!(decoded.width, width);
        assert_eq!(decoded.height, height);
    }

    #[test]
    fn test_webp_invalid_bytes() {
        let codec = WebpCodec;
        assert!(matches!(
            codec.decode(b"not a valid webp"),
            Err(CodecError::DecodeError(_))
        ));
    }
}
