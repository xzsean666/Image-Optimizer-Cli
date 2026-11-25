use serde::{Deserialize, Serialize};

use crate::analyzer::{probe_metadata_markers, ResourceLimits};
use crate::config::MetadataPolicy;
use crate::detect::detect_format;
use crate::error::{ErrorKind, ImgoptError, PipelineStage, Result};
use crate::model::{Dimensions, ImageFormat};

/// Requirements and invariants for validating candidate output bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationCriteria {
    pub expected_format: ImageFormat,
    pub expected_dimensions: Dimensions,
    pub metadata_policy: MetadataPolicy,
    pub max_allowed_bytes: Option<u64>,
}

impl ValidationCriteria {
    pub fn new(expected_format: ImageFormat, expected_dimensions: Dimensions) -> Self {
        Self {
            expected_format,
            expected_dimensions,
            metadata_policy: MetadataPolicy::StripAll,
            max_allowed_bytes: None,
        }
    }
}

/// A fully decoded and verified candidate output ready for disk write or storage upload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidatedOutput {
    pub format: ImageFormat,
    #[serde(skip_serializing, default)]
    pub bytes: Vec<u8>,
    pub byte_size: u64,
    pub original_size: u64,
    pub dimensions: Dimensions,
    pub savings_bytes: i64,
    pub savings_ratio: f64,
}

/// Validate candidate bytes against integrity, dimensional, metadata, and format invariants.
pub fn validate_candidate_output(
    candidate_bytes: &[u8],
    original_size: u64,
    criteria: &ValidationCriteria,
    limits: Option<&ResourceLimits>,
) -> Result<ValidatedOutput> {
    if candidate_bytes.is_empty() {
        return Err(ImgoptError::new(
            PipelineStage::OutputValidation,
            ErrorKind::InvalidInput("candidate output is 0 bytes".into()),
        ));
    }

    let cand_size = candidate_bytes.len() as u64;
    if let Some(max_b) = criteria.max_allowed_bytes {
        if cand_size > max_b {
            return Err(ImgoptError::new(
                PipelineStage::OutputValidation,
                ErrorKind::ResourceLimitExceeded(format!(
                    "candidate size {cand_size} exceeds maximum allowed size {max_b}"
                )),
            ));
        }
    }

    if let Some(lim) = limits {
        if cand_size > lim.max_input_bytes {
            return Err(ImgoptError::new(
                PipelineStage::OutputValidation,
                ErrorKind::ResourceLimitExceeded(format!(
                    "candidate size {cand_size} exceeds resource limit {}",
                    lim.max_input_bytes
                )),
            ));
        }
    }

    let detected_format = detect_format(candidate_bytes).map_err(|e| {
        ImgoptError::new(
            PipelineStage::OutputValidation,
            ErrorKind::Decode(format!(
                "failed to detect format of candidate output: {}",
                e.kind
            )),
        )
    })?;

    if detected_format != criteria.expected_format {
        return Err(ImgoptError::new(
            PipelineStage::OutputValidation,
            ErrorKind::InvalidInput(format!(
                "candidate output format mismatch: expected {}, detected {}",
                criteria.expected_format, detected_format
            )),
        ));
    }

    // Enforce metadata stripping policy on output container bytes
    let out_meta = probe_metadata_markers(candidate_bytes, detected_format);
    if !criteria.metadata_policy.preserves_exif() && out_meta.has_exif {
        return Err(ImgoptError::new(
            PipelineStage::OutputValidation,
            ErrorKind::InvalidInput(
                "candidate output unexpectedly contains EXIF metadata under strip policy".into(),
            ),
        ));
    }
    if !criteria.metadata_policy.preserves_icc() && out_meta.has_icc {
        return Err(ImgoptError::new(
            PipelineStage::OutputValidation,
            ErrorKind::InvalidInput(
                "candidate output unexpectedly contains ICC profile under strip policy".into(),
            ),
        ));
    }
    if !criteria.metadata_policy.preserves_xmp() && out_meta.has_xmp {
        return Err(ImgoptError::new(
            PipelineStage::OutputValidation,
            ErrorKind::InvalidInput(
                "candidate output unexpectedly contains XMP metadata under strip policy".into(),
            ),
        ));
    }

    let codec = imgopt_codecs::codec_for_format(criteria.expected_format.into());
    let decode_limits = limits
        .map(ResourceLimits::to_decode_limits)
        .unwrap_or_default();
    let decoded = codec
        .decode_with_limits(candidate_bytes, &decode_limits)
        .map_err(|e| {
            ImgoptError::new(
                PipelineStage::OutputValidation,
                ErrorKind::Decode(format!("corrupt candidate bytes failed to decode: {e}")),
            )
        })?;

    if decoded.width != criteria.expected_dimensions.width
        || decoded.height != criteria.expected_dimensions.height
    {
        return Err(ImgoptError::new(
            PipelineStage::OutputValidation,
            ErrorKind::InvalidInput(format!(
                "candidate dimensions {}x{} do not match expected {}x{}",
                decoded.width,
                decoded.height,
                criteria.expected_dimensions.width,
                criteria.expected_dimensions.height
            )),
        ));
    }

    let savings_bytes = if original_size >= cand_size {
        i64::try_from(original_size - cand_size).unwrap_or(i64::MAX)
    } else {
        -i64::try_from(cand_size - original_size).unwrap_or(i64::MAX)
    };
    let savings_ratio = if original_size > 0 {
        savings_bytes as f64 / original_size as f64
    } else {
        0.0
    };

    Ok(ValidatedOutput {
        format: criteria.expected_format,
        bytes: candidate_bytes.to_vec(),
        byte_size: cand_size,
        original_size,
        dimensions: criteria.expected_dimensions,
        savings_bytes,
        savings_ratio,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use imgopt_codecs::{DecodedPixels, EncodeOptions, ImageCodec, JpegCodec, PixelFormat};

    fn make_valid_jpeg(w: u32, h: u32) -> Vec<u8> {
        let codec = JpegCodec;
        let pixels = DecodedPixels::new(w, h, PixelFormat::Rgb8, vec![120; (w * h * 3) as usize]);
        codec
            .encode(&pixels, &EncodeOptions::default())
            .unwrap()
            .bytes
    }

    #[test]
    fn test_valid_output_passes_validation() {
        let bytes = make_valid_jpeg(4, 4);
        let orig_size = 1000u64;
        let criteria = ValidationCriteria::new(ImageFormat::Jpeg, Dimensions::new(4, 4));

        let out = validate_candidate_output(&bytes, orig_size, &criteria, None).unwrap();
        assert_eq!(out.format, ImageFormat::Jpeg);
        assert_eq!(out.dimensions, Dimensions::new(4, 4));
        assert_eq!(out.original_size, orig_size);
        assert_eq!(out.byte_size, bytes.len() as u64);
        assert_eq!(out.savings_bytes, 1000 - (bytes.len() as i64));
    }

    #[test]
    fn test_zero_bytes_rejected() {
        let criteria = ValidationCriteria::new(ImageFormat::Jpeg, Dimensions::new(4, 4));
        let res = validate_candidate_output(&[], 100, &criteria, None);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().stage, PipelineStage::OutputValidation);
    }

    #[test]
    fn test_corrupted_bytes_rejected() {
        let corrupt = vec![0xFF, 0xD8, 0xFF, 0x00, 0x00, 0x00];
        let criteria = ValidationCriteria::new(ImageFormat::Jpeg, Dimensions::new(4, 4));
        let res = validate_candidate_output(&corrupt, 100, &criteria, None);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().stage, PipelineStage::OutputValidation);
    }

    #[test]
    fn test_dimension_mismatch_rejected() {
        let bytes = make_valid_jpeg(4, 4);
        let criteria = ValidationCriteria::new(ImageFormat::Jpeg, Dimensions::new(8, 8));
        let res = validate_candidate_output(&bytes, 100, &criteria, None);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().stage, PipelineStage::OutputValidation);
    }
}
