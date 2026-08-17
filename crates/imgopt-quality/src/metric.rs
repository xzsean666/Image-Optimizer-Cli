use std::fmt;

/// Errors that can occur during image quality evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualityError {
    DimensionMismatch {
        orig_width: usize,
        orig_height: usize,
        cand_width: usize,
        cand_height: usize,
    },
    ChannelMismatch {
        orig_channels: usize,
        cand_channels: usize,
    },
    InvalidBuffer {
        expected: usize,
        actual: usize,
    },
    EmptyImage,
}

impl fmt::Display for QualityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionMismatch {
                orig_width,
                orig_height,
                cand_width,
                cand_height,
            } => write!(
                f,
                "dimension mismatch: original is {orig_width}x{orig_height}, candidate is {cand_width}x{cand_height}"
            ),
            Self::ChannelMismatch {
                orig_channels,
                cand_channels,
            } => write!(
                f,
                "channel mismatch: original has {orig_channels} channels, candidate has {cand_channels}"
            ),
            Self::InvalidBuffer { expected, actual } => write!(
                f,
                "invalid buffer size: expected {expected} bytes, got {actual}"
            ),
            Self::EmptyImage => write!(f, "empty image buffer"),
        }
    }
}

impl std::error::Error for QualityError {}

/// Comprehensive perceptual and statistical quality score.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QualityScore {
    /// Structural Similarity Index in range [0.0, 1.0]. 1.0 is identical.
    pub ssim: f64,
    /// Peak Signal-to-Noise Ratio in dB. Range: [0.0, 100.0+].
    pub psnr: f64,
}

/// Compute both SSIM and PSNR quality metrics.
pub fn compute_quality_score(
    orig: &[u8],
    cand: &[u8],
    width: usize,
    height: usize,
    channels: usize,
) -> Result<QualityScore, QualityError> {
    let ssim = compute_ssim(orig, cand, width, height, channels)?;
    let psnr = compute_psnr(orig, cand, width, height, channels)?;
    Ok(QualityScore { ssim, psnr })
}

/// Compute PSNR (Peak Signal-to-Noise Ratio) in decibels (dB).
pub fn compute_psnr(
    orig: &[u8],
    cand: &[u8],
    width: usize,
    height: usize,
    channels: usize,
) -> Result<f64, QualityError> {
    validate_buffers(orig, cand, width, height, channels)?;

    let total_samples = orig.len();
    if total_samples == 0 {
        return Err(QualityError::EmptyImage);
    }

    let mut sum_sq_diff = 0.0f64;
    for (&o, &c) in orig.iter().zip(cand.iter()) {
        let diff = (o as f64) - (c as f64);
        sum_sq_diff += diff * diff;
    }

    let mse = sum_sq_diff / (total_samples as f64);
    if mse < 1e-10 {
        // Practically identical images
        return Ok(100.0);
    }

    let max_val = 255.0f64;
    let psnr = 10.0 * (max_val * max_val / mse).log10();
    Ok(psnr)
}

/// Compute SSIM (Structural Similarity Index Measure) in range [0.0, 1.0].
///
/// Uses an 8x8 sliding/block window with standard constants C1 = (0.01*255)^2, C2 = (0.03*255)^2.
pub fn compute_ssim(
    orig: &[u8],
    cand: &[u8],
    width: usize,
    height: usize,
    channels: usize,
) -> Result<f64, QualityError> {
    validate_buffers(orig, cand, width, height, channels)?;

    if width == 0 || height == 0 || channels == 0 {
        return Err(QualityError::EmptyImage);
    }

    let c1 = (0.01 * 255.0) * (0.01 * 255.0); // 6.5025
    let c2 = (0.03 * 255.0) * (0.03 * 255.0); // 58.5225

    let block_size = 8;
    let num_blocks_x = width.div_ceil(block_size);
    let num_blocks_y = height.div_ceil(block_size);

    let mut total_ssim = 0.0f64;
    let mut total_channels_evaluated = 0usize;

    // Evaluate visible color channels (skip pure alpha channel if RGBA to measure visual color fidelity)
    let color_channels = if channels == 4 || channels == 2 {
        channels - 1
    } else {
        channels
    };

    for ch in 0..color_channels {
        for by in 0..num_blocks_y {
            for bx in 0..num_blocks_x {
                let start_x = bx * block_size;
                let start_y = by * block_size;
                let end_x = (start_x + block_size).min(width);
                let end_y = (start_y + block_size).min(height);

                let n_pixels = (end_x - start_x) * (end_y - start_y);
                if n_pixels == 0 {
                    continue;
                }

                let mut sum_x = 0.0f64;
                let mut sum_y = 0.0f64;

                for y in start_y..end_y {
                    for x in start_x..end_x {
                        let idx = (y * width + x) * channels + ch;
                        let val_x = orig[idx] as f64;
                        let val_y = cand[idx] as f64;
                        sum_x += val_x;
                        sum_y += val_y;
                    }
                }

                let mu_x = sum_x / (n_pixels as f64);
                let mu_y = sum_y / (n_pixels as f64);

                let mut var_x = 0.0f64;
                let mut var_y = 0.0f64;
                let mut cov_xy = 0.0f64;

                for y in start_y..end_y {
                    for x in start_x..end_x {
                        let idx = (y * width + x) * channels + ch;
                        let diff_x = (orig[idx] as f64) - mu_x;
                        let diff_y = (cand[idx] as f64) - mu_y;
                        var_x += diff_x * diff_x;
                        var_y += diff_y * diff_y;
                        cov_xy += diff_x * diff_y;
                    }
                }

                let sigma_x_sq = var_x / (n_pixels as f64);
                let sigma_y_sq = var_y / (n_pixels as f64);
                let sigma_xy = cov_xy / (n_pixels as f64);

                let numerator = (2.0 * mu_x * mu_y + c1) * (2.0 * sigma_xy + c2);
                let denominator = (mu_x * mu_x + mu_y * mu_y + c1) * (sigma_x_sq + sigma_y_sq + c2);

                let block_ssim = numerator / denominator;
                total_ssim += block_ssim;
                total_channels_evaluated += 1;
            }
        }
    }

    if total_channels_evaluated == 0 {
        return Ok(1.0);
    }

    let mean_ssim = total_ssim / (total_channels_evaluated as f64);
    // Clamp to [0.0, 1.0]
    Ok(mean_ssim.clamp(0.0, 1.0))
}

fn validate_buffers(
    orig: &[u8],
    cand: &[u8],
    width: usize,
    height: usize,
    channels: usize,
) -> Result<(), QualityError> {
    if width == 0 || height == 0 || channels == 0 {
        return Err(QualityError::EmptyImage);
    }

    let expected = width * height * channels;
    if orig.len() != expected {
        return Err(QualityError::InvalidBuffer {
            expected,
            actual: orig.len(),
        });
    }
    if cand.len() != expected {
        return Err(QualityError::InvalidBuffer {
            expected,
            actual: cand.len(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_images_have_max_score() {
        let data = vec![128u8; 16 * 16 * 3];
        let score = compute_quality_score(&data, &data, 16, 16, 3).unwrap();
        assert!((score.ssim - 1.0).abs() < 1e-6);
        assert!(score.psnr >= 100.0);
    }

    #[test]
    fn test_degraded_images_have_lower_score() {
        let width = 16;
        let height = 16;
        let channels = 3;
        let orig: Vec<u8> = (0..width * height * channels)
            .map(|i| (i % 256) as u8)
            .collect();

        // Slight perturbation
        let mut slight = orig.clone();
        for b in slight.iter_mut().take(20) {
            *b = b.saturating_add(5);
        }

        // Heavy degradation
        let mut heavy = orig.clone();
        for b in heavy.iter_mut() {
            *b = 255 - *b;
        }

        let score_slight = compute_quality_score(&orig, &slight, width, height, channels).unwrap();
        let score_heavy = compute_quality_score(&orig, &heavy, width, height, channels).unwrap();

        assert!(score_slight.ssim > 0.95);
        assert!(score_slight.ssim < 1.0);

        assert!(score_heavy.ssim < score_slight.ssim);
        assert!(score_heavy.psnr < score_slight.psnr);
    }

    #[test]
    fn test_invalid_buffer_returns_error() {
        let orig = vec![0u8; 100];
        let cand = vec![0u8; 90];
        let res = compute_ssim(&orig, &cand, 10, 10, 1);
        assert!(matches!(res, Err(QualityError::InvalidBuffer { .. })));
    }
}
