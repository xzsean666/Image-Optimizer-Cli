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

fn make_jpeg_fixture(path: &std::path::Path) {
    let codec = imgopt_codecs::JpegCodec;
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
fn test_optimize_single_file_cli() {
    let temp = TempDir::new().unwrap();
    let input_path = temp.path().join("source.png");
    let output_dir = temp.path().join("out");
    make_png_fixture(&input_path);

    let bin_path = env!("CARGO_BIN_EXE_imgopt");
    let status = Command::new(bin_path)
        .args([
            "optimize",
            input_path.to_str().unwrap(),
            "--output",
            output_dir.to_str().unwrap(),
            "--preset",
            "balanced",
        ])
        .status()
        .unwrap();

    assert!(status.success());
    let expected_out = output_dir.join("source.png");
    assert!(expected_out.exists());
    assert!(fs::metadata(&expected_out).unwrap().len() > 0);

    assert!(!temp.path().join("manifest.json").exists());
}

#[test]
fn test_optimize_prevents_output_collision_and_isolates_failures() {
    let temp = TempDir::new().unwrap();
    let input_dir = temp.path().join("inputs");
    let output_dir = temp.path().join("out");
    fs::create_dir_all(&input_dir).unwrap();

    // Same stem in two formats converting to webp
    make_jpeg_fixture(&input_dir.join("banner.jpg"));
    make_png_fixture(&input_dir.join("banner.png"));
    fs::write(
        input_dir.join("broken.png"),
        b"\x89PNG\r\n\x1a\ncorrupt-payload",
    )
    .unwrap();

    let bin_path = env!("CARGO_BIN_EXE_imgopt");
    let status = Command::new(bin_path)
        .args([
            "optimize",
            input_dir.to_str().unwrap(),
            "--output",
            output_dir.to_str().unwrap(),
            "--format",
            "webp",
        ])
        .status()
        .unwrap();

    // Non-zero exit due to broken.png, but both valid files must exist without overwriting each other
    assert!(!status.success());
    let out_files: Vec<_> = fs::read_dir(&output_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(out_files.len(), 2, "outputs: {out_files:?}");
}

#[test]
fn test_optimize_invalid_path_fails() {
    let bin_path = env!("CARGO_BIN_EXE_imgopt");
    let status = Command::new(bin_path)
        .args(["optimize", "non_existent_file_path_12345.jpg"])
        .status()
        .unwrap();

    assert!(!status.success());
}
