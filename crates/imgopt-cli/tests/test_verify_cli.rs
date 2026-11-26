use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::Command;
use std::thread;
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

    // Missing manifest must fail
    let missing_manifest = temp.path().join("missing.json");
    let status_missing = Command::new(bin_path)
        .args(["verify", "--manifest", missing_manifest.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(!status_missing.success());

    // Corrupted manifest
    let invalid_manifest = temp.path().join("invalid_manifest.json");
    fs::write(&invalid_manifest, "not valid json").unwrap();

    let status_invalid = Command::new(bin_path)
        .args(["verify", "--manifest", invalid_manifest.to_str().unwrap()])
        .status()
        .unwrap();

    assert!(!status_invalid.success());
}

#[test]
fn test_verify_check_storage_detects_missing_404_and_passes_200() {
    let temp = TempDir::new().unwrap();
    let manifest_path = temp.path().join("manifest.json");
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
        "public_url": null,
        "uploaded_at": "2026-08-17T00:00:00Z",
        "verified": true
      },
      "status": "uploaded",
      "known_paths": ["img.png"]
    }
  }
}"#;
    fs::write(&manifest_path, manifest_json).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = thread::spawn(move || {
        // Request 1: 200 OK matching size and ETag
        // Request 2: 404 Not Found
        for i in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut req_line = String::new();
            reader.read_line(&mut req_line).unwrap();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            if i == 0 {
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\nETag: \"etag_val\"\r\nConnection: close\r\n\r\n")
                    .unwrap();
            } else {
                stream
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .unwrap();
            }
        }
    });

    let bin_path = env!("CARGO_BIN_EXE_imgopt");
    let endpoint = format!("http://{addr}");

    let status_ok = Command::new(bin_path)
        .env("IMGOPT_S3_ACCESS_KEY_ID", "test-key")
        .env("IMGOPT_S3_SECRET_ACCESS_KEY", "test-secret")
        .args([
            "verify",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--check-storage",
            "--endpoint",
            &endpoint,
        ])
        .status()
        .unwrap();
    assert!(status_ok.success());

    let status_missing = Command::new(bin_path)
        .env("IMGOPT_S3_ACCESS_KEY_ID", "test-key")
        .env("IMGOPT_S3_SECRET_ACCESS_KEY", "test-secret")
        .args([
            "verify",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--check-storage",
            "--endpoint",
            &endpoint,
        ])
        .status()
        .unwrap();
    assert!(!status_missing.success());

    server.join().unwrap();
}
