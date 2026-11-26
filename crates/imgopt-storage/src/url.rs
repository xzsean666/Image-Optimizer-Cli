use std::path::{Component, Path};

use crate::interface::StorageError;

/// Strategy for deriving remote object keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjectKeyMode {
    /// Content-addressed by source SHA-256 and output format:
    /// `images/{source_sha256}/optimized.{format}`.
    #[default]
    Content,
    /// Content- and config-addressed (safe for `Cache-Control: immutable` when resize/preset changes):
    /// `images/{source_sha256}/{opt_hash_prefix}/optimized.{format}`.
    Variant,
    /// Path-preserving human-readable keys using source relative directory and filename stem:
    /// `{prefix}/{rel_dir}/{stem}.{format}`.
    Path,
}

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

fn prepend_key_prefix(prefix: Option<&str>, base_key: String) -> Result<String, StorageError> {
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

/// Derive a short, URL-safe, deterministic 12-hex-char tag from an optimization key.
fn short_variant_tag(optimization_key: &str) -> Result<String, StorageError> {
    let trimmed = optimization_key.trim();
    if trimmed.is_empty() {
        return Err(StorageError::InvalidParameter(
            "optimization_key cannot be empty".into(),
        ));
    }
    if let Some(tail) = trimmed.rsplit('-').next() {
        if tail.len() >= 12 && tail.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(tail[..12].to_ascii_lowercase());
        }
    }
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(trimmed.as_bytes());
    let hex_digest = hex::encode(hasher.finalize());
    Ok(hex_digest[..12].to_string())
}

/// Derive a deterministic object key with an optional prefix directory path.
pub fn derive_object_key_with_prefix(
    prefix: Option<&str>,
    source_sha256: &str,
    format: &str,
) -> Result<String, StorageError> {
    let base_key = derive_object_key(source_sha256, format)?;
    prepend_key_prefix(prefix, base_key)
}

/// Derive a human-readable, path-preserving object key from a relative source path and output format.
///
/// Format: `{prefix}/{rel_parent}/{stem}.{format}` (omitting `{prefix}/` or `{rel_parent}/` when empty).
pub fn derive_path_object_key(
    prefix: Option<&str>,
    relative_path: &Path,
    format: &str,
) -> Result<String, StorageError> {
    if format.is_empty() || !format.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(StorageError::InvalidParameter(
            "format contains invalid or unsafe characters".into(),
        ));
    }
    let clean_fmt = format.to_ascii_lowercase();

    let mut clean_parts: Vec<String> = Vec::new();
    for comp in relative_path.components() {
        match comp {
            Component::CurDir => continue,
            Component::Normal(seg) => {
                let s = seg.to_str().ok_or_else(|| {
                    StorageError::InvalidParameter(
                        "relative path contains non-UTF-8 characters".into(),
                    )
                })?;
                if s.is_empty()
                    || s == ".."
                    || s.chars()
                        .any(|c| c.is_ascii_control() || matches!(c, '\\' | '?' | '#'))
                {
                    return Err(StorageError::InvalidParameter(
                        "relative path contains invalid or unsafe characters".into(),
                    ));
                }
                clean_parts.push(s.to_string());
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(StorageError::InvalidParameter(
                    "relative path contains path traversal or root components".into(),
                ));
            }
        }
    }

    let filename = clean_parts.pop().ok_or_else(|| {
        StorageError::InvalidParameter("relative path must include a filename".into())
    })?;
    let stem = Path::new(&filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(filename.as_str());

    let out_filename = format!("{stem}.{clean_fmt}");
    let base_key = if clean_parts.is_empty() {
        out_filename
    } else {
        format!("{}/{out_filename}", clean_parts.join("/"))
    };

    prepend_key_prefix(prefix, base_key)
}

/// Derive a deterministic object key according to `ObjectKeyMode` and an optional prefix.
pub fn derive_object_key_for_mode(
    mode: ObjectKeyMode,
    prefix: Option<&str>,
    source_sha256: &str,
    optimization_key: Option<&str>,
    format: &str,
) -> Result<String, StorageError> {
    match mode {
        ObjectKeyMode::Content => derive_object_key_with_prefix(prefix, source_sha256, format),
        ObjectKeyMode::Variant => {
            let opt_key = optimization_key.ok_or_else(|| {
                StorageError::InvalidParameter(
                    "optimization_key is required for ObjectKeyMode::Variant".into(),
                )
            })?;
            let base_key = derive_object_key(source_sha256, format)?;
            let tag = short_variant_tag(opt_key)?;
            let variant_key = base_key.replacen("/optimized.", &format!("/{tag}/optimized."), 1);
            prepend_key_prefix(prefix, variant_key)
        }
        ObjectKeyMode::Path => Err(StorageError::InvalidParameter(
            "relative_path is required for ObjectKeyMode::Path; use derive_path_object_key".into(),
        )),
    }
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

    #[test]
    fn test_derive_object_key_variant_mode() {
        let sha = "a1b2c3d4e5f6";
        let opt_a = "opt-v1-webp-balanced-0123456789abcdef0123456789abcdef";
        let opt_b = "opt-v1-webp-maximum-fedcba9876543210fedcba9876543210";

        let key_a = derive_object_key_for_mode(
            ObjectKeyMode::Variant,
            Some("assets"),
            sha,
            Some(opt_a),
            "webp",
        )
        .unwrap();
        let key_b = derive_object_key_for_mode(
            ObjectKeyMode::Variant,
            Some("assets"),
            sha,
            Some(opt_b),
            "webp",
        )
        .unwrap();

        assert_eq!(
            key_a,
            "assets/images/a1b2c3d4e5f6/0123456789ab/optimized.webp"
        );
        assert_eq!(
            key_b,
            "assets/images/a1b2c3d4e5f6/fedcba987654/optimized.webp"
        );
        assert_ne!(key_a, key_b);
    }

    #[test]
    fn test_derive_path_object_key_preserves_names_and_folders() {
        assert_eq!(
            derive_path_object_key(Some("projectA/full"), Path::new("dog.jpeg"), "avif").unwrap(),
            "projectA/full/dog.avif"
        );
        assert_eq!(
            derive_path_object_key(
                Some("projectA/icon"),
                Path::new("ui/buttons/save.png"),
                "webp"
            )
            .unwrap(),
            "projectA/icon/ui/buttons/save.webp"
        );
        assert_eq!(
            derive_path_object_key(None, Path::new("logo.png"), "webp").unwrap(),
            "logo.webp"
        );
        assert!(derive_path_object_key(Some("proj"), Path::new("../secret.png"), "webp").is_err());
    }
}
