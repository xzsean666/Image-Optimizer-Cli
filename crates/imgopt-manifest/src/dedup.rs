use imgopt_core::ImageFormat;
use serde::{Deserialize, Serialize};

use crate::model::{EntryStatus, Manifest, ManifestOptimizationRecord};

/// Planned action for a source file based on manifest state and current optimization parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExecutionPlanAction {
    /// Fully optimized and verified in storage with identical optimization key. Can be safely skipped.
    SkipUpToDate {
        source_sha256: String,
        storage_url: Option<String>,
        optimized_bytes: u64,
        format: ImageFormat,
    },
    /// Optimization matches current key, but remote storage upload is missing or unverified.
    ResumeUpload {
        source_sha256: String,
        optimization: ManifestOptimizationRecord,
    },
    /// Completely new asset not present in manifest. Full pipeline required.
    ProcessNew { source_sha256: String },
    /// Present in manifest, but needs re-optimization (key mismatch, config change, or prior failure).
    Reprocess {
        source_sha256: String,
        reason: String,
    },
}

/// Pure decision function determining whether to skip, resume, or process an asset.
pub fn evaluate_action(
    source_sha256: &str,
    current_opt_key: &str,
    manifest: &Manifest,
) -> ExecutionPlanAction {
    let entry = match manifest.get(source_sha256) {
        Some(e) => e,
        None => {
            return ExecutionPlanAction::ProcessNew {
                source_sha256: source_sha256.to_string(),
            }
        }
    };

    match entry.status {
        EntryStatus::Failed => ExecutionPlanAction::Reprocess {
            source_sha256: source_sha256.to_string(),
            reason: "prior optimization or upload attempt failed".to_string(),
        },
        EntryStatus::Pending => ExecutionPlanAction::Reprocess {
            source_sha256: source_sha256.to_string(),
            reason: "prior execution was interrupted during pending state".to_string(),
        },
        EntryStatus::Optimized => {
            if let Some(ref opt) = entry.optimization {
                if opt.optimization_key == current_opt_key {
                    ExecutionPlanAction::ResumeUpload {
                        source_sha256: entry.source.sha256.clone(),
                        optimization: opt.clone(),
                    }
                } else {
                    ExecutionPlanAction::Reprocess {
                        source_sha256: source_sha256.to_string(),
                        reason: "optimization key changed".to_string(),
                    }
                }
            } else {
                ExecutionPlanAction::Reprocess {
                    source_sha256: source_sha256.to_string(),
                    reason: "missing optimization details".to_string(),
                }
            }
        }
        EntryStatus::Uploaded | EntryStatus::Verified | EntryStatus::Skipped => {
            if let Some(ref opt) = entry.optimization {
                if opt.optimization_key == current_opt_key {
                    // If storage is present and verified (or uploaded), we can skip
                    if let Some(ref st) = entry.storage {
                        if st.verified
                            || entry.status == EntryStatus::Uploaded
                            || entry.status == EntryStatus::Verified
                        {
                            return ExecutionPlanAction::SkipUpToDate {
                                source_sha256: entry.source.sha256.clone(),
                                storage_url: st.public_url.clone(),
                                optimized_bytes: opt.output_size,
                                format: opt.output_format,
                            };
                        }
                    }
                    // Storage record missing or not complete -> resume upload
                    ExecutionPlanAction::ResumeUpload {
                        source_sha256: entry.source.sha256.clone(),
                        optimization: opt.clone(),
                    }
                } else {
                    ExecutionPlanAction::Reprocess {
                        source_sha256: source_sha256.to_string(),
                        reason:
                            "optimization key changed (new preset, format policy, or parameters)"
                                .to_string(),
                    }
                }
            } else {
                ExecutionPlanAction::Reprocess {
                    source_sha256: source_sha256.to_string(),
                    reason: "missing optimization record for uploaded asset".to_string(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        ManifestEntry, ManifestOptimizationRecord, ManifestSourceRecord, ManifestStorageRecord,
    };
    use imgopt_core::{Dimensions, ImageFormat};

    fn make_test_manifest() -> Manifest {
        let mut manifest = Manifest::new("0.1.0");
        let entry = ManifestEntry {
            source: ManifestSourceRecord {
                sha256: "abc123hash".into(),
                format: ImageFormat::Png,
                byte_size: 1000,
                dimensions: Dimensions::new(100, 100),
            },
            optimization: Some(ManifestOptimizationRecord {
                optimization_key: "opt_key_v1_balanced".into(),
                output_format: ImageFormat::Webp,
                output_size: 500,
                quality_score: Some(0.95),
                preset: "balanced".into(),
                completed_at: "2026-08-17T00:00:00Z".into(),
            }),
            storage: Some(ManifestStorageRecord {
                bucket: "cdn-bucket".into(),
                object_key: "assets/abc123hash.webp".into(),
                etag: Some("etag_val".into()),
                public_url: Some("https://cdn.example.com/assets/abc123hash.webp".into()),
                uploaded_at: "2026-08-17T00:00:00Z".into(),
                verified: true,
            }),
            status: EntryStatus::Uploaded,
            known_paths: vec!["images/banner.png".into()],
        };
        manifest.insert(entry);
        manifest
    }

    #[test]
    fn test_new_source_processes_new() {
        let manifest = make_test_manifest();
        let action = evaluate_action("unknown_sha", "opt_key_v1_balanced", &manifest);
        assert_eq!(
            action,
            ExecutionPlanAction::ProcessNew {
                source_sha256: "unknown_sha".into()
            }
        );
    }

    #[test]
    fn test_matching_key_and_uploaded_skips() {
        let manifest = make_test_manifest();
        let action = evaluate_action("abc123hash", "opt_key_v1_balanced", &manifest);
        assert_eq!(
            action,
            ExecutionPlanAction::SkipUpToDate {
                source_sha256: "abc123hash".into(),
                storage_url: Some("https://cdn.example.com/assets/abc123hash.webp".into()),
                optimized_bytes: 500,
                format: ImageFormat::Webp,
            }
        );
    }

    #[test]
    fn test_key_mismatch_reprocesses() {
        let manifest = make_test_manifest();
        let action = evaluate_action("abc123hash", "opt_key_v2_maximum", &manifest);
        assert!(matches!(action, ExecutionPlanAction::Reprocess { .. }));
    }

    #[test]
    fn test_optimized_but_not_uploaded_resumes_upload() {
        let mut manifest = make_test_manifest();
        manifest.entries.get_mut("abc123hash").unwrap().status = EntryStatus::Optimized;

        let action = evaluate_action("abc123hash", "opt_key_v1_balanced", &manifest);
        assert!(matches!(action, ExecutionPlanAction::ResumeUpload { .. }));
    }

    #[test]
    fn test_prior_failed_status_reprocesses() {
        let mut manifest = make_test_manifest();
        manifest.entries.get_mut("abc123hash").unwrap().status = EntryStatus::Failed;

        let action = evaluate_action("abc123hash", "opt_key_v1_balanced", &manifest);
        assert!(matches!(action, ExecutionPlanAction::Reprocess { .. }));
    }
}
