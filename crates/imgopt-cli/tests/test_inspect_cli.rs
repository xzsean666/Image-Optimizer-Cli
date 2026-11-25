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
fn test_inspect_human_and_json() {
    let temp = TempDir::new().unwrap();
    let input_file = temp.path().join("photo.png");
    make_png_fixture(&input_file);

    let bin_path = env!("CARGO_BIN_EXE_imgopt");

    // Human output mode
    let output_human = Command::new(bin_path)
        .args(["inspect", input_file.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output_human.status.success());
    let human_str = String::from_utf8_lossy(&output_human.stdout);
    assert!(human_str.contains("Format:       png"));
    assert!(human_str.contains("Dimensions:   8x8"));

    // JSON output mode
    let output_json = Command::new(bin_path)
        .args(["inspect", input_file.to_str().unwrap(), "--json"])
        .output()
        .unwrap();

    assert!(output_json.status.success());
    let json_str = String::from_utf8_lossy(&output_json.stdout);
    assert!(json_str.contains("\"format\": \"png\""));
    assert!(json_str.contains("\"width\": 8"));
    assert!(json_str.contains("\"height\": 8"));

    // Verify no side-effect files were created in temp dir
    let entries: Vec<_> = fs::read_dir(temp.path()).unwrap().collect();
    assert_eq!(entries.len(), 1);

    // Directory JSON mode must produce a valid JSON array
    let dir_out = Command::new(bin_path)
        .args(["inspect", temp.path().to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(dir_out.status.success());
    let dir_json: serde_json::Value =
        serde_json::from_slice(&dir_out.stdout).expect("directory --json must be valid JSON");
    assert!(dir_json.is_array());
    assert_eq!(dir_json.as_array().unwrap().len(), 1);
}
