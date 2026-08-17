use serde::{Deserialize, Serialize};
use std::fmt;

use crate::config::OptimizationConfig;
use crate::hash::{hash_bytes, Sha256Digest};
use crate::model::ImageFormat;

/// Version identifier for the optimization key derivation scheme.
pub const OPTIMIZATION_KEY_SCHEME_VERSION: u32 = 1;

/// Deterministic identity for an optimization outcome derived from all inputs that affect output bytes.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OptimizationKey(pub String);

impl OptimizationKey {
    /// Compute a canonical optimization key from content hash, optimizer version, output format, and config.
    pub fn compute(
        source_sha256: &Sha256Digest,
        optimizer_version: &str,
        output_format: ImageFormat,
        config: &OptimizationConfig,
    ) -> Self {
        let canonical_str = format!(
            "imgopt-key-v{}:{}:{}:{}:{}",
            OPTIMIZATION_KEY_SCHEME_VERSION,
            source_sha256.to_hex(),
            optimizer_version.trim(),
            output_format.as_str(),
            config.normalized_key_component(),
        );

        let digest = hash_bytes(canonical_str.as_bytes());
        // Human-readable prefix + format + hex digest for actionable log / manifest inspections
        let key_string = format!(
            "opt-v1-{}-{}-{}",
            output_format.as_str(),
            config.preset.as_str(),
            digest.to_hex()
        );
        Self(key_string)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for OptimizationKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OptimizationKey({})", self.0)
    }
}

impl fmt::Display for OptimizationKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{MetadataPolicy, OptimizationPreset};

    #[test]
    fn test_determinism_across_identical_inputs() {
        let digest = hash_bytes(b"image data");
        let config = OptimizationConfig::from_preset(OptimizationPreset::Balanced);

        let key1 = OptimizationKey::compute(&digest, "0.1.0", ImageFormat::Webp, &config);
        let key2 = OptimizationKey::compute(&digest, "0.1.0", ImageFormat::Webp, &config);

        assert_eq!(key1, key2);
    }

    #[test]
    fn test_mutation_of_every_parameter_changes_key() {
        let digest1 = hash_bytes(b"image 1");
        let digest2 = hash_bytes(b"image 2");
        let config_base = OptimizationConfig::from_preset(OptimizationPreset::Balanced);

        let base_key = OptimizationKey::compute(&digest1, "0.1.0", ImageFormat::Webp, &config_base);

        // Mutate source hash
        let mutated_hash =
            OptimizationKey::compute(&digest2, "0.1.0", ImageFormat::Webp, &config_base);
        assert_ne!(base_key, mutated_hash, "Source hash must change key");

        // Mutate optimizer version
        let mutated_version =
            OptimizationKey::compute(&digest1, "0.2.0", ImageFormat::Webp, &config_base);
        assert_ne!(base_key, mutated_version, "Version must change key");

        // Mutate output format
        let mutated_format =
            OptimizationKey::compute(&digest1, "0.1.0", ImageFormat::Jpeg, &config_base);
        assert_ne!(base_key, mutated_format, "Output format must change key");

        // Mutate preset
        let config_max = OptimizationConfig::from_preset(OptimizationPreset::Maximum);
        let mutated_preset =
            OptimizationKey::compute(&digest1, "0.1.0", ImageFormat::Webp, &config_max);
        assert_ne!(base_key, mutated_preset, "Preset must change key");

        // Mutate quality target
        let mut config_q = config_base.clone();
        config_q.quality_target = 0.95;
        let mutated_q = OptimizationKey::compute(&digest1, "0.1.0", ImageFormat::Webp, &config_q);
        assert_ne!(base_key, mutated_q, "Quality target must change key");

        // Mutate effort
        let mut config_eff = config_base.clone();
        config_eff.effort = 6;
        let mutated_eff =
            OptimizationKey::compute(&digest1, "0.1.0", ImageFormat::Webp, &config_eff);
        assert_ne!(base_key, mutated_eff, "Effort must change key");

        // Mutate metadata policy
        let mut config_meta = config_base.clone();
        config_meta.metadata_policy = MetadataPolicy::PreserveAll;
        let mutated_meta =
            OptimizationKey::compute(&digest1, "0.1.0", ImageFormat::Webp, &config_meta);
        assert_ne!(base_key, mutated_meta, "Metadata policy must change key");

        // Mutate dimensions constraint
        let mut config_dim = config_base.clone();
        config_dim.max_width = Some(1024);
        let mutated_dim =
            OptimizationKey::compute(&digest1, "0.1.0", ImageFormat::Webp, &config_dim);
        assert_ne!(
            base_key, mutated_dim,
            "Max width constraint must change key"
        );
    }

    #[test]
    fn test_serde_roundtrip() {
        let digest = hash_bytes(b"sample");
        let config = OptimizationConfig::default();
        let key = OptimizationKey::compute(&digest, "0.1.0", ImageFormat::Jpeg, &config);

        let json = serde_json::to_string(&key).unwrap();
        let deserialized: OptimizationKey = serde_json::from_str(&json).unwrap();
        assert_eq!(key, deserialized);
    }
}
