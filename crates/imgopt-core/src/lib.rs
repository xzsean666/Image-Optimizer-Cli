pub mod analyzer;
pub mod benchmark;
pub mod candidate;
pub mod config;
pub mod detect;
pub mod error;
pub mod executor;
pub mod hash;
pub mod model;
pub mod optimization_key;
pub mod optimizer;
pub mod scanner;
pub mod search;
pub mod sync;
pub mod validation;

pub use analyzer::{analyze_image, AnalysisResult, ImageCharacteristics, ResourceLimits};
pub use benchmark::{run_benchmark, BenchmarkCaseResult, BenchmarkReport, BenchmarkSummary};
pub use candidate::{
    compare_candidates, select_best_candidate, OptimizationCandidate, RankingCriteria,
};
pub use config::{MetadataPolicy, OptimizationConfig, OptimizationPreset, OutputFormatPolicy};
pub use detect::{detect_format, detect_format_from_reader, MAX_HEADER_PROBE_BYTES};
pub use error::{ErrorKind, ImgoptError, PipelineStage, Result, StorageErrorKind};
pub use executor::{
    execute_batch, BatchItemResult, BatchOptions, BatchSummary, BatchTask, ConcurrencyProbe,
};
pub use hash::{hash_bytes, hash_file, hash_reader, Sha256Digest, HASH_BUFFER_SIZE};
pub use model::{
    AssetId, ColorType, Dimensions, EncodedCandidate, ImageFormat, ImageMetadata, ImageProfile,
    RawImage, SourceReference,
};
pub use optimization_key::{OptimizationKey, OPTIMIZATION_KEY_SCHEME_VERSION};
pub use optimizer::{OptimizationResult, OptimizationService};
pub use scanner::{scan_path, DiscoveredFile, ScanOptions, ScanResult};
pub use search::{search_best_candidate, SearchDiagnostics, SearchOptions, SearchResult};
pub use sync::{
    SyncItemResult, SyncItemStatus, SyncManifest, SyncPlanAction, SyncRunResult, SyncService,
    SyncStorage, SyncStorageOutput,
};
pub use validation::{validate_candidate_output, ValidatedOutput, ValidationCriteria};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert_eq!(VERSION, "0.1.0");
        assert_eq!(imgopt_codecs::VERSION, "0.1.0");
        assert_eq!(imgopt_quality::VERSION, "0.1.0");
    }
}
