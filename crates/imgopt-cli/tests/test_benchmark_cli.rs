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
fn test_benchmark_cli_run() {
    let temp = TempDir::new().unwrap();
    let dataset = temp.path().join("dataset");
    fs::create_dir_all(&dataset).unwrap();

    let img1 = dataset.join("img1.png");
    let img2 = dataset.join("img2.png");
    make_png_fixture(&img1);
    make_png_fixture(&img2);

    let json_report = temp.path().join("report.json");
    let bin_path = env!("CARGO_BIN_EXE_imgopt");

    let status = Command::new(bin_path)
        .args([
            "benchmark",
            dataset.to_str().unwrap(),
            "--json",
            json_report.to_str().unwrap(),
            "--preset",
            "balanced",
        ])
        .status()
        .unwrap();

    assert!(status.success());
    assert!(json_report.exists());

    let report_str = fs::read_to_string(&json_report).unwrap();
    assert!(report_str.contains("\"total_cases\": 2"));
    assert!(report_str.contains("\"successful_cases\": 2"));
}
