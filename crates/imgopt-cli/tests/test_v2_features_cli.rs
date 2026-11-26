use imgopt_codecs::{
    AnimationFrame, DecodedAnimation, DecodedPixels, EncodeOptions, ImageCodec, PixelFormat,
    PngCodec,
};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::Command;
use std::thread;
use tempfile::TempDir;

fn make_png(path: &std::path::Path, w: u32, h: u32) {
    let pixels = DecodedPixels::new(w, h, PixelFormat::Rgb8, vec![140; (w * h * 3) as usize]);
    let bytes = PngCodec
        .encode(&pixels, &EncodeOptions::default())
        .unwrap()
        .bytes;
    fs::write(path, bytes).unwrap();
}

#[test]
fn test_optimize_cli_resize_manifest_json_and_globs() {
    let temp = TempDir::new().unwrap();
    let input_dir = temp.path().join("imgs");
    let sub_skip = input_dir.join("skip");
    fs::create_dir_all(&sub_skip).unwrap();

    let keep_png = input_dir.join("hero.png");
    let ignored_png = sub_skip.join("thumb.png");
    make_png(&keep_png, 32, 16);
    make_png(&ignored_png, 16, 16);

    let out_dir = temp.path().join("dist");
    let manifest_path = temp.path().join("local_manifest.json");
    let json_path = temp.path().join("opt_report.json");
    let bin_path = env!("CARGO_BIN_EXE_imgopt");

    // 1st run: resize hero.png -> 16x8 AVIF, exclude skip/**, write manifest + json
    let out1 = Command::new(bin_path)
        .args([
            "optimize",
            input_dir.to_str().unwrap(),
            "--output",
            out_dir.to_str().unwrap(),
            "--format",
            "force-avif",
            "--resize-width",
            "16",
            "--exclude",
            "skip/**",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--json",
            json_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out1.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out1.stderr)
    );
    assert!(out_dir.join("hero.avif").exists());
    assert!(!out_dir.join("skip/thumb.avif").exists());
    assert!(manifest_path.exists());
    let report1 = fs::read_to_string(&json_path).unwrap();
    assert!(report1.contains("\"optimized_files\": 1"));

    // 2nd run: identical config + manifest -> skipped from cache
    let out2 = Command::new(bin_path)
        .args([
            "optimize",
            input_dir.to_str().unwrap(),
            "--output",
            out_dir.to_str().unwrap(),
            "--format",
            "force-avif",
            "--resize-width",
            "16",
            "--exclude",
            "skip/**",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--json",
            json_path.to_str().unwrap(),
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(out2.status.success());
    let report2 = fs::read_to_string(&json_path).unwrap();
    assert!(report2.contains("\"skipped_files\": 1"));
    assert!(report2.contains("\"optimized_files\": 0"));
}

#[test]
fn test_optimize_and_inspect_animated_gif_cli() {
    let temp = TempDir::new().unwrap();
    let gif_path = temp.path().join("anim.gif");
    let anim = DecodedAnimation {
        width: 8,
        height: 8,
        frames: vec![
            AnimationFrame {
                width: 8,
                height: 8,
                rgba: [255, 0, 0, 255].repeat(64),
                delay_millis: 40,
            },
            AnimationFrame {
                width: 8,
                height: 8,
                rgba: [0, 0, 255, 255].repeat(64),
                delay_millis: 40,
            },
        ],
    };
    let gif_bytes = imgopt_codecs::encode_animated_gif(&anim, &EncodeOptions::default()).unwrap();
    fs::write(&gif_path, gif_bytes).unwrap();

    let bin_path = env!("CARGO_BIN_EXE_imgopt");
    let inspect = Command::new(bin_path)
        .args(["inspect", gif_path.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(inspect.status.success());
    let inspect_str = String::from_utf8_lossy(&inspect.stdout);
    assert!(inspect_str.contains("\"animated\": true"));
    assert!(inspect_str.contains("\"frames\": 2"));

    let out_webp = temp.path().join("anim.webp");
    let opt = Command::new(bin_path)
        .args([
            "optimize",
            gif_path.to_str().unwrap(),
            "--output",
            out_webp.to_str().unwrap(),
            "--format",
            "force-webp",
        ])
        .output()
        .unwrap();
    assert!(
        opt.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&opt.stderr)
    );
    assert!(out_webp.exists());
}

#[test]
fn test_gc_cli_lists_and_deletes_orphans_against_mock_s3() {
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
        // Expect 1) GET list-type=2, 2) DELETE orphan key
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
                assert!(req_line.starts_with("GET /test-bucket?list-type=2"));
                let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult>
  <Name>test-bucket</Name>
  <IsTruncated>false</IsTruncated>
  <Contents><Key>images/abc123/optimized.webp</Key><Size>50</Size><ETag>"etag_val"</ETag></Contents>
  <Contents><Key>images/stale999/optimized.webp</Key><Size>99</Size><ETag>"etag_old"</ETag></Contents>
</ListBucketResult>"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(resp.as_bytes()).unwrap();
            } else {
                assert!(req_line.starts_with("DELETE /test-bucket/images/stale999/optimized.webp"));
                stream
                    .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .unwrap();
            }
        }
    });

    let bin_path = env!("CARGO_BIN_EXE_imgopt");
    let endpoint = format!("http://{addr}");
    let gc_json = temp.path().join("gc_report.json");

    let gc_out = Command::new(bin_path)
        .env("IMGOPT_S3_ACCESS_KEY_ID", "test-key")
        .env("IMGOPT_S3_SECRET_ACCESS_KEY", "test-secret")
        .args([
            "gc",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--endpoint",
            &endpoint,
            "--delete",
            "--json",
            gc_json.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        gc_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&gc_out.stderr)
    );
    let gc_report = fs::read_to_string(&gc_json).unwrap();
    assert!(gc_report.contains("images/stale999/optimized.webp"));
    assert!(gc_report.contains("\"deleted_objects\""));

    server.join().unwrap();
}
