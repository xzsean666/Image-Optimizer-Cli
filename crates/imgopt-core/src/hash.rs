use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::str::FromStr;

use crate::error::{ErrorKind, ImgoptError, PipelineStage, Result};

/// Default chunk size for streaming file hashing (64 KB).
pub const HASH_BUFFER_SIZE: usize = 64 * 1024;

/// Strongly typed 256-bit SHA-256 content digest.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256Digest(pub [u8; 32]);

impl Sha256Digest {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self> {
        let clean = s.trim();
        if clean.len() != 64 {
            return Err(ImgoptError::new(
                PipelineStage::Hashing,
                ErrorKind::InvalidInput(format!(
                    "invalid SHA-256 hex length: expected 64 hex chars, got {}",
                    clean.len()
                )),
            ));
        }

        let mut bytes = [0u8; 32];
        hex::decode_to_slice(clean, &mut bytes).map_err(|e| {
            ImgoptError::new(
                PipelineStage::Hashing,
                ErrorKind::InvalidInput(format!("invalid SHA-256 hex string: {e}")),
            )
        })?;

        Ok(Self(bytes))
    }
}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256Digest({})", self.to_hex())
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl FromStr for Sha256Digest {
    type Err = ImgoptError;

    fn from_str(s: &str) -> Result<Self> {
        Self::from_hex(s)
    }
}

impl Serialize for Sha256Digest {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

/// Compute SHA-256 digest of in-memory bytes.
pub fn hash_bytes(bytes: &[u8]) -> Sha256Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let result = hasher.finalize();
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&result);
    Sha256Digest(digest)
}

/// Compute SHA-256 digest by streaming from an `io::Read` source in bounded chunks.
pub fn hash_reader<R: Read>(mut reader: R) -> Result<Sha256Digest> {
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; HASH_BUFFER_SIZE];

    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buffer[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(ImgoptError::new(
                    PipelineStage::Hashing,
                    ErrorKind::Io(format!("failed to read stream during hashing: {e}")),
                ))
            }
        }
    }

    let result = hasher.finalize();
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&result);
    Ok(Sha256Digest(digest))
}

/// Compute SHA-256 content digest of a file at the given path.
pub fn hash_file<P: AsRef<Path>>(path: P) -> Result<Sha256Digest> {
    let p = path.as_ref();
    let file = File::open(p).map_err(|e| {
        ImgoptError::new(
            PipelineStage::Hashing,
            ErrorKind::Io(format!("failed to open file for hashing: {e}")),
        )
        .with_path(p)
    })?;

    hash_reader(file).map_err(|e| e.with_path(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_known_test_vectors() {
        // Empty string
        let empty_digest = hash_bytes(b"");
        assert_eq!(
            empty_digest.to_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        // "abc"
        let abc_digest = hash_bytes(b"abc");
        assert_eq!(
            abc_digest.to_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        // "The quick brown fox jumps over the lazy dog"
        let fox_digest = hash_bytes(b"The quick brown fox jumps over the lazy dog");
        assert_eq!(
            fox_digest.to_hex(),
            "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592"
        );
    }

    #[test]
    fn test_streaming_large_reader_and_chunk_boundaries() {
        // 256 KB of deterministic pattern crossing multiple 64 KB buffers
        let data: Vec<u8> = (0..256 * 1024).map(|i| (i % 251) as u8).collect();
        let expected = hash_bytes(&data);

        let reader = io::Cursor::new(&data);
        let streamed = hash_reader(reader).unwrap();
        assert_eq!(streamed, expected);
    }

    #[test]
    fn test_hash_files_path_independence() {
        let temp = TempDir::new().unwrap();
        let path1 = temp.path().join("first.jpg");
        let path2 = temp.path().join("subdir").join("second.png");
        std::fs::create_dir_all(path2.parent().unwrap()).unwrap();

        let content = b"Identical image file payload across different filenames and timestamps";
        let mut f1 = File::create(&path1).unwrap();
        f1.write_all(content).unwrap();

        let mut f2 = File::create(&path2).unwrap();
        f2.write_all(content).unwrap();

        let digest1 = hash_file(&path1).unwrap();
        let digest2 = hash_file(&path2).unwrap();

        assert_eq!(digest1, digest2);
        assert_eq!(digest1, hash_bytes(content));
    }

    #[test]
    fn test_serde_roundtrip() {
        let digest = hash_bytes(b"serde test");
        let json = serde_json::to_string(&digest).unwrap();
        assert_eq!(json, format!("\"{}\"", digest.to_hex()));

        let deserialized: Sha256Digest = serde_json::from_str(&json).unwrap();
        assert_eq!(digest, deserialized);
    }
}
