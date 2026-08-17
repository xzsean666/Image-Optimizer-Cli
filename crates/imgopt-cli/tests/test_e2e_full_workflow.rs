use imgopt_codecs::ImageCodec;
use std::fs::{self, File};
use std::io::Write;
use std::process::Command;
use tempfile::TempDir;

fn make_png_fixture(path: &std::path::Path, r: u8, g: u8, b: u8) {
    let codec = imgopt_codecs::PngCodec;
    let pixels = imgopt_codecs::DecodedPixels::new(
        8,
        8,
        imgopt_codecs::PixelFormat::Rgb8,
        [r, g, b].repeat(8 * 8),
    );
    let output = codec
        .encode(&pixels, &imgopt_codecs::EncodeOptions::default())
        .unwrap();
    let mut file = File::create(path).unwrap();
    file.write_all(&output.bytes).unwrap();
}

#[test]
fn test_e2e_full_workflow_lifecycle() {
    let temp = TempDir::new().unwrap();
    let bin_path = env!("CARGO_BIN_EXE_imgopt");

    // 1. Setup sample input images
    let input_dir = temp.path().join("input_images");
    fs::create_dir_all(&input_dir).unwrap();

    let img1 = input_dir.join("image1.png");
    let img2 = input_dir.join("image2.png");
    make_png_fixture(&img1, 200, 50, 50);
    make_png_fixture(&img2, 50, 200, 50);

    // 2. Test inspect command
    let inspect_out = Command::new(bin_path)
        .args(["inspect", img1.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(inspect_out.status.success());
    let inspect_str = String::from_utf8_lossy(&inspect_out.stdout);
    assert!(inspect_str.contains("\"format\": \"png\""));

    // 3. Test local optimize command
    let opt_out_dir = temp.path().join("local_optimized");
    let opt_status = Command::new(bin_path)
        .args([
            "optimize",
            input_dir.to_str().unwrap(),
            "--output",
            opt_out_dir.to_str().unwrap(),
            "--format",
            "webp",
        ])
        .status()
        .unwrap();
    assert!(opt_status.success());
    assert!(opt_out_dir.join("image1.webp").exists());
    assert!(opt_out_dir.join("image2.webp").exists());

    // 4. Test sync command (First run: optimizes, uploads, writes manifest and result JSON)
    let manifest_path = temp.path().join("sync_manifest.json");
    let result_json_path = temp.path().join("sync_result.json");

    let sync_status = Command::new(bin_path)
        .args([
            "sync",
            input_dir.to_str().unwrap(),
            "--dry-run",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--json",
            result_json_path.to_str().unwrap(),
            "--preset",
            "balanced",
        ])
        .status()
        .unwrap();
    assert!(sync_status.success());
    assert!(manifest_path.exists());
    assert!(result_json_path.exists());

    let result1_str = fs::read_to_string(&result_json_path).unwrap();
    assert!(result1_str.contains("\"total_files\": 2"));
    assert!(result1_str.contains("\"optimized_files\": 2"));

    // 5. Test verify command on generated manifest
    let verify_status = Command::new(bin_path)
        .args(["verify", "--manifest", manifest_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(verify_status.success());

    // 6. Test sync idempotence (Second run: everything skipped)
    let sync_status2 = Command::new(bin_path)
        .args([
            "sync",
            input_dir.to_str().unwrap(),
            "--dry-run",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--json",
            result_json_path.to_str().unwrap(),
            "--preset",
            "balanced",
        ])
        .status()
        .unwrap();
    assert!(sync_status2.success());

    let result2_str = fs::read_to_string(&result_json_path).unwrap();
    assert!(result2_str.contains("\"skipped_files\": 2"));
    assert!(result2_str.contains("\"optimized_files\": 0"));

    // 7. Test benchmark command on input images
    let bench_json = temp.path().join("benchmark_report.json");
    let bench_status = Command::new(bin_path)
        .args([
            "benchmark",
            input_dir.to_str().unwrap(),
            "--json",
            bench_json.to_str().unwrap(),
            "--preset",
            "maximum",
        ])
        .status()
        .unwrap();
    assert!(bench_status.success());
    assert!(bench_json.exists());
    let bench_str = fs::read_to_string(&bench_json).unwrap();
    assert!(bench_str.contains("\"total_cases\": 2"));
}

#[test]
fn test_e2e_partial_failure_isolation() {
    let temp = TempDir::new().unwrap();
    let bin_path = env!("CARGO_BIN_EXE_imgopt");

    let input_dir = temp.path().join("mixed_inputs");
    fs::create_dir_all(&input_dir).unwrap();

    let valid_img = input_dir.join("valid.png");
    make_png_fixture(&valid_img, 100, 100, 100);

    // Create a corrupted file with PNG magic bytes but corrupted payload
    let corrupted = input_dir.join("corrupted.png");
    let mut corrupted_bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    corrupted_bytes.extend_from_slice(b"corrupted payload bytes that fail decode");
    fs::write(&corrupted, &corrupted_bytes).unwrap();

    let manifest_path = temp.path().join("partial_manifest.json");
    let result_json = temp.path().join("partial_result.json");

    let sync_status = Command::new(bin_path)
        .args([
            "sync",
            input_dir.to_str().unwrap(),
            "--dry-run",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--json",
            result_json.to_str().unwrap(),
        ])
        .status()
        .unwrap();

    // The batch continues through all items and commits valid entries
    assert!(sync_status.success());
    assert!(manifest_path.exists());
    assert!(result_json.exists());

    let json_str = fs::read_to_string(&result_json).unwrap();
    assert!(json_str.contains("\"total_files\": 2"));
    assert!(json_str.contains("\"optimized_files\": 1"));
    assert!(json_str.contains("\"failed_files\": 1"));
}
