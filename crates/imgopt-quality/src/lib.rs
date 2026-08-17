pub mod metric;

pub use metric::{compute_psnr, compute_quality_score, compute_ssim, QualityError, QualityScore};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert_eq!(VERSION, "0.1.0");
    }
}
