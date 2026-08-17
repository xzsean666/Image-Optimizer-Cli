use sha2::Digest;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Parameters for uploading an object to remote storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadRequest {
    pub bucket: String,
    pub object_key: String,
    pub body: Vec<u8>,
    pub content_type: String,
    pub content_sha256: Option<String>,
}

impl UploadRequest {
    pub fn new(
        bucket: impl Into<String>,
        object_key: impl Into<String>,
        body: Vec<u8>,
        content_type: impl Into<String>,
    ) -> Self {
        Self {
            bucket: bucket.into(),
            object_key: object_key.into(),
            body,
            content_type: content_type.into(),
            content_sha256: None,
        }
    }

    pub fn with_sha256(mut self, sha256: impl Into<String>) -> Self {
        self.content_sha256 = Some(sha256.into());
        self
    }
}

/// Metadata returned after a successful upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadResponse {
    pub bucket: String,
    pub object_key: String,
    pub etag: Option<String>,
    pub byte_size: u64,
}

/// Metadata inspecting an existing remote object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMetadata {
    pub bucket: String,
    pub object_key: String,
    pub byte_size: u64,
    pub etag: Option<String>,
    pub content_type: Option<String>,
}

/// Typed storage error taxonomy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    NotFound { bucket: String, object_key: String },
    AuthenticationFailed(String),
    AccessDenied(String),
    Timeout(String),
    Network(String),
    ServerError(String),
    ChecksumMismatch { expected: String, actual: String },
    InvalidParameter(String),
}

impl StorageError {
    /// Whether this storage error is safe to retry.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Timeout(_) | Self::Network(_) | Self::ServerError(_)
        )
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { bucket, object_key } => {
                write!(f, "object not found: {bucket}/{object_key}")
            }
            Self::AuthenticationFailed(msg) => write!(f, "authentication failed: {msg}"),
            Self::AccessDenied(msg) => write!(f, "access denied: {msg}"),
            Self::Timeout(msg) => write!(f, "storage timeout: {msg}"),
            Self::Network(msg) => write!(f, "network error: {msg}"),
            Self::ServerError(msg) => write!(f, "remote storage server error: {msg}"),
            Self::ChecksumMismatch { expected, actual } => {
                write!(f, "checksum mismatch: expected {expected}, actual {actual}")
            }
            Self::InvalidParameter(msg) => write!(f, "invalid storage parameter: {msg}"),
        }
    }
}

impl std::error::Error for StorageError {}

/// Vendor-neutral contract for remote object storage operations.
pub trait ObjectStorage: Send + Sync {
    /// Upload a binary payload to the remote object key.
    fn put_object(&self, request: &UploadRequest) -> Result<UploadResponse, StorageError>;

    /// Retrieve metadata for an object if it exists.
    fn head_object(
        &self,
        bucket: &str,
        object_key: &str,
    ) -> Result<Option<ObjectMetadata>, StorageError>;

    /// Delete an object from remote storage.
    fn delete_object(&self, bucket: &str, object_key: &str) -> Result<(), StorageError>;
}

type StorageKey = (String, String);
type StoredObject = (Vec<u8>, ObjectMetadata);

/// In-memory storage mock for fast, isolated, deterministic unit and integration testing.
#[derive(Debug, Default, Clone)]
pub struct MemoryObjectStorage {
    objects: Arc<Mutex<HashMap<StorageKey, StoredObject>>>,
    fail_next_puts: Arc<AtomicUsize>,
}

impl MemoryObjectStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Simulate intermittent network / server failures for testing retry loops.
    pub fn fail_next_n_puts(&self, count: usize) {
        self.fail_next_puts.store(count, Ordering::SeqCst);
    }

    pub fn get_object_bytes(&self, bucket: &str, object_key: &str) -> Option<Vec<u8>> {
        let guard = self.objects.lock().unwrap();
        guard
            .get(&(bucket.to_string(), object_key.to_string()))
            .map(|(bytes, _)| bytes.clone())
    }
}

impl ObjectStorage for MemoryObjectStorage {
    fn put_object(&self, request: &UploadRequest) -> Result<UploadResponse, StorageError> {
        let remaining_fails = self.fail_next_puts.load(Ordering::SeqCst);
        if remaining_fails > 0 {
            self.fail_next_puts.fetch_sub(1, Ordering::SeqCst);
            return Err(StorageError::ServerError(
                "simulated transient storage failure".into(),
            ));
        }

        let byte_size = request.body.len() as u64;

        if let Some(ref expected_sha) = request.content_sha256 {
            let mut hasher = sha2::Sha256::new();
            hasher.update(&request.body);
            let actual_sha = hex::encode(hasher.finalize());
            if !expected_sha.eq_ignore_ascii_case(&actual_sha) {
                return Err(StorageError::ChecksumMismatch {
                    expected: expected_sha.clone(),
                    actual: actual_sha,
                });
            }
        }

        let etag = format!("\"etag-{byte_size}\"");

        let meta = ObjectMetadata {
            bucket: request.bucket.clone(),
            object_key: request.object_key.clone(),
            byte_size,
            etag: Some(etag.clone()),
            content_type: Some(request.content_type.clone()),
        };

        let mut guard = self.objects.lock().unwrap();
        guard.insert(
            (request.bucket.clone(), request.object_key.clone()),
            (request.body.clone(), meta),
        );

        Ok(UploadResponse {
            bucket: request.bucket.clone(),
            object_key: request.object_key.clone(),
            etag: Some(etag),
            byte_size,
        })
    }

    fn head_object(
        &self,
        bucket: &str,
        object_key: &str,
    ) -> Result<Option<ObjectMetadata>, StorageError> {
        let guard = self.objects.lock().unwrap();
        Ok(guard
            .get(&(bucket.to_string(), object_key.to_string()))
            .map(|(_, meta)| meta.clone()))
    }

    fn delete_object(&self, bucket: &str, object_key: &str) -> Result<(), StorageError> {
        let mut guard = self.objects.lock().unwrap();
        guard.remove(&(bucket.to_string(), object_key.to_string()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_storage_put_head_delete() {
        let storage = MemoryObjectStorage::new();
        let req = UploadRequest {
            bucket: "my-bucket".into(),
            object_key: "images/cat.webp".into(),
            body: vec![1, 2, 3, 4],
            content_type: "image/webp".into(),
            content_sha256: None,
        };

        let put_res = storage.put_object(&req).unwrap();
        assert_eq!(put_res.byte_size, 4);

        let head = storage.head_object("my-bucket", "images/cat.webp").unwrap();
        assert!(head.is_some());
        let meta = head.unwrap();
        assert_eq!(meta.byte_size, 4);

        storage
            .delete_object("my-bucket", "images/cat.webp")
            .unwrap();
        let head_after = storage.head_object("my-bucket", "images/cat.webp").unwrap();
        assert!(head_after.is_none());
    }

    #[test]
    fn test_injected_failure_is_retryable() {
        let storage = MemoryObjectStorage::new();
        storage.fail_next_n_puts(1);

        let req = UploadRequest {
            bucket: "my-bucket".into(),
            object_key: "images/dog.webp".into(),
            body: vec![1, 2, 3],
            content_type: "image/webp".into(),
            content_sha256: None,
        };

        let res1 = storage.put_object(&req);
        assert!(res1.is_err());
        assert!(res1.unwrap_err().is_retryable());

        // Subsequent call succeeds
        let res2 = storage.put_object(&req);
        assert!(res2.is_ok());
    }
}
