use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};

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
    /// Optional glob patterns; when non-empty, only matching relative paths are included.
    pub include_globs: Vec<String>,
    /// Optional glob patterns; matching relative paths are excluded.
    pub exclude_globs: Vec<String>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            follow_symlinks: false,
            ignore_hidden: true,
            max_depth: None,
            include_globs: Vec::new(),
            exclude_globs: Vec::new(),
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

    let is_symlink = meta.file_type().is_symlink();
    if is_symlink && !options.follow_symlinks {
        return Err(ImgoptError::new(
            PipelineStage::Discovery,
            ErrorKind::InvalidInput(format!(
                "root path is a symlink and follow_symlinks is false: {}",
                root_path.display()
            )),
        )
        .with_path(root_path));
    }

    let target_meta = if is_symlink {
        fs::metadata(root_path).map_err(|e| {
            ImgoptError::new(PipelineStage::Discovery, ErrorKind::Io(e.to_string()))
                .with_path(root_path)
        })?
    } else {
        meta
    };

    let mut result = ScanResult::default();

    if target_meta.is_dir() {
        let mut visited_dirs = HashSet::new();
        if options.follow_symlinks {
            if let Ok(canon) = fs::canonicalize(root_path) {
                visited_dirs.insert(canon);
            }
        }
        scan_dir_recursive(root_path, options, 0, &mut visited_dirs, &mut result);
    } else if target_meta.is_file() {
        inspect_and_add_file(root_path, options.follow_symlinks, &mut result);
    }

    // Sort files deterministically by path
    result.files.sort_by(|a, b| a.path.cmp(&b.path));

    if !options.include_globs.is_empty() || !options.exclude_globs.is_empty() {
        let include_set = build_globset(&options.include_globs)?;
        let exclude_set = build_globset(&options.exclude_globs)?;
        result.files.retain(|f| {
            let rel = f
                .path
                .strip_prefix(root_path)
                .ok()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| f.path.file_name().map(Path::new).unwrap_or(&f.path));
            let file_name = f.path.file_name().map(Path::new);
            let matches_set = |set: &GlobSet| {
                set.is_match(rel) || file_name.is_some_and(|name| set.is_match(name))
            };
            if let Some(ref inc) = include_set {
                if !matches_set(inc) {
                    return false;
                }
            }
            if let Some(ref exc) = exclude_set {
                if matches_set(exc) {
                    return false;
                }
            }
            true
        });
    }

    Ok(result)
}

fn build_globset(patterns: &[String]) -> Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let glob = Glob::new(pat).map_err(|e| {
            ImgoptError::new(
                PipelineStage::Discovery,
                ErrorKind::Configuration(format!("invalid glob pattern '{pat}': {e}")),
            )
        })?;
        builder.add(glob);
    }
    let set = builder.build().map_err(|e| {
        ImgoptError::new(
            PipelineStage::Discovery,
            ErrorKind::Configuration(format!("failed to build glob filter: {e}")),
        )
    })?;
    Ok(Some(set))
}

fn scan_dir_recursive(
    dir_path: &Path,
    options: &ScanOptions,
    current_depth: usize,
    visited_dirs: &mut HashSet<PathBuf>,
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
            if options.follow_symlinks {
                let Ok(canon) = fs::canonicalize(&path) else {
                    continue;
                };
                if !visited_dirs.insert(canon) {
                    // Break symlink cycles
                    continue;
                }
            }
            scan_dir_recursive(&path, options, current_depth + 1, visited_dirs, result);
        } else if is_regular_file {
            inspect_and_add_file(&path, options.follow_symlinks, result);
        }
    }
}

#[cfg(unix)]
fn unix_o_nofollow_flag() -> Option<i32> {
    #[cfg(all(
        target_os = "linux",
        any(
            target_arch = "x86_64",
            target_arch = "x86",
            target_arch = "riscv64",
            target_arch = "loongarch64",
            target_arch = "s390x",
            target_arch = "mips",
            target_arch = "mips64",
            target_arch = "sparc",
            target_arch = "sparc64"
        )
    ))]
    {
        return Some(0o400_000);
    }

    #[cfg(all(
        target_os = "linux",
        any(
            target_arch = "aarch64",
            target_arch = "arm",
            target_arch = "powerpc",
            target_arch = "powerpc64",
            target_arch = "m68k"
        )
    ))]
    {
        return Some(0o100_000);
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    {
        return Some(0x0100);
    }

    #[allow(unreachable_code)]
    None
}

fn open_scanned_file(path: &Path, follow_symlinks: bool) -> std::io::Result<File> {
    let pre_meta = if !follow_symlinks {
        let m = fs::symlink_metadata(path)?;
        if m.file_type().is_symlink() || !m.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "target is a symlink or not a regular file",
            ));
        }
        Some(m)
    } else {
        None
    };

    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    if !follow_symlinks {
        if let Some(flag) = unix_o_nofollow_flag() {
            use std::os::unix::fs::OpenOptionsExt;
            opts.custom_flags(flag);
        }
    }
    let file = opts.open(path)?;
    if let Some(sym_meta) = pre_meta {
        let opened_meta = file.metadata()?;
        if !opened_meta.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "target is not a regular file",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if sym_meta.dev() != opened_meta.dev() || sym_meta.ino() != opened_meta.ino() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "file changed or replaced during open",
                ));
            }
        }
    }
    Ok(file)
}

fn inspect_and_add_file(path: &Path, follow_symlinks: bool, result: &mut ScanResult) {
    let file = match open_scanned_file(path, follow_symlinks) {
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

    #[cfg(unix)]
    #[test]
    fn test_follow_symlinks_single_file_and_cycle_prevention() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("sub");
        fs::create_dir(&dir).unwrap();
        let real_img = dir.join("real.png");
        write_png(&real_img);

        // Symlink to single file
        let sym_file = temp.path().join("link.png");
        std::os::unix::fs::symlink(&real_img, &sym_file).unwrap();

        let opts_follow = ScanOptions {
            follow_symlinks: true,
            ..Default::default()
        };
        let single_res = scan_path(&sym_file, &opts_follow).unwrap();
        assert_eq!(single_res.files.len(), 1);

        // Directory symlink cycle: sub/loop -> temp.path()
        let loop_dir = dir.join("loop");
        std::os::unix::fs::symlink(temp.path(), &loop_dir).unwrap();

        let dir_res = scan_path(temp.path(), &opts_follow).unwrap();
        // Should terminate cleanly without infinite recursion and find the file(s)
        assert!(!dir_res.files.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn test_open_scanned_file_rejects_symlink_when_not_following() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("real.png");
        write_png(&target);
        let link = temp.path().join("sym.png");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(open_scanned_file(&link, false).is_err());
        assert!(open_scanned_file(&target, false).is_ok());
    }

    #[test]
    fn test_scan_include_and_exclude_globs() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("keep")).unwrap();
        fs::create_dir_all(root.join("skip")).unwrap();
        let a = root.join("keep/photo.jpg");
        let b = root.join("keep/thumb.png");
        let c = root.join("skip/banner.jpg");
        write_jpeg(&a);
        write_png(&b);
        write_jpeg(&c);

        let opts = ScanOptions {
            include_globs: vec!["**/*.jpg".into(), "*.jpg".into()],
            exclude_globs: vec!["skip/**".into()],
            ..Default::default()
        };
        let res = scan_path(root, &opts).unwrap();
        assert_eq!(res.files.len(), 1);
        assert_eq!(res.files[0].path, a);
    }
}
