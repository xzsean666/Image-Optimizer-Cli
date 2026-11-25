use std::borrow::Cow;

use imgopt_codecs::{DecodedPixels, EncodeOptions, ImageCodec, MetadataPreservation, PixelFormat};
use imgopt_quality::compute_quality_score;

use crate::candidate::{select_best_candidate, OptimizationCandidate, RankingCriteria};
use crate::config::MetadataPolicy;
use crate::error::{ErrorKind, ImgoptError, PipelineStage, Result};
use crate::model::{ColorType, ImageFormat, RawImage};

/// Parameters guiding the candidate exploration process.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchOptions {
    pub target_format: ImageFormat,
    pub quality_target: f64,
    pub effort: u8,
    pub quality_min: u8,
    pub quality_max: u8,
    pub quality_step: u8,
    pub try_lossless: bool,
    pub max_allowed_bytes: Option<u64>,
    pub metadata_policy: MetadataPolicy,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            target_format: ImageFormat::Webp,
            quality_target: 0.92,
            effort: 4,
            quality_min: 65,
            quality_max: 95,
            quality_step: 5,
            try_lossless: false,
            max_allowed_bytes: None,
            metadata_policy: MetadataPolicy::StripAll,
        }
    }
}

/// Detailed telemetry recorded during candidate search.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchDiagnostics {
    pub tested_candidates: Vec<OptimizationCandidate>,
    pub rejected_count: usize,
    pub total_evaluated: usize,
}

/// Output of the search process containing the winning candidate and telemetry.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub selected: OptimizationCandidate,
    pub diagnostics: SearchDiagnostics,
}

fn preservation_from_policy(policy: MetadataPolicy) -> MetadataPreservation {
    MetadataPreservation {
        preserve_exif: policy.preserves_exif(),
        preserve_icc: policy.preserves_icc(),
        preserve_xmp: policy.preserves_xmp(),
    }
}

/// Align reference pixel data to match candidate's decoded pixel format so cross-format
/// conversions (e.g. RGBA -> JPEG RGB, Grayscale -> WebP RGB, RGB -> OxiPNG Luma)
/// are compared apples-to-apples instead of silently failing on buffer length mismatch.
fn project_reference_for_candidate<'a>(
    raw: &'a RawImage,
    cand_format: PixelFormat,
) -> Cow<'a, [u8]> {
    let orig_format: PixelFormat = raw.color_type.into();
    if orig_format == cand_format {
        return Cow::Borrowed(&raw.data);
    }

    let num_pixels = (raw.dimensions.width as usize).saturating_mul(raw.dimensions.height as usize);
    match (orig_format, cand_format) {
        (PixelFormat::Grayscale8, PixelFormat::Rgb8) => {
            let mut rgb = Vec::with_capacity(num_pixels * 3);
            for &l in &raw.data {
                rgb.push(l);
                rgb.push(l);
                rgb.push(l);
            }
            Cow::Owned(rgb)
        }
        (PixelFormat::Grayscale8, PixelFormat::Rgba8) => {
            let mut rgba = Vec::with_capacity(num_pixels * 4);
            for &l in &raw.data {
                rgba.push(l);
                rgba.push(l);
                rgba.push(l);
                rgba.push(255);
            }
            Cow::Owned(rgba)
        }
        (PixelFormat::GrayscaleAlpha8, PixelFormat::Grayscale8) => {
            let mut gray = Vec::with_capacity(num_pixels);
            for chunk in raw.data.chunks_exact(2) {
                let a = u32::from(chunk[1]);
                let inv = 255 - a;
                gray.push(((u32::from(chunk[0]) * a + 255 * inv) / 255) as u8);
            }
            Cow::Owned(gray)
        }
        (PixelFormat::GrayscaleAlpha8, PixelFormat::Rgb8) => {
            let mut rgb = Vec::with_capacity(num_pixels * 3);
            for chunk in raw.data.chunks_exact(2) {
                let a = u32::from(chunk[1]);
                let inv = 255 - a;
                let v = ((u32::from(chunk[0]) * a + 255 * inv) / 255) as u8;
                rgb.push(v);
                rgb.push(v);
                rgb.push(v);
            }
            Cow::Owned(rgb)
        }
        (PixelFormat::GrayscaleAlpha8, PixelFormat::Rgba8) => {
            let mut rgba = Vec::with_capacity(num_pixels * 4);
            for chunk in raw.data.chunks_exact(2) {
                rgba.push(chunk[0]);
                rgba.push(chunk[0]);
                rgba.push(chunk[0]);
                rgba.push(chunk[1]);
            }
            Cow::Owned(rgba)
        }
        (PixelFormat::Rgb8, PixelFormat::Grayscale8) => {
            let mut gray = Vec::with_capacity(num_pixels);
            for chunk in raw.data.chunks_exact(3) {
                let l = ((u32::from(chunk[0]) * 299
                    + u32::from(chunk[1]) * 587
                    + u32::from(chunk[2]) * 114
                    + 500)
                    / 1000) as u8;
                gray.push(l);
            }
            Cow::Owned(gray)
        }
        (PixelFormat::Rgb8, PixelFormat::Rgba8) => {
            let mut rgba = Vec::with_capacity(num_pixels * 4);
            for chunk in raw.data.chunks_exact(3) {
                rgba.push(chunk[0]);
                rgba.push(chunk[1]);
                rgba.push(chunk[2]);
                rgba.push(255);
            }
            Cow::Owned(rgba)
        }
        (PixelFormat::Rgba8, PixelFormat::Rgb8) => {
            // Flatten over white background matching JpegCodec
            let mut rgb = Vec::with_capacity(num_pixels * 3);
            for chunk in raw.data.chunks_exact(4) {
                let a = u32::from(chunk[3]);
                let inv = 255 - a;
                rgb.push(((u32::from(chunk[0]) * a + 255 * inv) / 255) as u8);
                rgb.push(((u32::from(chunk[1]) * a + 255 * inv) / 255) as u8);
                rgb.push(((u32::from(chunk[2]) * a + 255 * inv) / 255) as u8);
            }
            Cow::Owned(rgb)
        }
        (PixelFormat::Rgba8, PixelFormat::Grayscale8) => {
            let mut gray = Vec::with_capacity(num_pixels);
            for chunk in raw.data.chunks_exact(4) {
                let a = u32::from(chunk[3]);
                let inv = 255 - a;
                let r = (u32::from(chunk[0]) * a + 255 * inv) / 255;
                let g = (u32::from(chunk[1]) * a + 255 * inv) / 255;
                let b = (u32::from(chunk[2]) * a + 255 * inv) / 255;
                gray.push(((r * 299 + g * 587 + b * 114 + 500) / 1000) as u8);
            }
            Cow::Owned(gray)
        }
        (PixelFormat::Rgba8, PixelFormat::GrayscaleAlpha8) => {
            let mut la = Vec::with_capacity(num_pixels * 2);
            for chunk in raw.data.chunks_exact(4) {
                let l = ((u32::from(chunk[0]) * 299
                    + u32::from(chunk[1]) * 587
                    + u32::from(chunk[2]) * 114
                    + 500)
                    / 1000) as u8;
                la.push(l);
                la.push(chunk[3]);
            }
            Cow::Owned(la)
        }
        _ => Cow::Borrowed(&raw.data),
    }
}

fn evaluate_single_candidate(
    decoded_input: &DecodedPixels,
    raw_image: &RawImage,
    codec: &dyn ImageCodec,
    target_format: ImageFormat,
    enc_opt: &EncodeOptions,
) -> Option<OptimizationCandidate> {
    let desc = if enc_opt.lossless {
        format!("{}-lossless-eff{}", target_format, enc_opt.effort)
    } else {
        format!(
            "{}-q{}-eff{}",
            target_format, enc_opt.quality, enc_opt.effort
        )
    };

    let encoded = codec.encode(decoded_input, enc_opt).ok()?;
    let decoded_cand = codec.decode(&encoded.bytes).ok()?;

    if decoded_cand.width != raw_image.dimensions.width
        || decoded_cand.height != raw_image.dimensions.height
    {
        return None;
    }

    let width = decoded_cand.width as usize;
    let height = decoded_cand.height as usize;
    let cand_color: ColorType = decoded_cand.pixel_format.into();
    let channels = cand_color.channels() as usize;

    let ref_data = project_reference_for_candidate(raw_image, decoded_cand.pixel_format);
    if decoded_cand.data.len() != ref_data.len() {
        return None;
    }

    // Fast path for exact lossless pixel match (avoids redundant SSIM/PSNR loops)
    let (ssim, psnr) = if enc_opt.lossless && decoded_cand.data == *ref_data {
        (1.0, 100.0)
    } else {
        let score =
            compute_quality_score(&ref_data, &decoded_cand.data, width, height, channels).ok()?;
        (score.ssim, score.psnr)
    };

    Some(OptimizationCandidate::new(
        target_format,
        encoded.bytes,
        ssim,
        Some(psnr),
        desc,
    ))
}

/// Search for the smallest candidate meeting the quality criteria using the provided codec.
pub fn search_best_candidate(
    raw_image: &RawImage,
    codec: &dyn ImageCodec,
    options: &SearchOptions,
) -> Result<SearchResult> {
    let caps = codec.capabilities();
    let decoded_input: DecodedPixels = raw_image.clone().into();
    let preservation = preservation_from_policy(options.metadata_policy);

    let mut evaluated_candidates = Vec::new();

    if options.try_lossless && caps.supports_lossless {
        let lossless_opt = EncodeOptions {
            quality: 100,
            effort: options.effort,
            lossless: true,
            preserve_metadata: preservation.preserve_exif
                && preservation.preserve_icc
                && preservation.preserve_xmp,
            metadata_preservation: preservation,
        };
        if let Some(cand) = evaluate_single_candidate(
            &decoded_input,
            raw_image,
            codec,
            options.target_format,
            &lossless_opt,
        ) {
            evaluated_candidates.push(cand);
        }
    }

    if caps.supports_lossy {
        if options.quality_step > 1 {
            let step = options.quality_step;
            let mut q = options.quality_min.clamp(1, 100);
            let q_max = options.quality_max.clamp(q, 100);

            while q <= q_max {
                let enc_opt = EncodeOptions {
                    quality: q,
                    effort: options.effort,
                    lossless: false,
                    preserve_metadata: preservation.preserve_exif
                        && preservation.preserve_icc
                        && preservation.preserve_xmp,
                    metadata_preservation: preservation,
                };
                if let Some(cand) = evaluate_single_candidate(
                    &decoded_input,
                    raw_image,
                    codec,
                    options.target_format,
                    &enc_opt,
                ) {
                    evaluated_candidates.push(cand);
                }
                if q == q_max {
                    break;
                }
                q = q.saturating_add(step).min(q_max);
            }
        } else {
            let mut low = options.quality_min.clamp(1, 100);
            let mut high = options.quality_max.clamp(low, 100);

            while low <= high {
                let mid = low + (high - low) / 2;
                let enc_opt = EncodeOptions {
                    quality: mid,
                    effort: options.effort,
                    lossless: false,
                    preserve_metadata: preservation.preserve_exif
                        && preservation.preserve_icc
                        && preservation.preserve_xmp,
                    metadata_preservation: preservation,
                };

                if let Some(cand) = evaluate_single_candidate(
                    &decoded_input,
                    raw_image,
                    codec,
                    options.target_format,
                    &enc_opt,
                ) {
                    let ssim = cand.quality_score;
                    evaluated_candidates.push(cand);

                    if ssim >= options.quality_target {
                        if mid == low {
                            break;
                        }
                        high = mid - 1;
                    } else {
                        low = mid + 1;
                    }
                } else {
                    low = mid + 1;
                }
            }
        }
    }

    if evaluated_candidates.is_empty() {
        return Err(ImgoptError::new(
            PipelineStage::CandidateGeneration,
            ErrorKind::Configuration(format!(
                "no viable encoding modes supported for format {}",
                options.target_format
            )),
        ));
    }

    let criteria = RankingCriteria {
        min_quality_target: options.quality_target,
        max_allowed_bytes: options.max_allowed_bytes,
    };

    let selected_ref = select_best_candidate(&evaluated_candidates, &criteria)?;
    let selected = selected_ref.clone();

    let total_evaluated = evaluated_candidates.len();
    let rejected_count = evaluated_candidates
        .iter()
        .filter(|c| c.quality_score < options.quality_target)
        .count();

    Ok(SearchResult {
        selected,
        diagnostics: SearchDiagnostics {
            tested_candidates: evaluated_candidates,
            rejected_count,
            total_evaluated,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ColorType, Dimensions};
    use imgopt_codecs::{
        CodecCapabilities, CodecError, CodecFormat, EncodedOutput, JpegCodec, WebpCodec,
    };

    struct TestCodec;

    impl ImageCodec for TestCodec {
        fn capabilities(&self) -> CodecCapabilities {
            CodecCapabilities {
                format: CodecFormat::Webp,
                supports_lossless: true,
                supports_lossy: true,
                supports_alpha: true,
            }
        }

        fn decode(&self, _input_bytes: &[u8]) -> std::result::Result<DecodedPixels, CodecError> {
            Ok(DecodedPixels::new(4, 4, PixelFormat::Rgb8, vec![100; 48]))
        }

        fn encode(
            &self,
            _pixels: &DecodedPixels,
            options: &EncodeOptions,
        ) -> std::result::Result<EncodedOutput, CodecError> {
            let size = (options.quality as usize) * 10;
            let bytes = vec![0u8; size];
            Ok(EncodedOutput {
                format: CodecFormat::Webp,
                bytes,
                width: 4,
                height: 4,
                effective_options: options.clone(),
            })
        }
    }

    #[test]
    fn test_search_selects_smallest_meeting_target() {
        let raw = RawImage::new(Dimensions::new(4, 4), ColorType::Rgb8, vec![100; 48]);
        let codec = TestCodec;
        let options = SearchOptions {
            target_format: ImageFormat::Webp,
            quality_target: 0.90,
            effort: 4,
            quality_min: 70,
            quality_max: 90,
            quality_step: 10,
            try_lossless: false,
            max_allowed_bytes: None,
            metadata_policy: MetadataPolicy::StripAll,
        };

        let result = search_best_candidate(&raw, &codec, &options).unwrap();
        assert_eq!(result.diagnostics.total_evaluated, 3);
        assert_eq!(result.selected.byte_size, 700);
        assert_eq!(result.selected.parameters_description, "webp-q70-eff4");
    }

    #[test]
    fn test_search_binary_search_finds_minimum_quality() {
        let raw = RawImage::new(Dimensions::new(4, 4), ColorType::Rgb8, vec![100; 48]);
        let codec = TestCodec;
        let options = SearchOptions {
            target_format: ImageFormat::Webp,
            quality_target: 0.90,
            effort: 4,
            quality_min: 50,
            quality_max: 100,
            quality_step: 1,
            try_lossless: false,
            max_allowed_bytes: None,
            metadata_policy: MetadataPolicy::StripAll,
        };

        let result = search_best_candidate(&raw, &codec, &options).unwrap();
        assert_eq!(result.selected.byte_size, 500);
        assert_eq!(result.selected.parameters_description, "webp-q50-eff4");
    }

    #[test]
    fn test_cross_channel_rgba_to_jpeg_and_grayscale_to_webp() {
        let rgba_raw = RawImage::new(
            Dimensions::new(8, 8),
            ColorType::Rgba8,
            [200, 100, 50, 200].repeat(64),
        );
        let jpeg_opts = SearchOptions {
            target_format: ImageFormat::Jpeg,
            quality_target: 0.85,
            ..Default::default()
        };
        let res_jpeg = search_best_candidate(&rgba_raw, &JpegCodec, &jpeg_opts).unwrap();
        assert_eq!(res_jpeg.selected.format, ImageFormat::Jpeg);

        let gray_raw = RawImage::new(Dimensions::new(8, 8), ColorType::Grayscale8, vec![160; 64]);
        let webp_opts = SearchOptions {
            target_format: ImageFormat::Webp,
            quality_target: 0.90,
            try_lossless: true,
            ..Default::default()
        };
        let res_webp = search_best_candidate(&gray_raw, &WebpCodec, &webp_opts).unwrap();
        assert_eq!(res_webp.selected.format, ImageFormat::Webp);
    }

    #[test]
    fn test_linear_search_saturates_large_step() {
        let raw = RawImage::new(Dimensions::new(4, 4), ColorType::Rgb8, vec![100; 48]);
        let options = SearchOptions {
            target_format: ImageFormat::Webp,
            quality_target: 0.90,
            effort: 4,
            quality_min: 90,
            quality_max: 100,
            quality_step: 250,
            try_lossless: false,
            max_allowed_bytes: None,
            metadata_policy: MetadataPolicy::StripAll,
        };
        let result = search_best_candidate(&raw, &TestCodec, &options).unwrap();
        assert_eq!(result.selected.byte_size, 900);
    }
}
