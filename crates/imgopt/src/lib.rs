//! Rust library facade for the `imgopt` image optimization engine.
//!
//! By default this crate re-exports the core optimization pipeline (`imgopt_core`),
//! codecs (`imgopt_codecs`), and quality metrics (`imgopt_quality`).
//! Enable optional features `manifest`, `storage`, or `full` to include
//! manifest persistence and S3-compatible object storage sync support.

pub use imgopt_core::*;

pub use imgopt_codecs as codecs;
pub use imgopt_core as core;
pub use imgopt_quality as quality;

#[cfg(feature = "manifest")]
pub use imgopt_manifest as manifest;

#[cfg(feature = "storage")]
pub use imgopt_storage as storage;

#[cfg(test)]
mod tests {
    use super::*;
    use codecs::{DecodedPixels, EncodeOptions, ImageCodec, PixelFormat, PngCodec};

    #[test]
    fn test_facade_reexports_and_optimize_bytes() {
        let codec = PngCodec;
        let pixels = DecodedPixels::new(8, 8, PixelFormat::Rgb8, vec![120; 8 * 8 * 3]);
        let encoded = codec.encode(&pixels, &EncodeOptions::default()).unwrap();

        let service = OptimizationService::new(VERSION);
        let config = OptimizationConfig::from_preset(OptimizationPreset::Balanced);
        let result = service.optimize_bytes(&encoded.bytes, &config).unwrap();

        assert_eq!(result.effective_format, ImageFormat::Png);
        assert!(!result.validated_output.bytes.is_empty());
        assert!(result.quality_score > 0.99);
    }

    #[cfg(feature = "manifest")]
    #[test]
    fn test_manifest_feature_reexport() {
        assert_eq!(manifest::VERSION, VERSION);
    }

    #[cfg(feature = "storage")]
    #[test]
    fn test_storage_feature_reexport() {
        assert_eq!(storage::VERSION, VERSION);
    }
}
