pub mod adapter;
pub mod config;
pub mod interface;
pub mod s3;
pub mod upload;
pub mod url;

pub use adapter::StorageSyncAdapter;
pub use config::S3StorageConfig;
pub use interface::{
    MemoryObjectStorage, ObjectMetadata, ObjectStorage, StorageError, UploadRequest, UploadResponse,
};
pub use s3::S3Client;
pub use upload::{upload_and_verify, RetryPolicy, VerifiedUploadResult};
pub use url::{build_public_url, derive_object_key};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert_eq!(VERSION, "0.1.0");
    }
}
