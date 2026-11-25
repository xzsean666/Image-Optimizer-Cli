use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::ImageEncoder;
use oxipng::StripChunks;

use crate::decode_with_image_reader;
use crate::interface::{
    CodecCapabilities, CodecError, CodecFormat, DecodeLimits, DecodedPixels, EncodeOptions,
    EncodedOutput, ImageCodec, PixelFormat,
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
        self.decode_with_limits(input_bytes, &DecodeLimits::default())
    }

    fn decode_with_limits(
        &self,
        input_bytes: &[u8],
        limits: &DecodeLimits,
    ) -> Result<DecodedPixels, CodecError> {
        decode_with_image_reader(
            input_bytes,
            image::ImageFormat::Png,
            CodecFormat::Png,
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

        // Fast initial serialization because oxipng immediately re-evaluates filters and DEFLATE.
        let mut encoded_bytes = Vec::new();
        let mut encoder = PngEncoder::new_with_quality(
            &mut encoded_bytes,
            CompressionType::Fast,
            FilterType::NoFilter,
        );

        let preserve = options.effective_preservation();
        if preserve.preserve_icc {
            if let Some(ref icc) = pixels.metadata.icc_profile {
                if !icc.is_empty() {
                    let _ = encoder.set_icc_profile(icc.clone());
                }
            }
        }
        if preserve.preserve_exif {
            if let Some(ref exif) = pixels.metadata.exif {
                if !exif.is_empty() {
                    let _ = encoder.set_exif_metadata(exif.clone());
                }
            }
        }

        let color_type = match pixels.pixel_format {
            PixelFormat::Grayscale8 => image::ExtendedColorType::L8,
            PixelFormat::GrayscaleAlpha8 => image::ExtendedColorType::La8,
            PixelFormat::Rgb8 => image::ExtendedColorType::Rgb8,
            PixelFormat::Rgba8 => image::ExtendedColorType::Rgba8,
        };

        encoder
            .write_image(&pixels.data, pixels.width, pixels.height, color_type)
            .map_err(|e| CodecError::EncodeError(format!("PNG encode error: {e}")))?;

        if preserve.preserve_xmp {
            if let Some(ref xmp) = pixels.metadata.xmp {
                if !xmp.is_empty() {
                    encoded_bytes = insert_png_xmp_itxt(encoded_bytes, xmp);
                }
            }
        }

        // Apply OxiPNG lossless compression optimization
        let opt_level = options.effort.clamp(1, 6);
        let mut oxipng_opts = oxipng::Options::from_preset(opt_level);
        oxipng_opts.fix_errors = true;
        oxipng_opts.strip = if preserve.any() {
            let mut keep = oxipng::IndexSet::new();
            if preserve.preserve_icc {
                keep.insert(*b"iCCP");
                keep.insert(*b"sRGB");
            }
            if preserve.preserve_exif {
                keep.insert(*b"eXIf");
            }
            if preserve.preserve_xmp {
                keep.insert(*b"iTXt");
            }
            StripChunks::Keep(keep)
        } else {
            StripChunks::All
        };

        if let Ok(optimized) = oxipng::optimize_from_memory(&encoded_bytes, &oxipng_opts) {
            if optimized.len() < encoded_bytes.len() || !preserve.any() {
                encoded_bytes = optimized;
            }
        }

        Ok(EncodedOutput {
            format: CodecFormat::Png,
            bytes: encoded_bytes,
            width: pixels.width,
            height: pixels.height,
            effective_options: options.clone(),
        })
    }
}

fn insert_png_xmp_itxt(png_bytes: Vec<u8>, xmp: &[u8]) -> Vec<u8> {
    if png_bytes.len() < 8 || &png_bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return png_bytes;
    }

    // Find the offset of the first IDAT chunk so PngDecoder::read_info() reads iTXt before IDAT
    let mut pos = 8usize;
    let mut insert_at = None;
    while pos + 12 <= png_bytes.len() {
        let len = u32::from_be_bytes([
            png_bytes[pos],
            png_bytes[pos + 1],
            png_bytes[pos + 2],
            png_bytes[pos + 3],
        ]) as usize;
        let chunk_type = &png_bytes[pos + 4..pos + 8];
        if chunk_type == b"IDAT" || chunk_type == b"IEND" {
            insert_at = Some(pos);
            break;
        }
        let Some(next) = pos.checked_add(12).and_then(|p| p.checked_add(len)) else {
            return png_bytes;
        };
        pos = next;
    }
    let Some(split_pos) = insert_at else {
        return png_bytes;
    };

    let mut payload = Vec::with_capacity(22 + xmp.len());
    payload.extend_from_slice(b"XML:com.adobe.xmp\0\0\0\0\0");
    payload.extend_from_slice(xmp);

    let Ok(len) = u32::try_from(payload.len()) else {
        return png_bytes;
    };

    let mut crc_input = Vec::with_capacity(4 + payload.len());
    crc_input.extend_from_slice(b"iTXt");
    crc_input.extend_from_slice(&payload);
    let crc = png_crc32(&crc_input);

    let mut out = Vec::with_capacity(png_bytes.len() + 12 + payload.len());
    out.extend_from_slice(&png_bytes[..split_pos]);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&crc_input);
    out.extend_from_slice(&crc.to_be_bytes());
    out.extend_from_slice(&png_bytes[split_pos..]);
    out
}

fn png_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::{ImageMetadataChunks, MetadataPreservation};

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
    fn test_png_metadata_preservation_and_stripping() {
        let codec = PngCodec;
        let icc = b"fake-png-icc".to_vec();
        let exif = vec![
            0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let xmp = b"<x:xmpmeta/>".to_vec();

        let pixels = DecodedPixels::with_metadata(
            4,
            4,
            PixelFormat::Rgb8,
            vec![120; 4 * 4 * 3],
            ImageMetadataChunks {
                icc_profile: Some(icc.clone()),
                exif: Some(exif.clone()),
                xmp: Some(xmp.clone()),
            },
        );

        let opts_keep = EncodeOptions {
            metadata_preservation: MetadataPreservation {
                preserve_exif: true,
                preserve_icc: true,
                preserve_xmp: true,
            },
            ..Default::default()
        };
        let encoded = codec.encode(&pixels, &opts_keep).unwrap();
        let decoded = codec.decode(&encoded.bytes).unwrap();
        assert_eq!(decoded.metadata.icc_profile.as_deref(), Some(icc.as_slice()));
        assert_eq!(decoded.metadata.exif.as_deref(), Some(exif.as_slice()));
        assert_eq!(decoded.metadata.xmp.as_deref(), Some(xmp.as_slice()));

        let encoded_strip = codec.encode(&pixels, &EncodeOptions::default()).unwrap();
        let decoded_strip = codec.decode(&encoded_strip.bytes).unwrap();
        assert!(decoded_strip.metadata.is_empty());
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
