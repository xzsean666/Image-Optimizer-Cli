use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

use crate::error::{ErrorKind, ImgoptError, PipelineStage, Result};
use crate::model::ImageFormat;

/// An evaluated candidate image from the search process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationCandidate {
    pub format: ImageFormat,
    #[serde(skip_serializing, default)]
    pub bytes: Vec<u8>,
    pub byte_size: u64,
    pub quality_score: f64,
    pub psnr_score: Option<f64>,
    pub parameters_description: String,
}

impl OptimizationCandidate {
    pub fn new(
        format: ImageFormat,
        bytes: Vec<u8>,
        quality_score: f64,
        psnr_score: Option<f64>,
        parameters_description: impl Into<String>,
    ) -> Self {
        let byte_size = bytes.len() as u64;
        Self {
            format,
            bytes,
            byte_size,
            quality_score,
            psnr_score,
            parameters_description: parameters_description.into(),
        }
    }
}

/// Criteria for filtering and selecting the optimal candidate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RankingCriteria {
    pub min_quality_target: f64,
    pub max_allowed_bytes: Option<u64>,
}

impl RankingCriteria {
    pub fn new(min_quality_target: f64) -> Self {
        Self {
            min_quality_target,
            max_allowed_bytes: None,
        }
    }

    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_allowed_bytes = Some(max_bytes);
        self
    }
}

/// Compare two accepted candidates according to the ranking policy:
/// 1. Smallest output byte size
/// 2. Highest perceptual quality score
/// 3. Alphabetical format name
/// 4. Alphabetical parameters description
pub fn compare_candidates(a: &OptimizationCandidate, b: &OptimizationCandidate) -> Ordering {
    a.byte_size
        .cmp(&b.byte_size)
        .then_with(|| {
            // Higher quality is better (reverse float comparison)
            b.quality_score
                .partial_cmp(&a.quality_score)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| a.format.as_str().cmp(b.format.as_str()))
        .then_with(|| a.parameters_description.cmp(&b.parameters_description))
}

/// Filter candidates meeting the quality threshold and return the ranked best candidate.
pub fn select_best_candidate<'a>(
    candidates: &'a [OptimizationCandidate],
    criteria: &RankingCriteria,
) -> Result<&'a OptimizationCandidate> {
    if candidates.is_empty() {
        return Err(ImgoptError::new(
            PipelineStage::QualityEvaluation,
            ErrorKind::Quality("no candidates generated for evaluation".into()),
        ));
    }

    let mut eligible: Vec<&OptimizationCandidate> = candidates
        .iter()
        .filter(|c| c.quality_score >= criteria.min_quality_target)
        .filter(|c| match criteria.max_allowed_bytes {
            Some(max_b) => c.byte_size <= max_b,
            None => true,
        })
        .collect();

    if eligible.is_empty() {
        return Err(ImgoptError::new(
            PipelineStage::QualityEvaluation,
            ErrorKind::Quality(format!(
                "no candidate met the quality target of {:.4} (tested {} candidates)",
                criteria.min_quality_target,
                candidates.len()
            )),
        ));
    }

    eligible.sort_by(|a, b| compare_candidates(a, b));
    Ok(eligible[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_smallest_acceptable_candidate() {
        let candidates = vec![
            OptimizationCandidate::new(ImageFormat::Webp, vec![0; 500], 0.94, None, "webp-q80"),
            OptimizationCandidate::new(ImageFormat::Webp, vec![0; 400], 0.92, None, "webp-q75"),
            OptimizationCandidate::new(ImageFormat::Jpeg, vec![0; 600], 0.96, None, "jpeg-q85"),
        ];

        let criteria = RankingCriteria::new(0.92);
        let best = select_best_candidate(&candidates, &criteria).unwrap();

        assert_eq!(best.byte_size, 400);
        assert_eq!(best.parameters_description, "webp-q75");
    }

    #[test]
    fn test_rejects_candidates_below_quality_target() {
        let candidates = vec![
            OptimizationCandidate::new(ImageFormat::Webp, vec![0; 200], 0.80, None, "webp-q50"),
            OptimizationCandidate::new(ImageFormat::Jpeg, vec![0; 300], 0.88, None, "jpeg-q70"),
        ];

        let criteria = RankingCriteria::new(0.90);
        let res = select_best_candidate(&candidates, &criteria);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().stage, PipelineStage::QualityEvaluation);
    }

    #[test]
    fn test_tie_breaking_same_size() {
        let candidates = vec![
            OptimizationCandidate::new(ImageFormat::Webp, vec![0; 500], 0.93, None, "variant-a"),
            OptimizationCandidate::new(ImageFormat::Webp, vec![0; 500], 0.95, None, "variant-b"),
        ];

        let criteria = RankingCriteria::new(0.90);
        let best = select_best_candidate(&candidates, &criteria).unwrap();
        // Higher quality wins on size tie
        assert_eq!(best.quality_score, 0.95);
        assert_eq!(best.parameters_description, "variant-b");
    }

    #[test]
    fn test_empty_candidates_returns_error() {
        let criteria = RankingCriteria::new(0.90);
        let res = select_best_candidate(&[], &criteria);
        assert!(res.is_err());
    }
}
