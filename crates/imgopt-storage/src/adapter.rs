use std::path::Path;

use imgopt_core::{
    hash_bytes, utc_now_rfc3339, ErrorKind, ImageFormat, ImgoptError, OptimizationResult,
    PipelineStage, Result as CoreResult, StorageErrorKind, SyncStorage, SyncStorageOutput,
};

use crate::interface::{ObjectStorage, StorageError, UploadRequest};
use crate::upload::{upload_and_verify, RetryPolicy};
use crate::url::{
    build_public_url, derive_object_key_for_mode, derive_path_object_key, ObjectKeyMode,
};

/// Type alias for dynamic presigned URL generator functions.
pub type PresignedUrlGenerator = Box<dyn Fn(&str, &str) -> Option<String> + Send + Sync>;

/// Adapter connecting any ObjectStorage implementation to the SyncStorage pipeline contract.
pub struct StorageSyncAdapter<S: ObjectStorage> {
    pub storage: S,
    pub public_url_base: Option<String>,
    pub key_prefix: Option<String>,
    pub cache_control: Option<String>,
    pub key_mode: ObjectKeyMode,
    pub default_bucket: Option<String>,
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
            key_mode: ObjectKeyMode::Content,
            default_bucket: None,
            presigned_url_generator: None,
            retry_policy,
        }
    }

    pub fn with_cache_control(mut self, cache_control: Option<String>) -> Self {
        self.cache_control = cache_control;
        self
    }

    pub fn with_key_mode(mut self, key_mode: ObjectKeyMode) -> Self {
        self.key_mode = key_mode;
        self
    }

    pub fn with_bucket(mut self, bucket: impl Into<String>) -> Self {
        self.default_bucket = Some(bucket.into());
        self
    }

    pub fn with_presigned_generator<F: Fn(&str, &str) -> Option<String> + Send + Sync + 'static>(
        mut self,
        gen: F,
    ) -> Self {
        self.presigned_url_generator = Some(Box::new(gen));
        self
    }

    /// Convenience helper for API/library callers: upload and verify a single `OptimizationResult`
    /// using the adapter's configured key mode, prefix, cache-control, and public URL base.
    pub fn upload_optimization_result(
        &self,
        bucket: &str,
        result: &OptimizationResult,
    ) -> CoreResult<SyncStorageOutput> {
        let sha_hex = result.source_sha256.to_hex();
        let object_key = self.derive_object_key_for_optimization(
            &sha_hex,
            result.optimization_key.as_str(),
            result.effective_format,
        )?;
        let content_type = result.effective_format.mime_type();
        let opt_sha = hash_bytes(&result.validated_output.bytes).to_hex();
        self.upload_and_verify(
            bucket,
            &object_key,
            &result.validated_output.bytes,
            content_type,
            Some(&opt_sha),
        )
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
        derive_object_key_for_mode(
            ObjectKeyMode::Content,
            self.key_prefix.as_deref(),
            source_sha256,
            None,
            format.extension(),
        )
        .map_err(|e| {
            ImgoptError::new(
                PipelineStage::StorageUpload,
                imgopt_core::ErrorKind::InvalidInput(e.to_string()),
            )
        })
    }

    fn derive_object_key_for_optimization(
        &self,
        source_sha256: &str,
        optimization_key: &str,
        format: ImageFormat,
    ) -> CoreResult<String> {
        derive_object_key_for_mode(
            self.key_mode,
            self.key_prefix.as_deref(),
            source_sha256,
            Some(optimization_key),
            format.extension(),
        )
        .map_err(|e| {
            ImgoptError::new(
                PipelineStage::StorageUpload,
                imgopt_core::ErrorKind::InvalidInput(e.to_string()),
            )
        })
    }

    fn derive_object_key_for_path(
        &self,
        source_sha256: &str,
        optimization_key: &str,
        relative_path: Option<&Path>,
        format: ImageFormat,
    ) -> CoreResult<String> {
        if self.key_mode == ObjectKeyMode::Path {
            let rel = relative_path.ok_or_else(|| {
                ImgoptError::new(
                    PipelineStage::StorageUpload,
                    imgopt_core::ErrorKind::InvalidInput(
                        "relative_path is required when ObjectKeyMode::Path is enabled".into(),
                    ),
                )
            })?;
            derive_path_object_key(self.key_prefix.as_deref(), rel, format.extension()).map_err(
                |e| {
                    ImgoptError::new(
                        PipelineStage::StorageUpload,
                        imgopt_core::ErrorKind::InvalidInput(e.to_string()),
                    )
                },
            )
        } else {
            self.derive_object_key_for_optimization(source_sha256, optimization_key, format)
        }
    }

    fn build_public_url(&self, object_key: &str) -> Option<String> {
        if let Some(ref base) = self.public_url_base {
            build_public_url(base, object_key).ok()
        } else if let Some(ref gen) = self.presigned_url_generator {
            gen(self.default_bucket.as_deref().unwrap_or(""), object_key)
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

    #[test]
    fn test_adapter_variant_key_mode() {
        let storage = crate::interface::MemoryObjectStorage::new();
        let adapter = StorageSyncAdapter::new(
            storage,
            Some("https://cdn.example.com".into()),
            Some("blog".into()),
            RetryPolicy::default(),
        )
        .with_key_mode(ObjectKeyMode::Variant);

        let key = adapter
            .derive_object_key_for_optimization(
                "deadbeef",
                "opt-v1-webp-balanced-1122334455667788",
                ImageFormat::Webp,
            )
            .unwrap();
        assert_eq!(key, "blog/images/deadbeef/112233445566/optimized.webp");
        assert_eq!(
            adapter.build_public_url(&key).as_deref(),
            Some("https://cdn.example.com/blog/images/deadbeef/112233445566/optimized.webp")
        );
    }
}
