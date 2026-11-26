use std::io::Cursor;

use image::codecs::gif::{GifDecoder, GifEncoder};
use image::{AnimationDecoder, Frame, RgbaImage};

use crate::decode_with_image_reader;
use crate::interface::{
    CodecCapabilities, CodecError, CodecFormat, DecodeLimits, DecodedPixels, EncodeOptions,
    EncodedOutput, ImageCodec, PixelFormat,
};

/// Codec adapter for single-frame GIF encoding and decoding.
/// Multi-frame animated GIFs are handled via `decode_animation_with_limits` / `encode_animated_gif`.
#[derive(Debug, Default, Clone, Copy)]
pub struct GifCodec;

impl ImageCodec for GifCodec {
    fn capabilities(&self) -> CodecCapabilities {
        CodecCapabilities {
            format: CodecFormat::Gif,
            supports_lossless: true,
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
        // Probe frame count before single-frame decode so animated GIFs route through animation pipeline
        let decoder = GifDecoder::new(Cursor::new(input_bytes))
            .map_err(|e| CodecError::DecodeError(format!("failed to decode GIF header: {e}")))?;
        let mut frames_iter = decoder.into_frames();
        let _first = frames_iter
            .next()
            .ok_or_else(|| CodecError::DecodeError("GIF contains zero frames".into()))?
            .map_err(|e| CodecError::DecodeError(format!("failed to decode GIF frame: {e}")))?;
        if frames_iter.next().is_some() {
            return Err(CodecError::UnsupportedFormat(
                "animated GIF contains multiple frames".into(),
            ));
        }

        decode_with_image_reader(
            input_bytes,
            image::ImageFormat::Gif,
            CodecFormat::Gif,
            limits,
        )
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

        let rgba_data = pixels_to_rgba(pixels);
        let img = RgbaImage::from_raw(pixels.width, pixels.height, rgba_data).ok_or_else(|| {
            CodecError::InvalidParameter("failed to construct RGBA image for GIF".into())
        })?;

        let mut encoded_bytes = Vec::new();
        // Map effort (1..=6) to GifEncoder speed (30 fastest ..= 1 highest quality)
        let speed = match options.effort.clamp(1, 6) {
            1 => 30,
            2 => 20,
            3 => 15,
            4 => 10,
            5 => 5,
            _ => 1,
        };
        {
            let mut encoder = GifEncoder::new_with_speed(&mut encoded_bytes, speed);
            encoder
                .encode_frame(Frame::new(img))
                .map_err(|e| CodecError::EncodeError(format!("GIF encode error: {e}")))?;
        }

        Ok(EncodedOutput {
            format: CodecFormat::Gif,
            bytes: encoded_bytes,
            width: pixels.width,
            height: pixels.height,
            effective_options: options.clone(),
        })
    }
}

fn pixels_to_rgba(pixels: &DecodedPixels) -> Vec<u8> {
    match pixels.pixel_format {
        PixelFormat::Rgba8 => pixels.data.clone(),
        PixelFormat::Rgb8 => pixels
            .data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        PixelFormat::Grayscale8 => pixels.data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        PixelFormat::GrayscaleAlpha8 => pixels
            .data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gif_single_frame_encode_decode() {
        let codec = GifCodec;
        let pixels = DecodedPixels::new(8, 8, PixelFormat::Rgb8, vec![120; 8 * 8 * 3]);
        let enc = codec.encode(&pixels, &EncodeOptions::default()).unwrap();
        assert_eq!(enc.format, CodecFormat::Gif);
        let dec = codec.decode(&enc.bytes).unwrap();
        assert_eq!(dec.width, 8);
        assert_eq!(dec.height, 8);
    }
}
