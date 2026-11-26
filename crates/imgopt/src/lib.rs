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

    #[cfg(feature = "storage")]
    #[test]
    fn test_api_optimize_and_upload_helper() {
        let codec = codecs::PngCodec;
        let pixels =
            codecs::DecodedPixels::new(8, 8, codecs::PixelFormat::Rgb8, vec![120; 8 * 8 * 3]);
        let encoded = codec
            .encode(&pixels, &codecs::EncodeOptions::default())
            .unwrap();

        let service = OptimizationService::new(VERSION);
        let config = OptimizationConfig::from_preset(OptimizationPreset::Balanced);
        let opt_res = service.optimize_bytes(&encoded.bytes, &config).unwrap();

        let mem_storage = storage::MemoryObjectStorage::new();
        let adapter = storage::StorageSyncAdapter::new(
            mem_storage,
            Some("https://cdn.example.com".into()),
            Some("uploads".into()),
            storage::RetryPolicy::default(),
        )
        .with_key_mode(storage::ObjectKeyMode::Variant);

        let uploaded = adapter
            .upload_optimization_result("my-bucket", &opt_res)
            .unwrap();
        assert!(uploaded
            .public_url
            .as_deref()
            .is_some_and(|u| u.starts_with("https://cdn.example.com/uploads/images/")));
    }

    #[test]
    fn test_readme_batch_api_signature() {
        let tasks = vec![BatchTask::new("nonexistent.png", ())];
        let (results, summary) =
            execute_batch_scoped(tasks, &BatchOptions::default(), |_path, ()| {
                Ok::<_, ImgoptError>(())
            });
        assert_eq!(results.len(), 1);
        assert_eq!(summary.succeeded, 1);
    }
}
