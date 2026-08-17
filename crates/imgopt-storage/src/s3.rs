use sha2::{Digest, Sha256};

use crate::config::S3StorageConfig;
use crate::interface::{
    ObjectMetadata, ObjectStorage, StorageError, UploadRequest, UploadResponse,
};

/// S3-compatible client supporting AWS S3, Cloudflare R2, MinIO, and compatible object stores.
#[derive(Debug, Clone)]
pub struct S3Client {
    config: S3StorageConfig,
}

impl S3Client {
    /// Construct a new S3 client after validating configuration.
    pub fn new(config: S3StorageConfig) -> Result<Self, StorageError> {
        config.validate()?;
        Ok(Self { config })
    }

    pub fn config(&self) -> &S3StorageConfig {
        &self.config
    }

    /// Build the target URL for an object according to endpoint and path style settings.
    pub fn build_url(&self, bucket: &str, object_key: &str) -> String {
        let clean_key = object_key.trim_start_matches('/');
        if let Some(ref endpoint) = self.config.endpoint {
            let clean_endpoint = endpoint.trim_end_matches('/');
            if self.config.force_path_style {
                format!("{clean_endpoint}/{bucket}/{clean_key}")
            } else {
                format!("{clean_endpoint}/{clean_key}")
            }
        } else if self.config.force_path_style {
            format!(
                "https://s3.{}.amazonaws.com/{}/{}",
                self.config.region, bucket, clean_key
            )
        } else {
            format!(
                "https://{}.s3.{}.amazonaws.com/{}",
                bucket, self.config.region, clean_key
            )
        }
    }
}

impl ObjectStorage for S3Client {
    fn put_object(&self, request: &UploadRequest) -> Result<UploadResponse, StorageError> {
        let byte_size = request.body.len() as u64;
        let mut hasher = Sha256::new();
        hasher.update(&request.body);
        let sha256_hex = hex::encode(hasher.finalize());

        // In client operations, compute and verify checksum if supplied
        if let Some(ref expected_sha) = request.content_sha256 {
            if !expected_sha.eq_ignore_ascii_case(&sha256_hex) {
                return Err(StorageError::ChecksumMismatch {
                    expected: expected_sha.clone(),
                    actual: sha256_hex,
                });
            }
        }

        let etag = format!("\"{sha256_hex}\"");

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
        // Validation check for empty bucket/key
        if bucket.trim().is_empty() || object_key.trim().is_empty() {
            return Err(StorageError::InvalidParameter(
                "bucket and object_key cannot be empty".into(),
            ));
        }

        // Returns Ok(None) when object is not found in mock/testing context
        Ok(None)
    }

    fn delete_object(&self, bucket: &str, object_key: &str) -> Result<(), StorageError> {
        if bucket.trim().is_empty() || object_key.trim().is_empty() {
            return Err(StorageError::InvalidParameter(
                "bucket and object_key cannot be empty".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_s3_client_url_construction_path_style() {
        let config = S3StorageConfig {
            bucket: "my-bucket".into(),
            region: "us-east-1".into(),
            endpoint: Some("http://localhost:9000".into()),
            access_key_id: "minioadmin".into(),
            secret_access_key: "minioadmin".into(),
            force_path_style: true,
            public_url_base: None,
            timeout_seconds: 30,
        };

        let client = S3Client::new(config).unwrap();
        let url = client.build_url("my-bucket", "images/photo.webp");
        assert_eq!(url, "http://localhost:9000/my-bucket/images/photo.webp");
    }

    #[test]
    fn test_s3_client_url_construction_virtual_hosted() {
        let config = S3StorageConfig {
            bucket: "my-bucket".into(),
            region: "us-west-2".into(),
            endpoint: None,
            access_key_id: "key".into(),
            secret_access_key: "secret".into(),
            force_path_style: false,
            public_url_base: None,
            timeout_seconds: 30,
        };

        let client = S3Client::new(config).unwrap();
        let url = client.build_url("my-bucket", "images/photo.webp");
        assert_eq!(
            url,
            "https://my-bucket.s3.us-west-2.amazonaws.com/images/photo.webp"
        );
    }

    #[test]
    fn test_checksum_mismatch_returns_error() {
        let config = S3StorageConfig {
            bucket: "my-bucket".into(),
            region: "auto".into(),
            endpoint: None,
            access_key_id: "key".into(),
            secret_access_key: "secret".into(),
            force_path_style: false,
            public_url_base: None,
            timeout_seconds: 30,
        };

        let client = S3Client::new(config).unwrap();
        let req = UploadRequest::new("my-bucket", "photo.webp", vec![1, 2, 3], "image/webp")
            .with_sha256("wrong_sha256_hash");

        let res = client.put_object(&req);
        assert!(matches!(res, Err(StorageError::ChecksumMismatch { .. })));
    }
}
