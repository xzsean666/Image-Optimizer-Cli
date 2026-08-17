use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::model::{Manifest, CURRENT_SCHEMA_VERSION};
use imgopt_core::{ErrorKind, ImgoptError, PipelineStage, Result};

/// Storage manager for loading and atomically persisting manifest files.
#[derive(Debug, Clone)]
pub struct ManifestStore {
    path: PathBuf,
}

impl ManifestStore {
    pub fn new<P: Into<PathBuf>>(path: P) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load an existing manifest, or return a new empty manifest if the file does not exist.
    pub fn load_or_init(&self, optimizer_version: &str) -> Result<Manifest> {
        if !self.path.exists() {
            return Ok(Manifest::new(optimizer_version));
        }
        self.load()
    }

    /// Load and validate an existing manifest file.
    pub fn load(&self) -> Result<Manifest> {
        if !self.path.exists() {
            return Err(ImgoptError::new(
                PipelineStage::ManifestLookup,
                ErrorKind::Persistence(format!("manifest file not found: {}", self.path.display())),
            )
            .with_path(&self.path));
        }

        let content = fs::read_to_string(&self.path).map_err(|e| {
            ImgoptError::new(
                PipelineStage::ManifestLookup,
                ErrorKind::Io(format!("failed to read manifest file: {e}")),
            )
            .with_path(&self.path)
        })?;

        let manifest: Manifest = serde_json::from_str(&content).map_err(|e| {
            ImgoptError::new(
                PipelineStage::ManifestLookup,
                ErrorKind::Persistence(format!("malformed manifest JSON: {e}")),
            )
            .with_path(&self.path)
        })?;

        if manifest.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(ImgoptError::new(
                PipelineStage::ManifestLookup,
                ErrorKind::Persistence(format!(
                    "unsupported manifest schema version: got {}, expected {}",
                    manifest.schema_version, CURRENT_SCHEMA_VERSION
                )),
            )
            .with_path(&self.path));
        }

        Ok(manifest)
    }

    /// Atomically save the manifest using write-to-temp-then-rename semantics.
    pub fn save(&self, manifest: &Manifest) -> Result<()> {
        if manifest.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(ImgoptError::new(
                PipelineStage::ManifestCommit,
                ErrorKind::Persistence(format!(
                    "cannot save manifest with unsupported schema version: {}",
                    manifest.schema_version
                )),
            )
            .with_path(&self.path));
        }

        let parent_dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        if !parent_dir.exists() {
            fs::create_dir_all(parent_dir).map_err(|e| {
                ImgoptError::new(
                    PipelineStage::ManifestCommit,
                    ErrorKind::Io(format!(
                        "failed to create parent directory for manifest: {e}"
                    )),
                )
                .with_path(parent_dir)
            })?;
        }

        let json_bytes = serde_json::to_vec_pretty(manifest).map_err(|e| {
            ImgoptError::new(
                PipelineStage::ManifestCommit,
                ErrorKind::Persistence(format!("failed to serialize manifest: {e}")),
            )
            .with_path(&self.path)
        })?;

        // Write to temporary file in the same directory to ensure atomic same-filesystem rename
        let tmp_path = parent_dir.join(format!(
            ".manifest-{}.tmp",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));

        let write_res = (|| -> Result<()> {
            let mut tmp_file = File::create(&tmp_path).map_err(|e| {
                ImgoptError::new(
                    PipelineStage::ManifestCommit,
                    ErrorKind::Io(format!("failed to create temporary manifest file: {e}")),
                )
                .with_path(&tmp_path)
            })?;

            tmp_file.write_all(&json_bytes).map_err(|e| {
                ImgoptError::new(
                    PipelineStage::ManifestCommit,
                    ErrorKind::Io(format!("failed to write temporary manifest file: {e}")),
                )
                .with_path(&tmp_path)
            })?;

            tmp_file.sync_all().map_err(|e| {
                ImgoptError::new(
                    PipelineStage::ManifestCommit,
                    ErrorKind::Io(format!("failed to sync temporary manifest file: {e}")),
                )
                .with_path(&tmp_path)
            })?;

            fs::rename(&tmp_path, &self.path).map_err(|e| {
                ImgoptError::new(
                    PipelineStage::ManifestCommit,
                    ErrorKind::Io(format!(
                        "failed to rename temporary manifest file to target: {e}"
                    )),
                )
                .with_path(&self.path)
            })?;

            Ok(())
        })();

        if write_res.is_err() && tmp_path.exists() {
            let _ = fs::remove_file(&tmp_path);
        }

        write_res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_or_init_missing_file() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("missing_manifest.json");
        let store = ManifestStore::new(&path);

        let manifest = store.load_or_init("0.1.0").unwrap();
        assert_eq!(manifest.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(manifest.optimizer_version, "0.1.0");
        assert_eq!(manifest.entries.len(), 0);
        // Ensure file was not created on read-only load_or_init
        assert!(!path.exists());
    }

    #[test]
    fn test_save_and_reload_atomic() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("sub").join("manifest.json");
        let store = ManifestStore::new(&path);

        let mut manifest = Manifest::new("0.1.0");
        manifest.entries.insert(
            "hash123".into(),
            crate::model::ManifestEntry {
                source: crate::model::ManifestSourceRecord {
                    sha256: "hash123".into(),
                    format: imgopt_core::ImageFormat::Jpeg,
                    byte_size: 100,
                    dimensions: imgopt_core::Dimensions::new(10, 10),
                },
                optimization: None,
                storage: None,
                status: crate::model::EntryStatus::Pending,
                known_paths: vec!["a.jpg".into()],
            },
        );

        store.save(&manifest).unwrap();
        assert!(path.exists());

        let reloaded_store = ManifestStore::new(&path);
        let loaded = reloaded_store.load().unwrap();
        assert_eq!(manifest, loaded);
    }

    #[test]
    fn test_malformed_json_leaves_file_intact() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("broken.json");
        fs::write(&path, b"not valid json {{{").unwrap();

        let store = ManifestStore::new(&path);
        let res = store.load();
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().stage, PipelineStage::ManifestLookup);

        // Verify original broken content remains untouched
        assert_eq!(fs::read(&path).unwrap(), b"not valid json {{{");
    }
}
