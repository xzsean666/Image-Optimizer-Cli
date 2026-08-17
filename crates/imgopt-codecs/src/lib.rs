pub mod interface;
pub mod jpeg;
pub mod png;
pub mod webp;

pub use interface::{
    CodecCapabilities, CodecError, CodecFormat, DecodedPixels, EncodeOptions, EncodedOutput,
    FakeCodec, ImageCodec, PixelFormat,
};
pub use jpeg::JpegCodec;
pub use png::PngCodec;
pub use webp::WebpCodec;

/// Helper function to get default codec for a given format.
pub fn codec_for_format(format: CodecFormat) -> Box<dyn ImageCodec> {
    match format {
        CodecFormat::Jpeg => Box::new(JpegCodec),
        CodecFormat::Png => Box::new(PngCodec),
        CodecFormat::Webp => Box::new(WebpCodec),
    }
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert_eq!(VERSION, "0.1.0");
    }

    #[test]
    fn test_codec_for_format() {
        assert_eq!(
            codec_for_format(CodecFormat::Jpeg).capabilities().format,
            CodecFormat::Jpeg
        );
        assert_eq!(
            codec_for_format(CodecFormat::Png).capabilities().format,
            CodecFormat::Png
        );
        assert_eq!(
            codec_for_format(CodecFormat::Webp).capabilities().format,
            CodecFormat::Webp
        );
    }
}
