use crate::decode_with_image_reader;
use crate::interface::{
    CodecCapabilities, CodecError, CodecFormat, DecodeLimits, DecodedPixels, EncodeOptions,
    EncodedOutput, ImageCodec, ImageMetadataChunks, MetadataPreservation, PixelFormat,
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
        self.decode_with_limits(input_bytes, &DecodeLimits::default())
    }

    fn decode_with_limits(
        &self,
        input_bytes: &[u8],
        limits: &DecodeLimits,
    ) -> Result<DecodedPixels, CodecError> {
        decode_with_image_reader(
            input_bytes,
            image::ImageFormat::WebP,
            CodecFormat::Webp,
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

        let mut config = webp::WebPConfig::new().map_err(|()| {
            CodecError::EncodeError("failed to initialize WebP encoder configuration".into())
        })?;
        config.lossless = if options.lossless { 1 } else { 0 };
        config.alpha_compression = if options.lossless { 0 } else { 1 };
        config.quality = options.quality.clamp(1, 100) as f32;
        config.method = i32::from(options.effort.clamp(1, 6));

        let mut encoded_bytes = match pixels.pixel_format {
            PixelFormat::Rgb8 => {
                let encoder = webp::Encoder::from_rgb(&pixels.data, pixels.width, pixels.height);
                encoder
                    .encode_advanced(&config)
                    .map_err(|e| CodecError::EncodeError(format!("WebP encode error: {e:?}")))?
                    .to_vec()
            }
            PixelFormat::Rgba8 => {
                let encoder = webp::Encoder::from_rgba(&pixels.data, pixels.width, pixels.height);
                encoder
                    .encode_advanced(&config)
                    .map_err(|e| CodecError::EncodeError(format!("WebP encode error: {e:?}")))?
                    .to_vec()
            }
            PixelFormat::Grayscale8 => {
                let mut rgb_data = Vec::with_capacity(pixels.data.len() * 3);
                for &l in &pixels.data {
                    rgb_data.push(l);
                    rgb_data.push(l);
                    rgb_data.push(l);
                }
                let encoder = webp::Encoder::from_rgb(&rgb_data, pixels.width, pixels.height);
                encoder
                    .encode_advanced(&config)
                    .map_err(|e| CodecError::EncodeError(format!("WebP encode error: {e:?}")))?
                    .to_vec()
            }
            PixelFormat::GrayscaleAlpha8 => {
                let mut rgba_data = Vec::with_capacity(pixels.data.len() * 2);
                for chunk in pixels.data.as_chunks::<2>().0 {
                    let l = chunk[0];
                    let a = chunk[1];
                    rgba_data.push(l);
                    rgba_data.push(l);
                    rgba_data.push(l);
                    rgba_data.push(a);
                }
                let encoder = webp::Encoder::from_rgba(&rgba_data, pixels.width, pixels.height);
                encoder
                    .encode_advanced(&config)
                    .map_err(|e| CodecError::EncodeError(format!("WebP encode error: {e:?}")))?
                    .to_vec()
            }
        };

        if encoded_bytes.is_empty() {
            return Err(CodecError::EncodeError(
                "WebP encoding produced empty output".into(),
            ));
        }

        let preserve = options.effective_preservation();
        if preserve.any() && !pixels.metadata.is_empty() {
            encoded_bytes = mux_webp_metadata(
                &encoded_bytes,
                pixels.width,
                pixels.height,
                pixels.pixel_format.has_alpha(),
                &pixels.metadata,
                preserve,
            )?;
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

/// Mux ICCP, EXIF, and XMP chunks into an extended WebP (VP8X) RIFF container.
fn mux_webp_metadata(
    webp_bytes: &[u8],
    width: u32,
    height: u32,
    has_alpha: bool,
    metadata: &ImageMetadataChunks,
    preserve: MetadataPreservation,
) -> Result<Vec<u8>, CodecError> {
    if webp_bytes.len() < 20 || &webp_bytes[0..4] != b"RIFF" || &webp_bytes[8..12] != b"WEBP" {
        return Err(CodecError::EncodeError(
            "invalid WebP RIFF container returned by encoder".into(),
        ));
    }

    let icc = preserve
        .preserve_icc
        .then_some(metadata.icc_profile.as_deref())
        .flatten()
        .filter(|b| !b.is_empty());
    let exif = preserve
        .preserve_exif
        .then_some(metadata.exif.as_deref())
        .flatten()
        .filter(|b| !b.is_empty());
    let xmp = preserve
        .preserve_xmp
        .then_some(metadata.xmp.as_deref())
        .flatten()
        .filter(|b| !b.is_empty());

    if icc.is_none() && exif.is_none() && xmp.is_none() {
        return Ok(webp_bytes.to_vec());
    }

    let Some((w_m1, h_m1)) = width.checked_sub(1).zip(height.checked_sub(1)) else {
        return Err(CodecError::InvalidParameter(
            "WebP dimensions cannot be zero".into(),
        ));
    };
    if w_m1 > 0x00FF_FFFF || h_m1 > 0x00FF_FFFF {
        return Err(CodecError::InvalidParameter(
            "WebP dimensions exceed 24-bit VP8X limit".into(),
        ));
    }

    // Inspect existing image chunks before emitting VP8X flags so the alpha bit accurately
    // reflects whether the encoded bitstream actually carries ALPH / VP8L alpha data.
    let mut chunk_slices: Vec<([u8; 4], &[u8])> = Vec::new();
    let mut saw_alph_or_vp8l = false;
    let mut saw_vp8x_alpha = false;
    let mut pos = 12usize;
    while pos + 8 <= webp_bytes.len() {
        let fourcc = &webp_bytes[pos..pos + 4];
        let size = u32::from_le_bytes([
            webp_bytes[pos + 4],
            webp_bytes[pos + 5],
            webp_bytes[pos + 6],
            webp_bytes[pos + 7],
        ]) as usize;
        let data_start = pos + 8;
        let data_end = data_start
            .checked_add(size)
            .filter(|&end| end <= webp_bytes.len())
            .ok_or_else(|| {
                CodecError::EncodeError("corrupt WebP RIFF chunk length during muxing".into())
            })?;

        if fourcc == b"VP8X" && size >= 1 {
            saw_vp8x_alpha = (webp_bytes[data_start] & (1 << 4)) != 0;
        }
        if fourcc == b"ALPH" || fourcc == b"VP8L" {
            saw_alph_or_vp8l = true;
        }
        if fourcc != b"VP8X" && fourcc != b"ICCP" && fourcc != b"EXIF" && fourcc != b"XMP " {
            let fourcc_arr: [u8; 4] = fourcc
                .try_into()
                .map_err(|_| CodecError::EncodeError("invalid WebP FourCC chunk header".into()))?;
            chunk_slices.push((fourcc_arr, &webp_bytes[data_start..data_end]));
        }

        let padded = size + (size & 1);
        pos = data_start.saturating_add(padded);
    }

    let mut flags: u8 = 0;
    if icc.is_some() {
        flags |= 1 << 5; // ICC profile
    }
    if has_alpha && (saw_alph_or_vp8l || saw_vp8x_alpha) {
        flags |= 1 << 4; // Alpha
    }
    if exif.is_some() {
        flags |= 1 << 3; // EXIF
    }
    if xmp.is_some() {
        flags |= 1 << 2; // XMP
    }

    let mut body = Vec::with_capacity(webp_bytes.len() + 64);
    body.extend_from_slice(b"WEBP");

    // Write VP8X chunk (10 bytes payload)
    body.extend_from_slice(b"VP8X");
    body.extend_from_slice(&10u32.to_le_bytes());
    body.push(flags);
    body.extend_from_slice(&[0u8; 3]);
    body.extend_from_slice(&w_m1.to_le_bytes()[..3]);
    body.extend_from_slice(&h_m1.to_le_bytes()[..3]);

    // ICCP must precede image bitstream chunks per WebP spec
    if let Some(data) = icc {
        append_riff_chunk(&mut body, b"ICCP", data)?;
    }

    for (fourcc_arr, slice) in chunk_slices {
        append_riff_chunk(&mut body, &fourcc_arr, slice)?;
    }

    if let Some(data) = exif {
        append_riff_chunk(&mut body, b"EXIF", data)?;
    }
    if let Some(data) = xmp {
        append_riff_chunk(&mut body, b"XMP ", data)?;
    }

    let riff_len = u32::try_from(body.len()).map_err(|_| {
        CodecError::EncodeError("WebP output exceeds 4GB RIFF container limit".into())
    })?;
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

fn append_riff_chunk(buf: &mut Vec<u8>, fourcc: &[u8; 4], data: &[u8]) -> Result<(), CodecError> {
    let len = u32::try_from(data.len()).map_err(|_| {
        CodecError::EncodeError("WebP metadata chunk exceeds 4GB RIFF chunk limit".into())
    })?;
    buf.extend_from_slice(fourcc);
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(data);
    if data.len() % 2 == 1 {
        buf.push(0);
    }
    Ok(())
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
            ..Default::default()
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
            ..Default::default()
        };

        let encoded = codec.encode(&pixels, &options).unwrap();
        assert_eq!(encoded.format, CodecFormat::Webp);

        let decoded = codec.decode(&encoded.bytes).unwrap();
        assert_eq!(decoded.width, width);
        assert_eq!(decoded.height, height);
    }

    #[test]
    fn test_webp_metadata_preservation_and_stripping() {
        let codec = WebpCodec;
        let icc = b"fake-webp-icc".to_vec();
        let exif = vec![
            0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let xmp = b"<x:xmpmeta/>".to_vec();

        let pixels = DecodedPixels::with_metadata(
            4,
            4,
            PixelFormat::Rgb8,
            vec![128; 4 * 4 * 3],
            ImageMetadataChunks {
                icc_profile: Some(icc.clone()),
                exif: Some(exif.clone()),
                xmp: Some(xmp.clone()),
            },
        );

        let opts_keep = EncodeOptions {
            preserve_metadata: true,
            ..Default::default()
        };
        let encoded = codec.encode(&pixels, &opts_keep).unwrap();
        let decoded = codec.decode(&encoded.bytes).unwrap();
        assert_eq!(
            decoded.metadata.icc_profile.as_deref(),
            Some(icc.as_slice())
        );
        assert_eq!(decoded.metadata.exif.as_deref(), Some(exif.as_slice()));
        assert_eq!(decoded.metadata.xmp.as_deref(), Some(xmp.as_slice()));

        let encoded_strip = codec.encode(&pixels, &EncodeOptions::default()).unwrap();
        let decoded_strip = codec.decode(&encoded_strip.bytes).unwrap();
        assert!(decoded_strip.metadata.is_empty());
    }

    #[test]
    fn test_webp_opaque_rgba_with_metadata_roundtrips() {
        let codec = WebpCodec;
        let pixels = DecodedPixels::with_metadata(
            4,
            4,
            PixelFormat::Rgba8,
            [100, 150, 200, 255].repeat(16),
            ImageMetadataChunks {
                icc_profile: Some(b"icc-opaque".to_vec()),
                exif: None,
                xmp: Some(b"<x:xmpmeta/>".to_vec()),
            },
        );
        let encoded = codec
            .encode(
                &pixels,
                &EncodeOptions {
                    preserve_metadata: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let decoded = codec.decode(&encoded.bytes).unwrap();
        assert_eq!(decoded.width, 4);
        assert_eq!(decoded.height, 4);
        assert_eq!(
            decoded.metadata.icc_profile.as_deref(),
            Some(b"icc-opaque".as_slice())
        );
        assert_eq!(
            decoded.metadata.xmp.as_deref(),
            Some(b"<x:xmpmeta/>".as_slice())
        );
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
