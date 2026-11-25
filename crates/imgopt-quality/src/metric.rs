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

    let mut sum_sq_diff = 0u64;
    for (&o, &c) in orig.iter().zip(cand.iter()) {
        let diff = (o as i32) - (c as i32);
        sum_sq_diff += (diff * diff) as u64;
    }

    let mse = (sum_sq_diff as f64) / (total_samples as f64);
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
/// Uses an 8x8 block window with standard constants C1 = (0.01*255)^2, C2 = (0.03*255)^2.
/// Accumulates single-pass integer sums per block (`u64`) for cache-friendly, SIMD-amenable speed.
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

    const C1: f64 = (0.01 * 255.0) * (0.01 * 255.0); // 6.5025
    const C2: f64 = (0.03 * 255.0) * (0.03 * 255.0); // 58.5225

    const BLOCK_SIZE: usize = 8;
    let num_blocks_x = width.div_ceil(BLOCK_SIZE);
    let num_blocks_y = height.div_ceil(BLOCK_SIZE);
    let row_stride = width * channels;

    // Evaluate visible color channels (skip pure alpha channel if RGBA/LA to measure visual color fidelity)
    let color_channels = if channels == 4 || channels == 2 {
        channels - 1
    } else {
        channels
    };

    let mut total_ssim = 0.0f64;
    let mut total_blocks_evaluated = 0usize;

    for by in 0..num_blocks_y {
        let start_y = by * BLOCK_SIZE;
        let end_y = (start_y + BLOCK_SIZE).min(height);
        let h_block = end_y - start_y;

        for bx in 0..num_blocks_x {
            let start_x = bx * BLOCK_SIZE;
            let end_x = (start_x + BLOCK_SIZE).min(width);
            let w_block = end_x - start_x;
            let n_pixels = w_block * h_block;
            if n_pixels == 0 {
                continue;
            }
            let n = n_pixels as f64;

            // Accumulate all visible channels together in sequential row memory order
            let mut sum_x = [0u64; 3];
            let mut sum_y = [0u64; 3];
            let mut sum_xx = [0u64; 3];
            let mut sum_yy = [0u64; 3];
            let mut sum_xy = [0u64; 3];

            for y in start_y..end_y {
                let row_start = y * row_stride + start_x * channels;
                let row_end = row_start + w_block * channels;
                let orig_row = &orig[row_start..row_end];
                let cand_row = &cand[row_start..row_end];

                match (channels, color_channels) {
                    (3, 3) => {
                        for (op, cp) in orig_row.chunks_exact(3).zip(cand_row.chunks_exact(3)) {
                            for ch in 0..3 {
                                let vx = u64::from(op[ch]);
                                let vy = u64::from(cp[ch]);
                                sum_x[ch] += vx;
                                sum_y[ch] += vy;
                                sum_xx[ch] += vx * vx;
                                sum_yy[ch] += vy * vy;
                                sum_xy[ch] += vx * vy;
                            }
                        }
                    }
                    (4, 3) => {
                        for (op, cp) in orig_row.chunks_exact(4).zip(cand_row.chunks_exact(4)) {
                            for ch in 0..3 {
                                let vx = u64::from(op[ch]);
                                let vy = u64::from(cp[ch]);
                                sum_x[ch] += vx;
                                sum_y[ch] += vy;
                                sum_xx[ch] += vx * vx;
                                sum_yy[ch] += vy * vy;
                                sum_xy[ch] += vx * vy;
                            }
                        }
                    }
                    (1, 1) => {
                        for (&ox, &cy) in orig_row.iter().zip(cand_row.iter()) {
                            let vx = u64::from(ox);
                            let vy = u64::from(cy);
                            sum_x[0] += vx;
                            sum_y[0] += vy;
                            sum_xx[0] += vx * vx;
                            sum_yy[0] += vy * vy;
                            sum_xy[0] += vx * vy;
                        }
                    }
                    (2, 1) => {
                        for (op, cp) in orig_row.chunks_exact(2).zip(cand_row.chunks_exact(2)) {
                            let vx = u64::from(op[0]);
                            let vy = u64::from(cp[0]);
                            sum_x[0] += vx;
                            sum_y[0] += vy;
                            sum_xx[0] += vx * vx;
                            sum_yy[0] += vy * vy;
                            sum_xy[0] += vx * vy;
                        }
                    }
                    _ => {
                        for (op, cp) in orig_row
                            .chunks_exact(channels)
                            .zip(cand_row.chunks_exact(channels))
                        {
                            for ch in 0..color_channels.min(3) {
                                let vx = u64::from(op[ch]);
                                let vy = u64::from(cp[ch]);
                                sum_x[ch] += vx;
                                sum_y[ch] += vy;
                                sum_xx[ch] += vx * vx;
                                sum_yy[ch] += vy * vy;
                                sum_xy[ch] += vx * vy;
                            }
                        }
                    }
                }
            }

            let active_ch = color_channels.min(3);
            for ch in 0..active_ch {
                let sx = sum_x[ch] as f64;
                let sy = sum_y[ch] as f64;
                let mu_x = sx / n;
                let mu_y = sy / n;

                // Exact integer identity: N * sum(x^2) - (sum x)^2, max value <= 64 * 4.16e6 < 2^28
                let n_u64 = n_pixels as u64;
                let var_x_num = (n_u64 * sum_xx[ch]).saturating_sub(sum_x[ch] * sum_x[ch]) as f64;
                let var_y_num = (n_u64 * sum_yy[ch]).saturating_sub(sum_y[ch] * sum_y[ch]) as f64;
                let cov_xy_num =
                    (n_u64 as i64 * sum_xy[ch] as i64 - sum_x[ch] as i64 * sum_y[ch] as i64) as f64;

                let n_sq = n * n;
                let sigma_x_sq = var_x_num / n_sq;
                let sigma_y_sq = var_y_num / n_sq;
                let sigma_xy = cov_xy_num / n_sq;

                let numerator = (2.0 * mu_x * mu_y + C1) * (2.0 * sigma_xy + C2);
                let denominator = (mu_x * mu_x + mu_y * mu_y + C1) * (sigma_x_sq + sigma_y_sq + C2);

                total_ssim += numerator / denominator;
                total_blocks_evaluated += 1;
            }
        }
    }

    if total_blocks_evaluated == 0 {
        return Ok(1.0);
    }

    let mean_ssim = total_ssim / (total_blocks_evaluated as f64);
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

    let expected = width
        .checked_mul(height)
        .and_then(|px| px.checked_mul(channels))
        .ok_or(QualityError::InvalidBuffer {
            expected: usize::MAX,
            actual: orig.len(),
        })?;
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
    fn test_ssim_matches_two_pass_reference() {
        let width = 19;
        let height = 13;
        let channels = 3;
        let orig: Vec<u8> = (0..width * height * channels)
            .map(|i| ((i * 37 + 11) % 256) as u8)
            .collect();
        let cand: Vec<u8> = orig
            .iter()
            .enumerate()
            .map(|(i, &v)| v.saturating_add((i % 9) as u8))
            .collect();

        let fast = compute_ssim(&orig, &cand, width, height, channels).unwrap();

        // Reference two-pass computation
        let c1 = (0.01 * 255.0) * (0.01 * 255.0);
        let c2 = (0.03 * 255.0) * (0.03 * 255.0);
        let mut total = 0.0;
        let mut count = 0usize;
        for ch in 0..channels {
            for by in 0..height.div_ceil(8) {
                for bx in 0..width.div_ceil(8) {
                    let sx = bx * 8;
                    let sy = by * 8;
                    let ex = (sx + 8).min(width);
                    let ey = (sy + 8).min(height);
                    let n = ((ex - sx) * (ey - sy)) as f64;
                    let (mut mx, mut my) = (0.0, 0.0);
                    for y in sy..ey {
                        for x in sx..ex {
                            let idx = (y * width + x) * channels + ch;
                            mx += orig[idx] as f64;
                            my += cand[idx] as f64;
                        }
                    }
                    mx /= n;
                    my /= n;
                    let (mut vx, mut vy, mut cxy) = (0.0, 0.0, 0.0);
                    for y in sy..ey {
                        for x in sx..ex {
                            let idx = (y * width + x) * channels + ch;
                            let dx = orig[idx] as f64 - mx;
                            let dy = cand[idx] as f64 - my;
                            vx += dx * dx;
                            vy += dy * dy;
                            cxy += dx * dy;
                        }
                    }
                    vx /= n;
                    vy /= n;
                    cxy /= n;
                    total += ((2.0 * mx * my + c1) * (2.0 * cxy + c2))
                        / ((mx * mx + my * my + c1) * (vx + vy + c2));
                    count += 1;
                }
            }
        }
        let reference = (total / count as f64).clamp(0.0, 1.0);
        assert!(
            (fast - reference).abs() < 1e-12,
            "fast={fast}, reference={reference}"
        );
    }

    #[test]
    fn test_invalid_buffer_returns_error() {
        let orig = vec![0u8; 100];
        let cand = vec![0u8; 90];
        let res = compute_ssim(&orig, &cand, 10, 10, 1);
        assert!(matches!(res, Err(QualityError::InvalidBuffer { .. })));
    }
}
