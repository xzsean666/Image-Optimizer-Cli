use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::config::OptimizationConfig;
use crate::error::Result;
use crate::executor::{execute_batch_scoped, BatchOptions, BatchTask};
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
    let default_concurrency = BatchOptions::default().concurrency;
    run_benchmark_with_concurrency(dataset_path, config, default_concurrency)
}

/// Run optimization benchmark using bounded concurrency.
pub fn run_benchmark_with_concurrency(
    dataset_path: &Path,
    config: &OptimizationConfig,
    concurrency: usize,
) -> Result<BenchmarkReport> {
    let start_time = Instant::now();
    config.validate()?;

    let version = env!("CARGO_PKG_VERSION");
    let service = OptimizationService::new(version);

    let scan_res = scan_path(dataset_path, &ScanOptions::default())?;
    let tasks: Vec<BatchTask<u64>> = scan_res
        .files
        .iter()
        .map(|f| BatchTask::new(&f.path, f.byte_size))
        .collect();

    let batch_opts = BatchOptions {
        concurrency: concurrency.max(1),
        stop_on_error: false,
    };

    let (batch_results, _) = execute_batch_scoped(tasks, &batch_opts, |path, expected_bytes| {
        let case_start = Instant::now();
        match service.optimize_file(&path, config) {
            Ok(opt_res) => Ok(BenchmarkCaseResult {
                path,
                format: Some(opt_res.effective_format),
                original_bytes: opt_res.validated_output.original_size,
                optimized_bytes: Some(opt_res.validated_output.byte_size),
                savings_ratio: Some(opt_res.validated_output.savings_ratio),
                quality_score: Some(opt_res.quality_score),
                psnr_score: opt_res.psnr_score,
                elapsed_millis: case_start.elapsed().as_millis() as u64,
                error: None,
            }),
            Err(e) => {
                let actual_size = std::fs::metadata(&path)
                    .map(|m| m.len())
                    .unwrap_or(expected_bytes);
                Ok(BenchmarkCaseResult {
                    path,
                    format: None,
                    original_bytes: actual_size,
                    optimized_bytes: None,
                    savings_ratio: None,
                    quality_score: None,
                    psnr_score: None,
                    elapsed_millis: case_start.elapsed().as_millis() as u64,
                    error: Some(e.to_string()),
                })
            }
        }
    });

    let mut cases = Vec::with_capacity(batch_results.len());
    let mut total_original = 0u64;
    let mut total_optimized = 0u64;
    let mut success_count = 0usize;
    let mut fail_count = 0usize;
    let mut sum_quality = 0.0f64;

    for res in batch_results {
        if let Ok(case) = res.output {
            total_original += case.original_bytes;
            if let (Some(opt_size), Some(q)) = (case.optimized_bytes, case.quality_score) {
                total_optimized += opt_size;
                success_count += 1;
                sum_quality += q;
            } else {
                fail_count += 1;
            }
            cases.push(case);
        }
    }

    let total_saved = if total_original >= total_optimized {
        i64::try_from(total_original - total_optimized).unwrap_or(i64::MAX)
    } else {
        -i64::try_from(total_optimized - total_original).unwrap_or(i64::MAX)
    };
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
        total_cases: cases.len(),
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
    use std::fs;
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
