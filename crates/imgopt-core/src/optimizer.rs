use std::path::Path;
use std::sync::Arc;

use imgopt_codecs::{
    compute_target_dimensions, convert_icc_to_srgb, decode_animation_with_limits,
    encode_animated_gif, encode_animated_webp, resize_pixels, CodecFormat, DecodedAnimation,
    DecodedPixels, EncodeOptions,
};
use serde::{Deserialize, Serialize};

use crate::analyzer::{analyze_image, has_ancillary_metadata, read_file_bounded, ResourceLimits};
use crate::candidate::{compare_candidates, OptimizationCandidate, RankingCriteria};
use crate::config::{MetadataPolicy, OptimizationConfig, OptimizationPreset, OutputFormatPolicy};
use crate::detect::detect_format;
use crate::error::{ErrorKind, ImgoptError, PipelineStage, Result};
use crate::executor::MemoryBudget;
use crate::hash::{hash_bytes, Sha256Digest};
use crate::model::{Dimensions, ImageFormat, ImageMetadata, ImageProfile};
use crate::optimization_key::OptimizationKey;
use crate::search::{search_best_candidate_with_limits, SearchDiagnostics, SearchOptions};
use crate::validation::{validate_predecoded_output, ValidatedOutput, ValidationCriteria};

const DEFAULT_MEMORY_BUDGET_BYTES: u64 = 1024 * 1024 * 1024; // 1 GiB inflight decode cap

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
    memory_budget: Arc<MemoryBudget>,
    codec_threads: Option<usize>,
}

impl OptimizationService {
    pub fn new(optimizer_version: impl Into<String>) -> Self {
        Self {
            optimizer_version: optimizer_version.into(),
            memory_budget: Arc::new(MemoryBudget::new(DEFAULT_MEMORY_BUDGET_BYTES)),
            codec_threads: None,
        }
    }

    pub fn with_memory_budget(mut self, max_inflight_bytes: u64) -> Self {
        self.memory_budget = Arc::new(MemoryBudget::new(max_inflight_bytes));
        self
    }

    /// Limit per-codec internal thread count (useful when running inside a multi-worker batch pool).
    pub fn with_codec_threads(mut self, max_threads: Option<usize>) -> Self {
        self.codec_threads = max_threads.map(|n| n.max(1));
        self
    }

    pub fn recommended_worker_codec_threads(concurrency: usize) -> usize {
        let avail = std::thread::available_parallelism().map_or(4, |n| n.get());
        (avail / concurrency.max(1)).clamp(1, 4)
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

        let limits = ResourceLimits {
            max_input_bytes: config.max_input_bytes,
            max_pixels: config.max_pixels,
            max_width: config.max_width,
            max_height: config.max_height,
        };
        let mut decode_limits = limits.to_decode_limits();
        decode_limits.max_threads = self.codec_threads;

        // Check for animated multi-frame images (GIF, animated WebP, APNG) only when needed
        let analysis = match analyze_image(bytes, Some(&limits)) {
            Ok(a) => a,
            Err(err) => {
                if matches!(&err.kind, ErrorKind::UnsupportedFormat(_)) {
                    if let Ok(source_fmt) = detect_format(bytes) {
                        if let Some(anim) =
                            decode_animation_with_limits(bytes, source_fmt.into(), &decode_limits)
                                .map_err(|e| match e {
                                imgopt_codecs::CodecError::ResourceLimitExceeded(msg) => {
                                    ImgoptError::new(
                                        PipelineStage::ResourceCheck,
                                        ErrorKind::ResourceLimitExceeded(msg),
                                    )
                                }
                                other => ImgoptError::new(
                                    PipelineStage::Decode,
                                    ErrorKind::Decode(format!(
                                        "failed to decode animation: {other}"
                                    )),
                                ),
                            })?
                        {
                            return self.optimize_animation(
                                bytes,
                                &source_sha256,
                                source_fmt,
                                anim,
                                config,
                                &limits,
                            );
                        }
                    }
                }
                return Err(err);
            }
        };
        let source_has_ancillary = has_ancillary_metadata(bytes, analysis.profile.format);
        let profile = analysis.profile;

        // Reserve estimated decoded + search scratch memory under the shared memory budget
        let estimated_mem = profile
            .dimensions
            .pixel_count()
            .saturating_mul(4)
            .saturating_mul(2)
            .max(1024);
        let _mem_lease = self.memory_budget.acquire(estimated_mem);

        let mut decoded_input: DecodedPixels = analysis.raw_image.into();
        let mut transform_changed_pixels = false;

        // Apply optional resizing before candidate search so SSIM & validation compare against resized reference
        let (target_w, target_h) = compute_target_dimensions(
            decoded_input.width,
            decoded_input.height,
            config.resize_width,
            config.resize_height,
            config.resize_mode.into(),
        );
        if (target_w, target_h) != (decoded_input.width, decoded_input.height) {
            decoded_input = resize_pixels(
                &decoded_input,
                target_w,
                target_h,
                config.resize_mode.into(),
            )
            .map_err(|e| {
                ImgoptError::new(PipelineStage::Encode, ErrorKind::Encode(e.to_string()))
            })?;
            transform_changed_pixels = true;
        }

        // Convert wide-gamut ICC profiles to sRGB before stripping ICC to avoid color washout
        if !config.metadata_policy.preserves_icc()
            && config.convert_to_srgb
            && decoded_input.metadata.icc_profile.is_some()
        {
            let before_ptr = decoded_input.metadata.icc_profile.clone();
            let converted = convert_icc_to_srgb(&decoded_input);
            if converted.data != decoded_input.data || before_ptr != converted.metadata.icc_profile
            {
                decoded_input = converted;
                transform_changed_pixels = true;
            }
        }

        let expected_dimensions = Dimensions::new(decoded_input.width, decoded_input.height);

        let target_formats: Vec<ImageFormat> = match config.output_format_policy {
            OutputFormatPolicy::PreserveOriginal => vec![profile.format],
            OutputFormatPolicy::AllowWebp => {
                if profile.format == ImageFormat::Webp {
                    vec![ImageFormat::Webp]
                } else {
                    vec![profile.format, ImageFormat::Webp]
                }
            }
            OutputFormatPolicy::AllowAvif => {
                if profile.format == ImageFormat::Avif {
                    vec![ImageFormat::Avif]
                } else {
                    vec![profile.format, ImageFormat::Avif]
                }
            }
            OutputFormatPolicy::Auto => {
                let mut fmts = vec![profile.format];
                if !fmts.contains(&ImageFormat::Webp) {
                    fmts.push(ImageFormat::Webp);
                }
                if !fmts.contains(&ImageFormat::Avif) {
                    fmts.push(ImageFormat::Avif);
                }
                fmts
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
                ImageFormat::Png | ImageFormat::Gif => true,
                ImageFormat::Webp => {
                    profile.format != ImageFormat::Jpeg || config.quality_target >= 0.995
                }
                ImageFormat::Jpeg | ImageFormat::Avif => false,
            };

            let (q_min, q_max, q_step) = match target_fmt {
                ImageFormat::Png => match config.preset {
                    OptimizationPreset::Maximum => (85, 99, 7),
                    OptimizationPreset::Balanced => (65, 95, 10),
                    OptimizationPreset::Aggressive => (40, 85, 10),
                },
                ImageFormat::Gif => (80, 80, 1),
                ImageFormat::Avif => match config.preset {
                    OptimizationPreset::Maximum => (70, 98, 1),
                    OptimizationPreset::Balanced => (50, 95, 1),
                    OptimizationPreset::Aggressive => (35, 90, 1),
                },
                ImageFormat::Jpeg | ImageFormat::Webp => match config.preset {
                    OptimizationPreset::Maximum => (70, 100, 1),
                    OptimizationPreset::Balanced => (45, 100, 1),
                    OptimizationPreset::Aggressive => (30, 100, 1),
                },
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

            match search_best_candidate_with_limits(
                &decoded_input,
                codec.as_ref(),
                &search_opts,
                &decode_limits,
                false,
            ) {
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
        // provided original format is permitted, dimensions/pixels didn't transform,
        // and source contains no disallowed metadata.
        let orig_bytes_len = bytes.len() as u64;
        let orig_format_allowed = target_formats.contains(&profile.format);
        let can_keep_original = !transform_changed_pixels
            && winner.byte_size >= orig_bytes_len
            && orig_format_allowed
            && source_satisfies_metadata_policy(
                &profile.metadata,
                source_has_ancillary,
                config.metadata_policy,
            );

        let (chosen_format, chosen_bytes, chosen_quality, chosen_psnr) = if can_keep_original {
            (profile.format, bytes, 1.0, Some(100.0))
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
            expected_dimensions,
            metadata_policy: config.metadata_policy,
            max_allowed_bytes: None,
        };

        let validated_output =
            validate_predecoded_output(chosen_bytes, orig_bytes_len, &val_criteria, Some(&limits))?;

        let optimization_key = OptimizationKey::compute(
            &source_sha256,
            &self.optimizer_version,
            chosen_format,
            config,
        );

        Ok(OptimizationResult {
            source_sha256,
            optimization_key,
            profile,
            validated_output,
            quality_score: chosen_quality,
            psnr_score: chosen_psnr,
            preset: config.preset,
            effective_format: chosen_format,
            search_diagnostics: Some(combined_diagnostics),
        })
    }

    fn optimize_animation(
        &self,
        orig_bytes: &[u8],
        source_sha256: &Sha256Digest,
        source_fmt: ImageFormat,
        mut anim: DecodedAnimation,
        config: &OptimizationConfig,
        limits: &ResourceLimits,
    ) -> Result<OptimizationResult> {
        let estimated_mem = u64::from(anim.width)
            .saturating_mul(u64::from(anim.height))
            .saturating_mul(4)
            .saturating_mul((anim.frames.len() as u64).max(1))
            .max(1024);
        let _mem_lease = self.memory_budget.acquire(estimated_mem);

        let orig_dims = Dimensions::new(anim.width, anim.height);
        let (target_w, target_h) = compute_target_dimensions(
            anim.width,
            anim.height,
            config.resize_width,
            config.resize_height,
            config.resize_mode.into(),
        );
        let mut resized = false;
        if (target_w, target_h) != (anim.width, anim.height) {
            for frame in &mut anim.frames {
                let frame_px = DecodedPixels::new(
                    frame.width,
                    frame.height,
                    imgopt_codecs::PixelFormat::Rgba8,
                    std::mem::take(&mut frame.rgba),
                );
                let out_px =
                    resize_pixels(&frame_px, target_w, target_h, config.resize_mode.into())
                        .map_err(|e| {
                            ImgoptError::new(
                                PipelineStage::Encode,
                                ErrorKind::Encode(e.to_string()),
                            )
                        })?;
                frame.width = target_w;
                frame.height = target_h;
                frame.rgba = out_px.data;
            }
            anim.width = target_w;
            anim.height = target_h;
            resized = true;
        }

        let target_anim_format = match config.output_format_policy {
            OutputFormatPolicy::Force(ImageFormat::Webp)
            | OutputFormatPolicy::AllowWebp
            | OutputFormatPolicy::Auto => ImageFormat::Webp,
            OutputFormatPolicy::Force(ImageFormat::Gif) => ImageFormat::Gif,
            OutputFormatPolicy::PreserveOriginal | OutputFormatPolicy::AllowAvif => {
                if source_fmt == ImageFormat::Webp {
                    ImageFormat::Webp
                } else {
                    ImageFormat::Gif
                }
            }
            OutputFormatPolicy::Force(other) => {
                return Err(ImgoptError::new(
                    PipelineStage::Config,
                    ErrorKind::UnsupportedFormat(format!(
                        "animated {source_fmt} cannot be force-converted to static format {other}"
                    )),
                ))
            }
        };

        let enc_opts = EncodeOptions {
            quality: ((config.quality_target * 100.0).round() as u8).clamp(30, 100),
            effort: config.effort,
            lossless: config.quality_target >= 0.995,
            preserve_metadata: false,
            metadata_preservation: Default::default(),
            max_threads: self.codec_threads,
        };

        let encoded_bytes = match target_anim_format {
            ImageFormat::Webp => encode_animated_webp(&anim, &enc_opts),
            ImageFormat::Gif => encode_animated_gif(&anim, &enc_opts),
            _ => unreachable!(),
        }
        .map_err(|e| ImgoptError::new(PipelineStage::Encode, ErrorKind::Encode(e.to_string())))?;

        let orig_len = orig_bytes.len() as u64;
        let source_allowed = match config.output_format_policy {
            OutputFormatPolicy::Force(f) => f == source_fmt,
            _ => true,
        };
        let source_meta = crate::analyzer::probe_metadata_markers(orig_bytes, source_fmt);
        let source_has_ancillary = has_ancillary_metadata(orig_bytes, source_fmt);
        let can_keep_original = !resized
            && source_allowed
            && (encoded_bytes.len() as u64) >= orig_len
            && source_satisfies_metadata_policy(
                &source_meta,
                source_has_ancillary,
                config.metadata_policy,
            );

        let (final_fmt, final_bytes) = if can_keep_original {
            (source_fmt, orig_bytes.to_vec())
        } else {
            (target_anim_format, encoded_bytes)
        };

        let mut decode_limits = limits.to_decode_limits();
        decode_limits.max_threads = self.codec_threads;
        let verified_anim = decode_animation_with_limits(
            &final_bytes,
            CodecFormat::from(final_fmt),
            &decode_limits,
        )
        .map_err(|e| {
            ImgoptError::new(
                PipelineStage::OutputValidation,
                ErrorKind::Decode(format!("corrupt animated output failed validation: {e}")),
            )
        })?
        .ok_or_else(|| {
            ImgoptError::new(
                PipelineStage::OutputValidation,
                ErrorKind::InvalidInput("animated output did not decode as an animation".into()),
            )
        })?;
        if verified_anim.width != anim.width || verified_anim.height != anim.height {
            return Err(ImgoptError::new(
                PipelineStage::OutputValidation,
                ErrorKind::InvalidInput(format!(
                    "animated output dimensions {}x{} do not match expected {}x{}",
                    verified_anim.width, verified_anim.height, anim.width, anim.height
                )),
            ));
        }

        let (savings_bytes, savings_ratio) =
            crate::model::compute_byte_savings(orig_len, final_bytes.len() as u64);
        let validated_output = ValidatedOutput {
            format: final_fmt,
            byte_size: final_bytes.len() as u64,
            bytes: final_bytes,
            original_size: orig_len,
            dimensions: Dimensions::new(anim.width, anim.height),
            savings_bytes,
            savings_ratio,
        };

        let profile = ImageProfile {
            format: source_fmt,
            dimensions: orig_dims,
            color_type: crate::model::ColorType::Rgba8,
            byte_size: orig_len,
            metadata: ImageMetadata::default(),
        };
        let optimization_key =
            OptimizationKey::compute(source_sha256, &self.optimizer_version, final_fmt, config);

        Ok(OptimizationResult {
            source_sha256: *source_sha256,
            optimization_key,
            profile,
            validated_output,
            quality_score: 1.0,
            psnr_score: None,
            preset: config.preset,
            effective_format: final_fmt,
            search_diagnostics: None,
        })
    }
}

fn source_satisfies_metadata_policy(
    meta: &ImageMetadata,
    has_any_ancillary: bool,
    policy: MetadataPolicy,
) -> bool {
    if policy == MetadataPolicy::StripAll && has_any_ancillary {
        return false;
    }
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

    #[test]
    fn test_strip_all_removes_jpeg_com_and_maximum_preset_handles_high_contrast_jpeg() {
        // High-contrast checkerboard pattern where 4:2:0 subsampling struggles to hit 0.97 SSIM
        let w = 32u32;
        let h = 32u32;
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                if (x + y) % 2 == 0 {
                    rgb.extend_from_slice(&[255, 0, 0]);
                } else {
                    rgb.extend_from_slice(&[0, 255, 255]);
                }
            }
        }
        let pixels = imgopt_codecs::DecodedPixels::new(w, h, PixelFormat::Rgb8, rgb);
        let base_jpeg = JpegCodec
            .encode(
                &pixels,
                &EncodeOptions {
                    quality: 96,
                    effort: 1,
                    ..Default::default()
                },
            )
            .unwrap()
            .bytes;

        // Inject a JPEG COM (0xFF 0xFE) comment marker after SOI (0xFF 0xD8)
        let mut with_com = Vec::with_capacity(base_jpeg.len() + 16);
        with_com.extend_from_slice(&base_jpeg[..2]);
        let comment = b"Secret Camera/Author Info";
        with_com.push(0xFF);
        with_com.push(0xFE);
        with_com.extend_from_slice(&((comment.len() + 2) as u16).to_be_bytes());
        with_com.extend_from_slice(comment);
        with_com.extend_from_slice(&base_jpeg[2..]);

        let service = OptimizationService::new("0.1.0");
        let max_cfg = OptimizationConfig::from_preset(OptimizationPreset::Maximum);
        let res = service.optimize_bytes(&with_com, &max_cfg).unwrap();
        assert!(res.quality_score >= 0.97);
        assert!(
            !has_ancillary_metadata(&res.validated_output.bytes, ImageFormat::Jpeg),
            "StripAll must strip JPEG COM segment even if original would otherwise be small"
        );
    }

    #[test]
    fn test_optimize_resize_and_avif_and_animated_gif() {
        let service = OptimizationService::new("0.1.0");
        let pixels = DecodedPixels::new(32, 16, PixelFormat::Rgb8, vec![120; 32 * 16 * 3]);
        let png_bytes = PngCodec
            .encode(&pixels, &EncodeOptions::default())
            .unwrap()
            .bytes;

        let mut cfg = OptimizationConfig::from_preset(OptimizationPreset::Balanced);
        cfg.resize_width = Some(16);
        cfg.output_format_policy = OutputFormatPolicy::Force(ImageFormat::Avif);
        let res = service.optimize_bytes(&png_bytes, &cfg).unwrap();
        assert_eq!(res.effective_format, ImageFormat::Avif);
        assert_eq!(res.validated_output.dimensions, Dimensions::new(16, 8));

        let anim = DecodedAnimation {
            width: 8,
            height: 8,
            frames: vec![
                imgopt_codecs::AnimationFrame {
                    width: 8,
                    height: 8,
                    rgba: [255, 0, 0, 255].repeat(64),
                    delay_millis: 50,
                },
                imgopt_codecs::AnimationFrame {
                    width: 8,
                    height: 8,
                    rgba: [0, 255, 0, 255].repeat(64),
                    delay_millis: 50,
                },
            ],
        };
        let gif_bytes = encode_animated_gif(&anim, &EncodeOptions::default()).unwrap();
        let mut anim_cfg = OptimizationConfig::from_preset(OptimizationPreset::Balanced);
        anim_cfg.output_format_policy = OutputFormatPolicy::AllowWebp;
        let anim_res = service.optimize_bytes(&gif_bytes, &anim_cfg).unwrap();
        assert!(matches!(
            anim_res.effective_format,
            ImageFormat::Webp | ImageFormat::Gif
        ));
    }
}
