use imgopt_core::{
    ImgoptError, OptimizationResult, PipelineStage, Result as CoreResult, SyncManifest,
    SyncPlanAction, SyncStorageOutput,
};

use crate::dedup::{evaluate_action, ExecutionPlanAction};
use crate::model::{
    EntryStatus, Manifest, ManifestEntry, ManifestOptimizationRecord, ManifestSourceRecord,
    ManifestStorageRecord,
};
use crate::store::ManifestStore;

/// Adapter connecting ManifestStore and durable Manifest to the SyncManifest pipeline contract.
pub struct ManifestSyncAdapter {
    pub store: ManifestStore,
    pub manifest: Manifest,
}

impl ManifestSyncAdapter {
    pub fn new(store: ManifestStore, optimizer_version: &str) -> CoreResult<Self> {
        let manifest = store.load_or_init(optimizer_version)?;
        Ok(Self { store, manifest })
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

    fn record_success(
        &mut self,
        source_path: &str,
        opt_res: &OptimizationResult,
        storage_out: &SyncStorageOutput,
    ) {
        let entry = ManifestEntry {
            source: ManifestSourceRecord {
                sha256: opt_res.source_sha256.to_hex(),
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

    fn record_failure(&mut self, _source_path: &str, _source_sha256: Option<&str>, _error: &str) {}

    fn commit(&self) -> CoreResult<()> {
        self.store.save(&self.manifest).map_err(|e| {
            ImgoptError::new(
                PipelineStage::ManifestCommit,
                imgopt_core::ErrorKind::Io(e.to_string()),
            )
        })
    }
}
