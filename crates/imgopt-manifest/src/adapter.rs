use imgopt_core::{
    ImgoptError, OptimizationResult, PipelineStage, Result as CoreResult, SyncManifest,
    SyncPlanAction, SyncStorageOutput,
};

use crate::dedup::{evaluate_action, ExecutionPlanAction};
use crate::model::{
    EntryStatus, Manifest, ManifestEntry, ManifestOptimizationRecord, ManifestSourceRecord,
    ManifestStorageRecord,
};
use crate::store::{ManifestLock, ManifestStore};

/// Adapter connecting ManifestStore and durable Manifest to the SyncManifest pipeline contract.
/// Holds an exclusive advisory lock for the lifetime of the sync run.
pub struct ManifestSyncAdapter {
    pub store: ManifestStore,
    pub manifest: Manifest,
    _lock: ManifestLock,
}

impl ManifestSyncAdapter {
    pub fn new(store: ManifestStore, optimizer_version: &str) -> CoreResult<Self> {
        let lock = store.acquire_lock()?;
        let manifest = store.load_or_init(optimizer_version)?;
        Ok(Self {
            store,
            manifest,
            _lock: lock,
        })
    }
}

impl SyncManifest for ManifestSyncAdapter {
    fn evaluate(&self, source_sha256: &str, current_opt_key: &str) -> SyncPlanAction {
        match evaluate_action(source_sha256, current_opt_key, &self.manifest) {
            ExecutionPlanAction::SkipUpToDate {
                source_sha256,
                storage_url,
                optimized_bytes,
                format,
            } => SyncPlanAction::SkipUpToDate {
                source_sha256,
                storage_url,
                optimized_bytes,
                format,
            },
            ExecutionPlanAction::ResumeUpload { source_sha256, .. } => SyncPlanAction::Reprocess {
                source_sha256,
                reason: "resume incomplete upload".into(),
            },
            ExecutionPlanAction::ProcessNew { source_sha256 } => {
                SyncPlanAction::ProcessNew { source_sha256 }
            }
            ExecutionPlanAction::Reprocess {
                source_sha256,
                reason,
            } => SyncPlanAction::Reprocess {
                source_sha256,
                reason,
            },
        }
    }

    fn evaluate_for_target(
        &self,
        source_sha256: &str,
        current_opt_key: &str,
        bucket: &str,
        expected_object_key: Option<&str>,
    ) -> SyncPlanAction {
        let action = self.evaluate(source_sha256, current_opt_key);
        if matches!(action, SyncPlanAction::SkipUpToDate { .. }) {
            if let Some(st) = self
                .manifest
                .get(source_sha256)
                .and_then(|e| e.storage.as_ref())
            {
                if st.bucket != bucket {
                    return SyncPlanAction::Reprocess {
                        source_sha256: source_sha256.to_string(),
                        reason: "target storage bucket changed".into(),
                    };
                }
                if let Some(exp_key) = expected_object_key {
                    if st.object_key != exp_key {
                        return SyncPlanAction::Reprocess {
                            source_sha256: source_sha256.to_string(),
                            reason: "target object key prefix changed".into(),
                        };
                    }
                }
            }
        }
        action
    }

    fn record_success(
        &mut self,
        source_path: &str,
        opt_res: &OptimizationResult,
        storage_out: &SyncStorageOutput,
    ) {
        let sha_hex = opt_res.source_sha256.to_hex();
        let entry = ManifestEntry {
            source: ManifestSourceRecord {
                sha256: sha_hex,
                format: opt_res.profile.format,
                byte_size: opt_res.profile.byte_size,
                dimensions: opt_res.profile.dimensions,
            },
            optimization: Some(ManifestOptimizationRecord {
                optimization_key: opt_res.optimization_key.to_string(),
                output_format: opt_res.effective_format,
                output_size: opt_res.validated_output.byte_size,
                quality_score: Some(opt_res.quality_score),
                preset: opt_res.preset.as_str().to_string(),
                completed_at: storage_out.uploaded_at.clone(),
            }),
            storage: Some(ManifestStorageRecord {
                bucket: storage_out.bucket.clone(),
                object_key: storage_out.object_key.clone(),
                etag: storage_out.etag.clone(),
                public_url: storage_out.public_url.clone(),
                uploaded_at: storage_out.uploaded_at.clone(),
                verified: true,
            }),
            status: EntryStatus::Uploaded,
            known_paths: vec![source_path.to_string()],
        };

        self.manifest.insert(entry);
    }

    fn record_failure(&mut self, source_path: &str, source_sha256: Option<&str>, _error: &str) {
        if let Some(sha) = source_sha256 {
            if let Some(existing) = self.manifest.get_mut(sha) {
                if !existing.known_paths.iter().any(|p| p == source_path) {
                    existing.known_paths.push(source_path.to_string());
                }
                if existing.status != EntryStatus::Uploaded
                    && existing.status != EntryStatus::Verified
                {
                    existing.status = EntryStatus::Failed;
                }
                self.manifest.touch();
            }
        }
    }

    fn record_path_alias(&mut self, source_sha256: &str, source_path: &str) {
        self.manifest.add_path_alias(source_sha256, source_path);
    }

    fn commit(&self) -> CoreResult<()> {
        self.store.save(&self.manifest).map_err(|e| {
            ImgoptError::new(
                PipelineStage::ManifestCommit,
                imgopt_core::ErrorKind::Io(e.to_string()),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imgopt_core::{Dimensions, ImageFormat};
    use tempfile::TempDir;

    #[test]
    fn test_evaluate_for_target_detects_bucket_and_prefix_changes() {
        let temp = TempDir::new().unwrap();
        let store = ManifestStore::new(temp.path().join("manifest.json"));
        let mut adapter = ManifestSyncAdapter::new(store, "0.1.0").unwrap();

        let sha = "a".repeat(64);
        adapter.manifest.insert(ManifestEntry {
            source: ManifestSourceRecord {
                sha256: sha.clone(),
                format: ImageFormat::Png,
                byte_size: 100,
                dimensions: Dimensions::new(8, 8),
            },
            optimization: Some(ManifestOptimizationRecord {
                optimization_key: "key-1".into(),
                output_format: ImageFormat::Webp,
                output_size: 50,
                quality_score: Some(0.95),
                preset: "balanced".into(),
                completed_at: "2026-11-25T00:00:00Z".into(),
            }),
            storage: Some(ManifestStorageRecord {
                bucket: "bucket-a".into(),
                object_key: format!("v1/images/{sha}/optimized.webp"),
                etag: Some("etag".into()),
                public_url: None,
                uploaded_at: "2026-11-25T00:00:00Z".into(),
                verified: true,
            }),
            status: EntryStatus::Uploaded,
            known_paths: vec!["a.png".into()],
        });

        let same_target = adapter.evaluate_for_target(
            &sha,
            "key-1",
            "bucket-a",
            Some(&format!("v1/images/{sha}/optimized.webp")),
        );
        assert!(matches!(same_target, SyncPlanAction::SkipUpToDate { .. }));

        let diff_bucket = adapter.evaluate_for_target(
            &sha,
            "key-1",
            "bucket-b",
            Some(&format!("v1/images/{sha}/optimized.webp")),
        );
        assert!(matches!(diff_bucket, SyncPlanAction::Reprocess { .. }));

        let diff_prefix = adapter.evaluate_for_target(
            &sha,
            "key-1",
            "bucket-a",
            Some(&format!("v2/images/{sha}/optimized.webp")),
        );
        assert!(matches!(diff_prefix, SyncPlanAction::Reprocess { .. }));
    }
}
