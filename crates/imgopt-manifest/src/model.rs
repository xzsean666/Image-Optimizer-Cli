use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use imgopt_core::{Dimensions, ImageFormat};

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// The status of an asset entry in the lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryStatus {
    Pending,
    Optimized,
    Uploaded,
    Verified,
    Skipped,
    Failed,
}

/// Source asset metadata in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestSourceRecord {
    pub sha256: String,
    pub format: ImageFormat,
    pub byte_size: u64,
    pub dimensions: Dimensions,
}

/// Optimization outcome record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestOptimizationRecord {
    pub optimization_key: String,
    pub output_format: ImageFormat,
    pub output_size: u64,
    pub quality_score: Option<f64>,
    pub preset: String,
    pub completed_at: String,
}

/// Remote storage location and verification record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestStorageRecord {
    pub bucket: String,
    pub object_key: String,
    pub etag: Option<String>,
    pub public_url: Option<String>,
    pub uploaded_at: String,
    pub verified: bool,
}

/// A durable manifest entry representing a single content-addressed asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub source: ManifestSourceRecord,
    pub optimization: Option<ManifestOptimizationRecord>,
    pub storage: Option<ManifestStorageRecord>,
    pub status: EntryStatus,
    /// Known source file paths referencing this identical content hash.
    #[serde(default)]
    pub known_paths: Vec<String>,
}

/// Persistent image optimizer manifest document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub optimizer_version: String,
    pub created_at: String,
    pub updated_at: String,
    pub entries: BTreeMap<String, ManifestEntry>,
}

impl Manifest {
    pub fn new(optimizer_version: impl Into<String>) -> Self {
        let now = "1970-01-01T00:00:00Z".to_string();
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            optimizer_version: optimizer_version.into(),
            created_at: now.clone(),
            updated_at: now,
            entries: BTreeMap::new(),
        }
    }

    pub fn with_timestamps(
        optimizer_version: impl Into<String>,
        created_at: impl Into<String>,
        updated_at: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            optimizer_version: optimizer_version.into(),
            created_at: created_at.into(),
            updated_at: updated_at.into(),
            entries: BTreeMap::new(),
        }
    }

    pub fn get(&self, source_sha256: &str) -> Option<&ManifestEntry> {
        self.entries.get(source_sha256)
    }

    pub fn get_mut(&mut self, source_sha256: &str) -> Option<&mut ManifestEntry> {
        self.entries.get_mut(source_sha256)
    }

    pub fn insert(&mut self, entry: ManifestEntry) {
        self.entries.insert(entry.source.sha256.clone(), entry);
    }

    pub fn add_path_alias(&mut self, source_sha256: &str, path: impl Into<String>) {
        if let Some(entry) = self.entries.get_mut(source_sha256) {
            let p = path.into();
            if !entry.known_paths.contains(&p) {
                entry.known_paths.push(p);
            }
        }
    }

    pub fn is_optimized(&self, source_sha256: &str, optimization_key: &str) -> bool {
        if let Some(entry) = self.entries.get(source_sha256) {
            if let Some(ref opt) = entry.optimization {
                return opt.optimization_key == optimization_key;
            }
        }
        false
    }

    pub fn is_verified_in_storage(&self, source_sha256: &str) -> bool {
        if let Some(entry) = self.entries.get(source_sha256) {
            if let Some(ref st) = entry.storage {
                return st.verified;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_golden_json_roundtrip() {
        let mut manifest =
            Manifest::with_timestamps("0.1.0", "2026-08-17T00:00:00Z", "2026-08-17T01:00:00Z");

        let hash1 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string();
        let entry1 = ManifestEntry {
            source: ManifestSourceRecord {
                sha256: hash1.clone(),
                format: ImageFormat::Jpeg,
                byte_size: 1048576,
                dimensions: Dimensions::new(1920, 1080),
            },
            optimization: Some(ManifestOptimizationRecord {
                optimization_key: "opt-key-v1-balanced-webp".into(),
                output_format: ImageFormat::Webp,
                output_size: 420000,
                quality_score: Some(0.985),
                preset: "balanced".into(),
                completed_at: "2026-08-17T00:30:00Z".into(),
            }),
            storage: Some(ManifestStorageRecord {
                bucket: "my-image-bucket".into(),
                object_key: format!("images/{hash1}/optimized.webp"),
                etag: Some("\"etag123\"".into()),
                public_url: Some(format!(
                    "https://cdn.example.com/images/{hash1}/optimized.webp"
                )),
                uploaded_at: "2026-08-17T00:45:00Z".into(),
                verified: true,
            }),
            status: EntryStatus::Verified,
            known_paths: vec!["photos/hero.jpg".into(), "assets/hero_copy.jpg".into()],
        };

        manifest.insert(entry1);

        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let deserialized: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest, deserialized);

        assert!(manifest.is_optimized(&hash1, "opt-key-v1-balanced-webp"));
        assert!(!manifest.is_optimized(&hash1, "opt-key-v2-lossless"));
        assert!(manifest.is_verified_in_storage(&hash1));
    }

    #[test]
    fn test_path_alias_deduplication() {
        let mut manifest = Manifest::new("0.1.0");
        let hash = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".to_string();

        manifest.insert(ManifestEntry {
            source: ManifestSourceRecord {
                sha256: hash.clone(),
                format: ImageFormat::Png,
                byte_size: 2048,
                dimensions: Dimensions::new(100, 100),
            },
            optimization: None,
            storage: None,
            status: EntryStatus::Pending,
            known_paths: vec!["dir1/img.png".into()],
        });

        manifest.add_path_alias(&hash, "dir2/copy.png");
        manifest.add_path_alias(&hash, "dir1/img.png"); // duplicate path should not be re-added

        let entry = manifest.get(&hash).unwrap();
        assert_eq!(entry.known_paths, vec!["dir1/img.png", "dir2/copy.png"]);
    }
}
