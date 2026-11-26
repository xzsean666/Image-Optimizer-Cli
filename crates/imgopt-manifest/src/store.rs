use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::model::{Manifest, CURRENT_SCHEMA_VERSION};
use imgopt_core::{read_file_bounded, ErrorKind, ImgoptError, PipelineStage, Result};

const MAX_MANIFEST_BYTES: u64 = 128 * 1024 * 1024; // 128 MB safety cap

/// Atomically persist raw bytes to `dest` via an exclusive temporary file, `sync_all`, and rename.
pub(crate) fn write_bytes_atomically(
    dest: &Path,
    bytes: &[u8],
    tmp_prefix: &str,
    tmp_suffix: &str,
) -> std::io::Result<()> {
    let parent_dir = match dest.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    if !parent_dir.exists() {
        fs::create_dir_all(parent_dir)?;
    }

    let mut tmp_file = tempfile::Builder::new()
        .prefix(tmp_prefix)
        .suffix(tmp_suffix)
        .tempfile_in(parent_dir)?;
    tmp_file.write_all(bytes)?;
    tmp_file.as_file().sync_all()?;
    tmp_file.persist(dest).map_err(|e| e.error)?;
    Ok(())
}

/// Advisory exclusive OS file lock guarding concurrent multi-process manifest updates.
/// Automatically released by the OS kernel when dropped or if the process terminates.
#[derive(Debug)]
pub struct ManifestLock {
    _file: File,
    path: PathBuf,
}

impl ManifestLock {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

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

    fn lock_file_path(&self) -> PathBuf {
        let mut os_name = self.path.as_os_str().to_os_string();
        os_name.push(".lock");
        PathBuf::from(os_name)
    }

    fn open_lock_file(&self) -> Result<(File, PathBuf)> {
        let parent_dir = match self.path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        if !parent_dir.exists() {
            fs::create_dir_all(parent_dir).map_err(|e| {
                ImgoptError::new(
                    PipelineStage::ManifestLookup,
                    ErrorKind::Io(format!("failed to create manifest directory: {e}")),
                )
                .with_path(parent_dir)
            })?;
        }
        let lock_path = self.lock_file_path();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| {
                ImgoptError::new(
                    PipelineStage::ManifestLookup,
                    ErrorKind::Io(format!("failed to open manifest lock file: {e}")),
                )
                .with_path(&lock_path)
            })?;
        Ok((file, lock_path))
    }

    /// Acquire a blocking exclusive cross-process lock for this manifest.
    pub fn acquire_lock(&self) -> Result<ManifestLock> {
        let (file, lock_path) = self.open_lock_file()?;
        file.lock().map_err(|e| {
            ImgoptError::new(
                PipelineStage::ManifestLookup,
                ErrorKind::Io(format!("failed to acquire manifest lock: {e}")),
            )
            .with_path(&lock_path)
        })?;
        Ok(ManifestLock {
            _file: file,
            path: lock_path,
        })
    }

    /// Try to acquire an exclusive cross-process lock without blocking.
    pub fn try_acquire_lock(&self) -> Result<ManifestLock> {
        let (file, lock_path) = self.open_lock_file()?;
        file.try_lock().map_err(|e| {
            ImgoptError::new(
                PipelineStage::ManifestLookup,
                ErrorKind::Persistence(format!("manifest is locked by another process: {e}")),
            )
            .with_path(&lock_path)
        })?;
        Ok(ManifestLock {
            _file: file,
            path: lock_path,
        })
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

        let bytes = read_file_bounded(
            &self.path,
            MAX_MANIFEST_BYTES,
            PipelineStage::ManifestLookup,
        )?;
        let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| {
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

    /// Atomically save the manifest using exclusive temporary file + fsync + rename.
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

        let json_bytes = serde_json::to_vec_pretty(manifest).map_err(|e| {
            ImgoptError::new(
                PipelineStage::ManifestCommit,
                ErrorKind::Persistence(format!("failed to serialize manifest: {e}")),
            )
            .with_path(&self.path)
        })?;

        write_bytes_atomically(&self.path, &json_bytes, ".manifest-", ".tmp").map_err(|e| {
            ImgoptError::new(
                PipelineStage::ManifestCommit,
                ErrorKind::Io(format!("failed to save manifest atomically: {e}")),
            )
            .with_path(&self.path)
        })?;

        Ok(())
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
    fn test_manifest_exclusive_lock_blocks_second_holder() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("manifest.json");
        let store1 = ManifestStore::new(&path);
        let store2 = ManifestStore::new(&path);

        let guard = store1.try_acquire_lock().unwrap();
        assert!(store2.try_acquire_lock().is_err());

        drop(guard);
        assert!(store2.try_acquire_lock().is_ok());
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

        assert_eq!(fs::read(&path).unwrap(), b"not valid json {{{");
    }
}
