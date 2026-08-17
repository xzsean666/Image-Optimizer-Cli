use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::config::OptimizationConfig;
use crate::error::Result;
use crate::model::ImageFormat;
use crate::optimizer::OptimizationService;
use crate::scanner::{scan_path, ScanOptions};

/// An individual test case result in a benchmark run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkCaseResult {
    pub path: PathBuf,
    pub format: Option<ImageFormat>,
    pub original_bytes: u64,
    pub optimized_bytes: Option<u64>,
    pub savings_ratio: Option<f64>,
    pub quality_score: Option<f64>,
    pub psnr_score: Option<f64>,
    pub elapsed_millis: u64,
    pub error: Option<String>,
}

/// Aggregated metrics across all benchmark cases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkSummary {
    pub total_cases: usize,
    pub successful_cases: usize,
    pub failed_cases: usize,
    pub total_original_bytes: u64,
    pub total_optimized_bytes: u64,
    pub total_saved_bytes: i64,
    pub overall_savings_ratio: f64,
    pub average_quality_score: f64,
    pub total_elapsed_millis: u64,
}

/// Comprehensive machine-readable benchmark report document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub optimizer_version: String,
    pub summary: BenchmarkSummary,
    pub cases: Vec<BenchmarkCaseResult>,
}

impl BenchmarkReport {
    pub fn to_json_pretty(&self) -> std::result::Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Run optimization benchmark across all supported images in a dataset directory or file.
pub fn run_benchmark(dataset_path: &Path, config: &OptimizationConfig) -> Result<BenchmarkReport> {
    let start_time = Instant::now();
    config.validate()?;

    let version = env!("CARGO_PKG_VERSION");
    let service = OptimizationService::new(version);

    let scan_res = scan_path(dataset_path, &ScanOptions::default())?;

    let mut cases = Vec::with_capacity(scan_res.files.len());
    let mut total_original = 0u64;
    let mut total_optimized = 0u64;
    let mut success_count = 0usize;
    let mut fail_count = 0usize;
    let mut sum_quality = 0.0f64;

    for discovered in &scan_res.files {
        let case_start = Instant::now();
        let file_bytes = match fs::read(&discovered.path) {
            Ok(b) => b,
            Err(e) => {
                fail_count += 1;
                cases.push(BenchmarkCaseResult {
                    path: discovered.path.clone(),
                    format: None,
                    original_bytes: discovered.byte_size,
                    optimized_bytes: None,
                    savings_ratio: None,
                    quality_score: None,
                    psnr_score: None,
                    elapsed_millis: case_start.elapsed().as_millis() as u64,
                    error: Some(format!("failed to read file: {e}")),
                });
                continue;
            }
        };

        let orig_size = file_bytes.len() as u64;
        total_original += orig_size;

        match service.optimize_bytes(&file_bytes, config) {
            Ok(opt_res) => {
                let opt_size = opt_res.validated_output.byte_size;
                total_optimized += opt_size;
                success_count += 1;
                sum_quality += opt_res.quality_score;

                cases.push(BenchmarkCaseResult {
                    path: discovered.path.clone(),
                    format: Some(opt_res.effective_format),
                    original_bytes: orig_size,
                    optimized_bytes: Some(opt_size),
                    savings_ratio: Some(opt_res.validated_output.savings_ratio),
                    quality_score: Some(opt_res.quality_score),
                    psnr_score: opt_res.psnr_score,
                    elapsed_millis: case_start.elapsed().as_millis() as u64,
                    error: None,
                });
            }
            Err(e) => {
                fail_count += 1;
                cases.push(BenchmarkCaseResult {
                    path: discovered.path.clone(),
                    format: None,
                    original_bytes: orig_size,
                    optimized_bytes: None,
                    savings_ratio: None,
                    quality_score: None,
                    psnr_score: None,
                    elapsed_millis: case_start.elapsed().as_millis() as u64,
                    error: Some(e.to_string()),
                });
            }
        }
    }

    let total_saved = (total_original as i64) - (total_optimized as i64);
    let overall_ratio = if total_original > 0 && total_saved > 0 {
        total_saved as f64 / total_original as f64
    } else {
        0.0
    };

    let average_quality_score = if success_count > 0 {
        sum_quality / (success_count as f64)
    } else {
        0.0
    };

    let summary = BenchmarkSummary {
        total_cases: scan_res.files.len(),
        successful_cases: success_count,
        failed_cases: fail_count,
        total_original_bytes: total_original,
        total_optimized_bytes: total_optimized,
        total_saved_bytes: total_saved,
        overall_savings_ratio: overall_ratio,
        average_quality_score,
        total_elapsed_millis: start_time.elapsed().as_millis() as u64,
    };

    Ok(BenchmarkReport {
        optimizer_version: version.to_string(),
        summary,
        cases,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OptimizationPreset;
    use imgopt_codecs::ImageCodec;
    use tempfile::TempDir;

    fn make_test_png(path: &Path) {
        let codec = imgopt_codecs::PngCodec;
        let pixels = imgopt_codecs::DecodedPixels::new(
            4,
            4,
            imgopt_codecs::PixelFormat::Rgb8,
            vec![180; 4 * 4 * 3],
        );
        let output = codec
            .encode(&pixels, &imgopt_codecs::EncodeOptions::default())
            .unwrap();
        fs::write(path, &output.bytes).unwrap();
    }

    #[test]
    fn test_benchmark_runs_and_emits_report() {
        let temp = TempDir::new().unwrap();
        let img1 = temp.path().join("img1.png");
        let img2 = temp.path().join("img2.png");
        make_test_png(&img1);
        make_test_png(&img2);

        let config = OptimizationConfig::from_preset(OptimizationPreset::Balanced);
        let report = run_benchmark(temp.path(), &config).unwrap();

        assert_eq!(report.summary.total_cases, 2);
        assert_eq!(report.summary.successful_cases, 2);
        assert_eq!(report.summary.failed_cases, 0);
        assert!(report.summary.average_quality_score >= 0.90);

        let json = report.to_json_pretty().unwrap();
        assert!(json.contains("\"total_cases\": 2"));
    }
}
