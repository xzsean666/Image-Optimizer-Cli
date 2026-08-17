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
            max_width: Some(16384),
            max_height: Some(16384),
        }
    }
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
    let decoded = codec.decode(bytes).map_err(|e| {
        ImgoptError::new(
            PipelineStage::Decode,
            ErrorKind::Decode(format!("failed to decode image during analysis: {e}")),
        )
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

    // Check metadata markers in bytes
    let metadata = probe_metadata_markers(bytes, format);

    // Compute pixel characteristics
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

fn probe_metadata_markers(bytes: &[u8], format: ImageFormat) -> ImageMetadata {
    let mut meta = ImageMetadata::default();
    match format {
        ImageFormat::Jpeg => {
            // Check for EXIF (APP1: 0xFFE1 + "Exif"), ICC (APP2: 0xFFE2 + "ICC_PROFILE"), XMP ("http://ns.adobe.com/xap/1.0/")
            if bytes
                .windows(6)
                .any(|w| w == b"Exif\0\0" || w == b"Exif\0\xff")
            {
                meta.has_exif = true;
            }
            if bytes.windows(11).any(|w| w == b"ICC_PROFILE") {
                meta.has_icc = true;
            }
            if bytes
                .windows(28)
                .any(|w| w == b"http://ns.adobe.com/xap/1.0/")
            {
                meta.has_xmp = true;
            }
        }
        ImageFormat::Png => {
            // PNG chunks: eXIf, iCCP, XML:com.adobe.xmp
            if bytes.windows(4).any(|w| w == b"eXIf") {
                meta.has_exif = true;
            }
            if bytes.windows(4).any(|w| w == b"iCCP") {
                meta.has_icc = true;
            }
            if bytes
                .windows(4)
                .any(|w| w == b"iTXt" || w == b"tEXt" || w == b"zTXt")
                && bytes
                    .windows(28)
                    .any(|w| w == b"http://ns.adobe.com/xap/1.0/")
            {
                meta.has_xmp = true;
            }
        }
        ImageFormat::Webp => {
            // WebP chunks: EXIF, ICCP, XMP
            if bytes.windows(4).any(|w| w == b"EXIF") {
                meta.has_exif = true;
            }
            if bytes.windows(4).any(|w| w == b"ICCP") {
                meta.has_icc = true;
            }
            if bytes.windows(4).any(|w| w == b"XMP ") {
                meta.has_xmp = true;
            }
        }
    }
    meta
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
            for chunk in raw.data.chunks_exact(2) {
                if chunk[1] < 255 {
                    has_transparency = true;
                    break;
                }
            }
        }
        ColorType::Rgb8 => {
            has_transparency = false;
            for chunk in raw.data.chunks_exact(3) {
                if chunk[0] != chunk[1] || chunk[1] != chunk[2] {
                    is_grayscale = false;
                    break;
                }
            }
        }
        ColorType::Rgba8 => {
            for chunk in raw.data.chunks_exact(4) {
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

    // Estimate Shannon entropy across raw sample
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
            rgba[7] = 120; // translucent pixel
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
