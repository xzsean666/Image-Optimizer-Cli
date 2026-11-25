use std::io::Read;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use url::Url;

use crate::config::S3StorageConfig;
use crate::interface::{
    ObjectMetadata, ObjectStorage, StorageError, UploadRequest, UploadResponse,
};

type HmacSha256 = Hmac<Sha256>;

const MAX_ERROR_BODY_BYTES: u64 = 4096;

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

/// AWS SigV4 RFC 3986 URI encoding (`A-Za-z0-9-_.~`, optionally keeping `/`).
fn sigv4_uri_encode(input: &str, encode_slash: bool) -> String {
    let mut out = String::with_capacity(input.len());
    for &b in input.as_bytes() {
        let is_unreserved = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~');
        if is_unreserved || (!encode_slash && b == b'/') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

fn read_bounded_error_body(resp: ureq::Response) -> String {
    let mut buf = Vec::new();
    let _ = resp
        .into_reader()
        .take(MAX_ERROR_BODY_BYTES)
        .read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).trim().to_string()
}

fn map_transport_error(err: ureq::Transport) -> StorageError {
    let msg = err.to_string();
    match err.kind() {
        ureq::ErrorKind::InvalidUrl
        | ureq::ErrorKind::UnknownScheme
        | ureq::ErrorKind::InvalidProxyUrl
        | ureq::ErrorKind::TooManyRedirects
        | ureq::ErrorKind::InsecureRequestHttpsOnly => {
            StorageError::InvalidParameter(format!("HTTP configuration/redirect error: {msg}"))
        }
        ureq::ErrorKind::ConnectionFailed | ureq::ErrorKind::Dns | ureq::ErrorKind::ProxyConnect => {
            StorageError::Network(format!("Network error: {msg}"))
        }
        ureq::ErrorKind::ProxyUnauthorized => {
            StorageError::AuthenticationFailed(format!("Proxy auth error: {msg}"))
        }
        _ => {
            if msg.to_ascii_lowercase().contains("timed out")
                || msg.to_ascii_lowercase().contains("timeout")
            {
                StorageError::Timeout(format!("Transport timeout: {msg}"))
            } else {
                StorageError::Network(format!("Transport error: {msg}"))
            }
        }
    }
}

fn map_http_status_error(
    code: u16,
    body: String,
    bucket: &str,
    object_key: &str,
) -> StorageError {
    match code {
        401 => StorageError::AuthenticationFailed(format!("HTTP 401 Unauthorized: {body}")),
        403 => StorageError::AccessDenied(format!("HTTP 403 Access Denied: {body}")),
        404 => StorageError::NotFound {
            bucket: bucket.to_string(),
            object_key: object_key.to_string(),
        },
        408 => StorageError::Timeout(format!("HTTP 408 Request Timeout: {body}")),
        429 | 500..=599 => {
            StorageError::ServerError(format!("HTTP {code} Server Error: {body}"))
        }
        _ => StorageError::InvalidParameter(format!("HTTP {code} Error: {body}")),
    }
}

fn get_amz_dates() -> (String, String) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (y, m, day, hour, minute, second) = imgopt_core::unix_to_utc_components(now);
    let amz_date = format!("{y:04}{m:02}{day:02}T{hour:02}{minute:02}{second:02}Z");
    let date_stamp = format!("{y:04}{m:02}{day:02}");
    (amz_date, date_stamp)
}

/// S3-compatible client supporting AWS S3, Cloudflare R2, MinIO, and compatible object stores.
/// Holds a reusable `ureq::Agent` for connection pooling and TLS session reuse, with redirects disabled.
#[derive(Debug, Clone)]
pub struct S3Client {
    config: S3StorageConfig,
    agent: ureq::Agent,
}

impl S3Client {
    /// Construct a new S3 client after validating configuration.
    pub fn new(config: S3StorageConfig) -> Result<Self, StorageError> {
        config.validate()?;
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .redirects(0)
            .build();
        Ok(Self { config, agent })
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
            } else if let Ok(mut parsed) = Url::parse(clean_endpoint) {
                if let Some(host) = parsed.host_str().map(ToString::to_string) {
                    let vhost = format!("{bucket}.{host}");
                    let _ = parsed.set_host(Some(&vhost));
                }
                let base_path = parsed.path().trim_end_matches('/');
                let base = format!(
                    "{}://{}{}",
                    parsed.scheme(),
                    parsed.host_str().unwrap_or_default(),
                    parsed.port().map(|p| format!(":{p}")).unwrap_or_default()
                );
                if base_path.is_empty() {
                    format!("{base}/{clean_key}")
                } else {
                    format!("{base}{base_path}/{clean_key}")
                }
            } else {
                format!("{clean_endpoint}/{bucket}/{clean_key}")
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

    /// Generate a presigned GET URL valid for `expires_seconds`.
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
        let credential_scope = format!("{date_stamp}/{}/s3/aws4_request", self.config.region);
        let credential = format!("{}/{}", self.config.access_key_id, credential_scope);

        let expires_str = expires_seconds.to_string();
        let mut query_params: Vec<(&str, &str)> = vec![
            ("X-Amz-Algorithm", "AWS4-HMAC-SHA256"),
            ("X-Amz-Credential", &credential),
            ("X-Amz-Date", &amz_date),
            ("X-Amz-Expires", &expires_str),
            ("X-Amz-SignedHeaders", "host"),
        ];
        if let Some(ref token) = self.config.session_token {
            query_params.push(("X-Amz-Security-Token", token));
        }

        query_params.sort_by(|a, b| a.0.cmp(b.0));

        let canonical_query = query_params
            .iter()
            .map(|(k, v)| {
                format!(
                    "{}={}",
                    sigv4_uri_encode(k, true),
                    sigv4_uri_encode(v, true)
                )
            })
            .collect::<Vec<_>>()
            .join("&");

        let canonical_uri = sigv4_uri_encode(parsed_url.path(), false);
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
        let sep = if target_url.contains('?') { '&' } else { '?' };

        Ok(format!(
            "{target_url}{sep}{canonical_query}&X-Amz-Signature={signature}"
        ))
    }

    /// Sign HTTP request headers using AWS SigV4.
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
        let canonical_uri = sigv4_uri_encode(parsed_url.path(), false);
        let canonical_query = parsed_url.query().unwrap_or("");

        let mut headers_to_sign = vec![
            ("host".to_string(), host_header),
            ("x-amz-content-sha256".to_string(), payload_hash.clone()),
            ("x-amz-date".to_string(), amz_date.clone()),
        ];

        if let Some(ref token) = self.config.session_token {
            headers_to_sign.push(("x-amz-security-token".to_string(), token.clone()));
        }

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

        let credential_scope = format!("{date_stamp}/{}/s3/aws4_request", self.config.region);
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

        let mut req = self.agent.put(&target_url);
        for (k, v) in headers {
            req = req.set(&k, &v);
        }

        let resp = match req.send_bytes(&request.body) {
            Ok(r) => r,
            Err(ureq::Error::Status(code, resp)) => {
                let err_body = read_bounded_error_body(resp);
                return Err(map_http_status_error(
                    code,
                    err_body,
                    &request.bucket,
                    &request.object_key,
                ));
            }
            Err(ureq::Error::Transport(t)) => return Err(map_transport_error(t)),
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

        let mut req = self.agent.head(&target_url);
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
                let body = read_bounded_error_body(resp);
                Err(map_http_status_error(code, body, bucket, object_key))
            }
            Err(ureq::Error::Transport(t)) => Err(map_transport_error(t)),
        }
    }

    fn delete_object(&self, bucket: &str, object_key: &str) -> Result<(), StorageError> {
        if bucket.trim().is_empty() || object_key.trim().is_empty() {
            return Err(StorageError::InvalidParameter(
                "bucket and object_key cannot be empty".into(),
            ));
        }

        let (target_url, headers) = self.sign_headers("DELETE", bucket, object_key, b"", None)?;

        let mut req = self.agent.delete(&target_url);
        for (k, v) in headers {
            req = req.set(&k, &v);
        }

        match req.call() {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(404, _)) => Ok(()),
            Err(ureq::Error::Status(code, resp)) => {
                let body = read_bounded_error_body(resp);
                Err(map_http_status_error(code, body, bucket, object_key))
            }
            Err(ureq::Error::Transport(t)) => Err(map_transport_error(t)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn test_s3_client_url_construction_path_style() {
        let config = S3StorageConfig {
            bucket: "my-bucket".into(),
            region: "us-east-1".into(),
            endpoint: Some("http://localhost:9000".into()),
            access_key_id: "minioadmin".into(),
            secret_access_key: "minioadmin".into(),
            force_path_style: true,
            ..Default::default()
        };

        let client = S3Client::new(config).unwrap();
        let url = client.build_url("my-bucket", "images/photo.webp");
        assert_eq!(url, "http://localhost:9000/my-bucket/images/photo.webp");
    }

    #[test]
    fn test_s3_client_url_construction_virtual_hosted_with_and_without_endpoint() {
        let config_aws = S3StorageConfig {
            bucket: "my-bucket".into(),
            region: "us-west-2".into(),
            endpoint: None,
            access_key_id: "key".into(),
            secret_access_key: "secret".into(),
            force_path_style: false,
            ..Default::default()
        };

        let client_aws = S3Client::new(config_aws).unwrap();
        assert_eq!(
            client_aws.build_url("my-bucket", "images/photo.webp"),
            "https://my-bucket.s3.us-west-2.amazonaws.com/images/photo.webp"
        );

        let config_custom = S3StorageConfig {
            bucket: "my-bucket".into(),
            region: "auto".into(),
            endpoint: Some("https://r2.example.com".into()),
            access_key_id: "key".into(),
            secret_access_key: "secret".into(),
            force_path_style: false,
            ..Default::default()
        };
        let client_custom = S3Client::new(config_custom).unwrap();
        assert_eq!(
            client_custom.build_url("my-bucket", "images/photo.webp"),
            "https://my-bucket.r2.example.com/images/photo.webp"
        );
    }

    #[test]
    fn test_sigv4_uri_encoding_and_presigned_url() {
        assert_eq!(
            sigv4_uri_encode("a+b/c d~e@1", false),
            "a%2Bb/c%20d~e%401"
        );
        let config = S3StorageConfig {
            bucket: "my-bucket".into(),
            region: "us-east-1".into(),
            endpoint: Some("http://localhost:9000".into()),
            access_key_id: "AKIAEXAMPLE".into(),
            secret_access_key: "secret".into(),
            session_token: Some("sess/tok+1".into()),
            force_path_style: true,
            ..Default::default()
        };
        let client = S3Client::new(config).unwrap();
        let url = client
            .generate_presigned_url("my-bucket", "images/a+b.webp", 300)
            .unwrap();
        assert!(url.contains("X-Amz-Security-Token=sess%2Ftok%2B1"));
        assert!(url.contains("X-Amz-Signature="));
        assert!(!url.contains(' '));
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
            ..Default::default()
        };

        let client = S3Client::new(config).unwrap();
        let req = UploadRequest::new("my-bucket", "photo.webp", vec![1, 2, 3], "image/webp")
            .with_sha256("wrong_sha256_hash");

        let res = client.put_object(&req);
        assert!(matches!(res, Err(StorageError::ChecksumMismatch { .. })));
    }

    #[test]
    fn test_s3_client_http_put_and_head_against_local_mock_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            // Handle 2 sequential requests: PUT then HEAD
            for i in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut req_line = String::new();
                reader.read_line(&mut req_line).unwrap();

                let mut content_len = 0usize;
                let mut has_auth = false;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if lower.starts_with("authorization:") {
                        has_auth = true;
                    }
                    if let Some(v) = lower.strip_prefix("content-length:") {
                        content_len = v.trim().parse::<usize>().unwrap_or(0);
                    }
                }
                assert!(has_auth, "SigV4 Authorization header must be sent");
                if content_len > 0 {
                    let mut body = vec![0u8; content_len];
                    reader.read_exact(&mut body).unwrap();
                }

                if i == 0 {
                    assert!(req_line.starts_with("PUT /my-bucket/images/abc/optimized.webp"));
                    stream
                        .write_all(b"HTTP/1.1 200 OK\r\nETag: \"abc123\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .unwrap();
                } else {
                    assert!(req_line.starts_with("HEAD /my-bucket/images/abc/optimized.webp"));
                    stream
                        .write_all(b"HTTP/1.1 200 OK\r\nETag: \"abc123\"\r\nContent-Length: 4\r\nContent-Type: image/webp\r\nConnection: close\r\n\r\n")
                        .unwrap();
                }
            }
        });

        let config = S3StorageConfig {
            bucket: "my-bucket".into(),
            region: "us-east-1".into(),
            endpoint: Some(format!("http://{addr}")),
            access_key_id: "test-key".into(),
            secret_access_key: "test-secret".into(),
            force_path_style: true,
            ..Default::default()
        };
        let client = S3Client::new(config).unwrap();
        let req = UploadRequest::new(
            "my-bucket",
            "images/abc/optimized.webp",
            vec![1, 2, 3, 4],
            "image/webp",
        );

        let put_res = client.put_object(&req).unwrap();
        assert_eq!(put_res.etag.as_deref(), Some("abc123"));

        let head_res = client
            .head_object("my-bucket", "images/abc/optimized.webp")
            .unwrap()
            .unwrap();
        assert_eq!(head_res.byte_size, 4);
        assert_eq!(head_res.etag.as_deref(), Some("abc123"));

        server.join().unwrap();
    }
}
