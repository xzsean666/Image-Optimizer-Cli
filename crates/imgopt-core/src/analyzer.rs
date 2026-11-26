use std::fs::File;
use std::io::Read;
use std::path::Path;

use imgopt_codecs::DecodeLimits;
use serde::{Deserialize, Serialize};

use crate::detect::detect_format;
use crate::error::{ErrorKind, ImgoptError, PipelineStage, Result};
use crate::model::{ColorType, Dimensions, ImageFormat, ImageMetadata, ImageProfile, RawImage};

/// Resource constraints to protect memory and CPU usage during decode and analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub max_input_bytes: u64,
    pub max_pixels: u64,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 100 * 1024 * 1024, // 100 MB
            max_pixels: 64 * 1024 * 1024,       // 64 MP (e.g., 8192 x 8192)
            max_width: Some(16_384),
            max_height: Some(16_384),
        }
    }
}

impl ResourceLimits {
    pub fn to_decode_limits(&self) -> DecodeLimits {
        let max_alloc_bytes = self.max_pixels.checked_mul(4).map(|b| b.max(1024 * 1024));
        DecodeLimits {
            max_width: self.max_width,
            max_height: self.max_height,
            max_pixels: Some(self.max_pixels),
            max_alloc_bytes,
            max_threads: None,
        }
    }
}

/// Read a file safely into memory enforcing `max_bytes` both via `metadata()` before
/// allocating and via bounded `Read::take` against TOCTOU file growth.
pub fn read_file_bounded(path: &Path, max_bytes: u64, stage: PipelineStage) -> Result<Vec<u8>> {
    let file = File::open(path).map_err(|e| {
        ImgoptError::new(stage, ErrorKind::Io(format!("failed to open file: {e}"))).with_path(path)
    })?;

    let meta = file.metadata().map_err(|e| {
        ImgoptError::new(
            stage,
            ErrorKind::Io(format!("failed to read file metadata: {e}")),
        )
        .with_path(path)
    })?;

    if meta.len() > max_bytes {
        return Err(ImgoptError::new(
            PipelineStage::ResourceCheck,
            ErrorKind::ResourceLimitExceeded(format!(
                "input size {} bytes exceeds configured limit of {} bytes",
                meta.len(),
                max_bytes
            )),
        )
        .with_path(path));
    }

    let capacity = usize::try_from(meta.len().min(max_bytes)).unwrap_or(0);
    let mut buf = Vec::with_capacity(capacity);
    let read_limit = max_bytes.saturating_add(1);
    file.take(read_limit).read_to_end(&mut buf).map_err(|e| {
        ImgoptError::new(stage, ErrorKind::Io(format!("failed to read file: {e}"))).with_path(path)
    })?;

    if (buf.len() as u64) > max_bytes {
        return Err(ImgoptError::new(
            PipelineStage::ResourceCheck,
            ErrorKind::ResourceLimitExceeded(format!(
                "input size {} bytes exceeds configured limit of {} bytes",
                buf.len(),
                max_bytes
            )),
        )
        .with_path(path));
    }

    Ok(buf)
}

/// Dynamic characteristics and classification hints discovered during decode.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ImageCharacteristics {
    /// True if format/channels have alpha and at least one pixel is translucent/transparent.
    pub has_transparency: bool,
    /// True if the image is grayscale or RGB with identical R=G=B on all pixels.
    pub is_grayscale: bool,
    /// Estimated Shannon entropy per byte (0.0..=8.0), useful as a compression compressibility hint.
    pub entropy_hint: f64,
}

/// Outcome of image analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisResult {
    pub profile: ImageProfile,
    pub raw_image: RawImage,
    pub characteristics: ImageCharacteristics,
}

/// Analyze source image bytes with resource guardrails.
pub fn analyze_image(bytes: &[u8], limits: Option<&ResourceLimits>) -> Result<AnalysisResult> {
    let default_limits = ResourceLimits::default();
    let limits = limits.unwrap_or(&default_limits);

    if (bytes.len() as u64) > limits.max_input_bytes {
        return Err(ImgoptError::new(
            PipelineStage::ResourceCheck,
            ErrorKind::ResourceLimitExceeded(format!(
                "input size {} bytes exceeds configured limit of {} bytes",
                bytes.len(),
                limits.max_input_bytes
            )),
        ));
    }

    let format = detect_format(bytes)?;

    let codec = imgopt_codecs::codec_for_format(format.into());
    let decode_limits = limits.to_decode_limits();
    let decoded = codec
        .decode_with_limits(bytes, &decode_limits)
        .map_err(|e| match e {
            imgopt_codecs::CodecError::ResourceLimitExceeded(msg) => ImgoptError::new(
                PipelineStage::ResourceCheck,
                ErrorKind::ResourceLimitExceeded(msg),
            ),
            imgopt_codecs::CodecError::AnimatedImage(msg)
            | imgopt_codecs::CodecError::UnsupportedFormat(msg) => {
                ImgoptError::new(PipelineStage::Decode, ErrorKind::UnsupportedFormat(msg))
            }
            other => ImgoptError::new(
                PipelineStage::Decode,
                ErrorKind::Decode(format!("failed to decode image during analysis: {other}")),
            ),
        })?;

    let dimensions = Dimensions::new(decoded.width, decoded.height);
    let pixel_count = dimensions.pixel_count();

    if pixel_count > limits.max_pixels {
        return Err(ImgoptError::new(
            PipelineStage::ResourceCheck,
            ErrorKind::ResourceLimitExceeded(format!(
                "pixel count {pixel_count} exceeds configured limit of {} pixels",
                limits.max_pixels
            )),
        ));
    }

    if let Some(max_w) = limits.max_width {
        if dimensions.width > max_w {
            return Err(ImgoptError::new(
                PipelineStage::ResourceCheck,
                ErrorKind::ResourceLimitExceeded(format!(
                    "image width {} exceeds configured limit of {} pixels",
                    dimensions.width, max_w
                )),
            ));
        }
    }

    if let Some(max_h) = limits.max_height {
        if dimensions.height > max_h {
            return Err(ImgoptError::new(
                PipelineStage::ResourceCheck,
                ErrorKind::ResourceLimitExceeded(format!(
                    "image height {} exceeds configured limit of {} pixels",
                    dimensions.height, max_h
                )),
            ));
        }
    }

    let raw_image: RawImage = decoded.into();

    // Combine decoder-extracted metadata with container header probing (e.g. for XMP)
    let mut metadata = probe_metadata_markers(bytes, format);
    if raw_image.metadata.exif.is_some() {
        metadata.has_exif = true;
    }
    if raw_image.metadata.icc_profile.is_some() {
        metadata.has_icc = true;
    }
    if raw_image.metadata.xmp.is_some() {
        metadata.has_xmp = true;
    }

    let characteristics = inspect_characteristics(&raw_image);

    let profile = ImageProfile {
        format,
        dimensions,
        color_type: raw_image.color_type,
        byte_size: bytes.len() as u64,
        metadata,
    };

    Ok(AnalysisResult {
        profile,
        raw_image,
        characteristics,
    })
}

/// Structurally parse container markers/chunks rather than scanning raw entropy.
pub(crate) fn probe_metadata_markers(bytes: &[u8], format: ImageFormat) -> ImageMetadata {
    let mut meta = ImageMetadata::default();
    match format {
        ImageFormat::Jpeg => {
            if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
                return meta;
            }
            let mut pos = 2usize;
            while pos + 1 < bytes.len() {
                if bytes[pos] != 0xFF {
                    pos += 1;
                    continue;
                }
                while pos < bytes.len() && bytes[pos] == 0xFF {
                    pos += 1;
                }
                if pos >= bytes.len() {
                    break;
                }
                let marker = bytes[pos];
                pos += 1;

                // Standalone markers (TEM, RST0..7, SOI, EOI) or SOS (compressed scan start)
                if marker == 0x00 || marker == 0x01 || (0xD0..=0xD9).contains(&marker) {
                    if marker == 0xD9 {
                        break;
                    }
                    continue;
                }
                if marker == 0xDA {
                    // Start of Scan: metadata markers precede SOS
                    break;
                }
                if pos + 2 > bytes.len() {
                    break;
                }
                let seg_len = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
                if seg_len < 2 || pos + seg_len > bytes.len() {
                    break;
                }
                let payload = &bytes[pos + 2..pos + seg_len];
                if marker == 0xE1 {
                    if payload.starts_with(b"Exif\0\0") || payload.starts_with(b"Exif\0\xFF") {
                        meta.has_exif = true;
                    } else if payload.starts_with(b"http://ns.adobe.com/xap/1.0/\0") {
                        meta.has_xmp = true;
                    }
                } else if marker == 0xE2 && payload.starts_with(b"ICC_PROFILE\0") {
                    meta.has_icc = true;
                }
                pos += seg_len;
            }
        }
        ImageFormat::Png => {
            if bytes.len() < 8 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
                return meta;
            }
            let mut pos = 8usize;
            while pos + 12 <= bytes.len() {
                let len = u32::from_be_bytes([
                    bytes[pos],
                    bytes[pos + 1],
                    bytes[pos + 2],
                    bytes[pos + 3],
                ]) as usize;
                let chunk_type = &bytes[pos + 4..pos + 8];
                let data_start = pos + 8;
                let Some(data_end) = data_start.checked_add(len) else {
                    break;
                };
                let Some(next_pos) = data_end.checked_add(4) else {
                    break;
                };
                if next_pos > bytes.len() {
                    break;
                }
                let payload = &bytes[data_start..data_end];
                match chunk_type {
                    b"eXIf" => meta.has_exif = true,
                    b"iCCP" => meta.has_icc = true,
                    b"iTXt" | b"tEXt" | b"zTXt" => {
                        if payload.starts_with(b"XML:com.adobe.xmp\0")
                            || payload
                                .windows(28)
                                .any(|w| w == b"http://ns.adobe.com/xap/1.0/")
                        {
                            meta.has_xmp = true;
                        }
                    }
                    b"IEND" => break,
                    _ => {}
                }
                pos = next_pos;
            }
        }
        ImageFormat::Webp => {
            if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
                return meta;
            }
            let mut pos = 12usize;
            while pos + 8 <= bytes.len() {
                let fourcc = &bytes[pos..pos + 4];
                let len = u32::from_le_bytes([
                    bytes[pos + 4],
                    bytes[pos + 5],
                    bytes[pos + 6],
                    bytes[pos + 7],
                ]) as usize;
                match fourcc {
                    b"EXIF" => meta.has_exif = true,
                    b"ICCP" => meta.has_icc = true,
                    b"XMP " => meta.has_xmp = true,
                    _ => {}
                }
                let padded = len.saturating_add(len & 1);
                let Some(next_pos) = pos.checked_add(8).and_then(|p| p.checked_add(padded)) else {
                    break;
                };
                pos = next_pos;
            }
        }
        ImageFormat::Avif => {
            // Scan ISO BMFF container boxes for ICC ('colr' with 'prof'/'rICC') or Exif/XMP item markers
            if bytes.windows(4).any(|w| w == b"prof" || w == b"rICC") {
                meta.has_icc = true;
            }
            if bytes.windows(4).any(|w| w == b"Exif") {
                meta.has_exif = true;
            }
            if bytes.windows(17).any(|w| w == b"application/rdf+x") {
                meta.has_xmp = true;
            }
        }
        ImageFormat::Gif => {
            if bytes.windows(11).any(|w| w == b"XMP DataXMP") {
                meta.has_xmp = true;
            }
        }
    }
    meta
}

/// Returns true if the container carries any ancillary metadata/text/comment chunks
/// (EXIF, ICC, XMP, JPEG COM/APPn, PNG tEXt/zTXt/iTXt/tIME) that `StripAll` must remove.
pub(crate) fn has_ancillary_metadata(bytes: &[u8], format: ImageFormat) -> bool {
    let standard = probe_metadata_markers(bytes, format);
    if standard.has_any() {
        return true;
    }

    match format {
        ImageFormat::Jpeg => {
            if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
                return false;
            }
            let mut pos = 2usize;
            while pos + 1 < bytes.len() {
                if bytes[pos] != 0xFF {
                    pos += 1;
                    continue;
                }
                while pos < bytes.len() && bytes[pos] == 0xFF {
                    pos += 1;
                }
                if pos >= bytes.len() {
                    break;
                }
                let marker = bytes[pos];
                pos += 1;
                if marker == 0x00 || marker == 0x01 || (0xD0..=0xD9).contains(&marker) {
                    if marker == 0xD9 {
                        break;
                    }
                    continue;
                }
                if marker == 0xDA || pos + 2 > bytes.len() {
                    break;
                }
                let seg_len = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
                if seg_len < 2 || pos + seg_len > bytes.len() {
                    break;
                }
                // 0xFE = COM comment; 0xE1..=0xEF = APP1..APP15 (excluding standard JFIF APP0 0xE0)
                if marker == 0xFE || (0xE1..=0xEF).contains(&marker) {
                    return true;
                }
                pos += seg_len;
            }
            false
        }
        ImageFormat::Png => {
            if bytes.len() < 8 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
                return false;
            }
            let mut pos = 8usize;
            while pos + 12 <= bytes.len() {
                let len = u32::from_be_bytes([
                    bytes[pos],
                    bytes[pos + 1],
                    bytes[pos + 2],
                    bytes[pos + 3],
                ]) as usize;
                let chunk_type = &bytes[pos + 4..pos + 8];
                let Some(next_pos) = pos
                    .checked_add(8)
                    .and_then(|p| p.checked_add(len))
                    .and_then(|p| p.checked_add(4))
                else {
                    break;
                };
                if next_pos > bytes.len() {
                    break;
                }
                if matches!(
                    chunk_type,
                    b"eXIf" | b"iCCP" | b"iTXt" | b"tEXt" | b"zTXt" | b"tIME"
                ) {
                    return true;
                }
                if chunk_type == b"IEND" {
                    break;
                }
                pos = next_pos;
            }
            false
        }
        ImageFormat::Webp | ImageFormat::Avif => false,
        ImageFormat::Gif => {
            // GIF Comment extension (0x21 0xFE) or XMP Application Extension
            bytes.windows(2).any(|w| w == [0x21, 0xFE])
        }
    }
}

fn inspect_characteristics(raw: &RawImage) -> ImageCharacteristics {
    let mut has_transparency = false;
    let mut is_grayscale = true;

    match raw.color_type {
        ColorType::Grayscale8 => {
            is_grayscale = true;
            has_transparency = false;
        }
        ColorType::GrayscaleAlpha8 => {
            is_grayscale = true;
            for chunk in raw.data.as_chunks::<2>().0 {
                if chunk[1] < 255 {
                    has_transparency = true;
                    break;
                }
            }
        }
        ColorType::Rgb8 => {
            has_transparency = false;
            for chunk in raw.data.as_chunks::<3>().0 {
                if chunk[0] != chunk[1] || chunk[1] != chunk[2] {
                    is_grayscale = false;
                    break;
                }
            }
        }
        ColorType::Rgba8 => {
            for chunk in raw.data.as_chunks::<4>().0 {
                if chunk[3] < 255 {
                    has_transparency = true;
                }
                if chunk[0] != chunk[1] || chunk[1] != chunk[2] {
                    is_grayscale = false;
                }
                if has_transparency && !is_grayscale {
                    break;
                }
            }
        }
    }

    let entropy_hint = calculate_entropy(&raw.data);

    ImageCharacteristics {
        has_transparency,
        is_grayscale,
        entropy_hint,
    }
}

fn calculate_entropy(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let mut counts = [0u64; 256];
    for &b in data {
        counts[b as usize] += 1;
    }
    let total = data.len() as f64;
    let mut entropy = 0.0;
    for &count in &counts {
        if count > 0 {
            let p = count as f64 / total;
            entropy -= p * p.log2();
        }
    }
    entropy
}

#[cfg(test)]
mod tests {
    use super::*;
    use imgopt_codecs::{
        DecodedPixels, EncodeOptions, ImageCodec, JpegCodec, PixelFormat, PngCodec, WebpCodec,
    };

    fn make_test_jpeg() -> Vec<u8> {
        let codec = JpegCodec;
        let pixels = DecodedPixels::new(4, 4, PixelFormat::Rgb8, vec![128; 48]);
        codec
            .encode(&pixels, &EncodeOptions::default())
            .unwrap()
            .bytes
    }

    fn make_test_png_rgba(transparent: bool) -> Vec<u8> {
        let codec = PngCodec;
        let mut rgba = vec![255, 0, 0, 255, 0, 255, 0, 255];
        if transparent {
            rgba[7] = 120;
        }
        let pixels = DecodedPixels::new(2, 1, PixelFormat::Rgba8, rgba);
        codec
            .encode(&pixels, &EncodeOptions::default())
            .unwrap()
            .bytes
    }

    fn make_test_webp() -> Vec<u8> {
        let codec = WebpCodec;
        let pixels = DecodedPixels::new(4, 4, PixelFormat::Rgb8, vec![200; 48]);
        codec
            .encode(&pixels, &EncodeOptions::default())
            .unwrap()
            .bytes
    }

    #[test]
    fn test_analyze_jpeg_png_webp() {
        let jpeg_bytes = make_test_jpeg();
        let res_jpeg = analyze_image(&jpeg_bytes, None).unwrap();
        assert_eq!(res_jpeg.profile.format, ImageFormat::Jpeg);
        assert_eq!(res_jpeg.profile.dimensions, Dimensions::new(4, 4));
        assert!(!res_jpeg.characteristics.has_transparency);

        let png_bytes = make_test_png_rgba(true);
        let res_png = analyze_image(&png_bytes, None).unwrap();
        assert_eq!(res_png.profile.format, ImageFormat::Png);
        assert_eq!(res_png.profile.dimensions, Dimensions::new(2, 1));
        assert!(res_png.characteristics.has_transparency);

        let webp_bytes = make_test_webp();
        let res_webp = analyze_image(&webp_bytes, None).unwrap();
        assert_eq!(res_webp.profile.format, ImageFormat::Webp);
        assert_eq!(res_webp.profile.dimensions, Dimensions::new(4, 4));
    }

    #[test]
    fn test_resource_limit_exceeded() {
        let jpeg_bytes = make_test_jpeg();
        let tight_limits = ResourceLimits {
            max_input_bytes: 10,
            max_pixels: 64,
            max_width: None,
            max_height: None,
        };

        let err = analyze_image(&jpeg_bytes, Some(&tight_limits)).unwrap_err();
        assert_eq!(err.stage, PipelineStage::ResourceCheck);
    }
}
