use imgopt_codecs::ImageCodec;
use std::fs::{self, File};
use std::io::Write;
use std::process::Command;
use tempfile::TempDir;

fn make_png_fixture(path: &std::path::Path) {
    let codec = imgopt_codecs::PngCodec;
    let pixels = imgopt_codecs::DecodedPixels::new(
        8,
        8,
        imgopt_codecs::PixelFormat::Rgb8,
        vec![150; 8 * 8 * 3],
    );
    let output = codec
        .encode(&pixels, &imgopt_codecs::EncodeOptions::default())
        .unwrap();
    let mut file = File::create(path).unwrap();
    file.write_all(&output.bytes).unwrap();
}

#[test]
fn test_sync_cli_dry_run_and_idempotence() {
    let temp = TempDir::new().unwrap();
    let input_dir = temp.path().join("inputs");
    fs::create_dir_all(&input_dir).unwrap();

    let img1 = input_dir.join("img1.png");
    let img2 = input_dir.join("img2.png");
    make_png_fixture(&img1);
    make_png_fixture(&img2);

    let manifest_path = temp.path().join("manifest.json");
    let json_path = temp.path().join("result.json");

    let bin_path = env!("CARGO_BIN_EXE_imgopt");

    // First sync run: optimizes and records in manifest + result JSON
    let status1 = Command::new(bin_path)
        .args([
            "sync",
            input_dir.to_str().unwrap(),
            "--dry-run",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--json",
            json_path.to_str().unwrap(),
            "--preset",
            "balanced",
        ])
        .status()
        .unwrap();

    assert!(status1.success());
    assert!(manifest_path.exists());
    assert!(json_path.exists());

    let json_content = fs::read_to_string(&json_path).unwrap();
    assert!(json_content.contains("\"total_files\": 2"));

    // Second sync run: should skip
    let status2 = Command::new(bin_path)
        .args([
            "sync",
            input_dir.to_str().unwrap(),
            "--dry-run",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--json",
            json_path.to_str().unwrap(),
            "--preset",
            "balanced",
        ])
        .status()
        .unwrap();

    assert!(status2.success());
    let json_content2 = fs::read_to_string(&json_path).unwrap();
    assert!(json_content2.contains("\"skipped_files\": 2"));
}

#[test]
fn test_sync_cli_missing_storage_config_fails() {
    let temp = TempDir::new().unwrap();
    let input_dir = temp.path().join("inputs");
    fs::create_dir_all(&input_dir).unwrap();
    let img = input_dir.join("img.png");
    make_png_fixture(&img);

    let bin_path = env!("CARGO_BIN_EXE_imgopt");
    let status = Command::new(bin_path)
        .env_remove("IMGOPT_S3_BUCKET")
        .env_remove("AWS_S3_BUCKET")
        .args(["sync", input_dir.to_str().unwrap()])
        .status()
        .unwrap();

    assert!(!status.success());
}
