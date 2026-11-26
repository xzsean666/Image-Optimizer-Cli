use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::{ErrorKind, ImgoptError, PipelineStage, Result};
use crate::model::ImageFormat;

/// Predefined optimization profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OptimizationPreset {
    #[default]
    Balanced,
    Maximum,
    Aggressive,
}

impl OptimizationPreset {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::Maximum => "maximum",
            Self::Aggressive => "aggressive",
        }
    }

    /// Base SSIM quality target for this preset.
    pub const fn default_quality_target(&self) -> f64 {
        match self {
            Self::Maximum => 0.97,
            Self::Balanced => 0.92,
            Self::Aggressive => 0.85,
        }
    }

    /// Default encoding effort level (1..=6).
    pub const fn default_effort(&self) -> u8 {
        match self {
            Self::Maximum => 6,
            Self::Balanced => 4,
            Self::Aggressive => 3,
        }
    }
}

impl fmt::Display for OptimizationPreset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for OptimizationPreset {
    type Err = ImgoptError;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "balanced" | "default" => Ok(Self::Balanced),
            "maximum" | "max" | "high" => Ok(Self::Maximum),
            "aggressive" | "min" | "low" => Ok(Self::Aggressive),
            other => Err(ImgoptError::new(
                PipelineStage::Config,
                ErrorKind::Configuration(format!(
                    "unknown preset '{other}', expected balanced|maximum|aggressive"
                )),
            )),
        }
    }
}

/// Policy for metadata preservation vs stripping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MetadataPolicy {
    #[default]
    StripAll,
    /// Strip privacy metadata (EXIF/XMP) while preserving ICC color profile.
    PreserveIcc,
    PreserveAll,
    Custom {
        preserve_exif: bool,
        preserve_icc: bool,
        preserve_xmp: bool,
    },
}

impl MetadataPolicy {
    pub const fn preserves_exif(&self) -> bool {
        match self {
            Self::StripAll | Self::PreserveIcc => false,
            Self::PreserveAll => true,
            Self::Custom { preserve_exif, .. } => *preserve_exif,
        }
    }

    pub const fn preserves_icc(&self) -> bool {
        match self {
            Self::StripAll => false,
            Self::PreserveIcc | Self::PreserveAll => true,
            Self::Custom { preserve_icc, .. } => *preserve_icc,
        }
    }

    pub const fn preserves_xmp(&self) -> bool {
        match self {
            Self::StripAll | Self::PreserveIcc => false,
            Self::PreserveAll => true,
            Self::Custom { preserve_xmp, .. } => *preserve_xmp,
        }
    }
}

/// Output format conversion policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormatPolicy {
    /// Keep the original image format.
    #[default]
    PreserveOriginal,
    /// Allow converting JPEG/PNG/GIF to WebP if size is smaller.
    AllowWebp,
    /// Allow converting JPEG/PNG/WebP/GIF to AVIF if size is smaller.
    AllowAvif,
    /// Evaluate original, WebP, and AVIF and pick the smallest acceptable candidate.
    Auto,
    /// Force a specific target format.
    Force(ImageFormat),
}

/// Resizing behavior when `resize_width` or `resize_height` is configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResizeMode {
    /// Fit within bounds preserving aspect ratio; never upscale beyond original dimensions.
    #[default]
    ScaleDown,
    /// Fit within bounds preserving aspect ratio (allows upscaling).
    Contain,
    /// Fill target dimensions completely by center-cropping excess edges.
    Cover,
    /// Resize to exact target dimensions.
    Exact,
}

impl From<ResizeMode> for imgopt_codecs::ResizeMode {
    fn from(m: ResizeMode) -> Self {
        match m {
            ResizeMode::ScaleDown => Self::ScaleDown,
            ResizeMode::Contain => Self::Contain,
            ResizeMode::Cover => Self::Cover,
            ResizeMode::Exact => Self::Exact,
        }
    }
}

/// Complete optimization configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationConfig {
    pub preset: OptimizationPreset,
    pub quality_target: f64,
    pub effort: u8,
    pub metadata_policy: MetadataPolicy,
    pub output_format_policy: OutputFormatPolicy,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    pub max_input_bytes: u64,
    pub max_pixels: u64,
    #[serde(default)]
    pub resize_width: Option<u32>,
    #[serde(default)]
    pub resize_height: Option<u32>,
    #[serde(default)]
    pub resize_mode: ResizeMode,
    #[serde(default = "default_convert_to_srgb")]
    pub convert_to_srgb: bool,
}

const fn default_convert_to_srgb() -> bool {
    true
}

impl Default for OptimizationConfig {
    fn default() -> Self {
        Self::from_preset(OptimizationPreset::Balanced)
    }
}

impl OptimizationConfig {
    /// Create standard configuration derived from a preset.
    pub fn from_preset(preset: OptimizationPreset) -> Self {
        Self {
            preset,
            quality_target: preset.default_quality_target(),
            effort: preset.default_effort(),
            metadata_policy: MetadataPolicy::StripAll,
            output_format_policy: OutputFormatPolicy::PreserveOriginal,
            max_width: Some(16_384),
            max_height: Some(16_384),
            max_input_bytes: 100 * 1024 * 1024, // 100 MB
            max_pixels: 64 * 1024 * 1024,       // 64 Megapixels (e.g. 8192x8192)
            resize_width: None,
            resize_height: None,
            resize_mode: ResizeMode::ScaleDown,
            convert_to_srgb: true,
        }
    }

    /// Validate configuration invariants.
    pub fn validate(&self) -> Result<()> {
        if !(0.0..=1.0).contains(&self.quality_target) {
            return Err(ImgoptError::new(
                PipelineStage::Config,
                ErrorKind::Configuration(format!(
                    "quality_target must be between 0.0 and 1.0, got {}",
                    self.quality_target
                )),
            ));
        }

        if !(1..=6).contains(&self.effort) {
            return Err(ImgoptError::new(
                PipelineStage::Config,
                ErrorKind::Configuration(format!(
                    "effort level must be between 1 and 6, got {}",
                    self.effort
                )),
            ));
        }

        if let Some(w) = self.max_width {
            if w == 0 {
                return Err(ImgoptError::new(
                    PipelineStage::Config,
                    ErrorKind::Configuration("max_width cannot be 0".into()),
                ));
            }
        }

        if let Some(h) = self.max_height {
            if h == 0 {
                return Err(ImgoptError::new(
                    PipelineStage::Config,
                    ErrorKind::Configuration("max_height cannot be 0".into()),
                ));
            }
        }

        if let Some(w) = self.resize_width {
            if w == 0 {
                return Err(ImgoptError::new(
                    PipelineStage::Config,
                    ErrorKind::Configuration("resize_width cannot be 0".into()),
                ));
            }
        }

        if let Some(h) = self.resize_height {
            if h == 0 {
                return Err(ImgoptError::new(
                    PipelineStage::Config,
                    ErrorKind::Configuration("resize_height cannot be 0".into()),
                ));
            }
        }

        if self.max_input_bytes == 0 {
            return Err(ImgoptError::new(
                PipelineStage::Config,
                ErrorKind::Configuration("max_input_bytes cannot be 0".into()),
            ));
        }

        if self.max_pixels == 0 {
            return Err(ImgoptError::new(
                PipelineStage::Config,
                ErrorKind::Configuration("max_pixels cannot be 0".into()),
            ));
        }

        Ok(())
    }

    /// Return a normalized, deterministic string representing this configuration
    /// for cache and optimization-key derivation.
    pub fn normalized_key_component(&self) -> String {
        format!(
            "preset={}|q={:.4}|effort={}|meta={:?}|format={:?}|max_dim={:?}x{:?}|resize={:?}x{:?}:{:?}|srgb={}",
            self.preset.as_str(),
            self.quality_target,
            self.effort,
            self.metadata_policy,
            self.output_format_policy,
            self.max_width,
            self.max_height,
            self.resize_width,
            self.resize_height,
            self.resize_mode,
            self.convert_to_srgb,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_presets_defaults_and_parsing() {
        let bal = OptimizationConfig::from_preset(OptimizationPreset::Balanced);
        assert_eq!(bal.preset, OptimizationPreset::Balanced);
        assert!((bal.quality_target - 0.92).abs() < 1e-6);
        assert_eq!(bal.effort, 4);
        bal.validate().unwrap();

        let max = OptimizationConfig::from_preset(OptimizationPreset::Maximum);
        assert_eq!(max.preset, OptimizationPreset::Maximum);
        assert!((max.quality_target - 0.97).abs() < 1e-6);
        assert_eq!(max.effort, 6);
        max.validate().unwrap();

        let agg = OptimizationConfig::from_preset(OptimizationPreset::Aggressive);
        assert_eq!(agg.preset, OptimizationPreset::Aggressive);
        assert!((agg.quality_target - 0.85).abs() < 1e-6);
        assert_eq!(agg.effort, 3);
        agg.validate().unwrap();

        assert_eq!(
            "balanced".parse::<OptimizationPreset>().unwrap(),
            OptimizationPreset::Balanced
        );
        assert_eq!(
            "max".parse::<OptimizationPreset>().unwrap(),
            OptimizationPreset::Maximum
        );
        assert_eq!(
            "aggressive".parse::<OptimizationPreset>().unwrap(),
            OptimizationPreset::Aggressive
        );
        assert!("invalid".parse::<OptimizationPreset>().is_err());
    }

    #[test]
    fn test_validation_boundaries() {
        let mut config = OptimizationConfig {
            quality_target: 1.5,
            ..Default::default()
        };
        assert!(config.validate().is_err());

        config.quality_target = -0.1;
        assert!(config.validate().is_err());

        config.quality_target = 0.9;
        config.effort = 0;
        assert!(config.validate().is_err());

        config.effort = 7;
        assert!(config.validate().is_err());

        config.effort = 4;
        config.max_width = Some(0);
        assert!(config.validate().is_err());

        config.max_width = Some(1920);
        config.max_height = Some(0);
        assert!(config.validate().is_err());

        config.max_height = Some(1080);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_normalization_determinism() {
        let c1 = OptimizationConfig::from_preset(OptimizationPreset::Balanced);
        let c2 = OptimizationConfig {
            quality_target: 0.92,
            effort: 4,
            ..Default::default()
        };

        assert_eq!(c1.normalized_key_component(), c2.normalized_key_component());
    }
}
