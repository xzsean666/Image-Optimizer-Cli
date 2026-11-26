use std::path::Path;

use imgopt_core::compute_byte_savings;
use serde::{Deserialize, Serialize};

use crate::store::write_bytes_atomically;

/// Status of an individual item in a sync or optimization run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunItemStatus {
    Optimized,
    Skipped,
    Failed,
}

/// Detailed per-file record in the run result JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResultRecord {
    pub path: String,
    pub status: RunItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_sha256: Option<String>,
    pub source_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimized_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Aggregated totals and savings for a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    pub total_files: usize,
    pub optimized_files: usize,
    pub skipped_files: usize,
    pub failed_files: usize,
    pub original_bytes: u64,
    pub optimized_bytes: u64,
    pub saved_bytes: i64,
    pub saved_ratio: f64,
    pub elapsed_millis: u64,
}

/// Machine-readable per-run result document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    pub version: u32,
    pub summary: RunSummary,
    pub items: Vec<RunResultRecord>,
}

impl RunResult {
    pub const SCHEMA_VERSION: u32 = 1;

    /// Build a RunResult from records and elapsed runtime.
    pub fn new(items: Vec<RunResultRecord>, elapsed_millis: u64) -> Self {
        let summary = Self::compute_summary(&items, elapsed_millis);
        Self {
            version: Self::SCHEMA_VERSION,
            summary,
            items,
        }
    }

    /// Compute summary statistics from a slice of result records.
    pub fn compute_summary(items: &[RunResultRecord], elapsed_millis: u64) -> RunSummary {
        let mut total_files = 0usize;
        let mut optimized_files = 0usize;
        let mut skipped_files = 0usize;
        let mut failed_files = 0usize;
        let mut original_bytes = 0u64;
        let mut optimized_bytes = 0u64;

        for item in items {
            total_files += 1;
            original_bytes += item.source_bytes;

            match item.status {
                RunItemStatus::Optimized => {
                    optimized_files += 1;
                    if let Some(opt_b) = item.optimized_bytes {
                        optimized_bytes += opt_b;
                    }
                }
                RunItemStatus::Skipped => {
                    skipped_files += 1;
                    if let Some(opt_b) = item.optimized_bytes {
                        optimized_bytes += opt_b;
                    } else {
                        optimized_bytes += item.source_bytes;
                    }
                }
                RunItemStatus::Failed => {
                    failed_files += 1;
                }
            }
        }

        let (saved_bytes, raw_ratio) = compute_byte_savings(original_bytes, optimized_bytes);
        let saved_ratio = if saved_bytes > 0 { raw_ratio } else { 0.0 };

        RunSummary {
            total_files,
            optimized_files,
            skipped_files,
            failed_files,
            original_bytes,
            optimized_bytes,
            saved_bytes,
            saved_ratio,
            elapsed_millis,
        }
    }

    /// Serialize this run result to formatted JSON.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Write this run result to the destination path atomically via exclusive temp file.
    pub fn write_to_file<P: AsRef<Path>>(&self, path: P) -> std::io::Result<()> {
        let json_bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        write_bytes_atomically(path.as_ref(), &json_bytes, ".tmp-result-", ".json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_summary_calculation() {
        let items = vec![
            RunResultRecord {
                path: "img1.png".into(),
                status: RunItemStatus::Optimized,
                source_sha256: Some("sha_1".into()),
                source_bytes: 1000,
                optimized_bytes: Some(600),
                format: Some("webp".into()),
                quality_score: Some(0.95),
                remote_url: Some("https://cdn.example.com/img1.webp".into()),
                error: None,
            },
            RunResultRecord {
                path: "img2.jpg".into(),
                status: RunItemStatus::Skipped,
                source_sha256: Some("sha_2".into()),
                source_bytes: 500,
                optimized_bytes: Some(400),
                format: Some("webp".into()),
                quality_score: Some(0.93),
                remote_url: Some("https://cdn.example.com/img2.webp".into()),
                error: None,
            },
            RunResultRecord {
                path: "broken.jpg".into(),
                status: RunItemStatus::Failed,
                source_sha256: None,
                source_bytes: 200,
                optimized_bytes: None,
                format: None,
                quality_score: None,
                remote_url: None,
                error: Some("corrupted header".into()),
            },
        ];

        let result = RunResult::new(items, 150);
        assert_eq!(result.summary.total_files, 3);
        assert_eq!(result.summary.optimized_files, 1);
        assert_eq!(result.summary.skipped_files, 1);
        assert_eq!(result.summary.failed_files, 1);
        assert_eq!(result.summary.original_bytes, 1700);
        assert_eq!(result.summary.optimized_bytes, 1000);
        assert_eq!(result.summary.saved_bytes, 700);
        assert!((result.summary.saved_ratio - (700.0 / 1700.0)).abs() < 1e-6);
        assert_eq!(result.summary.elapsed_millis, 150);
    }

    #[test]
    fn test_atomic_write_and_golden_roundtrip() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("sub").join("result.json");

        let items = vec![RunResultRecord {
            path: "hero.png".into(),
            status: RunItemStatus::Optimized,
            source_sha256: Some("abc".into()),
            source_bytes: 2000,
            optimized_bytes: Some(1200),
            format: Some("webp".into()),
            quality_score: Some(0.96),
            remote_url: None,
            error: None,
        }];

        let result = RunResult::new(items, 50);
        result.write_to_file(&path).unwrap();

        let read_back = fs::read_to_string(&path).unwrap();
        let deserialized: RunResult = serde_json::from_str(&read_back).unwrap();
        assert_eq!(result, deserialized);
    }
}
