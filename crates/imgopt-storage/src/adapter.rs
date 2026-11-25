use imgopt_core::{
    utc_now_rfc3339, ImageFormat, ImgoptError, PipelineStage, Result as CoreResult, SyncStorage,
    SyncStorageOutput,
};

use crate::interface::{ObjectStorage, UploadRequest};
use crate::upload::{upload_and_verify, RetryPolicy};
use crate::url::{build_public_url, derive_object_key_with_prefix};

/// Type alias for dynamic presigned URL generator functions.
pub type PresignedUrlGenerator = Box<dyn Fn(&str, &str) -> Option<String> + Send + Sync>;

/// Adapter connecting any ObjectStorage implementation to the SyncStorage pipeline contract.
pub struct StorageSyncAdapter<S: ObjectStorage> {
    pub storage: S,
    pub public_url_base: Option<String>,
    pub key_prefix: Option<String>,
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
            presigned_url_generator: None,
            retry_policy,
        }
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

        let res = upload_and_verify(&self.storage, &req, &self.retry_policy).map_err(|e| {
            ImgoptError::new(
                PipelineStage::StorageUpload,
                imgopt_core::ErrorKind::Io(e.to_string()),
            )
        })?;

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
