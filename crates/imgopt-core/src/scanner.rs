use std::fs::{self, File};
use std::path::{Path, PathBuf};

use crate::detect::detect_format_from_reader;
use crate::error::{ErrorKind, ImgoptError, PipelineStage, Result};
use crate::model::ImageFormat;

/// Configuration options for scanning image files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanOptions {
    /// Whether to follow filesystem symlinks. Default is false.
    pub follow_symlinks: bool,
    /// Whether to ignore hidden files/directories (starting with a dot). Default is true.
    pub ignore_hidden: bool,
    /// Maximum directory recursion depth (None means unlimited).
    pub max_depth: Option<usize>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            follow_symlinks: false,
            ignore_hidden: true,
            max_depth: None,
        }
    }
}

/// A discovered valid image file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    pub path: PathBuf,
    pub format: ImageFormat,
    pub byte_size: u64,
}

/// The result of a scan operation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanResult {
    pub files: Vec<DiscoveredFile>,
    pub errors: Vec<ImgoptError>,
}

/// Scan a path (file or directory) for supported image files.
pub fn scan_path<P: AsRef<Path>>(root: P, options: &ScanOptions) -> Result<ScanResult> {
    let root_path = root.as_ref();
    if !root_path.exists() {
        return Err(ImgoptError::new(
            PipelineStage::Discovery,
            ErrorKind::InvalidInput(format!("path does not exist: {}", root_path.display())),
        )
        .with_path(root_path));
    }

    let meta = fs::symlink_metadata(root_path).map_err(|e| {
        ImgoptError::new(PipelineStage::Discovery, ErrorKind::Io(e.to_string()))
            .with_path(root_path)
    })?;

    if meta.file_type().is_symlink() && !options.follow_symlinks {
        return Err(ImgoptError::new(
            PipelineStage::Discovery,
            ErrorKind::InvalidInput(format!(
                "root path is a symlink and follow_symlinks is false: {}",
                root_path.display()
            )),
        )
        .with_path(root_path));
    }

    let is_dir = if meta.file_type().is_symlink() {
        fs::metadata(root_path).map(|m| m.is_dir()).unwrap_or(false)
    } else {
        meta.is_dir()
    };

    let mut result = ScanResult::default();

    if is_dir {
        scan_dir_recursive(root_path, options, 0, &mut result);
    } else if meta.is_file() {
        inspect_and_add_file(root_path, &mut result);
    }

    // Sort files deterministically by path
    result.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

fn scan_dir_recursive(
    dir_path: &Path,
    options: &ScanOptions,
    current_depth: usize,
    result: &mut ScanResult,
) {
    if let Some(max) = options.max_depth {
        if current_depth > max {
            return;
        }
    }

    let entries = match fs::read_dir(dir_path) {
        Ok(entries) => entries,
        Err(e) => {
            result.errors.push(
                ImgoptError::new(
                    PipelineStage::Discovery,
                    ErrorKind::Io(format!("failed to read directory: {e}")),
                )
                .with_path(dir_path),
            );
            return;
        }
    };

    let mut entry_paths = Vec::new();
    for entry_res in entries {
        match entry_res {
            Ok(entry) => entry_paths.push(entry.path()),
            Err(e) => {
                result.errors.push(
                    ImgoptError::new(
                        PipelineStage::Discovery,
                        ErrorKind::Io(format!("failed to read dir entry: {e}")),
                    )
                    .with_path(dir_path),
                );
            }
        }
    }

    // Sort entries to walk in deterministic order
    entry_paths.sort();

    for path in entry_paths {
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name,
            None => continue,
        };

        if options.ignore_hidden && file_name.starts_with('.') {
            continue;
        }

        let sym_meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                result.errors.push(
                    ImgoptError::new(PipelineStage::Discovery, ErrorKind::Io(e.to_string()))
                        .with_path(&path),
                );
                continue;
            }
        };

        let file_type = sym_meta.file_type();
        if file_type.is_symlink() && !options.follow_symlinks {
            // Symlinks skipped by default
            continue;
        }

        let is_directory = if file_type.is_symlink() {
            fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false)
        } else {
            file_type.is_dir()
        };

        let is_regular_file = if file_type.is_symlink() {
            fs::metadata(&path).map(|m| m.is_file()).unwrap_or(false)
        } else {
            file_type.is_file()
        };

        if is_directory {
            scan_dir_recursive(&path, options, current_depth + 1, result);
        } else if is_regular_file {
            inspect_and_add_file(&path, result);
        }
    }
}

fn inspect_and_add_file(path: &Path, result: &mut ScanResult) {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            result.errors.push(
                ImgoptError::new(
                    PipelineStage::Discovery,
                    ErrorKind::Io(format!("failed to open file: {e}")),
                )
                .with_path(path),
            );
            return;
        }
    };

    let byte_size = match file.metadata() {
        Ok(m) => m.len(),
        Err(e) => {
            result.errors.push(
                ImgoptError::new(
                    PipelineStage::Discovery,
                    ErrorKind::Io(format!("failed to read file metadata: {e}")),
                )
                .with_path(path),
            );
            return;
        }
    };

    match detect_format_from_reader(file) {
        Ok(format) => {
            result.files.push(DiscoveredFile {
                path: path.to_path_buf(),
                format,
                byte_size,
            });
        }
        Err(err) => {
            match &err.kind {
                // Non-image files or unsupported formats are simply skipped from discovered image files
                ErrorKind::UnsupportedFormat(_) | ErrorKind::InvalidInput(_) => {}
                _ => {
                    result.errors.push(err.with_path(path));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_jpeg(path: &Path) {
        let mut f = File::create(path).unwrap();
        f.write_all(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46])
            .unwrap();
    }

    fn write_png(path: &Path) {
        let mut f = File::create(path).unwrap();
        f.write_all(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00])
            .unwrap();
    }

    fn write_webp(path: &Path) {
        let mut f = File::create(path).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&[0x20, 0x00, 0x00, 0x00]);
        bytes.extend_from_slice(b"WEBP");
        bytes.extend_from_slice(b"VP8 ");
        f.write_all(&bytes).unwrap();
    }

    fn write_text(path: &Path, content: &str) {
        let mut f = File::create(path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn test_recursive_discovery_and_ordering() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        let sub_b = root.join("b_dir");
        let sub_a = root.join("a_dir");
        fs::create_dir(&sub_b).unwrap();
        fs::create_dir(&sub_a).unwrap();

        let img_b = sub_b.join("2_photo.jpg");
        let img_a = sub_a.join("1_graphic.png");
        let img_c = root.join("0_banner.webp");
        let txt = root.join("notes.txt");

        write_jpeg(&img_b);
        write_png(&img_a);
        write_webp(&img_c);
        write_text(&txt, "just a text file");

        let scan_res = scan_path(root, &ScanOptions::default()).unwrap();
        assert_eq!(scan_res.errors.len(), 0);
        assert_eq!(scan_res.files.len(), 3);

        // Verify sorted order
        assert_eq!(scan_res.files[0].path, img_c);
        assert_eq!(scan_res.files[0].format, ImageFormat::Webp);

        assert_eq!(scan_res.files[1].path, img_a);
        assert_eq!(scan_res.files[1].format, ImageFormat::Png);

        assert_eq!(scan_res.files[2].path, img_b);
        assert_eq!(scan_res.files[2].format, ImageFormat::Jpeg);
    }

    #[test]
    fn test_symlinks_skipped_by_default() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        let real_file = root.join("real.jpg");
        write_jpeg(&real_file);

        let sym_file = root.join("symlink.jpg");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_file, &sym_file).unwrap();

        let scan_res = scan_path(root, &ScanOptions::default()).unwrap();
        assert_eq!(scan_res.files.len(), 1);
        assert_eq!(scan_res.files[0].path, real_file);
    }

    #[test]
    fn test_missing_root_returns_error() {
        let temp = TempDir::new().unwrap();
        let non_existent = temp.path().join("does_not_exist");
        let res = scan_path(&non_existent, &ScanOptions::default());
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().stage, PipelineStage::Discovery);
    }

    #[test]
    fn test_single_file_scan() {
        let temp = TempDir::new().unwrap();
        let img = temp.path().join("test.png");
        write_png(&img);

        let res = scan_path(&img, &ScanOptions::default()).unwrap();
        assert_eq!(res.files.len(), 1);
        assert_eq!(res.files[0].format, ImageFormat::Png);
    }
}
