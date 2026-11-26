use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{OptimizationConfig, OutputFormatPolicy};
use crate::detect::{detect_format, MAX_HEADER_PROBE_BYTES};
use crate::error::Result;
use crate::executor::{execute_batch_scoped, BatchOptions, BatchTask};
use crate::hash::{hash_bytes, Sha256Digest, HASH_BUFFER_SIZE};
use crate::model::{compute_byte_savings, ImageFormat};
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
    fn evaluate_for_target(
        &self,
        source_sha256: &str,
        current_opt_key: &str,
        _bucket: &str,
        _expected_object_key: Option<&str>,
    ) -> SyncPlanAction {
        self.evaluate(source_sha256, current_opt_key)
    }
    fn record_success(
        &mut self,
        source_path: &str,
        opt_res: &OptimizationResult,
        storage_out: &SyncStorageOutput,
    );
    fn record_failure(&mut self, source_path: &str, source_sha256: Option<&str>, error: &str);
    fn record_path_alias(&mut self, _source_sha256: &str, _source_path: &str) {}
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

struct PendingAssetWork {
    item_index: usize,
    source_sha: Sha256Digest,
    sha_hex: String,
    byte_size: u64,
}

struct IntraBatchDup {
    item_index: usize,
    path: PathBuf,
    source_sha: Sha256Digest,
    sha_hex: String,
    byte_size: u64,
}

enum WorkerOutcome {
    Succeeded {
        item_index: usize,
        source_sha: Sha256Digest,
        byte_size: u64,
        opt_res: Box<OptimizationResult>,
        storage_out: SyncStorageOutput,
    },
    Failed {
        item_index: usize,
        source_sha: Sha256Digest,
        sha_hex: String,
        byte_size: u64,
        error: String,
    },
}

struct PreflightSuccess {
    byte_size: u64,
    source_sha: Sha256Digest,
    format: ImageFormat,
}

struct PreflightError {
    byte_size: u64,
    source_sha: Option<Sha256Digest>,
    error: String,
}

type PrimaryOutcomeMap =
    HashMap<String, std::result::Result<(u64, ImageFormat, Option<String>), String>>;

/// Single-open bounded streaming preflight: reads file size, hashes payload, and detects format.
fn preflight_hash_and_detect(
    path: &Path,
    max_input_bytes: u64,
) -> std::result::Result<PreflightSuccess, PreflightError> {
    let mut file = File::open(path).map_err(|e| PreflightError {
        byte_size: 0,
        source_sha: None,
        error: format!("failed to open file: {e}"),
    })?;
    let file_len = file
        .metadata()
        .map_err(|e| PreflightError {
            byte_size: 0,
            source_sha: None,
            error: format!("failed to read file metadata: {e}"),
        })?
        .len();

    if file_len > max_input_bytes {
        return Err(PreflightError {
            byte_size: file_len,
            source_sha: None,
            error: format!(
                "input size {} bytes exceeds configured limit of {} bytes",
                file_len, max_input_bytes
            ),
        });
    }

    let mut hasher = Sha256::new();
    let mut header = [0u8; MAX_HEADER_PROBE_BYTES];
    let mut header_len = 0usize;
    let mut buf = [0u8; HASH_BUFFER_SIZE];
    let mut total_read = 0u64;

    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                total_read = total_read.saturating_add(n as u64);
                if total_read > max_input_bytes {
                    return Err(PreflightError {
                        byte_size: total_read,
                        source_sha: None,
                        error: format!(
                            "input size {} bytes exceeds configured limit of {} bytes",
                            total_read, max_input_bytes
                        ),
                    });
                }
                if header_len < MAX_HEADER_PROBE_BYTES {
                    let take = (MAX_HEADER_PROBE_BYTES - header_len).min(n);
                    header[header_len..header_len + take].copy_from_slice(&buf[..take]);
                    header_len += take;
                }
                hasher.update(&buf[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(PreflightError {
                    byte_size: file_len,
                    source_sha: None,
                    error: format!("failed to read file during hashing: {e}"),
                })
            }
        }
    }

    let mut digest_bytes = [0u8; 32];
    digest_bytes.copy_from_slice(&hasher.finalize());
    let source_sha = Sha256Digest::from_bytes(digest_bytes);

    let format = detect_format(&header[..header_len]).map_err(|e| PreflightError {
        byte_size: total_read,
        source_sha: Some(source_sha),
        error: format!("format detection failed: {e}"),
    })?;

    Ok(PreflightSuccess {
        byte_size: total_read,
        source_sha,
        format,
    })
}

/// Orchestrates recursive scanning, streaming hashing, deduplication, bounded parallel optimization,
/// upload verification, and periodic atomic manifest commit.
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
        self.run_sync_with_options(
            input_path,
            config,
            &ScanOptions::default(),
            storage,
            manifest,
        )
    }

    pub fn run_sync_with_options<S: SyncStorage, M: SyncManifest>(
        &self,
        input_path: &Path,
        config: &OptimizationConfig,
        scan_options: &ScanOptions,
        storage: &S,
        manifest: &mut M,
    ) -> Result<SyncRunResult> {
        let start_time = Instant::now();
        config.validate()?;

        let scan_res = scan_path(input_path, scan_options)?;
        if scan_res.files.is_empty() && scan_res.errors.is_empty() {
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

        let discovered_count = scan_res.files.len();
        let mut items: Vec<Option<SyncItemResult>> = vec![None; discovered_count];
        let mut total_original = 0u64;
        let mut total_optimized = 0u64;
        let mut optimized_count = 0usize;
        let mut skipped_count = 0usize;
        let mut failed_count = 0usize;

        let mut scan_failed_items = Vec::with_capacity(scan_res.errors.len());
        for scan_err in &scan_res.errors {
            let err_path = scan_err
                .source_path
                .clone()
                .unwrap_or_else(|| input_path.to_path_buf());
            let err_msg = scan_err.to_string();
            manifest.record_failure(&err_path.to_string_lossy(), None, &err_msg);
            failed_count += 1;
            scan_failed_items.push(SyncItemResult {
                path: err_path,
                source_sha256: None,
                status: SyncItemStatus::Failed,
                source_bytes: 0,
                optimized_bytes: None,
                format: None,
                quality_score: None,
                remote_url: None,
                error: Some(err_msg),
            });
        }

        let mut pending_tasks: Vec<BatchTask<PendingAssetWork>> = Vec::new();
        let mut scheduled_shas: HashMap<String, usize> = HashMap::new();
        let mut intra_batch_dups: Vec<IntraBatchDup> = Vec::new();

        // Phase 1: Single-open preflight size check, streaming SHA-256 hashing, format detection,
        // and target-aware deduplication planning. Skipped files never load image payloads into memory.
        for (idx, discovered) in scan_res.files.iter().enumerate() {
            let path_str = discovered.path.to_string_lossy().to_string();

            let PreflightSuccess {
                byte_size: file_len,
                source_sha,
                format: detected_fmt,
            } = match preflight_hash_and_detect(&discovered.path, config.max_input_bytes) {
                Ok(ok) => ok,
                Err(PreflightError {
                    byte_size: size,
                    source_sha: sha_opt,
                    error: err_msg,
                }) => {
                    total_original += size;
                    failed_count += 1;
                    let sha_hex_opt = sha_opt.as_ref().map(Sha256Digest::to_hex);
                    manifest.record_failure(&path_str, sha_hex_opt.as_deref(), &err_msg);
                    items[idx] = Some(SyncItemResult {
                        path: discovered.path.clone(),
                        source_sha256: sha_opt,
                        status: SyncItemStatus::Failed,
                        source_bytes: if size > 0 { size } else { discovered.byte_size },
                        optimized_bytes: None,
                        format: None,
                        quality_score: None,
                        remote_url: None,
                        error: Some(err_msg),
                    });
                    continue;
                }
            };

            total_original += file_len;
            let sha_str = source_sha.to_hex();

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
            let expected_obj_key = storage.derive_object_key(&sha_str, expected_format).ok();

            let mut action = manifest.evaluate_for_target(
                &sha_str,
                opt_key.as_str(),
                &self.bucket,
                expected_obj_key.as_deref(),
            );
            let fallback_candidates: &[ImageFormat] = match config.output_format_policy {
                OutputFormatPolicy::AllowWebp => &[ImageFormat::Webp],
                OutputFormatPolicy::AllowAvif => &[ImageFormat::Avif],
                OutputFormatPolicy::Auto => &[ImageFormat::Webp, ImageFormat::Avif],
                _ => &[],
            };
            for &alt_fmt in fallback_candidates {
                if alt_fmt == expected_format
                    || matches!(action, SyncPlanAction::SkipUpToDate { .. })
                {
                    continue;
                }
                let alt_opt_key = OptimizationKey::compute(
                    &source_sha,
                    self.optimizer.optimizer_version(),
                    alt_fmt,
                    config,
                );
                let alt_obj_key = storage.derive_object_key(&sha_str, alt_fmt).ok();
                let alt_action = manifest.evaluate_for_target(
                    &sha_str,
                    alt_opt_key.as_str(),
                    &self.bucket,
                    alt_obj_key.as_deref(),
                );
                if matches!(alt_action, SyncPlanAction::SkipUpToDate { .. }) {
                    action = alt_action;
                    break;
                }
            }

            match action {
                SyncPlanAction::SkipUpToDate {
                    storage_url,
                    optimized_bytes,
                    format,
                    ..
                } => {
                    manifest.record_path_alias(&sha_str, &path_str);
                    skipped_count += 1;
                    total_optimized += optimized_bytes;
                    let resolved_url = storage
                        .derive_object_key(&sha_str, format)
                        .ok()
                        .and_then(|k| storage.build_public_url(&k))
                        .or(storage_url);
                    items[idx] = Some(SyncItemResult {
                        path: discovered.path.clone(),
                        source_sha256: Some(source_sha),
                        status: SyncItemStatus::Skipped,
                        source_bytes: file_len,
                        optimized_bytes: Some(optimized_bytes),
                        format: Some(format),
                        quality_score: None,
                        remote_url: resolved_url,
                        error: None,
                    });
                }
                SyncPlanAction::ProcessNew { .. } | SyncPlanAction::Reprocess { .. } => {
                    if scheduled_shas.contains_key(&sha_str) {
                        intra_batch_dups.push(IntraBatchDup {
                            item_index: idx,
                            path: discovered.path.clone(),
                            source_sha,
                            sha_hex: sha_str,
                            byte_size: file_len,
                        });
                    } else {
                        scheduled_shas.insert(sha_str.clone(), idx);
                        pending_tasks.push(BatchTask::new(
                            &discovered.path,
                            PendingAssetWork {
                                item_index: idx,
                                source_sha,
                                sha_hex: sha_str,
                                byte_size: file_len,
                            },
                        ));
                    }
                }
            }
        }

        // Phase 2: Bounded concurrent optimization + upload verification in checkpointed waves.
        let wave_size = self
            .batch_options
            .concurrency
            .max(1)
            .saturating_mul(4)
            .max(4);
        let mut primary_outcomes: PrimaryOutcomeMap = HashMap::new();

        let mut remaining = pending_tasks;
        while !remaining.is_empty() {
            let split_at = remaining.len().min(wave_size);
            let wave: Vec<_> = remaining.drain(..split_at).collect();

            let (wave_results, _summary) =
                execute_batch_scoped(wave, &self.batch_options, |path, work| {
                    let item_index = work.item_index;
                    let source_sha = work.source_sha;
                    let sha_hex = work.sha_hex;
                    let byte_size = work.byte_size;

                    let work_res: Result<(OptimizationResult, SyncStorageOutput)> = (|| {
                        let opt_res = self.optimizer.optimize_file(&path, config)?;
                        let object_key =
                            storage.derive_object_key(&sha_hex, opt_res.effective_format)?;
                        let content_type = opt_res.effective_format.mime_type();
                        let opt_sha = hash_bytes(&opt_res.validated_output.bytes).to_hex();
                        let storage_out = storage.upload_and_verify(
                            &self.bucket,
                            &object_key,
                            &opt_res.validated_output.bytes,
                            content_type,
                            Some(&opt_sha),
                        )?;
                        Ok((opt_res, storage_out))
                    })(
                    );

                    Ok(match work_res {
                        Ok((opt_res, storage_out)) => WorkerOutcome::Succeeded {
                            item_index,
                            source_sha,
                            byte_size,
                            opt_res: Box::new(opt_res),
                            storage_out,
                        },
                        Err(e) => WorkerOutcome::Failed {
                            item_index,
                            source_sha,
                            sha_hex,
                            byte_size,
                            error: e.to_string(),
                        },
                    })
                });

            for res in wave_results {
                let path_str = res.source_path.to_string_lossy().to_string();
                match res.output {
                    Ok(WorkerOutcome::Succeeded {
                        item_index,
                        source_sha,
                        byte_size,
                        opt_res,
                        storage_out,
                    }) => {
                        let sha_hex = source_sha.to_hex();
                        manifest.record_success(&path_str, &opt_res, &storage_out);
                        optimized_count += 1;
                        total_optimized += opt_res.validated_output.byte_size;
                        primary_outcomes.insert(
                            sha_hex,
                            Ok((
                                opt_res.validated_output.byte_size,
                                opt_res.effective_format,
                                storage_out.public_url.clone(),
                            )),
                        );
                        items[item_index] = Some(SyncItemResult {
                            path: res.source_path,
                            source_sha256: Some(source_sha),
                            status: SyncItemStatus::Optimized,
                            source_bytes: byte_size,
                            optimized_bytes: Some(opt_res.validated_output.byte_size),
                            format: Some(opt_res.effective_format),
                            quality_score: Some(opt_res.quality_score),
                            remote_url: storage_out.public_url,
                            error: None,
                        });
                    }
                    Ok(WorkerOutcome::Failed {
                        item_index,
                        source_sha,
                        sha_hex,
                        byte_size,
                        error,
                    }) => {
                        manifest.record_failure(&path_str, Some(&sha_hex), &error);
                        failed_count += 1;
                        primary_outcomes.insert(sha_hex, Err(error.clone()));
                        items[item_index] = Some(SyncItemResult {
                            path: res.source_path,
                            source_sha256: Some(source_sha),
                            status: SyncItemStatus::Failed,
                            source_bytes: byte_size,
                            optimized_bytes: None,
                            format: None,
                            quality_score: None,
                            remote_url: None,
                            error: Some(error),
                        });
                    }
                    Err(e) => {
                        let err_msg = e.to_string();
                        manifest.record_failure(&path_str, None, &err_msg);
                        failed_count += 1;
                    }
                }
            }

            // Persist checkpoint after each wave so interrupted large syncs resume safely
            manifest.commit()?;
        }

        // Phase 3: Resolve intra-batch identical content duplicates
        for dup in intra_batch_dups {
            let path_str = dup.path.to_string_lossy().to_string();
            match primary_outcomes.get(&dup.sha_hex) {
                Some(Ok((opt_size, fmt, remote_url))) => {
                    manifest.record_path_alias(&dup.sha_hex, &path_str);
                    skipped_count += 1;
                    total_optimized += *opt_size;
                    items[dup.item_index] = Some(SyncItemResult {
                        path: dup.path,
                        source_sha256: Some(dup.source_sha),
                        status: SyncItemStatus::Skipped,
                        source_bytes: dup.byte_size,
                        optimized_bytes: Some(*opt_size),
                        format: Some(*fmt),
                        quality_score: None,
                        remote_url: remote_url.clone(),
                        error: None,
                    });
                }
                Some(Err(err_msg)) => {
                    manifest.record_failure(&path_str, Some(&dup.sha_hex), err_msg);
                    failed_count += 1;
                    items[dup.item_index] = Some(SyncItemResult {
                        path: dup.path,
                        source_sha256: Some(dup.source_sha),
                        status: SyncItemStatus::Failed,
                        source_bytes: dup.byte_size,
                        optimized_bytes: None,
                        format: None,
                        quality_score: None,
                        remote_url: None,
                        error: Some(err_msg.clone()),
                    });
                }
                None => {
                    let err_msg = "internal error: duplicate primary outcome missing".to_string();
                    manifest.record_failure(&path_str, Some(&dup.sha_hex), &err_msg);
                    failed_count += 1;
                    items[dup.item_index] = Some(SyncItemResult {
                        path: dup.path,
                        source_sha256: Some(dup.source_sha),
                        status: SyncItemStatus::Failed,
                        source_bytes: dup.byte_size,
                        optimized_bytes: None,
                        format: None,
                        quality_score: None,
                        remote_url: None,
                        error: Some(err_msg),
                    });
                }
            }
        }

        // Final atomic commit (captures skipped path aliases and final state)
        manifest.commit()?;

        let mut final_items: Vec<SyncItemResult> = items
            .into_iter()
            .enumerate()
            .map(|(idx, slot)| {
                slot.unwrap_or_else(|| SyncItemResult {
                    path: scan_res.files[idx].path.clone(),
                    source_sha256: None,
                    status: SyncItemStatus::Failed,
                    source_bytes: scan_res.files[idx].byte_size,
                    optimized_bytes: None,
                    format: None,
                    quality_score: None,
                    remote_url: None,
                    error: Some("missing task outcome".into()),
                })
            })
            .collect();
        final_items.extend(scan_failed_items);
        let total_files = final_items.len();

        let (saved_bytes, raw_ratio) = compute_byte_savings(total_original, total_optimized);
        let saved_ratio = if saved_bytes > 0 { raw_ratio } else { 0.0 };

        Ok(SyncRunResult {
            total_files,
            optimized_files: optimized_count,
            skipped_files: skipped_count,
            failed_files: failed_count,
            original_bytes: total_original,
            optimized_bytes: total_optimized,
            saved_bytes,
            saved_ratio,
            elapsed_millis: start_time.elapsed().as_millis() as u64,
            items: final_items,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OptimizationPreset;
    use imgopt_codecs::ImageCodec;
    use std::fs;
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
        aliases: HashMap<String, Vec<String>>,
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
            source_path: &str,
            opt_res: &OptimizationResult,
            storage_out: &SyncStorageOutput,
        ) {
            let sha = opt_res.source_sha256.to_hex();
            self.records.insert(
                sha.clone(),
                (
                    opt_res.optimization_key.to_string(),
                    opt_res.validated_output.byte_size,
                    opt_res.effective_format,
                    storage_out.public_url.clone(),
                ),
            );
            self.record_path_alias(&sha, source_path);
        }

        fn record_failure(
            &mut self,
            _source_path: &str,
            _source_sha256: Option<&str>,
            _error: &str,
        ) {
        }

        fn record_path_alias(&mut self, source_sha256: &str, source_path: &str) {
            let list = self.aliases.entry(source_sha256.to_string()).or_default();
            if !list.iter().any(|p| p == source_path) {
                list.push(source_path.to_string());
            }
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

        let run1 = service
            .run_sync(temp.path(), &config, &storage, &mut manifest)
            .unwrap();
        assert_eq!(run1.total_files, 1);
        assert_eq!(run1.optimized_files, 1);
        assert_eq!(run1.skipped_files, 0);
        assert_eq!(run1.failed_files, 0);

        let run2 = service
            .run_sync(temp.path(), &config, &storage, &mut manifest)
            .unwrap();
        assert_eq!(run2.total_files, 1);
        assert_eq!(run2.optimized_files, 0);
        assert_eq!(run2.skipped_files, 1);
        assert_eq!(run2.failed_files, 0);
    }

    #[test]
    fn test_sync_intra_batch_dedup_records_all_aliases_and_uploads_once() {
        let temp = TempDir::new().unwrap();
        let p1 = temp.path().join("a.png");
        let p2 = temp.path().join("b.png");
        make_test_png(&p1);
        make_test_png(&p2);

        let service = SyncService::new("0.1.0", BatchOptions::default(), "my-bucket");
        let config = OptimizationConfig::from_preset(OptimizationPreset::Balanced);
        let storage = MockSyncStorage {
            public_url_base: Some("https://cdn.example.com".into()),
            ..Default::default()
        };
        let mut manifest = MockSyncManifest::default();

        let run = service
            .run_sync(temp.path(), &config, &storage, &mut manifest)
            .unwrap();
        assert_eq!(run.total_files, 2);
        assert_eq!(run.optimized_files, 1);
        assert_eq!(run.skipped_files, 1);
        assert_eq!(storage.uploaded.lock().unwrap().len(), 1);
        assert_eq!(manifest.aliases.values().next().unwrap().len(), 2);
    }
}
