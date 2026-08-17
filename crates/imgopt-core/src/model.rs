use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::{ErrorKind, ImgoptError, PipelineStage, Result};

/// Supported image formats for Image Optimizer V1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    #[serde(alias = "jpg")]
    Jpeg,
    Png,
    Webp,
}

impl ImageFormat {
    /// Returns canonical file extension without leading dot.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Webp => "webp",
        }
    }

    /// Returns the MIME content type.
    pub fn mime_type(&self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Webp => "image/webp",
        }
    }

    /// Whether this format supports transparency / alpha channel.
    pub fn supports_alpha(&self) -> bool {
        match self {
            Self::Jpeg => false,
            Self::Png | Self::Webp => true,
        }
    }

    /// Parse format from standard file extension (case-insensitive, optional leading dot).
    pub fn from_extension(ext: &str) -> Option<Self> {
        let cleaned = ext.trim().trim_start_matches('.').to_ascii_lowercase();
        match cleaned.as_str() {
            "jpg" | "jpeg" | "jpe" | "jif" | "jfif" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            "webp" => Some(Self::Webp),
            _ => None,
        }
    }

    /// Parse format from MIME type string.
    pub fn from_mime_type(mime: &str) -> Option<Self> {
        let cleaned = mime.trim().to_ascii_lowercase();
        match cleaned.as_str() {
            "image/jpeg" | "image/jpg" | "image/pjpeg" => Some(Self::Jpeg),
            "image/png" => Some(Self::Png),
            "image/webp" => Some(Self::Webp),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Jpeg => "jpeg",
            Self::Png => "png",
            Self::Webp => "webp",
        }
    }
}

impl fmt::Display for ImageFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for ImageFormat {
    type Err = ImgoptError;

    fn from_str(s: &str) -> Result<Self> {
        let cleaned = s.trim().to_ascii_lowercase();
        match cleaned.as_str() {
            "jpeg" | "jpg" => Ok(Self::Jpeg),
            "png" => Ok(Self::Png),
            "webp" => Ok(Self::Webp),
            unsupported => Err(ImgoptError::new(
                PipelineStage::FormatDetection,
                ErrorKind::UnsupportedFormat(format!(
                    "unknown or unsupported format: '{unsupported}'"
                )),
            )),
        }
    }
}

/// Image 2D dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

impl Dimensions {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub const fn pixel_count(&self) -> u64 {
        (self.width as u64) * (self.height as u64)
    }

    pub fn aspect_ratio(&self) -> f64 {
        if self.height == 0 {
            0.0
        } else {
            self.width as f64 / self.height as f64
        }
    }
}

impl fmt::Display for Dimensions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

/// Color representation and channel layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorType {
    Grayscale8,
    GrayscaleAlpha8,
    Rgb8,
    Rgba8,
}

impl ColorType {
    pub const fn bytes_per_pixel(&self) -> usize {
        match self {
            Self::Grayscale8 => 1,
            Self::GrayscaleAlpha8 => 2,
            Self::Rgb8 => 3,
            Self::Rgba8 => 4,
        }
    }

    pub const fn has_alpha(&self) -> bool {
        match self {
            Self::GrayscaleAlpha8 | Self::Rgba8 => true,
            Self::Grayscale8 | Self::Rgb8 => false,
        }
    }

    pub const fn channels(&self) -> u8 {
        match self {
            Self::Grayscale8 => 1,
            Self::GrayscaleAlpha8 => 2,
            Self::Rgb8 => 3,
            Self::Rgba8 => 4,
        }
    }
}

/// Summary of metadata chunks present in source image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct ImageMetadata {
    pub has_exif: bool,
    pub has_icc: bool,
    pub has_xmp: bool,
}

/// Profile and characteristics of an image file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageProfile {
    pub format: ImageFormat,
    pub dimensions: Dimensions,
    pub color_type: ColorType,
    pub byte_size: u64,
    pub metadata: ImageMetadata,
}

/// Decoded raw image pixel buffer, codec-neutral.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawImage {
    pub dimensions: Dimensions,
    pub color_type: ColorType,
    pub data: Vec<u8>,
}

impl RawImage {
    pub fn new(dimensions: Dimensions, color_type: ColorType, data: Vec<u8>) -> Self {
        Self {
            dimensions,
            color_type,
            data,
        }
    }

    pub fn expected_byte_len(&self) -> usize {
        (self.dimensions.width as usize)
            * (self.dimensions.height as usize)
            * self.color_type.bytes_per_pixel()
    }

    pub fn is_valid(&self) -> bool {
        self.data.len() == self.expected_byte_len()
    }
}

/// An encoded image candidate produced during optimization search.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedCandidate {
    pub format: ImageFormat,
    pub bytes: Vec<u8>,
    pub quality_score: Option<f64>,
    pub parameters_description: String,
}

impl EncodedCandidate {
    pub fn byte_size(&self) -> u64 {
        self.bytes.len() as u64
    }
}

/// Content-addressed unique identifier for an asset (SHA-256 hex string).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AssetId(pub String);

impl AssetId {
    pub fn new(hash: impl Into<String>) -> Self {
        Self(hash.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Content identity and basic properties of a source image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceReference {
    pub source_sha256: String,
    pub format: ImageFormat,
    pub byte_size: u64,
    pub dimensions: Dimensions,
}

impl From<imgopt_codecs::CodecFormat> for ImageFormat {
    fn from(f: imgopt_codecs::CodecFormat) -> Self {
        match f {
            imgopt_codecs::CodecFormat::Jpeg => Self::Jpeg,
            imgopt_codecs::CodecFormat::Png => Self::Png,
            imgopt_codecs::CodecFormat::Webp => Self::Webp,
        }
    }
}

impl From<ImageFormat> for imgopt_codecs::CodecFormat {
    fn from(f: ImageFormat) -> Self {
        match f {
            ImageFormat::Jpeg => Self::Jpeg,
            ImageFormat::Png => Self::Png,
            ImageFormat::Webp => Self::Webp,
        }
    }
}

impl From<imgopt_codecs::PixelFormat> for ColorType {
    fn from(p: imgopt_codecs::PixelFormat) -> Self {
        match p {
            imgopt_codecs::PixelFormat::Grayscale8 => Self::Grayscale8,
            imgopt_codecs::PixelFormat::GrayscaleAlpha8 => Self::GrayscaleAlpha8,
            imgopt_codecs::PixelFormat::Rgb8 => Self::Rgb8,
            imgopt_codecs::PixelFormat::Rgba8 => Self::Rgba8,
        }
    }
}

impl From<ColorType> for imgopt_codecs::PixelFormat {
    fn from(c: ColorType) -> Self {
        match c {
            ColorType::Grayscale8 => Self::Grayscale8,
            ColorType::GrayscaleAlpha8 => Self::GrayscaleAlpha8,
            ColorType::Rgb8 => Self::Rgb8,
            ColorType::Rgba8 => Self::Rgba8,
        }
    }
}

impl From<imgopt_codecs::DecodedPixels> for RawImage {
    fn from(d: imgopt_codecs::DecodedPixels) -> Self {
        Self {
            dimensions: Dimensions::new(d.width, d.height),
            color_type: d.pixel_format.into(),
            data: d.data,
        }
    }
}

impl From<RawImage> for imgopt_codecs::DecodedPixels {
    fn from(r: RawImage) -> Self {
        Self {
            width: r.dimensions.width,
            height: r.dimensions.height,
            pixel_format: r.color_type.into(),
            data: r.data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_parsing_and_extension() {
        assert_eq!(ImageFormat::from_extension("jpg"), Some(ImageFormat::Jpeg));
        assert_eq!(
            ImageFormat::from_extension(".JPEG"),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(ImageFormat::from_extension("png"), Some(ImageFormat::Png));
        assert_eq!(ImageFormat::from_extension("webp"), Some(ImageFormat::Webp));
        assert_eq!(ImageFormat::from_extension("gif"), None);

        assert_eq!(
            ImageFormat::from_mime_type("image/jpeg"),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(
            ImageFormat::from_mime_type("image/png"),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            ImageFormat::from_mime_type("image/webp"),
            Some(ImageFormat::Webp)
        );
        assert_eq!(ImageFormat::from_mime_type("image/avif"), None);

        assert_eq!(ImageFormat::Jpeg.extension(), "jpg");
        assert_eq!(ImageFormat::Png.extension(), "png");
        assert_eq!(ImageFormat::Webp.extension(), "webp");

        assert_eq!("jpeg".parse::<ImageFormat>().unwrap(), ImageFormat::Jpeg);
        assert_eq!("png".parse::<ImageFormat>().unwrap(), ImageFormat::Png);
        assert_eq!("webp".parse::<ImageFormat>().unwrap(), ImageFormat::Webp);
        assert!("gif".parse::<ImageFormat>().is_err());
    }

    #[test]
    fn test_dimensions() {
        let dim = Dimensions::new(1920, 1080);
        assert_eq!(dim.pixel_count(), 1920 * 1080);
        assert!((dim.aspect_ratio() - (1920.0 / 1080.0)).abs() < 1e-6);
        assert_eq!(dim.to_string(), "1920x1080");
    }

    #[test]
    fn test_raw_image_validation() {
        let dim = Dimensions::new(2, 2);
        let raw = RawImage::new(dim, ColorType::Rgb8, vec![0u8; 12]);
        assert_eq!(raw.expected_byte_len(), 12);
        assert!(raw.is_valid());

        let invalid_raw = RawImage::new(dim, ColorType::Rgb8, vec![0u8; 10]);
        assert!(!invalid_raw.is_valid());
    }

    #[test]
    fn test_json_roundtrips() {
        let profile = ImageProfile {
            format: ImageFormat::Webp,
            dimensions: Dimensions::new(800, 600),
            color_type: ColorType::Rgba8,
            byte_size: 12345,
            metadata: ImageMetadata {
                has_exif: false,
                has_icc: true,
                has_xmp: false,
            },
        };

        let json = serde_json::to_string(&profile).unwrap();
        let deserialized: ImageProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(profile, deserialized);

        let source_ref = SourceReference {
            source_sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                .into(),
            format: ImageFormat::Jpeg,
            byte_size: 4096,
            dimensions: Dimensions::new(400, 300),
        };

        let source_json = serde_json::to_string(&source_ref).unwrap();
        let deserialized_src: SourceReference = serde_json::from_str(&source_json).unwrap();
        assert_eq!(source_ref, deserialized_src);
    }
}
