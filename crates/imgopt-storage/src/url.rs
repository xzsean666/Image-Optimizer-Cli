use crate::interface::StorageError;

/// Derive a deterministic, path-independent object key in storage.
///
/// Format: `images/{source_sha256}/optimized.{format}`
pub fn derive_object_key(source_sha256: &str, format: &str) -> Result<String, StorageError> {
    if source_sha256.is_empty() {
        return Err(StorageError::InvalidParameter(
            "source_sha256 cannot be empty".into(),
        ));
    }
    if format.is_empty() {
        return Err(StorageError::InvalidParameter(
            "format extension cannot be empty".into(),
        ));
    }

    if !source_sha256
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(StorageError::InvalidParameter(
            "source_sha256 contains invalid or unsafe characters".into(),
        ));
    }

    if !format.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(StorageError::InvalidParameter(
            "format contains invalid or unsafe characters".into(),
        ));
    }

    let clean_sha = source_sha256.to_ascii_lowercase();
    let clean_fmt = format.to_ascii_lowercase();

    Ok(format!("images/{clean_sha}/optimized.{clean_fmt}"))
}

/// Derive a deterministic object key with an optional prefix directory path.
pub fn derive_object_key_with_prefix(
    prefix: Option<&str>,
    source_sha256: &str,
    format: &str,
) -> Result<String, StorageError> {
    let base_key = derive_object_key(source_sha256, format)?;
    if let Some(p) = prefix {
        let clean_p = p.trim().trim_matches('/');
        if !clean_p.is_empty() {
            if clean_p.contains("..")
                || clean_p
                    .chars()
                    .any(|c| c.is_ascii_control() || matches!(c, '\\' | '?' | '#'))
            {
                return Err(StorageError::InvalidParameter(
                    "key prefix contains invalid or path traversal characters".into(),
                ));
            }
            return Ok(format!("{clean_p}/{base_key}"));
        }
    }
    Ok(base_key)
}

/// Construct a safe public URL given a public base URL and an object key.
pub fn build_public_url(public_url_base: &str, object_key: &str) -> Result<String, StorageError> {
    let base = public_url_base.trim();
    if base.is_empty() {
        return Err(StorageError::InvalidParameter(
            "public_url_base cannot be empty".into(),
        ));
    }

    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(StorageError::InvalidParameter(
            "public_url_base must start with http:// or https://".into(),
        ));
    }

    if base
        .chars()
        .any(|c| c.is_ascii_control() || matches!(c, '?' | '#'))
    {
        return Err(StorageError::InvalidParameter(
            "public_url_base contains invalid characters".into(),
        ));
    }

    let clean_base = base.trim_end_matches('/');
    let clean_key = object_key.trim_start_matches('/');

    if clean_key.is_empty()
        || clean_key.contains("..")
        || clean_key
            .chars()
            .any(|c| c.is_ascii_control() || matches!(c, '\\' | '?' | '#'))
    {
        return Err(StorageError::InvalidParameter(
            "object_key contains invalid or path traversal characters".into(),
        ));
    }

    Ok(format!("{clean_base}/{clean_key}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_object_key_stability() {
        let key1 = derive_object_key("a1b2c3d4e5", "webp").unwrap();
        let key2 = derive_object_key("A1B2C3D4E5", "WEBP").unwrap();
        assert_eq!(key1, "images/a1b2c3d4e5/optimized.webp");
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_derive_object_key_with_prefix() {
        let key =
            derive_object_key_with_prefix(Some("my-prefix/uploads"), "a1b2c3d4e5", "webp").unwrap();
        assert_eq!(key, "my-prefix/uploads/images/a1b2c3d4e5/optimized.webp");
        assert!(derive_object_key_with_prefix(Some("a?b=c"), "a1b2c3d4e5", "webp").is_err());
    }

    #[test]
    fn test_derive_object_key_rejects_unsafe_chars() {
        assert!(derive_object_key("../../../etc/passwd", "webp").is_err());
        assert!(derive_object_key("a1b2c3", "webp/..").is_err());
        assert!(derive_object_key("a1b2\0c3", "webp").is_err());
    }

    #[test]
    fn test_build_public_url_clean_joining() {
        let base1 = "https://cdn.example.com";
        let base2 = "https://cdn.example.com/";
        let key1 = "images/abc/optimized.webp";
        let key2 = "/images/abc/optimized.webp";

        let url1 = build_public_url(base1, key1).unwrap();
        let url2 = build_public_url(base2, key2).unwrap();
        assert_eq!(url1, "https://cdn.example.com/images/abc/optimized.webp");
        assert_eq!(url1, url2);
    }

    #[test]
    fn test_build_public_url_invalid_base_or_traversal() {
        assert!(build_public_url("ftp://not-http.com", "img.webp").is_err());
        assert!(build_public_url("https://cdn.example.com", "images/../../escape.webp").is_err());
        assert!(build_public_url("https://cdn.example.com", "images/a?foo=1").is_err());
    }
}
