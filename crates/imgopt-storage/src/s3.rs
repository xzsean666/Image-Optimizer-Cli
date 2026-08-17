use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use url::Url;

use crate::config::S3StorageConfig;
use crate::interface::{
    ObjectMetadata, ObjectStorage, StorageError, UploadRequest, UploadResponse,
};

type HmacSha256 = Hmac<Sha256>;

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC can take key of any size");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

fn get_amz_dates() -> (String, String) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let days = (now / 86400) as i64;
    let rem_secs = (now % 86400) as i64;
    let hour = rem_secs / 3600;
    let minute = (rem_secs % 3600) / 60;
    let second = rem_secs % 60;

    let mut y = 1970i64;
    let mut d = days;
    loop {
        let leap = if (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0) {
            1
        } else {
            0
        };
        let days_in_year = 365 + leap;
        if d < days_in_year {
            break;
        }
        d -= days_in_year;
        y += 1;
    }

    let leap = if (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0) {
        1
    } else {
        0
    };
    let month_days = [31, 28 + leap, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 1;
    for &md in &month_days {
        if d < md {
            break;
        }
        d -= md;
        m += 1;
    }
    let day = d + 1;

    let amz_date = format!("{y:04}{m:02}{day:02}T{hour:02}{minute:02}{second:02}Z");
    let date_stamp = format!("{y:04}{m:02}{day:02}");
    (amz_date, date_stamp)
}

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

    /// Generate a presigned GET URL valid for `expires_seconds` (up to 7 days).
    pub fn generate_presigned_url(
        &self,
        bucket: &str,
        object_key: &str,
        expires_seconds: u64,
    ) -> Result<String, StorageError> {
        let target_url = self.build_url(bucket, object_key);
        let parsed_url = Url::parse(&target_url)
            .map_err(|e| StorageError::InvalidParameter(format!("invalid target URL: {e}")))?;

        let host = parsed_url
            .host_str()
            .ok_or_else(|| StorageError::InvalidParameter("URL missing host".into()))?;
        let host_header = if let Some(port) = parsed_url.port() {
            format!("{host}:{port}")
        } else {
            host.to_string()
        };

        let (amz_date, date_stamp) = get_amz_dates();
        let credential_scope = format!("{date_stamp}/{}/{}/aws4_request", self.config.region, "s3");
        let credential = format!("{}/{}", self.config.access_key_id, credential_scope);

        let expires_str = expires_seconds.to_string();
        let query_params = vec![
            ("X-Amz-Algorithm", "AWS4-HMAC-SHA256"),
            ("X-Amz-Credential", &credential),
            ("X-Amz-Date", &amz_date),
            ("X-Amz-Expires", &expires_str),
            ("X-Amz-SignedHeaders", "host"),
        ];

        let mut sorted_params = query_params.clone();
        sorted_params.sort_by(|a, b| a.0.cmp(b.0));

        let canonical_query = sorted_params
            .iter()
            .map(|(k, v)| {
                format!(
                    "{}={}",
                    url::form_urlencoded::byte_serialize(k.as_bytes()).collect::<String>(),
                    url::form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join("&");

        let canonical_uri = parsed_url.path();
        let canonical_headers = format!("host:{host_header}\n");
        let signed_headers = "host";
        let payload_hash = "UNSIGNED-PAYLOAD";

        let canonical_request = format!(
            "GET\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
        );

        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );

        let k_secret = format!("AWS4{}", self.config.secret_access_key);
        let k_date = hmac_sha256(k_secret.as_bytes(), date_stamp.as_bytes());
        let k_region = hmac_sha256(&k_date, self.config.region.as_bytes());
        let k_service = hmac_sha256(&k_region, b"s3");
        let k_signing = hmac_sha256(&k_service, b"aws4_request");

        let signature = hex::encode(hmac_sha256(&k_signing, string_to_sign.as_bytes()));

        Ok(format!(
            "{target_url}?{canonical_query}&X-Amz-Signature={signature}"
        ))
    }

    /// Sign and execute an HTTP request using AWS SigV4.
    fn sign_headers(
        &self,
        method: &str,
        bucket: &str,
        object_key: &str,
        body: &[u8],
        content_type: Option<&str>,
    ) -> Result<(String, Vec<(String, String)>), StorageError> {
        let target_url = self.build_url(bucket, object_key);
        let parsed_url = Url::parse(&target_url)
            .map_err(|e| StorageError::InvalidParameter(format!("invalid target URL: {e}")))?;

        let host = parsed_url
            .host_str()
            .ok_or_else(|| StorageError::InvalidParameter("URL missing host".into()))?;
        let host_header = if let Some(port) = parsed_url.port() {
            format!("{host}:{port}")
        } else {
            host.to_string()
        };

        let (amz_date, date_stamp) = get_amz_dates();
        let payload_hash = sha256_hex(body);
        let canonical_uri = parsed_url.path();
        let canonical_query = parsed_url.query().unwrap_or("");

        let mut headers_to_sign = vec![
            ("host".to_string(), host_header.clone()),
            ("x-amz-content-sha256".to_string(), payload_hash.clone()),
            ("x-amz-date".to_string(), amz_date.clone()),
        ];

        if let Some(ct) = content_type {
            headers_to_sign.push(("content-type".to_string(), ct.to_string()));
        }

        headers_to_sign.sort_by(|a, b| a.0.cmp(&b.0));

        let signed_headers = headers_to_sign
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>()
            .join(";");

        let mut canonical_headers = String::new();
        for (k, v) in &headers_to_sign {
            canonical_headers.push_str(&format!("{k}:{v}\n"));
        }

        let canonical_request = format!(
            "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
        );

        let credential_scope = format!("{date_stamp}/{}/{}/aws4_request", self.config.region, "s3");
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );

        let k_secret = format!("AWS4{}", self.config.secret_access_key);
        let k_date = hmac_sha256(k_secret.as_bytes(), date_stamp.as_bytes());
        let k_region = hmac_sha256(&k_date, self.config.region.as_bytes());
        let k_service = hmac_sha256(&k_region, b"s3");
        let k_signing = hmac_sha256(&k_service, b"aws4_request");

        let signature = hex::encode(hmac_sha256(&k_signing, string_to_sign.as_bytes()));

        let auth_header = format!(
            "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
            self.config.access_key_id
        );

        let mut out_headers = headers_to_sign;
        out_headers.push(("Authorization".to_string(), auth_header));

        Ok((target_url, out_headers))
    }
}

impl ObjectStorage for S3Client {
    fn put_object(&self, request: &UploadRequest) -> Result<UploadResponse, StorageError> {
        let byte_size = request.body.len() as u64;
        let payload_sha = sha256_hex(&request.body);

        if let Some(ref expected_sha) = request.content_sha256 {
            if !expected_sha.eq_ignore_ascii_case(&payload_sha) {
                return Err(StorageError::ChecksumMismatch {
                    expected: expected_sha.clone(),
                    actual: payload_sha,
                });
            }
        }

        let (target_url, headers) = self.sign_headers(
            "PUT",
            &request.bucket,
            &request.object_key,
            &request.body,
            Some(&request.content_type),
        )?;

        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(self.config.timeout_seconds))
            .build();

        let mut req = agent.put(&target_url);
        for (k, v) in headers {
            req = req.set(&k, &v);
        }

        let resp = match req.send_bytes(&request.body) {
            Ok(r) => r,
            Err(ureq::Error::Status(code, resp)) => {
                let err_body = resp.into_string().unwrap_or_default();
                if code == 401 || code == 403 {
                    return Err(StorageError::AccessDenied(format!(
                        "HTTP {code} Access Denied: {err_body}"
                    )));
                }
                if code == 404 {
                    return Err(StorageError::NotFound {
                        bucket: request.bucket.clone(),
                        object_key: request.object_key.clone(),
                    });
                }
                if code >= 500 {
                    return Err(StorageError::ServerError(format!(
                        "HTTP {code} Server Error: {err_body}"
                    )));
                }
                return Err(StorageError::InvalidParameter(format!(
                    "HTTP {code} Error: {err_body}"
                )));
            }
            Err(ureq::Error::Transport(t)) => {
                return Err(StorageError::Timeout(format!("Transport Error: {t}")));
            }
        };

        let etag = resp.header("etag").map(|s| s.trim_matches('"').to_string());

        Ok(UploadResponse {
            bucket: request.bucket.clone(),
            object_key: request.object_key.clone(),
            etag,
            byte_size,
        })
    }

    fn head_object(
        &self,
        bucket: &str,
        object_key: &str,
    ) -> Result<Option<ObjectMetadata>, StorageError> {
        if bucket.trim().is_empty() || object_key.trim().is_empty() {
            return Err(StorageError::InvalidParameter(
                "bucket and object_key cannot be empty".into(),
            ));
        }

        let (target_url, headers) = self.sign_headers("HEAD", bucket, object_key, b"", None)?;

        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(self.config.timeout_seconds))
            .build();

        let mut req = agent.head(&target_url);
        for (k, v) in headers {
            req = req.set(&k, &v);
        }

        match req.call() {
            Ok(resp) => {
                let byte_size = resp
                    .header("content-length")
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
                let etag = resp.header("etag").map(|s| s.trim_matches('"').to_string());
                let content_type = resp.header("content-type").map(|s| s.to_string());

                Ok(Some(ObjectMetadata {
                    bucket: bucket.to_string(),
                    object_key: object_key.to_string(),
                    byte_size,
                    etag,
                    content_type,
                }))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                if code == 401 || code == 403 {
                    return Err(StorageError::AccessDenied(format!(
                        "HTTP {code} Access Denied: {body}"
                    )));
                }
                if code >= 500 {
                    return Err(StorageError::ServerError(format!(
                        "HTTP {code} Server Error: {body}"
                    )));
                }
                Err(StorageError::InvalidParameter(format!(
                    "HTTP {code} Error: {body}"
                )))
            }
            Err(ureq::Error::Transport(t)) => {
                Err(StorageError::Timeout(format!("Transport Error: {t}")))
            }
        }
    }

    fn delete_object(&self, bucket: &str, object_key: &str) -> Result<(), StorageError> {
        if bucket.trim().is_empty() || object_key.trim().is_empty() {
            return Err(StorageError::InvalidParameter(
                "bucket and object_key cannot be empty".into(),
            ));
        }

        let (target_url, headers) = self.sign_headers("DELETE", bucket, object_key, b"", None)?;

        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(self.config.timeout_seconds))
            .build();

        let mut req = agent.delete(&target_url);
        for (k, v) in headers {
            req = req.set(&k, &v);
        }

        match req.call() {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(404, _)) => Ok(()),
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                if code >= 500 {
                    return Err(StorageError::ServerError(format!(
                        "HTTP {code} Server Error: {body}"
                    )));
                }
                Err(StorageError::InvalidParameter(format!(
                    "HTTP {code} Error: {body}"
                )))
            }
            Err(ureq::Error::Transport(t)) => {
                Err(StorageError::Timeout(format!("Transport Error: {t}")))
            }
        }
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
            key_prefix: None,
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
            key_prefix: None,
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
            key_prefix: None,
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
