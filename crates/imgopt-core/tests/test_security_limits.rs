use imgopt_codecs::ImageCodec;
use imgopt_core::{
    analyze_image, OptimizationConfig, OptimizationPreset, OptimizationService, PipelineStage,
    ResourceLimits,
};
use tempfile::TempDir;

fn make_png(width: u32, height: u32) -> Vec<u8> {
    let codec = imgopt_codecs::PngCodec;
    let pixels = imgopt_codecs::DecodedPixels::new(
        width,
        height,
        imgopt_codecs::PixelFormat::Rgb8,
        vec![120; (width * height * 3) as usize],
    );
    let output = codec
        .encode(&pixels, &imgopt_codecs::EncodeOptions::default())
        .unwrap();
    output.bytes
}

fn png_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Craft a minimal PNG header claiming huge dimensions (decompression bomb) with valid IHDR CRC.
fn make_png_bomb_header(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    // IHDR length = 13
    bytes.extend_from_slice(&13u32.to_be_bytes());
    let mut ihdr = Vec::with_capacity(17);
    ihdr.extend_from_slice(b"IHDR");
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(2); // RGB
    ihdr.push(0); // compression
    ihdr.push(0); // filter
    ihdr.push(0); // interlace
    let crc = png_crc32(&ihdr);
    bytes.extend_from_slice(&ihdr);
    bytes.extend_from_slice(&crc.to_be_bytes());
    bytes
}

#[test]
fn test_security_input_bytes_limit_exceeded() {
    let bytes = make_png(16, 16);
    let limits = ResourceLimits {
        max_input_bytes: 10,
        max_pixels: 10_000_000,
        max_width: None,
        max_height: None,
    };

    let res = analyze_image(&bytes, Some(&limits));
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(err.to_string().contains("exceeds configured limit"));
}

#[test]
fn test_security_max_pixels_limit_exceeded() {
    let bytes = make_png(32, 32); // 1024 pixels
    let limits = ResourceLimits {
        max_input_bytes: 10_000_000,
        max_pixels: 500,
        max_width: None,
        max_height: None,
    };

    let res = analyze_image(&bytes, Some(&limits));
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(err.to_string().contains("exceeds configured limit"));
}

#[test]
fn test_security_max_dimensions_limit_exceeded() {
    let bytes = make_png(64, 16);
    let limits = ResourceLimits {
        max_input_bytes: 10_000_000,
        max_pixels: 10_000_000,
        max_width: Some(32),
        max_height: None,
    };

    let res = analyze_image(&bytes, Some(&limits));
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(err.to_string().contains("exceeds configured limit"));
}

#[test]
fn test_security_decompression_bomb_rejected_before_allocation() {
    // 100,000 x 100,000 pixels (~30 GB uncompressed) in a 33-byte header
    let bomb = make_png_bomb_header(100_000, 100_000);
    let res = analyze_image(&bomb, Some(&ResourceLimits::default()));
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(err.stage, PipelineStage::ResourceCheck);
}

#[test]
fn test_security_optimizer_enforces_limits_and_file_preflight() {
    let bytes = make_png(32, 32);
    let mut config = OptimizationConfig::from_preset(OptimizationPreset::Balanced);
    config.max_input_bytes = 20;

    let service = OptimizationService::new("0.1.0");
    let res = service.optimize_bytes(&bytes, &config);
    assert!(res.is_err());

    let temp = TempDir::new().unwrap();
    let file_path = temp.path().join("oversized.png");
    std::fs::write(&file_path, &bytes).unwrap();
    let file_err = service.optimize_file(&file_path, &config).unwrap_err();
    assert_eq!(file_err.stage, PipelineStage::ResourceCheck);
}
