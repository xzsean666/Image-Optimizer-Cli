use std::fmt;

/// Codec format identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodecFormat {
    Jpeg,
    Png,
    Webp,
}

impl CodecFormat {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Jpeg => "jpeg",
            Self::Png => "png",
            Self::Webp => "webp",
        }
    }
}

impl fmt::Display for CodecFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Pixel layout and channel formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    Grayscale8,
    GrayscaleAlpha8,
    Rgb8,
    Rgba8,
}

impl PixelFormat {
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
}

/// Errors originating from image codec operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    DecodeError(String),
    EncodeError(String),
    UnsupportedFormat(String),
    InvalidParameter(String),
    Io(String),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DecodeError(msg) => write!(f, "decode error: {msg}"),
            Self::EncodeError(msg) => write!(f, "encode error: {msg}"),
            Self::UnsupportedFormat(msg) => write!(f, "unsupported format: {msg}"),
            Self::InvalidParameter(msg) => write!(f, "invalid parameter: {msg}"),
            Self::Io(msg) => write!(f, "I/O error: {msg}"),
        }
    }
}

impl std::error::Error for CodecError {}

/// Decoded pixel buffer with metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedPixels {
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
    pub data: Vec<u8>,
}

impl DecodedPixels {
    pub fn new(width: u32, height: u32, pixel_format: PixelFormat, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            pixel_format,
            data,
        }
    }

    pub fn expected_byte_len(&self) -> usize {
        (self.width as usize) * (self.height as usize) * self.pixel_format.bytes_per_pixel()
    }

    pub fn is_valid(&self) -> bool {
        self.data.len() == self.expected_byte_len()
    }
}

/// Encoding parameters passed to codec adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeOptions {
    pub quality: u8,
    pub effort: u8,
    pub lossless: bool,
    pub preserve_metadata: bool,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            quality: 80,
            effort: 4,
            lossless: false,
            preserve_metadata: false,
        }
    }
}

/// Output of a successful encode operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedOutput {
    pub format: CodecFormat,
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub effective_options: EncodeOptions,
}

/// Capabilities exposed by a codec implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodecCapabilities {
    pub format: CodecFormat,
    pub supports_lossless: bool,
    pub supports_lossy: bool,
    pub supports_alpha: bool,
}

/// Unified boundary interface for image codecs.
pub trait ImageCodec: Send + Sync {
    fn capabilities(&self) -> CodecCapabilities;
    fn decode(&self, input_bytes: &[u8]) -> Result<DecodedPixels, CodecError>;
    fn encode(
        &self,
        pixels: &DecodedPixels,
        options: &EncodeOptions,
    ) -> Result<EncodedOutput, CodecError>;
}

/// Fake codec implementation for unit testing and interface contract validation.
#[derive(Debug, Clone)]
pub struct FakeCodec {
    pub capabilities: CodecCapabilities,
    pub should_fail_decode: bool,
    pub should_fail_encode: bool,
}

impl FakeCodec {
    pub fn new(format: CodecFormat) -> Self {
        Self {
            capabilities: CodecCapabilities {
                format,
                supports_lossless: true,
                supports_lossy: true,
                supports_alpha: true,
            },
            should_fail_decode: false,
            should_fail_encode: false,
        }
    }
}

impl ImageCodec for FakeCodec {
    fn capabilities(&self) -> CodecCapabilities {
        self.capabilities
    }

    fn decode(&self, input_bytes: &[u8]) -> Result<DecodedPixels, CodecError> {
        if self.should_fail_decode {
            return Err(CodecError::DecodeError("fake decode failed".into()));
        }
        if input_bytes.is_empty() {
            return Err(CodecError::DecodeError("empty input bytes".into()));
        }
        // Return a mock 2x2 RGB image
        Ok(DecodedPixels::new(2, 2, PixelFormat::Rgb8, vec![255; 12]))
    }

    fn encode(
        &self,
        pixels: &DecodedPixels,
        options: &EncodeOptions,
    ) -> Result<EncodedOutput, CodecError> {
        if self.should_fail_encode {
            return Err(CodecError::EncodeError("fake encode failed".into()));
        }
        if !pixels.is_valid() {
            return Err(CodecError::InvalidParameter(
                "invalid pixel buffer length".into(),
            ));
        }
        // Produce simulated output bytes
        let mut bytes =
            format!("FAKE-{}-Q{}", self.capabilities.format, options.quality).into_bytes();
        bytes.extend_from_slice(&pixels.data);

        Ok(EncodedOutput {
            format: self.capabilities.format,
            bytes,
            width: pixels.width,
            height: pixels.height,
            effective_options: options.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fake_codec_decode_and_encode() {
        let codec = FakeCodec::new(CodecFormat::Jpeg);
        assert_eq!(codec.capabilities().format, CodecFormat::Jpeg);

        let decoded = codec.decode(b"mock_jpeg_bytes").unwrap();
        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 2);
        assert!(decoded.is_valid());

        let options = EncodeOptions {
            quality: 85,
            effort: 5,
            lossless: false,
            preserve_metadata: false,
        };

        let encoded = codec.encode(&decoded, &options).unwrap();
        assert_eq!(encoded.format, CodecFormat::Jpeg);
        assert_eq!(encoded.width, 2);
        assert_eq!(encoded.height, 2);
        assert_eq!(encoded.effective_options, options);
    }

    #[test]
    fn test_fake_codec_error_conditions() {
        let mut codec = FakeCodec::new(CodecFormat::Png);
        codec.should_fail_decode = true;
        assert!(matches!(
            codec.decode(b"test"),
            Err(CodecError::DecodeError(_))
        ));

        codec.should_fail_decode = false;
        codec.should_fail_encode = true;
        let pixels = DecodedPixels::new(2, 2, PixelFormat::Rgb8, vec![0; 12]);
        assert!(matches!(
            codec.encode(&pixels, &EncodeOptions::default()),
            Err(CodecError::EncodeError(_))
        ));

        codec.should_fail_encode = false;
        let invalid_pixels = DecodedPixels::new(2, 2, PixelFormat::Rgb8, vec![0; 5]);
        assert!(matches!(
            codec.encode(&invalid_pixels, &EncodeOptions::default()),
            Err(CodecError::InvalidParameter(_))
        ));
    }
}
