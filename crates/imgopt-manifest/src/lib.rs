pub mod adapter;
pub mod dedup;
pub mod model;
pub mod result;
pub mod store;

pub use adapter::ManifestSyncAdapter;
pub use dedup::{evaluate_action, ExecutionPlanAction};
pub use model::{
    EntryStatus, Manifest, ManifestEntry, ManifestOptimizationRecord, ManifestSourceRecord,
    ManifestStorageRecord, CURRENT_SCHEMA_VERSION,
};
pub use result::{RunItemStatus, RunResult, RunResultRecord, RunSummary};
pub use store::{ManifestLock, ManifestStore};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert_eq!(VERSION, "0.1.0");
    }
}
