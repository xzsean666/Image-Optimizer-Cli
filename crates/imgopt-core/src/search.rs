use imgopt_codecs::{DecodedPixels, EncodeOptions, ImageCodec};
use imgopt_quality::compute_quality_score;

use crate::candidate::{select_best_candidate, OptimizationCandidate, RankingCriteria};
use crate::error::{ErrorKind, ImgoptError, PipelineStage, Result};
use crate::model::{ImageFormat, RawImage};

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

    let width = raw_image.dimensions.width as usize;
    let height = raw_image.dimensions.height as usize;
    let channels = raw_image.color_type.channels() as usize;

    let cand_data = if decoded_cand.data.len() == raw_image.data.len() {
        decoded_cand.data
    } else {
        return None;
    };

    let quality_score =
        compute_quality_score(&raw_image.data, &cand_data, width, height, channels).ok()?;

    Some(OptimizationCandidate::new(
        target_format,
        encoded.bytes,
        quality_score.ssim,
        Some(quality_score.psnr),
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

    let mut evaluated_candidates = Vec::new();

    // If lossless is requested and supported, evaluate lossless option
    if options.try_lossless && caps.supports_lossless {
        let lossless_opt = EncodeOptions {
            quality: 100,
            effort: options.effort,
            lossless: true,
            preserve_metadata: false,
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
            // Linear step mode for coarser scans
            let step = options.quality_step;
            let mut q = options.quality_min.clamp(1, 100);
            let q_max = options.quality_max.clamp(q, 100);

            while q <= q_max {
                let enc_opt = EncodeOptions {
                    quality: q,
                    effort: options.effort,
                    lossless: false,
                    preserve_metadata: false,
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
                q = (q + step).min(q_max);
            }
        } else {
            // High-precision binary search (Bisection) mode for exact rate-distortion optimization
            let mut low = options.quality_min.clamp(1, 100);
            let mut high = options.quality_max.clamp(low, 100);

            while low <= high {
                let mid = low + (high - low) / 2;
                let enc_opt = EncodeOptions {
                    quality: mid,
                    effort: options.effort,
                    lossless: false,
                    preserve_metadata: false,
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
                        // Quality threshold satisfied; try lower qualities to find smaller size
                        if mid == low {
                            break;
                        }
                        high = mid - 1;
                    } else {
                        // Quality threshold not satisfied; must increase quality
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
    use imgopt_codecs::{CodecCapabilities, CodecError, CodecFormat, EncodedOutput, PixelFormat};

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
            // Return identical 4x4 image
            Ok(DecodedPixels::new(4, 4, PixelFormat::Rgb8, vec![100; 48]))
        }

        fn encode(
            &self,
            _pixels: &DecodedPixels,
            options: &EncodeOptions,
        ) -> std::result::Result<EncodedOutput, CodecError> {
            // Simulate output size proportional to quality
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
        };

        let result = search_best_candidate(&raw, &codec, &options).unwrap();
        assert_eq!(result.diagnostics.total_evaluated, 3); // q70, q80, q90
                                                           // Smallest bytes (q70 -> size 700) should be chosen
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
            quality_step: 1, // trigger binary search
            try_lossless: false,
            max_allowed_bytes: None,
        };

        let result = search_best_candidate(&raw, &codec, &options).unwrap();
        // Since all candidates return perfect quality in TestCodec, binary search will search down to 50
        assert_eq!(result.selected.byte_size, 500);
        assert_eq!(result.selected.parameters_description, "webp-q50-eff4");
    }
}
