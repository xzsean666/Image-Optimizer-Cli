use std::env;
use std::fmt;

use url::Url;

use crate::interface::StorageError;

/// Configuration for connecting to AWS S3 or an S3-compatible storage provider (e.g. Cloudflare R2, MinIO).
#[derive(Clone, PartialEq, Eq)]
pub struct S3StorageConfig {
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    pub force_path_style: bool,
    pub key_prefix: Option<String>,
    pub public_url_base: Option<String>,
    pub cache_control: Option<String>,
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
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "***REDACTED***"),
            )
            .field("force_path_style", &self.force_path_style)
            .field("key_prefix", &self.key_prefix)
            .field("public_url_base", &self.public_url_base)
            .field("cache_control", &self.cache_control)
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
            session_token: None,
            force_path_style: true,
            key_prefix: None,
            public_url_base: None,
            cache_control: None,
            timeout_seconds: 30,
        }
    }
}

impl S3StorageConfig {
    /// Load S3 configuration from environment variables.
    pub fn from_env() -> Result<Self, StorageError> {
        Self::from_env_with_overrides(None, None, None, None)
    }

    /// Load S3 configuration from environment variables while allowing explicit CLI overrides
    /// before validation (so passing `--bucket` on the CLI still uses env credentials).
    pub fn from_env_with_overrides(
        bucket_override: Option<String>,
        endpoint_override: Option<String>,
        prefix_override: Option<String>,
        public_url_override: Option<String>,
    ) -> Result<Self, StorageError> {
        Self::from_env_with_cli_overrides(
            bucket_override,
            endpoint_override,
            prefix_override,
            public_url_override,
            None,
            None,
        )
    }

    /// Load S3 configuration from environment variables with full CLI overrides including
    /// `force_path_style` and `cache_control`.
    pub fn from_env_with_cli_overrides(
        bucket_override: Option<String>,
        endpoint_override: Option<String>,
        prefix_override: Option<String>,
        public_url_override: Option<String>,
        force_path_style_override: Option<bool>,
        cache_control_override: Option<String>,
    ) -> Result<Self, StorageError> {
        Self::from_lookup_with_overrides(
            |key| env::var(key).ok(),
            bucket_override,
            endpoint_override,
            prefix_override,
            public_url_override,
            force_path_style_override,
            cache_control_override,
        )
    }

    pub(crate) fn from_lookup_with_overrides<F>(
        lookup: F,
        bucket_override: Option<String>,
        endpoint_override: Option<String>,
        prefix_override: Option<String>,
        public_url_override: Option<String>,
        force_path_style_override: Option<bool>,
        cache_control_override: Option<String>,
    ) -> Result<Self, StorageError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let bucket = bucket_override
            .or_else(|| lookup("IMGOPT_S3_BUCKET").or_else(|| lookup("AWS_S3_BUCKET")))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                StorageError::InvalidParameter(
                    "missing required storage bucket (IMGOPT_S3_BUCKET / AWS_S3_BUCKET or --bucket)".into(),
                )
            })?;

        let access_key_id = lookup("IMGOPT_S3_ACCESS_KEY_ID")
            .or_else(|| lookup("AWS_ACCESS_KEY_ID"))
            .ok_or_else(|| {
                StorageError::InvalidParameter(
                    "missing required environment variable IMGOPT_S3_ACCESS_KEY_ID or AWS_ACCESS_KEY_ID".into(),
                )
            })?
            .trim()
            .to_string();

        let secret_access_key = lookup("IMGOPT_S3_SECRET_ACCESS_KEY")
            .or_else(|| lookup("AWS_SECRET_ACCESS_KEY"))
            .ok_or_else(|| {
                StorageError::InvalidParameter(
                    "missing required environment variable IMGOPT_S3_SECRET_ACCESS_KEY or AWS_SECRET_ACCESS_KEY".into(),
                )
            })?
            .trim()
            .to_string();

        let session_token = lookup("IMGOPT_S3_SESSION_TOKEN")
            .or_else(|| lookup("AWS_SESSION_TOKEN"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let region = lookup("IMGOPT_S3_REGION")
            .or_else(|| lookup("AWS_REGION"))
            .or_else(|| lookup("AWS_DEFAULT_REGION"))
            .unwrap_or_else(|| "us-east-1".into())
            .trim()
            .to_string();

        let endpoint = endpoint_override
            .or_else(|| lookup("IMGOPT_S3_ENDPOINT").or_else(|| lookup("AWS_ENDPOINT_URL")))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let force_path_style = force_path_style_override.unwrap_or_else(|| {
            lookup("IMGOPT_S3_FORCE_PATH_STYLE")
                .map(|v| {
                    let v = v.trim();
                    v == "1" || v.eq_ignore_ascii_case("true")
                })
                .unwrap_or(true)
        });

        let key_prefix = prefix_override
            .or_else(|| lookup("IMGOPT_S3_KEY_PREFIX").or_else(|| lookup("IMGOPT_S3_PREFIX")))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let public_url_base = public_url_override
            .or_else(|| {
                lookup("IMGOPT_PUBLIC_URL_BASE").or_else(|| lookup("IMGOPT_S3_PUBLIC_URL_BASE"))
            })
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let cache_control = cache_control_override
            .or_else(|| lookup("IMGOPT_S3_CACHE_CONTROL"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let timeout_seconds = lookup("IMGOPT_S3_TIMEOUT")
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(30);

        let config = Self {
            bucket,
            region,
            endpoint,
            access_key_id,
            secret_access_key,
            session_token,
            force_path_style,
            key_prefix,
            public_url_base,
            cache_control,
            timeout_seconds,
        };

        config.validate()?;
        Ok(config)
    }

    /// Validate that all required configuration fields are present and well-formed.
    pub fn validate(&self) -> Result<(), StorageError> {
        let bucket = self.bucket.trim();
        if bucket.is_empty() {
            return Err(StorageError::InvalidParameter(
                "S3 bucket name cannot be empty".into(),
            ));
        }
        if bucket.contains("..")
            || bucket.chars().any(|c| {
                c.is_ascii_control()
                    || c.is_whitespace()
                    || matches!(c, '/' | '\\' | '?' | '#' | '@')
            })
        {
            return Err(StorageError::InvalidParameter(
                "S3 bucket name contains invalid characters".into(),
            ));
        }

        let region = self.region.trim();
        if region.is_empty()
            || !region
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(StorageError::InvalidParameter(
                "S3 region must contain only alphanumeric, '-', or '_' characters".into(),
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

        if let Some(ref ep) = self.endpoint {
            let parsed = Url::parse(ep.trim()).map_err(|e| {
                StorageError::InvalidParameter(format!("invalid S3 endpoint URL: {e}"))
            })?;
            if parsed.scheme() != "http" && parsed.scheme() != "https" {
                return Err(StorageError::InvalidParameter(
                    "S3 endpoint URL scheme must be http or https".into(),
                ));
            }
            if parsed.host_str().is_none() {
                return Err(StorageError::InvalidParameter(
                    "S3 endpoint URL must include a host".into(),
                ));
            }
            if !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
            {
                return Err(StorageError::InvalidParameter(
                    "S3 endpoint URL must not include credentials, query strings, or fragments"
                        .into(),
                ));
            }
        }

        if let Some(ref prefix) = self.key_prefix {
            if prefix.contains("..")
                || prefix
                    .chars()
                    .any(|c| c.is_ascii_control() || matches!(c, '\\' | '?' | '#'))
            {
                return Err(StorageError::InvalidParameter(
                    "S3 key_prefix contains invalid characters".into(),
                ));
            }
        }

        if let Some(ref base) = self.public_url_base {
            let parsed = Url::parse(base.trim()).map_err(|e| {
                StorageError::InvalidParameter(format!("invalid public_url_base URL: {e}"))
            })?;
            if parsed.scheme() != "http" && parsed.scheme() != "https" {
                return Err(StorageError::InvalidParameter(
                    "public_url_base must start with http:// or https://".into(),
                ));
            }
            if parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
            {
                return Err(StorageError::InvalidParameter(
                    "public_url_base must be a clean http(s) base URL".into(),
                ));
            }
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
            session_token: Some("tok789".into()),
            force_path_style: false,
            key_prefix: Some("custom/prefix".into()),
            public_url_base: Some("https://cdn.example.com".into()),
            cache_control: Some("public, max-age=31536000, immutable".into()),
            timeout_seconds: 30,
        };

        assert!(config.validate().is_ok());
        let debug_str = format!("{config:?}");
        assert!(!debug_str.contains("key123"));
        assert!(!debug_str.contains("secret456"));
        assert!(!debug_str.contains("tok789"));
        assert!(debug_str.contains("***REDACTED***"));
    }

    #[test]
    fn test_empty_and_invalid_fields_fail_validation() {
        let mut config = S3StorageConfig::default();
        assert!(config.validate().is_err());

        config.bucket = "test-bucket".into();
        assert!(config.validate().is_err());

        config.access_key_id = "test-key".into();
        assert!(config.validate().is_err());

        config.secret_access_key = "test-secret".into();
        assert!(config.validate().is_ok());

        config.bucket = "bad/bucket".into();
        assert!(config.validate().is_err());
        config.bucket = "test-bucket".into();

        config.endpoint = Some("ftp://bad.example.com".into());
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_from_lookup_trims_credentials_and_optional_fields() {
        let cfg = S3StorageConfig::from_lookup_with_overrides(
            |k| match k {
                "IMGOPT_S3_BUCKET" => Some("  my-bucket \n".into()),
                "IMGOPT_S3_ACCESS_KEY_ID" => Some("  AKIA123  ".into()),
                "IMGOPT_S3_SECRET_ACCESS_KEY" => Some("\tsecret456\n".into()),
                "IMGOPT_S3_SESSION_TOKEN" => Some("   ".into()),
                "IMGOPT_S3_REGION" => Some(" auto ".into()),
                _ => None,
            },
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(cfg.bucket, "my-bucket");
        assert_eq!(cfg.access_key_id, "AKIA123");
        assert_eq!(cfg.secret_access_key, "secret456");
        assert_eq!(cfg.session_token, None);
        assert_eq!(cfg.region, "auto");
    }
}
