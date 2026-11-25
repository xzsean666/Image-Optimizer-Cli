use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::analyzer::{analyze_image, read_file_bounded, ResourceLimits};
use crate::candidate::{compare_candidates, OptimizationCandidate, RankingCriteria};
use crate::config::{MetadataPolicy, OptimizationConfig, OptimizationPreset, OutputFormatPolicy};
use crate::error::{ErrorKind, ImgoptError, PipelineStage, Result};
use crate::hash::{hash_bytes, Sha256Digest};
use crate::model::{ImageFormat, ImageMetadata, ImageProfile};
use crate::optimization_key::OptimizationKey;
use crate::search::{search_best_candidate, SearchDiagnostics, SearchOptions};
use crate::validation::{validate_candidate_output, ValidatedOutput, ValidationCriteria};

/// Comprehensive result of a successful local optimization run for a single image asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationResult {
    pub source_sha256: Sha256Digest,
    pub optimization_key: OptimizationKey,
    pub profile: ImageProfile,
    pub validated_output: ValidatedOutput,
    pub quality_score: f64,
    pub psnr_score: Option<f64>,
    pub preset: OptimizationPreset,
    pub effective_format: ImageFormat,
    #[serde(skip)]
    pub search_diagnostics: Option<SearchDiagnostics>,
}

/// Service managing the lifecycle of image analysis, candidate search, and output validation.
#[derive(Debug, Clone)]
pub struct OptimizationService {
    optimizer_version: String,
}

impl OptimizationService {
    pub fn new(optimizer_version: impl Into<String>) -> Self {
        Self {
            optimizer_version: optimizer_version.into(),
        }
    }

    pub fn optimizer_version(&self) -> &str {
        &self.optimizer_version
    }

    /// Optimize an image file at the specified filesystem path using bounded preflight read.
    pub fn optimize_file<P: AsRef<Path>>(
        &self,
        path: P,
        config: &OptimizationConfig,
    ) -> Result<OptimizationResult> {
        config.validate()?;
        let p = path.as_ref();
        let bytes = read_file_bounded(p, config.max_input_bytes, PipelineStage::Discovery)?;

        self.optimize_bytes(&bytes, config)
            .map_err(|e| e.with_path(p))
    }

    /// Optimize in-memory image bytes according to configuration.
    pub fn optimize_bytes(
        &self,
        bytes: &[u8],
        config: &OptimizationConfig,
    ) -> Result<OptimizationResult> {
        config.validate()?;

        let source_sha256 = hash_bytes(bytes);

        let default_limits = ResourceLimits::default();
        let limits = ResourceLimits {
            max_input_bytes: config.max_input_bytes,
            max_pixels: config.max_pixels,
            max_width: config.max_width.or(default_limits.max_width),
            max_height: config.max_height.or(default_limits.max_height),
        };

        let analysis = analyze_image(bytes, Some(&limits))?;

        let target_formats: Vec<ImageFormat> = match config.output_format_policy {
            OutputFormatPolicy::PreserveOriginal => vec![analysis.profile.format],
            OutputFormatPolicy::AllowWebp => {
                if analysis.profile.format == ImageFormat::Webp {
                    vec![ImageFormat::Webp]
                } else {
                    vec![analysis.profile.format, ImageFormat::Webp]
                }
            }
            OutputFormatPolicy::Force(fmt) => vec![fmt],
        };

        let mut all_candidates: Vec<OptimizationCandidate> = Vec::new();
        let mut combined_diagnostics = SearchDiagnostics {
            tested_candidates: Vec::new(),
            rejected_count: 0,
            total_evaluated: 0,
        };

        let mut last_error: Option<ImgoptError> = None;

        for target_fmt in &target_formats {
            let codec = imgopt_codecs::codec_for_format((*target_fmt).into());

            // Skip lossless WebP trial when converting continuous-tone JPEG photos unless near-lossless SSIM is requested
            let try_lossless = match target_fmt {
                ImageFormat::Png => true,
                ImageFormat::Webp => {
                    analysis.profile.format != ImageFormat::Jpeg || config.quality_target >= 0.995
                }
                ImageFormat::Jpeg => false,
            };

            let (q_min, q_max, q_step) = match config.preset {
                OptimizationPreset::Maximum => (70, 100, 1),
                OptimizationPreset::Balanced => (45, 98, 1),
                OptimizationPreset::Aggressive => (30, 95, 1),
            };

            let search_opts = SearchOptions {
                target_format: *target_fmt,
                quality_target: config.quality_target,
                effort: config.effort,
                quality_min: q_min,
                quality_max: q_max,
                quality_step: q_step,
                try_lossless,
                max_allowed_bytes: None,
                metadata_policy: config.metadata_policy,
            };

            match search_best_candidate(&analysis.raw_image, codec.as_ref(), &search_opts) {
                Ok(search_res) => {
                    combined_diagnostics.total_evaluated += search_res.diagnostics.total_evaluated;
                    combined_diagnostics.rejected_count += search_res.diagnostics.rejected_count;
                    combined_diagnostics
                        .tested_candidates
                        .extend(search_res.diagnostics.tested_candidates);
                    all_candidates.push(search_res.selected);
                }
                Err(e) => {
                    last_error = Some(e);
                }
            }
        }

        if all_candidates.is_empty() {
            return Err(last_error.unwrap_or_else(|| {
                ImgoptError::new(
                    PipelineStage::CandidateGeneration,
                    ErrorKind::Quality(format!(
                        "no candidate met the quality target of {:.4} for image of {} bytes",
                        config.quality_target,
                        bytes.len()
                    )),
                )
            }));
        }

        let ranking_criteria = RankingCriteria {
            min_quality_target: config.quality_target,
            max_allowed_bytes: None,
        };

        let mut eligible: Vec<&OptimizationCandidate> = all_candidates
            .iter()
            .filter(|c| c.quality_score >= ranking_criteria.min_quality_target)
            .collect();

        if eligible.is_empty() {
            return Err(ImgoptError::new(
                PipelineStage::QualityEvaluation,
                ErrorKind::Quality(format!(
                    "no acceptable candidate met quality threshold {:.4}",
                    config.quality_target
                )),
            ));
        }

        eligible.sort_by(|a, b| compare_candidates(a, b));
        let winner = eligible[0];

        // No-bloat fallback: if re-encoding didn't shrink the image, keep original bytes
        // provided original format is permitted and source contains no disallowed metadata.
        let orig_bytes_len = bytes.len() as u64;
        let orig_format_allowed = target_formats.contains(&analysis.profile.format);
        let can_keep_original = winner.byte_size >= orig_bytes_len
            && orig_format_allowed
            && source_satisfies_metadata_policy(&analysis.profile.metadata, config.metadata_policy);

        let (chosen_format, chosen_bytes, chosen_quality, chosen_psnr) = if can_keep_original {
            (
                analysis.profile.format,
                bytes,
                1.0,
                Some(100.0),
            )
        } else {
            (
                winner.format,
                winner.bytes.as_slice(),
                winner.quality_score,
                winner.psnr_score,
            )
        };

        let val_criteria = ValidationCriteria {
            expected_format: chosen_format,
            expected_dimensions: analysis.profile.dimensions,
            metadata_policy: config.metadata_policy,
            max_allowed_bytes: None,
        };

        let validated_output = validate_candidate_output(
            chosen_bytes,
            orig_bytes_len,
            &val_criteria,
            Some(&limits),
        )?;

        let optimization_key = OptimizationKey::compute(
            &source_sha256,
            &self.optimizer_version,
            chosen_format,
            config,
        );

        Ok(OptimizationResult {
            source_sha256,
            optimization_key,
            profile: analysis.profile,
            validated_output,
            quality_score: chosen_quality,
            psnr_score: chosen_psnr,
            preset: config.preset,
            effective_format: chosen_format,
            search_diagnostics: Some(combined_diagnostics),
        })
    }
}

fn source_satisfies_metadata_policy(meta: &ImageMetadata, policy: MetadataPolicy) -> bool {
    let disallow_exif = !policy.preserves_exif() && meta.has_exif;
    let disallow_icc = !policy.preserves_icc() && meta.has_icc;
    let disallow_xmp = !policy.preserves_xmp() && meta.has_xmp;
    !(disallow_exif || disallow_icc || disallow_xmp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use imgopt_codecs::{
        DecodedPixels, EncodeOptions, ImageCodec, JpegCodec, PixelFormat, PngCodec, WebpCodec,
    };

    fn make_test_jpeg() -> Vec<u8> {
        let codec = JpegCodec;
        let pixels = DecodedPixels::new(8, 8, PixelFormat::Rgb8, vec![140; 8 * 8 * 3]);
        codec
            .encode(&pixels, &EncodeOptions::default())
            .unwrap()
            .bytes
    }

    fn make_test_png() -> Vec<u8> {
        let codec = PngCodec;
        let pixels = DecodedPixels::new(8, 8, PixelFormat::Rgb8, vec![140; 8 * 8 * 3]);
        codec
            .encode(&pixels, &EncodeOptions::default())
            .unwrap()
            .bytes
    }

    fn make_test_webp() -> Vec<u8> {
        let codec = WebpCodec;
        let pixels = DecodedPixels::new(8, 8, PixelFormat::Rgb8, vec![140; 8 * 8 * 3]);
        codec
            .encode(&pixels, &EncodeOptions::default())
            .unwrap()
            .bytes
    }

    #[test]
    fn test_optimize_service_jpeg() {
        let service = OptimizationService::new("0.1.0");
        let bytes = make_test_jpeg();
        let config = OptimizationConfig::from_preset(OptimizationPreset::Balanced);

        let result = service.optimize_bytes(&bytes, &config).unwrap();
        assert_eq!(result.profile.format, ImageFormat::Jpeg);
        assert_eq!(result.effective_format, ImageFormat::Jpeg);
        assert!(result.quality_score >= config.quality_target);
        assert_eq!(result.validated_output.format, ImageFormat::Jpeg);
        assert!(result.validated_output.byte_size <= bytes.len() as u64);
    }

    #[test]
    fn test_optimize_service_png() {
        let service = OptimizationService::new("0.1.0");
        let bytes = make_test_png();
        let config = OptimizationConfig::from_preset(OptimizationPreset::Balanced);

        let result = service.optimize_bytes(&bytes, &config).unwrap();
        assert_eq!(result.profile.format, ImageFormat::Png);
        assert_eq!(result.effective_format, ImageFormat::Png);
        assert!(result.validated_output.byte_size <= bytes.len() as u64);
    }

    #[test]
    fn test_optimize_service_webp() {
        let service = OptimizationService::new("0.1.0");
        let bytes = make_test_webp();
        let config = OptimizationConfig::from_preset(OptimizationPreset::Balanced);

        let result = service.optimize_bytes(&bytes, &config).unwrap();
        assert_eq!(result.profile.format, ImageFormat::Webp);
        assert_eq!(result.effective_format, ImageFormat::Webp);
        assert!(result.validated_output.byte_size <= bytes.len() as u64);
    }

    #[test]
    fn test_optimize_invalid_config_fails() {
        let service = OptimizationService::new("0.1.0");
        let bytes = make_test_jpeg();
        let config = OptimizationConfig {
            quality_target: 1.5,
            ..Default::default()
        };

        assert!(service.optimize_bytes(&bytes, &config).is_err());
    }

    #[test]
    fn test_optimize_preserves_metadata_and_normalizes_exif_orientation() {
        let codec = JpegCodec;
        // Build 4x2 (landscape) pixel buffer with EXIF Orientation=6 (Rotate 90 CW -> becomes 2x4)
        let exif_orient_6: Vec<u8> = vec![
            0x49, 0x49, 0x2A, 0x00, // Little-endian TIFF
            0x08, 0x00, 0x00, 0x00, // IFD0 offset 8
            0x01, 0x00, // 1 entry
            0x12, 0x01, // Tag 0x0112 (Orientation)
            0x03, 0x00, // Type SHORT (3)
            0x01, 0x00, 0x00, 0x00, // Count 1
            0x06, 0x00, 0x00, 0x00, // Value 6 (Rotate90)
            0x00, 0x00, 0x00, 0x00, // Next IFD = 0
        ];
        let pixels = imgopt_codecs::DecodedPixels::with_metadata(
            4,
            2,
            PixelFormat::Rgb8,
            vec![180; 4 * 2 * 3],
            imgopt_codecs::ImageMetadataChunks {
                icc_profile: Some(b"test-icc".to_vec()),
                exif: Some(exif_orient_6),
                xmp: None,
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
            .unwrap()
            .bytes;

        let service = OptimizationService::new("0.1.0");

        // StripAll normalizes orientation to 2x4 and strips EXIF/ICC
        let strip_cfg = OptimizationConfig::from_preset(OptimizationPreset::Balanced);
        let res_strip = service.optimize_bytes(&encoded, &strip_cfg).unwrap();
        assert_eq!(
            res_strip.validated_output.dimensions,
            crate::model::Dimensions::new(2, 4)
        );

        // PreserveAll normalizes orientation to 2x4 and keeps ICC + EXIF (with Orientation reset to 1)
        let mut keep_cfg = OptimizationConfig::from_preset(OptimizationPreset::Balanced);
        keep_cfg.metadata_policy = MetadataPolicy::PreserveAll;
        let res_keep = service.optimize_bytes(&encoded, &keep_cfg).unwrap();
        assert_eq!(
            res_keep.validated_output.dimensions,
            crate::model::Dimensions::new(2, 4)
        );
        let redecoded = codec.decode(&res_keep.validated_output.bytes).unwrap();
        assert_eq!(
            redecoded.metadata.icc_profile.as_deref(),
            Some(b"test-icc".as_slice())
        );
        assert!(redecoded.metadata.exif.is_some());
    }
}
