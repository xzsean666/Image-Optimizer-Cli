use std::fs;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn test_verify_valid_and_invalid_manifest() {
    let temp = TempDir::new().unwrap();
    let valid_manifest = temp.path().join("valid_manifest.json");

    let manifest_json = r#"{
  "schema_version": 1,
  "optimizer_version": "0.1.0",
  "created_at": "2026-08-17T00:00:00Z",
  "updated_at": "2026-08-17T00:00:00Z",
  "entries": {
    "abc123": {
      "source": {
        "sha256": "abc123",
        "format": "png",
        "byte_size": 100,
        "dimensions": [10, 10]
      },
      "optimization": {
        "optimization_key": "opt_key_v1",
        "output_format": "webp",
        "output_size": 50,
        "quality_score": 0.95,
        "preset": "balanced",
        "completed_at": "2026-08-17T00:00:00Z"
      },
      "storage": {
        "bucket": "test-bucket",
        "object_key": "images/abc123/optimized.webp",
        "etag": "etag_val",
        "public_url": "https://cdn.example.com/images/abc123/optimized.webp",
        "uploaded_at": "2026-08-17T00:00:00Z",
        "verified": true
      },
      "status": "uploaded",
      "known_paths": ["img.png"]
    }
  }
}"#;
    fs::write(&valid_manifest, manifest_json).unwrap();

    let bin_path = env!("CARGO_BIN_EXE_imgopt");

    // Valid manifest
    let status_valid = Command::new(bin_path)
        .args(["verify", "--manifest", valid_manifest.to_str().unwrap()])
        .status()
        .unwrap();

    assert!(status_valid.success());

    // Corrupted manifest
    let invalid_manifest = temp.path().join("invalid_manifest.json");
    fs::write(&invalid_manifest, "not valid json").unwrap();

    let status_invalid = Command::new(bin_path)
        .args(["verify", "--manifest", invalid_manifest.to_str().unwrap()])
        .status()
        .unwrap();

    assert!(!status_invalid.success());
}
