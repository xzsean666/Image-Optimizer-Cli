use std::io::Read;

use crate::error::{ErrorKind, ImgoptError, PipelineStage, Result};
use crate::model::ImageFormat;

/// Magic bytes constants
const PNG_MAGIC: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const JPEG_MAGIC: &[u8; 3] = &[0xFF, 0xD8, 0xFF];
const RIFF_MAGIC: &[u8; 4] = b"RIFF";
const WEBP_MAGIC: &[u8; 4] = b"WEBP";
const GIF87A_MAGIC: &[u8; 6] = b"GIF87a";
const GIF89A_MAGIC: &[u8; 6] = b"GIF89a";
const BMP_MAGIC: &[u8; 2] = b"BM";
const TIFF_LE_MAGIC: &[u8; 4] = &[0x49, 0x49, 0x2A, 0x00];
const TIFF_BE_MAGIC: &[u8; 4] = &[0x4D, 0x4D, 0x00, 0x2A];

/// Maximum number of bytes needed to inspect header magic numbers.
pub const MAX_HEADER_PROBE_BYTES: usize = 32;

/// Detect image format by inspecting magic bytes of the provided byte slice.
///
/// Bounded probe: examines at most `MAX_HEADER_PROBE_BYTES` bytes.
pub fn detect_format(bytes: &[u8]) -> Result<ImageFormat> {
    if bytes.is_empty() {
        return Err(ImgoptError::new(
            PipelineStage::FormatDetection,
            ErrorKind::InvalidInput("cannot detect format from empty byte buffer".into()),
        ));
    }

    if bytes.len() < 3 {
        return Err(ImgoptError::new(
            PipelineStage::FormatDetection,
            ErrorKind::InvalidInput(
                "file truncated: insufficient bytes for header detection".into(),
            ),
        ));
    }

    // JPEG check: starts with FF D8 FF
    if bytes.starts_with(JPEG_MAGIC) {
        return Ok(ImageFormat::Jpeg);
    }

    // PNG check: 89 50 4E 47 0D 0A 1A 0A
    if bytes.starts_with(PNG_MAGIC) {
        return Ok(ImageFormat::Png);
    }

    // WebP check: RIFF <4 bytes size> WEBP
    if bytes.len() >= 12 && bytes.starts_with(RIFF_MAGIC) && &bytes[8..12] == WEBP_MAGIC {
        return Ok(ImageFormat::Webp);
    }

    // GIF check: GIF87a or GIF89a
    if bytes.starts_with(GIF87A_MAGIC) || bytes.starts_with(GIF89A_MAGIC) {
        return Ok(ImageFormat::Gif);
    }

    if bytes.starts_with(BMP_MAGIC) {
        return Err(ImgoptError::new(
            PipelineStage::FormatDetection,
            ErrorKind::UnsupportedFormat("BMP format is not supported for optimization".into()),
        ));
    }

    if bytes.starts_with(TIFF_LE_MAGIC) || bytes.starts_with(TIFF_BE_MAGIC) {
        return Err(ImgoptError::new(
            PipelineStage::FormatDetection,
            ErrorKind::UnsupportedFormat("TIFF format is not supported for optimization".into()),
        ));
    }

    // Check for ISO Base Media File Format (AVIF, HEIC, etc.)
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let brand = &bytes[8..12];
        if brand == b"avif" || brand == b"avis" {
            return Ok(ImageFormat::Avif);
        }
        let brand_str = String::from_utf8_lossy(brand);
        return Err(ImgoptError::new(
            PipelineStage::FormatDetection,
            ErrorKind::UnsupportedFormat(format!(
                "ISO media container format ('{brand_str}') is not supported for optimization"
            )),
        ));
    }

    Err(ImgoptError::new(
        PipelineStage::FormatDetection,
        ErrorKind::UnsupportedFormat("unrecognized or invalid image header magic bytes".into()),
    ))
}

/// Detect image format by reading a bounded prefix from an `io::Read` stream.
pub fn detect_format_from_reader<R: Read>(mut reader: R) -> Result<ImageFormat> {
    let mut header = [0u8; MAX_HEADER_PROBE_BYTES];
    let mut total_read = 0;
    while total_read < MAX_HEADER_PROBE_BYTES {
        match reader.read(&mut header[total_read..]) {
            Ok(0) => break,
            Ok(n) => total_read += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(ImgoptError::new(
                    PipelineStage::FormatDetection,
                    ErrorKind::Io(e.to_string()),
                ))
            }
        }
    }
    detect_format(&header[..total_read])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_valid_jpeg() {
        let jpeg_bytes = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46];
        assert_eq!(detect_format(&jpeg_bytes).unwrap(), ImageFormat::Jpeg);
    }

    #[test]
    fn test_detect_valid_png() {
        let png_bytes = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00];
        assert_eq!(detect_format(&png_bytes).unwrap(), ImageFormat::Png);
    }

    #[test]
    fn test_detect_valid_webp() {
        let mut webp_bytes = Vec::new();
        webp_bytes.extend_from_slice(b"RIFF");
        webp_bytes.extend_from_slice(&[0x20, 0x00, 0x00, 0x00]); // length
        webp_bytes.extend_from_slice(b"WEBP");
        webp_bytes.extend_from_slice(b"VP8 ");
        assert_eq!(detect_format(&webp_bytes).unwrap(), ImageFormat::Webp);
    }

    #[test]
    fn test_detect_supported_gif_and_avif_and_unsupported_formats() {
        let gif_bytes = b"GIF89a\x01\x00\x01\x00";
        assert_eq!(detect_format(gif_bytes).unwrap(), ImageFormat::Gif);

        let avif_bytes = b"\x00\x00\x00\x1cftypavif\x00\x00\x00\x00";
        assert_eq!(detect_format(avif_bytes).unwrap(), ImageFormat::Avif);

        let bmp_bytes = b"BM\x36\x00\x00\x00";
        assert!(matches!(
            detect_format(bmp_bytes).unwrap_err().kind,
            ErrorKind::UnsupportedFormat(ref msg) if msg.contains("BMP")
        ));

        let tiff_bytes = &[0x49, 0x49, 0x2A, 0x00, 0x08, 0x00];
        assert!(matches!(
            detect_format(tiff_bytes).unwrap_err().kind,
            ErrorKind::UnsupportedFormat(ref msg) if msg.contains("TIFF")
        ));

        let heic_bytes = b"\x00\x00\x00\x1cftypheic\x00\x00\x00\x00";
        assert!(matches!(
            detect_format(heic_bytes).unwrap_err().kind,
            ErrorKind::UnsupportedFormat(ref msg) if msg.contains("heic")
        ));
    }

    #[test]
    fn test_detect_empty_and_truncated() {
        assert!(matches!(
            detect_format(&[]).unwrap_err().kind,
            ErrorKind::InvalidInput(_)
        ));

        assert!(matches!(
            detect_format(&[0xFF, 0xD8]).unwrap_err().kind,
            ErrorKind::InvalidInput(_)
        ));
    }

    #[test]
    fn test_detect_random_bytes() {
        let random = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC];
        assert!(matches!(
            detect_format(&random).unwrap_err().kind,
            ErrorKind::UnsupportedFormat(_)
        ));
    }

    #[test]
    fn test_detect_from_reader() {
        let png_bytes = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00];
        let cursor = std::io::Cursor::new(png_bytes);
        assert_eq!(detect_format_from_reader(cursor).unwrap(), ImageFormat::Png);
    }
}
