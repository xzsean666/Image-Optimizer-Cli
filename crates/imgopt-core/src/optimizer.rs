use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::analyzer::{analyze_image, ResourceLimits};
use crate::candidate::{compare_candidates, OptimizationCandidate, RankingCriteria};
use crate::config::{OptimizationConfig, OptimizationPreset, OutputFormatPolicy};
use crate::error::{ErrorKind, ImgoptError, PipelineStage, Result};
use crate::hash::{hash_bytes, Sha256Digest};
use crate::model::{ImageFormat, ImageProfile};
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

    /// Optimize an image file at the specified filesystem path.
    pub fn optimize_file<P: AsRef<Path>>(
        &self,
        path: P,
        config: &OptimizationConfig,
    ) -> Result<OptimizationResult> {
        let p = path.as_ref();
        let bytes = fs::read(p).map_err(|e| {
            ImgoptError::new(
                PipelineStage::Discovery,
                ErrorKind::Io(format!("failed to read image file for optimization: {e}")),
            )
            .with_path(p)
        })?;

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

        let limits = ResourceLimits {
            max_input_bytes: config.max_input_bytes,
            max_pixels: config.max_pixels,
            max_width: config.max_width,
            max_height: config.max_height,
        };

        let analysis = analyze_image(bytes, Some(&limits))?;

        // Determine formats to search
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

        for target_fmt in target_formats {
            let codec = imgopt_codecs::codec_for_format(target_fmt.into());

            let try_lossless = match target_fmt {
                ImageFormat::Png => true,
                ImageFormat::Webp => true,
                ImageFormat::Jpeg => false,
            };

            let (q_min, q_max, q_step) = match config.preset {
                OptimizationPreset::Maximum => (70, 98, 1),
                OptimizationPreset::Balanced => (45, 95, 1),
                OptimizationPreset::Aggressive => (30, 88, 1),
            };

            let search_opts = SearchOptions {
                target_format: target_fmt,
                quality_target: config.quality_target,
                effort: config.effort,
                quality_min: q_min,
                quality_max: q_max,
                quality_step: q_step,
                try_lossless,
                max_allowed_bytes: None,
            };

            if let Ok(search_res) =
                search_best_candidate(&analysis.raw_image, codec.as_ref(), &search_opts)
            {
                combined_diagnostics.total_evaluated += search_res.diagnostics.total_evaluated;
                combined_diagnostics.rejected_count += search_res.diagnostics.rejected_count;
                combined_diagnostics
                    .tested_candidates
                    .extend(search_res.diagnostics.tested_candidates);
                all_candidates.push(search_res.selected);
            }
        }

        if all_candidates.is_empty() {
            return Err(ImgoptError::new(
                PipelineStage::CandidateGeneration,
                ErrorKind::Quality(format!(
                    "no candidate met the quality target of {:.4} for image of {} bytes",
                    config.quality_target,
                    bytes.len()
                )),
            ));
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

        // Validate the winning candidate
        let val_criteria = ValidationCriteria {
            expected_format: winner.format,
            expected_dimensions: analysis.profile.dimensions,
            metadata_policy: config.metadata_policy,
            max_allowed_bytes: None,
        };

        let validated_output = validate_candidate_output(
            &winner.bytes,
            bytes.len() as u64,
            &val_criteria,
            Some(&limits),
        )?;

        let optimization_key = OptimizationKey::compute(
            &source_sha256,
            &self.optimizer_version,
            winner.format,
            config,
        );

        Ok(OptimizationResult {
            source_sha256,
            optimization_key,
            profile: analysis.profile,
            validated_output,
            quality_score: winner.quality_score,
            psnr_score: winner.psnr_score,
            preset: config.preset,
            effective_format: winner.format,
            search_diagnostics: Some(combined_diagnostics),
        })
    }
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
    }

    #[test]
    fn test_optimize_service_png() {
        let service = OptimizationService::new("0.1.0");
        let bytes = make_test_png();
        let config = OptimizationConfig::from_preset(OptimizationPreset::Balanced);

        let result = service.optimize_bytes(&bytes, &config).unwrap();
        assert_eq!(result.profile.format, ImageFormat::Png);
        assert_eq!(result.effective_format, ImageFormat::Png);
    }

    #[test]
    fn test_optimize_service_webp() {
        let service = OptimizationService::new("0.1.0");
        let bytes = make_test_webp();
        let config = OptimizationConfig::from_preset(OptimizationPreset::Balanced);

        let result = service.optimize_bytes(&bytes, &config).unwrap();
        assert_eq!(result.profile.format, ImageFormat::Webp);
        assert_eq!(result.effective_format, ImageFormat::Webp);
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
}
