use imgopt_codecs::ImageCodec;
use imgopt_core::{
    analyze_image, OptimizationConfig, OptimizationPreset, OptimizationService, ResourceLimits,
};

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

#[test]
fn test_security_input_bytes_limit_exceeded() {
    let bytes = make_png(16, 16);
    let limits = ResourceLimits {
        max_input_bytes: 10, // Unreasonably low limit to test preflight enforcement
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
        max_pixels: 500, // Limit to 500 pixels
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
        max_width: Some(32), // Limit width to 32
        max_height: None,
    };

    let res = analyze_image(&bytes, Some(&limits));
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(err.to_string().contains("exceeds configured limit"));
}

#[test]
fn test_security_optimizer_enforces_limits() {
    let bytes = make_png(32, 32);
    let mut config = OptimizationConfig::from_preset(OptimizationPreset::Balanced);
    config.max_input_bytes = 20; // smaller than PNG header

    let service = OptimizationService::new("0.1.0");
    let res = service.optimize_bytes(&bytes, &config);
    assert!(res.is_err());
}
