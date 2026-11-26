use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::interface::{ObjectStorage, StorageError, UploadRequest};

/// Configuration for upload retries with exponential backoff and jitter.
#[derive(Debug, Clone, PartialEq)]
pub struct RetryPolicy {
    pub max_attempts: usize,
    pub initial_backoff_millis: u64,
    pub max_backoff_millis: u64,
    pub backoff_multiplier: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff_millis: 100,
            max_backoff_millis: 2000,
            backoff_multiplier: 2.0,
        }
    }
}

/// The result of an upload that has been confirmed and verified in remote storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedUploadResult {
    pub bucket: String,
    pub object_key: String,
    pub byte_size: u64,
    pub etag: Option<String>,
    pub attempts_used: usize,
    pub verified: bool,
}

fn jitter_millis(max_jitter: u64) -> u64 {
    if max_jitter == 0 {
        return 0;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    nanos % (max_jitter + 1)
}

/// Retry a storage operation on retryable errors with exponential backoff and jitter.
pub fn retry_storage_op<T, F>(retry_policy: &RetryPolicy, mut op: F) -> Result<T, StorageError>
where
    F: FnMut() -> Result<T, StorageError>,
{
    let max_attempts = retry_policy.max_attempts.max(1);
    let mut backoff = retry_policy.initial_backoff_millis;
    let mut last_error = StorageError::InvalidParameter("operation not attempted".into());

    for attempt in 1..=max_attempts {
        match op() {
            Ok(val) => return Ok(val),
            Err(e) => {
                let is_retryable = e.is_retryable();
                last_error = e;
                if !is_retryable || attempt == max_attempts {
                    return Err(last_error);
                }
            }
        }

        let jitter = jitter_millis(backoff / 4);
        thread::sleep(Duration::from_millis(backoff.saturating_add(jitter)));
        backoff = ((backoff as f64) * retry_policy.backoff_multiplier) as u64;
        backoff = backoff.min(retry_policy.max_backoff_millis);
    }

    Err(last_error)
}

/// Upload an object with bounded retries and strict post-upload verification.
pub fn upload_and_verify<S: ObjectStorage>(
    storage: &S,
    request: &UploadRequest,
    retry_policy: &RetryPolicy,
) -> Result<VerifiedUploadResult, StorageError> {
    let max_attempts = retry_policy.max_attempts.max(1);
    let mut last_error = StorageError::InvalidParameter("upload not attempted".into());
    let mut backoff = retry_policy.initial_backoff_millis;

    for attempt in 1..=max_attempts {
        match storage.put_object(request) {
            Ok(upload_resp) => {
                // Post-upload verification: object MUST exist and match byte size (and ETag if available)
                match storage.head_object(&request.bucket, &request.object_key) {
                    Ok(Some(meta)) => {
                        let expected_size = request.body.len() as u64;
                        if meta.byte_size != expected_size {
                            return Err(StorageError::ChecksumMismatch {
                                expected: format!("{expected_size} bytes"),
                                actual: format!("{} bytes", meta.byte_size),
                            });
                        }

                        if let (Some(ref put_etag), Some(ref head_etag)) =
                            (&upload_resp.etag, &meta.etag)
                        {
                            let norm_put = put_etag.trim_matches('"');
                            let norm_head = head_etag.trim_matches('"');
                            if !norm_put.is_empty()
                                && !norm_head.is_empty()
                                && norm_put != norm_head
                            {
                                return Err(StorageError::ChecksumMismatch {
                                    expected: norm_put.to_string(),
                                    actual: norm_head.to_string(),
                                });
                            }
                        }

                        return Ok(VerifiedUploadResult {
                            bucket: upload_resp.bucket,
                            object_key: upload_resp.object_key,
                            byte_size: meta.byte_size,
                            etag: meta.etag.or(upload_resp.etag),
                            attempts_used: attempt,
                            verified: true,
                        });
                    }
                    Ok(None) => {
                        let missing_err = StorageError::NotFound {
                            bucket: request.bucket.clone(),
                            object_key: request.object_key.clone(),
                        };
                        last_error = missing_err;
                        if attempt == max_attempts {
                            return Err(last_error);
                        }
                    }
                    Err(e) => {
                        let is_retryable = e.is_retryable();
                        last_error = e;
                        if !is_retryable || attempt == max_attempts {
                            return Err(last_error);
                        }
                    }
                }
            }
            Err(e) => {
                let is_retryable = e.is_retryable();
                last_error = e;

                if !is_retryable || attempt == max_attempts {
                    return Err(last_error);
                }
            }
        }

        let jitter = jitter_millis(backoff / 4);
        thread::sleep(Duration::from_millis(backoff.saturating_add(jitter)));
        backoff = ((backoff as f64) * retry_policy.backoff_multiplier) as u64;
        backoff = backoff.min(retry_policy.max_backoff_millis);
    }

    Err(last_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::{MemoryObjectStorage, ObjectMetadata, UploadResponse};

    struct VanishingStorage;

    impl ObjectStorage for VanishingStorage {
        fn put_object(&self, req: &UploadRequest) -> Result<UploadResponse, StorageError> {
            Ok(UploadResponse {
                bucket: req.bucket.clone(),
                object_key: req.object_key.clone(),
                etag: Some("etag".into()),
                byte_size: req.body.len() as u64,
            })
        }

        fn head_object(
            &self,
            _bucket: &str,
            _object_key: &str,
        ) -> Result<Option<ObjectMetadata>, StorageError> {
            // Simulates missing object (404) after PUT
            Ok(None)
        }

        fn delete_object(&self, _bucket: &str, _object_key: &str) -> Result<(), StorageError> {
            Ok(())
        }

        fn list_objects(
            &self,
            _bucket: &str,
            _prefix: Option<&str>,
        ) -> Result<Vec<crate::interface::ListedObject>, StorageError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn test_immediate_success() {
        let storage = MemoryObjectStorage::new();
        let req = UploadRequest::new("bucket", "key.webp", vec![1, 2, 3], "image/webp");
        let policy = RetryPolicy::default();

        let res = upload_and_verify(&storage, &req, &policy).unwrap();
        assert_eq!(res.attempts_used, 1);
        assert!(res.verified);
        assert_eq!(res.byte_size, 3);
    }

    #[test]
    fn test_missing_object_after_put_fails_verification() {
        let storage = VanishingStorage;
        let req = UploadRequest::new("bucket", "missing.webp", vec![1, 2, 3], "image/webp");
        let policy = RetryPolicy {
            max_attempts: 2,
            initial_backoff_millis: 1,
            max_backoff_millis: 2,
            backoff_multiplier: 1.0,
        };

        let res = upload_and_verify(&storage, &req, &policy);
        assert!(matches!(res, Err(StorageError::NotFound { .. })));
    }

    #[test]
    fn test_transient_failure_retries_and_succeeds() {
        let storage = MemoryObjectStorage::new();
        storage.fail_next_n_puts(1);

        let req = UploadRequest::new("bucket", "key.webp", vec![1, 2, 3, 4], "image/webp");
        let policy = RetryPolicy {
            max_attempts: 3,
            initial_backoff_millis: 5,
            max_backoff_millis: 20,
            backoff_multiplier: 2.0,
        };

        let res = upload_and_verify(&storage, &req, &policy).unwrap();
        assert_eq!(res.attempts_used, 2);
        assert!(res.verified);
    }

    #[test]
    fn test_permanent_failure_does_not_retry() {
        let storage = MemoryObjectStorage::new();
        let mut req = UploadRequest::new("bucket", "key.webp", vec![1, 2, 3], "image/webp");
        req.content_sha256 = Some("mismatched_hash".into());

        let policy = RetryPolicy::default();
        let res = upload_and_verify(&storage, &req, &policy);
        assert!(res.is_err());
    }

    #[test]
    fn test_exhausted_retries_fails() {
        let storage = MemoryObjectStorage::new();
        storage.fail_next_n_puts(10);

        let req = UploadRequest::new("bucket", "key.webp", vec![1, 2], "image/webp");
        let policy = RetryPolicy {
            max_attempts: 2,
            initial_backoff_millis: 5,
            max_backoff_millis: 10,
            backoff_multiplier: 1.5,
        };

        let res = upload_and_verify(&storage, &req, &policy);
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), StorageError::ServerError(_)));
    }
}
