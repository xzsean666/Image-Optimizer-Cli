use std::env;
use std::fmt;

use crate::interface::StorageError;

/// Configuration for connecting to AWS S3 or an S3-compatible storage provider (e.g. Cloudflare R2, MinIO).
#[derive(Clone, PartialEq, Eq)]
pub struct S3StorageConfig {
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub force_path_style: bool,
    pub key_prefix: Option<String>,
    pub public_url_base: Option<String>,
    pub timeout_seconds: u64,
}

impl fmt::Debug for S3StorageConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3StorageConfig")
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("endpoint", &self.endpoint)
            .field("access_key_id", &"***REDACTED***")
            .field("secret_access_key", &"***REDACTED***")
            .field("force_path_style", &self.force_path_style)
            .field("key_prefix", &self.key_prefix)
            .field("public_url_base", &self.public_url_base)
            .field("timeout_seconds", &self.timeout_seconds)
            .finish()
    }
}

impl Default for S3StorageConfig {
    fn default() -> Self {
        Self {
            bucket: String::new(),
            region: "us-east-1".to_string(),
            endpoint: None,
            access_key_id: String::new(),
            secret_access_key: String::new(),
            force_path_style: true,
            key_prefix: None,
            public_url_base: None,
            timeout_seconds: 30,
        }
    }
}

impl S3StorageConfig {
    /// Load S3 configuration from environment variables.
    pub fn from_env() -> Result<Self, StorageError> {
        let bucket = env::var("IMGOPT_S3_BUCKET")
            .or_else(|_| env::var("AWS_S3_BUCKET"))
            .map_err(|_| {
                StorageError::InvalidParameter(
                    "missing required environment variable IMGOPT_S3_BUCKET".into(),
                )
            })?;

        let access_key_id = env::var("IMGOPT_S3_ACCESS_KEY_ID")
            .or_else(|_| env::var("AWS_ACCESS_KEY_ID"))
            .map_err(|_| {
                StorageError::InvalidParameter(
                    "missing required environment variable IMGOPT_S3_ACCESS_KEY_ID or AWS_ACCESS_KEY_ID".into(),
                )
            })?;

        let secret_access_key = env::var("IMGOPT_S3_SECRET_ACCESS_KEY")
            .or_else(|_| env::var("AWS_SECRET_ACCESS_KEY"))
            .map_err(|_| {
                StorageError::InvalidParameter(
                    "missing required environment variable IMGOPT_S3_SECRET_ACCESS_KEY or AWS_SECRET_ACCESS_KEY".into(),
                )
            })?;

        let region = env::var("IMGOPT_S3_REGION")
            .or_else(|_| env::var("AWS_REGION"))
            .or_else(|_| env::var("AWS_DEFAULT_REGION"))
            .unwrap_or_else(|_| "us-east-1".into());

        let endpoint = env::var("IMGOPT_S3_ENDPOINT")
            .or_else(|_| env::var("AWS_ENDPOINT_URL"))
            .ok()
            .filter(|s| !s.is_empty());

        let force_path_style = env::var("IMGOPT_S3_FORCE_PATH_STYLE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(true);

        let key_prefix = env::var("IMGOPT_S3_KEY_PREFIX")
            .or_else(|_| env::var("IMGOPT_S3_PREFIX"))
            .ok()
            .filter(|s| !s.is_empty());

        let public_url_base = env::var("IMGOPT_PUBLIC_URL_BASE")
            .ok()
            .filter(|s| !s.is_empty());

        let timeout_seconds = env::var("IMGOPT_S3_TIMEOUT")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(30);

        let config = Self {
            bucket,
            region,
            endpoint,
            access_key_id,
            secret_access_key,
            force_path_style,
            key_prefix,
            public_url_base,
            timeout_seconds,
        };

        config.validate()?;
        Ok(config)
    }

    /// Validate that all required configuration fields are present and valid.
    pub fn validate(&self) -> Result<(), StorageError> {
        if self.bucket.trim().is_empty() {
            return Err(StorageError::InvalidParameter(
                "S3 bucket name cannot be empty".into(),
            ));
        }
        if self.access_key_id.trim().is_empty() {
            return Err(StorageError::InvalidParameter(
                "S3 access_key_id cannot be empty".into(),
            ));
        }
        if self.secret_access_key.trim().is_empty() {
            return Err(StorageError::InvalidParameter(
                "S3 secret_access_key cannot be empty".into(),
            ));
        }
        if self.timeout_seconds == 0 {
            return Err(StorageError::InvalidParameter(
                "timeout_seconds must be greater than 0".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_config() {
        let config = S3StorageConfig {
            bucket: "my-bucket".into(),
            region: "auto".into(),
            endpoint: Some("https://account.r2.cloudflarestorage.com".into()),
            access_key_id: "key123".into(),
            secret_access_key: "secret456".into(),
            force_path_style: false,
            key_prefix: Some("custom/prefix".into()),
            public_url_base: Some("https://cdn.example.com".into()),
            timeout_seconds: 30,
        };

        assert!(config.validate().is_ok());
        let debug_str = format!("{config:?}");
        assert!(!debug_str.contains("key123"));
        assert!(!debug_str.contains("secret456"));
        assert!(debug_str.contains("***REDACTED***"));
    }

    #[test]
    fn test_empty_fields_fail_validation() {
        let mut config = S3StorageConfig::default();
        assert!(config.validate().is_err());

        config.bucket = "test-bucket".into();
        assert!(config.validate().is_err());

        config.access_key_id = "test-key".into();
        assert!(config.validate().is_err());

        config.secret_access_key = "test-secret".into();
        assert!(config.validate().is_ok());
    }
}
