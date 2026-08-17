use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::config::{OptimizationConfig, OutputFormatPolicy};
use crate::error::Result;
use crate::executor::BatchOptions;
use crate::hash::{hash_bytes, Sha256Digest};
use crate::model::ImageFormat;
use crate::optimization_key::OptimizationKey;
use crate::optimizer::{OptimizationResult, OptimizationService};
use crate::scanner::{scan_path, ScanOptions};

/// Plan action for an individual file evaluated against the sync state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncPlanAction {
    SkipUpToDate {
        source_sha256: String,
        storage_url: Option<String>,
        optimized_bytes: u64,
        format: ImageFormat,
    },
    ProcessNew {
        source_sha256: String,
    },
    Reprocess {
        source_sha256: String,
        reason: String,
    },
}

/// Abstract contract for remote storage integration with the sync pipeline.
pub trait SyncStorage: Send + Sync {
    fn upload_and_verify(
        &self,
        bucket: &str,
        object_key: &str,
        body: &[u8],
        content_type: &str,
        content_sha256: Option<&str>,
    ) -> Result<SyncStorageOutput>;

    fn derive_object_key(&self, source_sha256: &str, format: ImageFormat) -> Result<String>;
    fn build_public_url(&self, object_key: &str) -> Option<String>;
}

/// Output metadata of a verified storage upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncStorageOutput {
    pub bucket: String,
    pub object_key: String,
    pub byte_size: u64,
    pub etag: Option<String>,
    pub public_url: Option<String>,
    pub uploaded_at: String,
}

/// Abstract contract for manifest lookup, state transitions, and atomic commit.
pub trait SyncManifest: Send + Sync {
    fn evaluate(&self, source_sha256: &str, current_opt_key: &str) -> SyncPlanAction;
    fn record_success(
        &mut self,
        source_path: &str,
        opt_res: &OptimizationResult,
        storage_out: &SyncStorageOutput,
    );
    fn record_failure(&mut self, source_path: &str, source_sha256: Option<&str>, error: &str);
    fn commit(&self) -> Result<()>;
}

/// Status of an individual item in a sync run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncItemStatus {
    Optimized,
    Skipped,
    Failed,
}

/// Detailed per-file record produced by a sync run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncItemResult {
    pub path: PathBuf,
    pub source_sha256: Option<Sha256Digest>,
    pub status: SyncItemStatus,
    pub source_bytes: u64,
    pub optimized_bytes: Option<u64>,
    pub format: Option<ImageFormat>,
    pub quality_score: Option<f64>,
    pub remote_url: Option<String>,
    pub error: Option<String>,
}

/// Comprehensive outcome of a sync run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncRunResult {
    pub total_files: usize,
    pub optimized_files: usize,
    pub skipped_files: usize,
    pub failed_files: usize,
    pub original_bytes: u64,
    pub optimized_bytes: u64,
    pub saved_bytes: i64,
    pub saved_ratio: f64,
    pub elapsed_millis: u64,
    pub items: Vec<SyncItemResult>,
}

/// Orchestrates recursive scanning, content hashing, deduplication, optimization, upload, and manifest commit.
#[derive(Debug, Clone)]
pub struct SyncService {
    optimizer: OptimizationService,
    batch_options: BatchOptions,
    bucket: String,
}

impl SyncService {
    pub fn new(
        optimizer_version: impl Into<String>,
        batch_options: BatchOptions,
        bucket: impl Into<String>,
    ) -> Self {
        Self {
            optimizer: OptimizationService::new(optimizer_version),
            batch_options,
            bucket: bucket.into(),
        }
    }

    pub fn optimizer(&self) -> &OptimizationService {
        &self.optimizer
    }

    pub fn batch_options(&self) -> &BatchOptions {
        &self.batch_options
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }
    pub fn run_sync<S: SyncStorage, M: SyncManifest>(
        &self,
        input_path: &Path,
        config: &OptimizationConfig,
        storage: &S,
        manifest: &mut M,
    ) -> Result<SyncRunResult> {
        let start_time = Instant::now();
        config.validate()?;

        let scan_res = scan_path(input_path, &ScanOptions::default())?;
        if scan_res.files.is_empty() {
            return Ok(SyncRunResult {
                total_files: 0,
                optimized_files: 0,
                skipped_files: 0,
                failed_files: 0,
                original_bytes: 0,
                optimized_bytes: 0,
                saved_bytes: 0,
                saved_ratio: 0.0,
                elapsed_millis: start_time.elapsed().as_millis() as u64,
                items: Vec::new(),
            });
        }

        let mut items = Vec::with_capacity(scan_res.files.len());
        let mut total_original = 0u64;
        let mut total_optimized = 0u64;
        let mut optimized_count = 0usize;
        let mut skipped_count = 0usize;
        let mut failed_count = 0usize;

        for discovered in &scan_res.files {
            let path_str = discovered.path.to_string_lossy().to_string();

            // Read file bytes and hash
            let file_bytes = match fs::read(&discovered.path) {
                Ok(b) => b,
                Err(e) => {
                    failed_count += 1;
                    let err_msg = format!("failed to read file: {e}");
                    manifest.record_failure(&path_str, None, &err_msg);
                    items.push(SyncItemResult {
                        path: discovered.path.clone(),
                        source_sha256: None,
                        status: SyncItemStatus::Failed,
                        source_bytes: discovered.byte_size,
                        optimized_bytes: None,
                        format: None,
                        quality_score: None,
                        remote_url: None,
                        error: Some(err_msg),
                    });
                    continue;
                }
            };

            let source_sha = hash_bytes(&file_bytes);
            let sha_str = source_sha.to_hex();
            total_original += file_bytes.len() as u64;

            // Target format projection for key computation
            let detected_fmt = match crate::detect::detect_format(&file_bytes) {
                Ok(fmt) => fmt,
                Err(e) => {
                    failed_count += 1;
                    let err_msg = format!("format detection failed: {e}");
                    manifest.record_failure(&path_str, Some(&sha_str), &err_msg);
                    items.push(SyncItemResult {
                        path: discovered.path.clone(),
                        source_sha256: Some(source_sha),
                        status: SyncItemStatus::Failed,
                        source_bytes: file_bytes.len() as u64,
                        optimized_bytes: None,
                        format: None,
                        quality_score: None,
                        remote_url: None,
                        error: Some(err_msg),
                    });
                    continue;
                }
            };

            let expected_format = match config.output_format_policy {
                OutputFormatPolicy::Force(fmt) => fmt,
                _ => detected_fmt,
            };

            let opt_key = OptimizationKey::compute(
                &source_sha,
                self.optimizer.optimizer_version(),
                expected_format,
                config,
            );

            // Deduplication and resume decision
            let mut action = manifest.evaluate(&sha_str, opt_key.as_str());
            if config.output_format_policy == OutputFormatPolicy::AllowWebp
                && expected_format != ImageFormat::Webp
                && !matches!(action, SyncPlanAction::SkipUpToDate { .. })
            {
                let opt_key_webp = OptimizationKey::compute(
                    &source_sha,
                    self.optimizer.optimizer_version(),
                    ImageFormat::Webp,
                    config,
                );
                let webp_action = manifest.evaluate(&sha_str, opt_key_webp.as_str());
                if matches!(webp_action, SyncPlanAction::SkipUpToDate { .. }) {
                    action = webp_action;
                }
            }

            match action {
                SyncPlanAction::SkipUpToDate {
                    storage_url,
                    optimized_bytes,
                    format,
                    ..
                } => {
                    skipped_count += 1;
                    total_optimized += optimized_bytes;
                    items.push(SyncItemResult {
                        path: discovered.path.clone(),
                        source_sha256: Some(source_sha),
                        status: SyncItemStatus::Skipped,
                        source_bytes: file_bytes.len() as u64,
                        optimized_bytes: Some(optimized_bytes),
                        format: Some(format),
                        quality_score: None,
                        remote_url: storage_url,
                        error: None,
                    });
                }
                SyncPlanAction::ProcessNew { .. } | SyncPlanAction::Reprocess { .. } => {
                    // Step 1: Optimize
                    let opt_res = match self.optimizer.optimize_bytes(&file_bytes, config) {
                        Ok(res) => res,
                        Err(e) => {
                            failed_count += 1;
                            let err_msg = format!("optimization failed: {e}");
                            manifest.record_failure(&path_str, Some(&sha_str), &err_msg);
                            items.push(SyncItemResult {
                                path: discovered.path.clone(),
                                source_sha256: Some(source_sha),
                                status: SyncItemStatus::Failed,
                                source_bytes: file_bytes.len() as u64,
                                optimized_bytes: None,
                                format: None,
                                quality_score: None,
                                remote_url: None,
                                error: Some(err_msg),
                            });
                            continue;
                        }
                    };

                    // Step 2: Upload and verify
                    let object_key =
                        match storage.derive_object_key(&sha_str, opt_res.effective_format) {
                            Ok(k) => k,
                            Err(e) => {
                                failed_count += 1;
                                let err_msg = format!("failed to derive object key: {e}");
                                manifest.record_failure(&path_str, Some(&sha_str), &err_msg);
                                items.push(SyncItemResult {
                                    path: discovered.path.clone(),
                                    source_sha256: Some(source_sha),
                                    status: SyncItemStatus::Failed,
                                    source_bytes: file_bytes.len() as u64,
                                    optimized_bytes: None,
                                    format: Some(opt_res.effective_format),
                                    quality_score: Some(opt_res.quality_score),
                                    remote_url: None,
                                    error: Some(err_msg),
                                });
                                continue;
                            }
                        };

                    let content_type = match opt_res.effective_format {
                        ImageFormat::Jpeg => "image/jpeg",
                        ImageFormat::Png => "image/png",
                        ImageFormat::Webp => "image/webp",
                    };

                    let opt_sha = hash_bytes(&opt_res.validated_output.bytes).to_hex();
                    let storage_out = match storage.upload_and_verify(
                        &self.bucket,
                        &object_key,
                        &opt_res.validated_output.bytes,
                        content_type,
                        Some(&opt_sha),
                    ) {
                        Ok(out) => out,
                        Err(e) => {
                            failed_count += 1;
                            let err_msg = format!("storage upload failed: {e}");
                            manifest.record_failure(&path_str, Some(&sha_str), &err_msg);
                            items.push(SyncItemResult {
                                path: discovered.path.clone(),
                                source_sha256: Some(source_sha),
                                status: SyncItemStatus::Failed,
                                source_bytes: file_bytes.len() as u64,
                                optimized_bytes: Some(opt_res.validated_output.byte_size),
                                format: Some(opt_res.effective_format),
                                quality_score: Some(opt_res.quality_score),
                                remote_url: None,
                                error: Some(err_msg),
                            });
                            continue;
                        }
                    };

                    // Step 3: Record success in manifest
                    manifest.record_success(&path_str, &opt_res, &storage_out);

                    optimized_count += 1;
                    total_optimized += opt_res.validated_output.byte_size;

                    items.push(SyncItemResult {
                        path: discovered.path.clone(),
                        source_sha256: Some(source_sha),
                        status: SyncItemStatus::Optimized,
                        source_bytes: file_bytes.len() as u64,
                        optimized_bytes: Some(opt_res.validated_output.byte_size),
                        format: Some(opt_res.effective_format),
                        quality_score: Some(opt_res.quality_score),
                        remote_url: storage_out.public_url,
                        error: None,
                    });
                }
            }
        }

        // Commit manifest atomically after all batch items have settled
        manifest.commit()?;

        let saved_bytes = (total_original as i64) - (total_optimized as i64);
        let saved_ratio = if total_original > 0 && saved_bytes > 0 {
            saved_bytes as f64 / total_original as f64
        } else {
            0.0
        };

        Ok(SyncRunResult {
            total_files: scan_res.files.len(),
            optimized_files: optimized_count,
            skipped_files: skipped_count,
            failed_files: failed_count,
            original_bytes: total_original,
            optimized_bytes: total_optimized,
            saved_bytes,
            saved_ratio,
            elapsed_millis: start_time.elapsed().as_millis() as u64,
            items,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OptimizationPreset;
    use imgopt_codecs::ImageCodec;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tempfile::TempDir;

    #[derive(Default)]
    struct MockSyncStorage {
        uploaded: Mutex<HashMap<String, Vec<u8>>>,
        public_url_base: Option<String>,
    }

    impl SyncStorage for MockSyncStorage {
        fn upload_and_verify(
            &self,
            bucket: &str,
            object_key: &str,
            body: &[u8],
            _content_type: &str,
            _content_sha256: Option<&str>,
        ) -> Result<SyncStorageOutput> {
            let mut guard = self.uploaded.lock().unwrap();
            guard.insert(object_key.to_string(), body.to_vec());

            let public_url = self
                .public_url_base
                .as_ref()
                .map(|base| format!("{base}/{object_key}"));

            Ok(SyncStorageOutput {
                bucket: bucket.to_string(),
                object_key: object_key.to_string(),
                byte_size: body.len() as u64,
                etag: Some("etag".into()),
                public_url,
                uploaded_at: "2026-08-17T00:00:00Z".into(),
            })
        }

        fn derive_object_key(&self, source_sha256: &str, format: ImageFormat) -> Result<String> {
            Ok(format!(
                "images/{source_sha256}/optimized.{}",
                format.extension()
            ))
        }

        fn build_public_url(&self, object_key: &str) -> Option<String> {
            self.public_url_base
                .as_ref()
                .map(|b| format!("{b}/{object_key}"))
        }
    }

    #[derive(Default)]
    struct MockSyncManifest {
        records: HashMap<String, (String, u64, ImageFormat, Option<String>)>,
    }

    impl SyncManifest for MockSyncManifest {
        fn evaluate(&self, source_sha256: &str, current_opt_key: &str) -> SyncPlanAction {
            if let Some((key, size, fmt, url)) = self.records.get(source_sha256) {
                if key == current_opt_key {
                    return SyncPlanAction::SkipUpToDate {
                        source_sha256: source_sha256.to_string(),
                        storage_url: url.clone(),
                        optimized_bytes: *size,
                        format: *fmt,
                    };
                } else {
                    return SyncPlanAction::Reprocess {
                        source_sha256: source_sha256.to_string(),
                        reason: "key mismatch".into(),
                    };
                }
            }
            SyncPlanAction::ProcessNew {
                source_sha256: source_sha256.to_string(),
            }
        }

        fn record_success(
            &mut self,
            _source_path: &str,
            opt_res: &OptimizationResult,
            storage_out: &SyncStorageOutput,
        ) {
            self.records.insert(
                opt_res.source_sha256.to_hex(),
                (
                    opt_res.optimization_key.to_string(),
                    opt_res.validated_output.byte_size,
                    opt_res.effective_format,
                    storage_out.public_url.clone(),
                ),
            );
        }

        fn record_failure(
            &mut self,
            _source_path: &str,
            _source_sha256: Option<&str>,
            _error: &str,
        ) {
        }

        fn commit(&self) -> Result<()> {
            Ok(())
        }
    }

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
    fn test_sync_pipeline_first_run_then_skip() {
        let temp = TempDir::new().unwrap();
        let img_path = temp.path().join("photo.png");
        make_test_png(&img_path);

        let service = SyncService::new("0.1.0", BatchOptions::default(), "my-bucket");
        let config = OptimizationConfig::from_preset(OptimizationPreset::Balanced);
        let storage = MockSyncStorage {
            public_url_base: Some("https://cdn.example.com".into()),
            ..Default::default()
        };
        let mut manifest = MockSyncManifest::default();

        // First run: optimizes and uploads
        let run1 = service
            .run_sync(temp.path(), &config, &storage, &mut manifest)
            .unwrap();
        assert_eq!(run1.total_files, 1);
        assert_eq!(run1.optimized_files, 1);
        assert_eq!(run1.skipped_files, 0);
        assert_eq!(run1.failed_files, 0);

        // Second run: should be skipped
        let run2 = service
            .run_sync(temp.path(), &config, &storage, &mut manifest)
            .unwrap();
        assert_eq!(run2.total_files, 1);
        assert_eq!(run2.optimized_files, 0);
        assert_eq!(run2.skipped_files, 1);
        assert_eq!(run2.failed_files, 0);
    }
}
