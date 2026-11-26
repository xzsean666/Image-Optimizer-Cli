use imgopt_core::{
    utc_now_rfc3339, ErrorKind, ImageFormat, ImgoptError, PipelineStage, Result as CoreResult,
    StorageErrorKind, SyncStorage, SyncStorageOutput,
};

use crate::interface::{ObjectStorage, StorageError, UploadRequest};
use crate::upload::{upload_and_verify, RetryPolicy};
use crate::url::{build_public_url, derive_object_key_with_prefix};

/// Type alias for dynamic presigned URL generator functions.
pub type PresignedUrlGenerator = Box<dyn Fn(&str, &str) -> Option<String> + Send + Sync>;

/// Adapter connecting any ObjectStorage implementation to the SyncStorage pipeline contract.
pub struct StorageSyncAdapter<S: ObjectStorage> {
    pub storage: S,
    pub public_url_base: Option<String>,
    pub key_prefix: Option<String>,
    pub cache_control: Option<String>,
    pub presigned_url_generator: Option<PresignedUrlGenerator>,
    pub retry_policy: RetryPolicy,
}

impl<S: ObjectStorage> StorageSyncAdapter<S> {
    pub fn new(
        storage: S,
        public_url_base: Option<String>,
        key_prefix: Option<String>,
        retry_policy: RetryPolicy,
    ) -> Self {
        Self {
            storage,
            public_url_base,
            key_prefix,
            cache_control: None,
            presigned_url_generator: None,
            retry_policy,
        }
    }

    pub fn with_cache_control(mut self, cache_control: Option<String>) -> Self {
        self.cache_control = cache_control;
        self
    }

    pub fn with_presigned_generator<F: Fn(&str, &str) -> Option<String> + Send + Sync + 'static>(
        mut self,
        gen: F,
    ) -> Self {
        self.presigned_url_generator = Some(Box::new(gen));
        self
    }
}

impl<S: ObjectStorage> SyncStorage for StorageSyncAdapter<S> {
    fn upload_and_verify(
        &self,
        bucket: &str,
        object_key: &str,
        body: &[u8],
        content_type: &str,
        content_sha256: Option<&str>,
    ) -> CoreResult<SyncStorageOutput> {
        let mut req = UploadRequest::new(bucket, object_key, body.to_vec(), content_type);
        if let Some(sha) = content_sha256 {
            req = req.with_sha256(sha);
        }
        if let Some(ref cc) = self.cache_control {
            if !cc.trim().is_empty() {
                req = req.with_cache_control(cc.clone());
            }
        }

        let res = upload_and_verify(&self.storage, &req, &self.retry_policy)
            .map_err(map_storage_error)?;

        let public_url = if let Some(ref base) = self.public_url_base {
            build_public_url(base, object_key).ok()
        } else if let Some(ref gen) = self.presigned_url_generator {
            gen(bucket, object_key)
        } else {
            None
        };

        Ok(SyncStorageOutput {
            bucket: res.bucket,
            object_key: res.object_key,
            byte_size: res.byte_size,
            etag: res.etag,
            public_url,
            uploaded_at: utc_now_rfc3339(),
        })
    }

    fn derive_object_key(&self, source_sha256: &str, format: ImageFormat) -> CoreResult<String> {
        derive_object_key_with_prefix(
            self.key_prefix.as_deref(),
            source_sha256,
            format.extension(),
        )
        .map_err(|e| {
            ImgoptError::new(
                PipelineStage::StorageUpload,
                imgopt_core::ErrorKind::InvalidInput(e.to_string()),
            )
        })
    }

    fn build_public_url(&self, object_key: &str) -> Option<String> {
        if let Some(ref base) = self.public_url_base {
            build_public_url(base, object_key).ok()
        } else if let Some(ref gen) = self.presigned_url_generator {
            gen("", object_key)
        } else {
            None
        }
    }
}

fn map_storage_error(err: StorageError) -> ImgoptError {
    let msg = err.to_string();
    let kind = match err {
        StorageError::AuthenticationFailed(_) | StorageError::AccessDenied(_) => {
            StorageErrorKind::AuthenticationFailed
        }
        StorageError::NotFound { .. } => StorageErrorKind::NotFound,
        StorageError::Timeout(_) => StorageErrorKind::Timeout,
        StorageError::ServerError(ref body) => {
            let status_code = body
                .strip_prefix("HTTP ")
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|code| code.parse::<u16>().ok())
                .unwrap_or(500);
            StorageErrorKind::ServerSide5xx { status_code }
        }
        StorageError::Network(_) => StorageErrorKind::Network,
        StorageError::ChecksumMismatch { .. } | StorageError::InvalidParameter(_) => {
            StorageErrorKind::Other
        }
    };
    ImgoptError::new(
        PipelineStage::StorageUpload,
        ErrorKind::Storage { kind, message: msg },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_error_preserves_retryable_kind() {
        let timeout_err = map_storage_error(StorageError::Timeout("timed out".into()));
        assert!(timeout_err.is_retryable());

        let srv_err = map_storage_error(StorageError::ServerError("HTTP 503 Unavailable".into()));
        assert!(srv_err.is_retryable());

        let auth_err = map_storage_error(StorageError::AuthenticationFailed("bad key".into()));
        assert!(!auth_err.is_retryable());
    }
}
