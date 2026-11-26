use crate::decode_with_image_reader;
use crate::interface::{
    CodecCapabilities, CodecError, CodecFormat, DecodeLimits, DecodedPixels, EncodeOptions,
    EncodedOutput, ImageCodec, PixelFormat,
};

const XMP_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

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
        self.decode_with_limits(input_bytes, &DecodeLimits::default())
    }

    fn decode_with_limits(
        &self,
        input_bytes: &[u8],
        limits: &DecodeLimits,
    ) -> Result<DecodedPixels, CodecError> {
        decode_with_image_reader(
            input_bytes,
            image::ImageFormat::Jpeg,
            CodecFormat::Jpeg,
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

        let width_u16 = u16::try_from(pixels.width).map_err(|_| {
            CodecError::InvalidParameter(format!(
                "JPEG width {} exceeds 16-bit maximum of 65535",
                pixels.width
            ))
        })?;
        let height_u16 = u16::try_from(pixels.height).map_err(|_| {
            CodecError::InvalidParameter(format!(
                "JPEG height {} exceeds 16-bit maximum of 65535",
                pixels.height
            ))
        })?;

        let quality = options.quality.clamp(1, 100);
        let mut encoded_bytes = Vec::new();
        let mut encoder = jpeg_encoder::Encoder::new(&mut encoded_bytes, quality);
        encoder.set_optimized_huffman_tables(true);
        if options.effort >= 4 {
            encoder.set_progressive(true);
        }
        // Use 4:4:4 chroma when near-lossless quality is requested or effort == 1,
        // so high-SSIM targets (e.g. Maximum preset) remain achievable on sharp graphics.
        if quality >= 95 || options.effort <= 1 {
            encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_4_4);
        } else {
            encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_2_0);
        }

        let preserve = options.effective_preservation();
        if preserve.preserve_icc {
            if let Some(ref icc) = pixels.metadata.icc_profile {
                if !icc.is_empty() {
                    encoder.add_icc_profile(icc).map_err(|e| {
                        CodecError::EncodeError(format!("failed to embed JPEG ICC profile: {e}"))
                    })?;
                }
            }
        }
        if preserve.preserve_exif {
            if let Some(ref exif) = pixels.metadata.exif {
                if !exif.is_empty() {
                    if exif.len() > 65_527 {
                        return Err(CodecError::EncodeError(format!(
                            "EXIF payload ({} bytes) exceeds JPEG APP1 65527-byte limit",
                            exif.len()
                        )));
                    }
                    encoder.add_exif_metadata(exif).map_err(|e| {
                        CodecError::EncodeError(format!("failed to embed JPEG EXIF metadata: {e}"))
                    })?;
                }
            }
        }
        if preserve.preserve_xmp {
            if let Some(ref xmp) = pixels.metadata.xmp {
                if !xmp.is_empty() {
                    let mut seg = Vec::with_capacity(XMP_HEADER.len() + xmp.len());
                    if !xmp.starts_with(XMP_HEADER) {
                        seg.extend_from_slice(XMP_HEADER);
                    }
                    seg.extend_from_slice(xmp);
                    if seg.len() > 65_533 {
                        return Err(CodecError::EncodeError(format!(
                            "XMP payload ({} bytes) exceeds JPEG APP1 65533-byte limit",
                            seg.len()
                        )));
                    }
                    encoder.add_app_segment(1, seg).map_err(|e| {
                        CodecError::EncodeError(format!("failed to embed JPEG XMP metadata: {e}"))
                    })?;
                }
            }
        }

        match pixels.pixel_format {
            PixelFormat::Grayscale8 => {
                encoder
                    .encode(
                        &pixels.data,
                        width_u16,
                        height_u16,
                        jpeg_encoder::ColorType::Luma,
                    )
                    .map_err(|e| CodecError::EncodeError(format!("JPEG encode error: {e}")))?;
            }
            PixelFormat::Rgb8 => {
                encoder
                    .encode(
                        &pixels.data,
                        width_u16,
                        height_u16,
                        jpeg_encoder::ColorType::Rgb,
                    )
                    .map_err(|e| CodecError::EncodeError(format!("JPEG encode error: {e}")))?;
            }
            PixelFormat::Rgba8 => {
                let rgb_data = flatten_rgba_on_white(&pixels.data);
                encoder
                    .encode(
                        &rgb_data,
                        width_u16,
                        height_u16,
                        jpeg_encoder::ColorType::Rgb,
                    )
                    .map_err(|e| CodecError::EncodeError(format!("JPEG encode error: {e}")))?;
            }
            PixelFormat::GrayscaleAlpha8 => {
                let gray_data = flatten_la_on_white(&pixels.data);
                encoder
                    .encode(
                        &gray_data,
                        width_u16,
                        height_u16,
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

fn flatten_rgba_on_white(rgba: &[u8]) -> Vec<u8> {
    let mut rgb_data = Vec::with_capacity((rgba.len() / 4) * 3);
    for chunk in rgba.as_chunks::<4>().0 {
        let a = u32::from(chunk[3]);
        let inv = 255 - a;
        rgb_data.push(((u32::from(chunk[0]) * a + 255 * inv) / 255) as u8);
        rgb_data.push(((u32::from(chunk[1]) * a + 255 * inv) / 255) as u8);
        rgb_data.push(((u32::from(chunk[2]) * a + 255 * inv) / 255) as u8);
    }
    rgb_data
}

fn flatten_la_on_white(la: &[u8]) -> Vec<u8> {
    let mut gray_data = Vec::with_capacity(la.len() / 2);
    for chunk in la.as_chunks::<2>().0 {
        let a = u32::from(chunk[1]);
        let inv = 255 - a;
        gray_data.push(((u32::from(chunk[0]) * a + 255 * inv) / 255) as u8);
    }
    gray_data
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::{ImageMetadataChunks, MetadataPreservation};

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
            ..Default::default()
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
    fn test_jpeg_rejects_dimensions_above_u16() {
        let codec = JpegCodec;
        // Construct invalid-for-JPEG dimension without allocating multi-GB via manual check
        let pixels = DecodedPixels::new(70_000, 1, PixelFormat::Grayscale8, vec![0; 70_000]);
        let err = codec
            .encode(&pixels, &EncodeOptions::default())
            .unwrap_err();
        assert!(matches!(err, CodecError::InvalidParameter(_)));
    }

    #[test]
    fn test_jpeg_metadata_preservation() {
        let codec = JpegCodec;
        let exif_tiff = vec![
            0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let xmp = b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>".to_vec();
        let icc = b"fake-icc-profile".to_vec();

        let pixels = DecodedPixels::with_metadata(
            4,
            4,
            PixelFormat::Rgb8,
            vec![128; 4 * 4 * 3],
            ImageMetadataChunks {
                icc_profile: Some(icc.clone()),
                exif: Some(exif_tiff.clone()),
                xmp: Some(xmp.clone()),
            },
        );

        let opts_preserve = EncodeOptions {
            metadata_preservation: MetadataPreservation {
                preserve_exif: true,
                preserve_icc: true,
                preserve_xmp: true,
            },
            ..Default::default()
        };
        let encoded = codec.encode(&pixels, &opts_preserve).unwrap();
        let decoded = codec.decode(&encoded.bytes).unwrap();
        assert_eq!(
            decoded.metadata.icc_profile.as_deref(),
            Some(icc.as_slice())
        );
        assert_eq!(decoded.metadata.exif.as_deref(), Some(exif_tiff.as_slice()));
        assert_eq!(decoded.metadata.xmp.as_deref(), Some(xmp.as_slice()));

        let encoded_stripped = codec.encode(&pixels, &EncodeOptions::default()).unwrap();
        let decoded_stripped = codec.decode(&encoded_stripped.bytes).unwrap();
        assert!(decoded_stripped.metadata.is_empty());
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
