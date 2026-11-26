use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use clap::{Args, Parser, Subcommand, ValueEnum};
use imgopt_codecs::decode_animation_with_limits;
use imgopt_core::{
    analyze_image, compute_byte_savings, detect_format_from_reader, execute_batch_scoped,
    hash_file, read_file_bounded, run_benchmark_with_options, scan_path, utc_now_rfc3339,
    BatchOptions, BatchTask, Dimensions, ImageFormat, MetadataPolicy, OptimizationConfig,
    OptimizationKey, OptimizationPreset, OptimizationService, OutputFormatPolicy, PipelineStage,
    ResizeMode, ResourceLimits, ScanOptions, SyncItemStatus, SyncRunResult, SyncService,
};
use imgopt_manifest::{
    EntryStatus, ManifestEntry, ManifestOptimizationRecord, ManifestSourceRecord, ManifestStore,
    ManifestSyncAdapter, RunItemStatus, RunResult, RunResultRecord,
};
use imgopt_storage::{
    MemoryObjectStorage, ObjectStorage, RetryPolicy, S3Client, S3StorageConfig, StorageSyncAdapter,
};

const DEFAULT_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

#[derive(Parser, Debug)]
#[command(name = "imgopt", version, about = "Image Optimizer CLI")]
struct Cli {
    /// Path to custom environment variable file (.env)
    #[arg(global = true, long = "env-file")]
    env_file: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

fn load_dotenv_file(path: &Path, required: bool) -> Result<(), Box<dyn std::error::Error>> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if !required && e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(format!("failed to read env file '{}': {e}", path.display()).into());
        }
    };

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").map(str::trim).unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }

        let val_trimmed = v.trim();
        let val = if (val_trimmed.starts_with('"')
            && val_trimmed.ends_with('"')
            && val_trimmed.len() >= 2)
            || (val_trimmed.starts_with('\'')
                && val_trimmed.ends_with('\'')
                && val_trimmed.len() >= 2)
        {
            &val_trimmed[1..val_trimmed.len() - 1]
        } else {
            val_trimmed
                .split_once(" #")
                .map(|(head, _)| head.trim_end())
                .unwrap_or(val_trimmed)
        };

        // Never clobber variables already set in the process environment
        if std::env::var_os(key).is_none() {
            std::env::set_var(key, val);
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let needs_storage_env = matches!(
        &cli.command,
        Commands::Sync(args) if !args.dry_run
    ) || matches!(
        &cli.command,
        Commands::Verify(args) if args.check_storage
    ) || matches!(&cli.command, Commands::Gc(_));

    if let Some(ref explicit_env) = cli.env_file {
        if let Err(err) = load_dotenv_file(explicit_env, true) {
            eprintln!("Error: {err}");
            return ExitCode::FAILURE;
        }
    } else if needs_storage_env {
        if let Err(err) = load_dotenv_file(Path::new(".env"), false) {
            eprintln!("Error: {err}");
            return ExitCode::FAILURE;
        }
    }

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Optimize images locally without uploading or modifying storage
    Optimize(OptimizeArgs),
    /// Scan, optimize, upload to S3-compatible storage, and commit manifest
    Sync(SyncArgs),
    /// Inspect image profile, dimensions, color type, and metadata
    Inspect(InspectArgs),
    /// Verify manifest consistency and remote storage objects
    Verify(VerifyArgs),
    /// Benchmark optimization performance across image datasets
    Benchmark(BenchmarkArgs),
    /// List or delete orphaned remote storage objects not referenced in manifest
    Gc(GcArgs),
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum PresetArg {
    Balanced,
    Maximum,
    Aggressive,
}

impl From<PresetArg> for OptimizationPreset {
    fn from(p: PresetArg) -> Self {
        match p {
            PresetArg::Balanced => OptimizationPreset::Balanced,
            PresetArg::Maximum => OptimizationPreset::Maximum,
            PresetArg::Aggressive => OptimizationPreset::Aggressive,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum ResizeModeArg {
    ScaleDown,
    Contain,
    Cover,
    Exact,
}

impl From<ResizeModeArg> for ResizeMode {
    fn from(m: ResizeModeArg) -> Self {
        match m {
            ResizeModeArg::ScaleDown => ResizeMode::ScaleDown,
            ResizeModeArg::Contain => ResizeMode::Contain,
            ResizeModeArg::Cover => ResizeMode::Cover,
            ResizeModeArg::Exact => ResizeMode::Exact,
        }
    }
}

#[derive(Args, Debug, Clone)]
struct OptimizationArgs {
    /// Optimization preset (balanced, maximum, aggressive)
    #[arg(long, value_enum, default_value_t = PresetArg::Balanced)]
    preset: PresetArg,

    /// Target SSIM quality (0.0 to 1.0)
    #[arg(short, long)]
    quality: Option<f64>,

    /// Encoding effort level (1 to 6)
    #[arg(short, long)]
    effort: Option<u8>,

    /// Preserve all EXIF / ICC / XMP metadata (default strips all)
    #[arg(long)]
    preserve_metadata: bool,

    /// Preserve ICC color profile while stripping EXIF and XMP privacy metadata
    #[arg(long)]
    preserve_icc: bool,

    /// Disable automatic ICC-to-sRGB conversion when stripping ICC profiles
    #[arg(long)]
    no_srgb_convert: bool,

    /// Maximum allowed input image width in pixels
    #[arg(long)]
    max_width: Option<u32>,

    /// Maximum allowed input image height in pixels
    #[arg(long)]
    max_height: Option<u32>,

    /// Target resize width in pixels
    #[arg(long)]
    resize_width: Option<u32>,

    /// Target resize height in pixels
    #[arg(long)]
    resize_height: Option<u32>,

    /// Resize strategy (scale-down, contain, cover, exact)
    #[arg(long, value_enum, default_value_t = ResizeModeArg::ScaleDown)]
    resize_mode: ResizeModeArg,
}

#[derive(Args, Debug, Clone)]
struct FilterArgs {
    /// Max concurrency for worker pool
    #[arg(long)]
    concurrency: Option<usize>,

    /// Include files matching glob pattern (repeatable, e.g. "**/*.png")
    #[arg(long = "include")]
    include: Vec<String>,

    /// Exclude files matching glob pattern (repeatable, e.g. "thumbs/**")
    #[arg(long = "exclude")]
    exclude: Vec<String>,
}

impl FilterArgs {
    fn to_scan_options(&self) -> ScanOptions {
        ScanOptions {
            include_globs: self.include.clone(),
            exclude_globs: self.exclude.clone(),
            ..Default::default()
        }
    }
}

#[derive(Parser, Debug)]
struct OptimizeArgs {
    /// Input file or directory to optimize
    input: PathBuf,

    /// Output destination directory or file path
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Output format policy (preserve, webp, avif, auto, jpeg, png, gif)
    #[arg(long, default_value = "preserve")]
    format: String,

    /// Optional manifest JSON path for local incremental caching/dedup
    #[arg(long)]
    manifest: Option<PathBuf>,

    /// Path to write machine-readable run result JSON
    #[arg(long)]
    json: Option<PathBuf>,

    /// Suppress per-file stdout progress lines
    #[arg(short = 'Q', long)]
    quiet: bool,

    #[command(flatten)]
    opt: OptimizationArgs,

    #[command(flatten)]
    filter: FilterArgs,
}

#[derive(Parser, Debug)]
struct SyncArgs {
    /// Input directory or file to synchronize
    input: PathBuf,

    /// Path to persistent manifest JSON file
    #[arg(long)]
    manifest: Option<PathBuf>,

    /// Path to write machine-readable run result JSON
    #[arg(long)]
    json: Option<PathBuf>,

    /// Output format policy (preserve, webp, avif, auto, jpeg, png, gif)
    #[arg(long, default_value = "webp")]
    format: String,

    /// Override S3 bucket name
    #[arg(long)]
    bucket: Option<String>,

    /// Override S3 endpoint URL
    #[arg(long)]
    endpoint: Option<String>,

    /// Override S3 key prefix
    #[arg(long)]
    prefix: Option<String>,

    /// Override public URL base
    #[arg(long)]
    public_url_base: Option<String>,

    /// Override Cache-Control header on uploaded objects
    #[arg(long)]
    cache_control: Option<String>,

    /// Override S3 path-style URL setting (true/false)
    #[arg(long)]
    force_path_style: Option<bool>,

    /// Use mock in-memory storage for dry runs or testing without remote S3 credentials
    #[arg(long)]
    dry_run: bool,

    /// Exit with a non-zero status code if any file fails during sync
    #[arg(long)]
    fail_on_error: bool,

    /// Suppress per-file stdout progress lines
    #[arg(short = 'Q', long)]
    quiet: bool,

    #[command(flatten)]
    opt: OptimizationArgs,

    #[command(flatten)]
    filter: FilterArgs,
}

#[derive(Parser, Debug)]
struct InspectArgs {
    /// Input file or directory to inspect
    input: PathBuf,

    /// Output inspection report as machine-readable JSON
    #[arg(long)]
    json: bool,

    /// Maximum allowed image width in pixels
    #[arg(long)]
    max_width: Option<u32>,

    /// Maximum allowed image height in pixels
    #[arg(long)]
    max_height: Option<u32>,

    /// Include files matching glob pattern (repeatable)
    #[arg(long = "include")]
    include: Vec<String>,

    /// Exclude files matching glob pattern (repeatable)
    #[arg(long = "exclude")]
    exclude: Vec<String>,
}

#[derive(Parser, Debug)]
struct VerifyArgs {
    /// Path to manifest file to verify
    #[arg(long)]
    manifest: Option<PathBuf>,

    /// Verify referenced objects exist in remote storage
    #[arg(long)]
    check_storage: bool,

    /// Override S3 bucket for verification
    #[arg(long)]
    bucket: Option<String>,

    /// Override S3 endpoint URL for verification
    #[arg(long)]
    endpoint: Option<String>,

    /// Override S3 path-style URL setting (true/false)
    #[arg(long)]
    force_path_style: Option<bool>,

    /// Max concurrency for remote storage HEAD checks
    #[arg(long)]
    concurrency: Option<usize>,
}

#[derive(Parser, Debug)]
struct BenchmarkArgs {
    /// Dataset directory or image file to benchmark
    dataset: PathBuf,

    /// Output format policy (preserve, webp, avif, auto, jpeg, png, gif)
    #[arg(long, default_value = "preserve")]
    format: String,

    /// Path to write machine-readable benchmark JSON report
    #[arg(long)]
    json: Option<PathBuf>,

    /// Exit with a non-zero status code if any benchmark case fails
    #[arg(long)]
    fail_on_error: bool,

    #[command(flatten)]
    opt: OptimizationArgs,

    #[command(flatten)]
    filter: FilterArgs,
}

#[derive(Parser, Debug)]
struct GcArgs {
    /// Path to manifest file referencing active objects
    #[arg(long)]
    manifest: Option<PathBuf>,

    /// Override S3 bucket
    #[arg(long)]
    bucket: Option<String>,

    /// Override S3 endpoint URL
    #[arg(long)]
    endpoint: Option<String>,

    /// Override S3 key prefix to scan for orphaned objects
    #[arg(long)]
    prefix: Option<String>,

    /// Override S3 path-style URL setting (true/false)
    #[arg(long)]
    force_path_style: Option<bool>,

    /// Actually delete orphaned objects (default is safe dry-run)
    #[arg(long)]
    delete: bool,

    /// Path to write machine-readable GC report JSON
    #[arg(long)]
    json: Option<PathBuf>,
}

fn parse_format_policy(format: &str) -> Result<OutputFormatPolicy, Box<dyn std::error::Error>> {
    match format.trim().to_ascii_lowercase().as_str() {
        "preserve" | "original" => Ok(OutputFormatPolicy::PreserveOriginal),
        "webp" => Ok(OutputFormatPolicy::AllowWebp),
        "avif" | "allow-avif" => Ok(OutputFormatPolicy::AllowAvif),
        "auto" => Ok(OutputFormatPolicy::Auto),
        "jpeg" | "jpg" => Ok(OutputFormatPolicy::Force(ImageFormat::Jpeg)),
        "png" => Ok(OutputFormatPolicy::Force(ImageFormat::Png)),
        "gif" => Ok(OutputFormatPolicy::Force(ImageFormat::Gif)),
        "force-webp" => Ok(OutputFormatPolicy::Force(ImageFormat::Webp)),
        "force-avif" => Ok(OutputFormatPolicy::Force(ImageFormat::Avif)),
        other => Err(format!("unsupported format policy: {other}").into()),
    }
}

fn build_optimization_config(
    opts: &OptimizationArgs,
    format: &str,
) -> Result<OptimizationConfig, Box<dyn std::error::Error>> {
    let mut config = OptimizationConfig::from_preset(opts.preset.into());
    if let Some(q) = opts.quality {
        config.quality_target = q;
    }
    if let Some(eff) = opts.effort {
        config.effort = eff;
    }
    if opts.preserve_metadata {
        config.metadata_policy = MetadataPolicy::PreserveAll;
    } else if opts.preserve_icc {
        config.metadata_policy = MetadataPolicy::PreserveIcc;
    }
    if opts.no_srgb_convert {
        config.convert_to_srgb = false;
    }
    if opts.max_width.is_some() {
        config.max_width = opts.max_width;
    }
    if opts.max_height.is_some() {
        config.max_height = opts.max_height;
    }
    config.resize_width = opts.resize_width;
    config.resize_height = opts.resize_height;
    config.resize_mode = opts.resize_mode.into();
    config.output_format_policy = parse_format_policy(format)?;
    config.validate()?;
    Ok(config)
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Commands::Optimize(args) => run_optimize(args),
        Commands::Sync(args) => run_sync(args),
        Commands::Inspect(args) => run_inspect(args),
        Commands::Verify(args) => run_verify(args),
        Commands::Benchmark(args) => run_benchmark_cmd(args),
        Commands::Gc(args) => run_gc(args),
    }
}

fn is_safe_relative(path: &Path) -> bool {
    !path.is_absolute()
        && !path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

fn compute_dest_path(
    input_root: &Path,
    output_arg: Option<&PathBuf>,
    src_path: &Path,
    is_single_file: bool,
    target_format: ImageFormat,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if is_single_file {
        if let Some(out_dest) = output_arg {
            let is_dest_dir = out_dest.is_dir()
                || out_dest.extension().is_none()
                || out_dest.to_string_lossy().ends_with('/')
                || out_dest.to_string_lossy().ends_with('\\');

            if is_dest_dir {
                let orig_name = src_path.file_stem().unwrap_or_default();
                Ok(out_dest.join(format!(
                    "{}.{}",
                    orig_name.to_string_lossy(),
                    target_format.extension()
                )))
            } else {
                Ok(out_dest.clone())
            }
        } else {
            let orig_stem = src_path.file_stem().unwrap_or_default();
            Ok(src_path.with_file_name(format!(
                "{}-optimized.{}",
                orig_stem.to_string_lossy(),
                target_format.extension()
            )))
        }
    } else {
        let relative = src_path.strip_prefix(input_root).map_err(|_| {
            format!(
                "discovered path '{}' escaped input root '{}'",
                src_path.display(),
                input_root.display()
            )
        })?;
        if !is_safe_relative(relative) {
            return Err(format!(
                "unsafe relative path '{}' for '{}'",
                relative.display(),
                src_path.display()
            )
            .into());
        }
        let out_base = output_arg
            .cloned()
            .unwrap_or_else(|| PathBuf::from("optimized_output"));
        let stem = relative.file_stem().unwrap_or_default();
        let parent = relative.parent().unwrap_or_else(|| Path::new(""));
        Ok(out_base.join(parent).join(format!(
            "{}.{}",
            stem.to_string_lossy(),
            target_format.extension()
        )))
    }
}

fn resolve_unique_dest_path(
    base_dest: PathBuf,
    source_path: &Path,
    claimed: &mut HashSet<PathBuf>,
) -> PathBuf {
    if claimed.insert(base_dest.clone()) {
        return base_dest;
    }

    let stem = base_dest
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    let ext = base_dest.extension().and_then(|e| e.to_str()).unwrap_or("");
    let src_ext = source_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("orig");
    let parent = base_dest.parent().unwrap_or_else(|| Path::new("."));

    // Disambiguate using source extension first (e.g. a.jpg + a.png -> a-jpg.webp, a-png.webp)
    let ext_tagged = if ext.is_empty() {
        parent.join(format!("{stem}-{src_ext}"))
    } else {
        parent.join(format!("{stem}-{src_ext}.{ext}"))
    };
    if claimed.insert(ext_tagged.clone()) {
        return ext_tagged;
    }

    let mut counter = 2usize;
    loop {
        let candidate = if ext.is_empty() {
            parent.join(format!("{stem}-{src_ext}-{counter}"))
        } else {
            parent.join(format!("{stem}-{src_ext}-{counter}.{ext}"))
        };
        if claimed.insert(candidate.clone()) {
            return candidate;
        }
        counter += 1;
    }
}

fn run_optimize(args: OptimizeArgs) -> Result<(), Box<dyn std::error::Error>> {
    let start_time = Instant::now();
    let config = build_optimization_config(&args.opt, &args.format)?;
    let version = env!("CARGO_PKG_VERSION");
    let service = OptimizationService::new(version);

    let scan_opts = args.filter.to_scan_options();
    let scan_res = scan_path(&args.input, &scan_opts)?;

    if scan_res.files.is_empty() && scan_res.errors.is_empty() {
        return Err(format!("no supported images found at '{}'", args.input.display()).into());
    }

    let is_single_file = scan_res.files.len() == 1 && args.input.is_file();
    let default_concurrency = BatchOptions::default().concurrency;
    let batch_opts = BatchOptions {
        concurrency: args
            .filter
            .concurrency
            .unwrap_or(default_concurrency)
            .max(1),
        stop_on_error: false,
    };

    let mut manifest_adapter = if let Some(ref m_path) = args.manifest {
        let store = ManifestStore::new(m_path);
        Some(ManifestSyncAdapter::new(store, version)?)
    } else {
        None
    };

    let mut total_original_bytes = 0u64;
    let mut total_optimized_bytes = 0u64;
    let mut processed_count = 0usize;
    let mut skipped_count = 0usize;
    let mut failed_count = scan_res.errors.len();
    let mut run_records: Vec<RunResultRecord> = Vec::new();
    for scan_err in &scan_res.errors {
        eprintln!("Failed: {scan_err}");
        run_records.push(RunResultRecord {
            path: scan_err
                .source_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| args.input.to_string_lossy().to_string()),
            status: RunItemStatus::Failed,
            source_sha256: None,
            source_bytes: 0,
            optimized_bytes: None,
            format: None,
            quality_score: None,
            remote_url: None,
            error: Some(scan_err.to_string()),
        });
    }

    let mut claimed_destinations = HashSet::new();
    let mut tasks: Vec<BatchTask<(Option<String>, u64)>> = Vec::new();

    // If local manifest caching is enabled, skip files whose SHA-256 + OptimizationKey already match
    // and whose destination file still exists on disk.
    for file in &scan_res.files {
        if let Some(ref mut adapter) = manifest_adapter {
            if let Ok(digest) = hash_file(&file.path) {
                let sha_hex = digest.to_hex();
                let mut matched_skip: Option<(ImageFormat, u64, PathBuf)> = None;
                if let Some(entry) = adapter.manifest.get(&sha_hex) {
                    if let Some(ref opt) = entry.optimization {
                        let expected_key =
                            OptimizationKey::compute(&digest, version, opt.output_format, &config);
                        if opt.optimization_key == expected_key.as_str() {
                            if let Ok(raw_dest) = compute_dest_path(
                                &args.input,
                                args.output.as_ref(),
                                &file.path,
                                is_single_file,
                                opt.output_format,
                            ) {
                                let dest_path = if is_single_file {
                                    raw_dest
                                } else {
                                    resolve_unique_dest_path(
                                        raw_dest,
                                        &file.path,
                                        &mut claimed_destinations,
                                    )
                                };
                                if dest_path.exists() {
                                    matched_skip =
                                        Some((opt.output_format, opt.output_size, dest_path));
                                }
                            }
                        }
                    }
                }
                if let Some((fmt, opt_bytes, dest_path)) = matched_skip {
                    adapter
                        .manifest
                        .add_path_alias(&sha_hex, file.path.to_string_lossy().into_owned());
                    skipped_count += 1;
                    total_original_bytes += file.byte_size;
                    total_optimized_bytes += opt_bytes;
                    if !args.quiet {
                        println!(
                            "Skipped (cached): {} -> {} ({} bytes)",
                            file.path.display(),
                            dest_path.display(),
                            opt_bytes
                        );
                    }
                    run_records.push(RunResultRecord {
                        path: file.path.to_string_lossy().to_string(),
                        status: RunItemStatus::Skipped,
                        source_sha256: Some(sha_hex),
                        source_bytes: file.byte_size,
                        optimized_bytes: Some(opt_bytes),
                        format: Some(fmt.to_string()),
                        quality_score: None,
                        remote_url: None,
                        error: None,
                    });
                    continue;
                }
                tasks.push(BatchTask::new(&file.path, (Some(sha_hex), file.byte_size)));
                continue;
            }
        }
        tasks.push(BatchTask::new(&file.path, (None, file.byte_size)));
    }

    let (batch_results, _) = execute_batch_scoped(tasks, &batch_opts, |path, (_sha, _sz)| {
        service.optimize_file(&path, &config)
    });

    for res in batch_results {
        let src_path = res.source_path;
        let opt_res = match res.output {
            Ok(r) => r,
            Err(e) => {
                failed_count += 1;
                eprintln!("Failed: {}: {e}", src_path.display());
                run_records.push(RunResultRecord {
                    path: src_path.to_string_lossy().to_string(),
                    status: RunItemStatus::Failed,
                    source_sha256: None,
                    source_bytes: fs::metadata(&src_path).map(|m| m.len()).unwrap_or(0),
                    optimized_bytes: None,
                    format: None,
                    quality_score: None,
                    remote_url: None,
                    error: Some(e.to_string()),
                });
                continue;
            }
        };

        let raw_dest_path = match compute_dest_path(
            &args.input,
            args.output.as_ref(),
            &src_path,
            is_single_file,
            opt_res.effective_format,
        ) {
            Ok(p) => p,
            Err(e) => {
                failed_count += 1;
                eprintln!("Failed: {e}");
                run_records.push(RunResultRecord {
                    path: src_path.to_string_lossy().to_string(),
                    status: RunItemStatus::Failed,
                    source_sha256: Some(opt_res.source_sha256.to_hex()),
                    source_bytes: opt_res.validated_output.original_size,
                    optimized_bytes: None,
                    format: None,
                    quality_score: None,
                    remote_url: None,
                    error: Some(e.to_string()),
                });
                continue;
            }
        };

        let dest_path = if is_single_file {
            raw_dest_path
        } else {
            resolve_unique_dest_path(raw_dest_path, &src_path, &mut claimed_destinations)
        };

        if let Err(e) = write_atomically(&dest_path, &opt_res.validated_output.bytes) {
            failed_count += 1;
            eprintln!("Failed writing {}: {e}", dest_path.display());
            run_records.push(RunResultRecord {
                path: src_path.to_string_lossy().to_string(),
                status: RunItemStatus::Failed,
                source_sha256: Some(opt_res.source_sha256.to_hex()),
                source_bytes: opt_res.validated_output.original_size,
                optimized_bytes: None,
                format: None,
                quality_score: None,
                remote_url: None,
                error: Some(e.to_string()),
            });
            continue;
        }

        if let Some(ref mut adapter) = manifest_adapter {
            let sha_hex = opt_res.source_sha256.to_hex();
            let existing_storage = adapter
                .manifest
                .get(&sha_hex)
                .and_then(|e| e.storage.clone());
            adapter.manifest.insert(ManifestEntry {
                source: ManifestSourceRecord {
                    sha256: sha_hex.clone(),
                    format: opt_res.profile.format,
                    byte_size: opt_res.profile.byte_size,
                    dimensions: opt_res.profile.dimensions,
                },
                optimization: Some(ManifestOptimizationRecord {
                    optimization_key: opt_res.optimization_key.to_string(),
                    output_format: opt_res.effective_format,
                    output_size: opt_res.validated_output.byte_size,
                    quality_score: Some(opt_res.quality_score),
                    preset: opt_res.preset.as_str().to_string(),
                    completed_at: utc_now_rfc3339(),
                }),
                storage: existing_storage,
                status: EntryStatus::Optimized,
                known_paths: vec![src_path.to_string_lossy().to_string()],
            });
        }

        total_original_bytes += opt_res.validated_output.original_size;
        total_optimized_bytes += opt_res.validated_output.byte_size;
        processed_count += 1;

        run_records.push(RunResultRecord {
            path: src_path.to_string_lossy().to_string(),
            status: RunItemStatus::Optimized,
            source_sha256: Some(opt_res.source_sha256.to_hex()),
            source_bytes: opt_res.validated_output.original_size,
            optimized_bytes: Some(opt_res.validated_output.byte_size),
            format: Some(opt_res.effective_format.to_string()),
            quality_score: Some(opt_res.quality_score),
            remote_url: None,
            error: None,
        });

        if !args.quiet {
            println!(
                "Optimized: {} -> {} ({} -> {} bytes, {:.1}% saved, SSIM: {:.4})",
                src_path.display(),
                dest_path.display(),
                opt_res.validated_output.original_size,
                opt_res.validated_output.byte_size,
                opt_res.validated_output.savings_ratio * 100.0,
                opt_res.quality_score,
            );
        }
    }

    if let Some(ref adapter) = manifest_adapter {
        adapter.store.save(&adapter.manifest)?;
    }

    let elapsed_millis = start_time.elapsed().as_millis() as u64;
    if let Some(ref json_path) = args.json {
        let run_result = RunResult::new(run_records, elapsed_millis);
        run_result.write_to_file(json_path)?;
    }

    let (overall_saved, raw_ratio) =
        compute_byte_savings(total_original_bytes, total_optimized_bytes);
    let overall_ratio = raw_ratio * 100.0;

    println!("\nSummary:");
    println!("  Processed: {processed_count} files");
    if skipped_count > 0 {
        println!("  Skipped:   {skipped_count} files");
    }
    if failed_count > 0 {
        println!("  Failed:    {failed_count} files");
    }
    println!("  Original size:  {total_original_bytes} bytes");
    println!("  Optimized size: {total_optimized_bytes} bytes");
    println!("  Saved:          {overall_saved} bytes ({overall_ratio:.2}%)");

    if failed_count > 0 {
        return Err(format!("optimization finished with {failed_count} failed file(s)").into());
    }

    Ok(())
}

fn run_sync(args: SyncArgs) -> Result<(), Box<dyn std::error::Error>> {
    let config = build_optimization_config(&args.opt, &args.format)?;
    let scan_opts = args.filter.to_scan_options();
    let version = env!("CARGO_PKG_VERSION");

    let manifest_path = args.manifest.unwrap_or_else(|| {
        if args.input.is_dir() {
            args.input.join(".imgopt-manifest.json")
        } else {
            PathBuf::from(".imgopt-manifest.json")
        }
    });

    let manifest_store = ManifestStore::new(&manifest_path);
    let mut manifest_adapter = ManifestSyncAdapter::new(manifest_store, version)?;

    let batch_options = BatchOptions {
        concurrency: args.filter.concurrency.unwrap_or(4).max(1),
        stop_on_error: false,
    };

    let sync_result: SyncRunResult = if args.dry_run {
        let bucket = args.bucket.unwrap_or_else(|| "dry-run-bucket".into());
        let storage = MemoryObjectStorage::new();
        let cache_control = args
            .cache_control
            .or_else(|| Some(DEFAULT_CACHE_CONTROL.to_string()));
        let storage_adapter = StorageSyncAdapter::new(
            storage,
            args.public_url_base
                .or_else(|| Some("https://dry-run.local".into())),
            args.prefix,
            RetryPolicy::default(),
        )
        .with_cache_control(cache_control);
        let sync_service = SyncService::new(version, batch_options, &bucket);
        sync_service.run_sync_with_options(
            &args.input,
            &config,
            &scan_opts,
            &storage_adapter,
            &mut manifest_adapter,
        )?
    } else {
        let s3_config = S3StorageConfig::from_env_with_cli_overrides(
            args.bucket,
            args.endpoint,
            args.prefix,
            args.public_url_base,
            args.force_path_style,
            args.cache_control,
        )
        .map_err(|e| format!("storage configuration failed: {e}"))?;

        let bucket = s3_config.bucket.clone();
        let key_prefix = s3_config.key_prefix.clone();
        // Only emit public URLs when an explicit public URL base is configured;
        // never persist credential-bearing presigned URLs or private API endpoints.
        let public_url_base = s3_config.public_url_base.clone();
        let cache_control = s3_config
            .cache_control
            .clone()
            .or_else(|| Some(DEFAULT_CACHE_CONTROL.to_string()));
        let s3_client = S3Client::new(s3_config)?;
        let storage_adapter = StorageSyncAdapter::new(
            s3_client,
            public_url_base,
            key_prefix,
            RetryPolicy::default(),
        )
        .with_cache_control(cache_control);
        let sync_service = SyncService::new(version, batch_options, &bucket);
        sync_service.run_sync_with_options(
            &args.input,
            &config,
            &scan_opts,
            &storage_adapter,
            &mut manifest_adapter,
        )?
    };

    if let Some(ref json_path) = args.json {
        let records: Vec<RunResultRecord> = sync_result
            .items
            .iter()
            .map(|item| RunResultRecord {
                path: item.path.to_string_lossy().to_string(),
                status: match item.status {
                    SyncItemStatus::Optimized => RunItemStatus::Optimized,
                    SyncItemStatus::Skipped => RunItemStatus::Skipped,
                    SyncItemStatus::Failed => RunItemStatus::Failed,
                },
                source_sha256: item.source_sha256.as_ref().map(|s| s.to_hex()),
                source_bytes: item.source_bytes,
                optimized_bytes: item.optimized_bytes,
                format: item.format.map(|f| f.to_string()),
                quality_score: item.quality_score,
                remote_url: item.remote_url.clone(),
                error: item.error.clone(),
            })
            .collect();

        let run_result = RunResult::new(records, sync_result.elapsed_millis);
        run_result.write_to_file(json_path)?;
    }

    println!("\nSync Completed:");
    println!("  Total files:      {}", sync_result.total_files);
    println!("  Optimized:        {}", sync_result.optimized_files);
    println!("  Skipped (cached): {}", sync_result.skipped_files);
    println!("  Failed:           {}", sync_result.failed_files);
    if !args.quiet {
        for item in &sync_result.items {
            if let Some(ref err) = item.error {
                eprintln!("  - {}: {}", item.path.display(), err);
            } else if let Some(ref url) = item.remote_url {
                println!("  + {}: {}", item.path.display(), url);
            }
        }
    }
    println!("  Original bytes:  {}", sync_result.original_bytes);
    println!("  Optimized bytes: {}", sync_result.optimized_bytes);
    println!(
        "  Saved bytes:     {} ({:.2}%)",
        sync_result.saved_bytes,
        sync_result.saved_ratio * 100.0
    );
    println!("  Elapsed time:    {} ms", sync_result.elapsed_millis);

    if args.fail_on_error && sync_result.failed_files > 0 {
        return Err(format!(
            "sync finished with {} failed file(s)",
            sync_result.failed_files
        )
        .into());
    }

    if sync_result.failed_files > 0
        && sync_result.optimized_files == 0
        && sync_result.skipped_files == 0
    {
        return Err("all files failed during sync".into());
    }

    Ok(())
}

fn run_inspect(args: InspectArgs) -> Result<(), Box<dyn std::error::Error>> {
    let scan_opts = ScanOptions {
        include_globs: args.include,
        exclude_globs: args.exclude,
        ..Default::default()
    };
    let scan_res = scan_path(&args.input, &scan_opts)?;
    if scan_res.files.is_empty() && scan_res.errors.is_empty() {
        return Err(format!("no supported images found at '{}'", args.input.display()).into());
    }

    let mut limits = ResourceLimits::default();
    if args.max_width.is_some() {
        limits.max_width = args.max_width;
    }
    if args.max_height.is_some() {
        limits.max_height = args.max_height;
    }
    let decode_limits = limits.to_decode_limits();

    let is_single_file = args.input.is_file() && scan_res.files.len() == 1;
    let mut json_entries = Vec::new();
    let mut failed_count = 0usize;

    for scan_err in &scan_res.errors {
        failed_count += 1;
        if args.json {
            json_entries.push(serde_json::json!({
                "path": scan_err.source_path,
                "error": scan_err.to_string(),
            }));
        } else {
            eprintln!("Error scanning: {scan_err}");
        }
    }

    for file in &scan_res.files {
        let bytes_res =
            read_file_bounded(&file.path, limits.max_input_bytes, PipelineStage::Discovery);
        match bytes_res {
            Ok(bytes) => {
                // Check for animated GIF/WebP/APNG first when inspecting
                let mut fmt_opt = None;
                if let Ok(file_handle) = fs::File::open(&file.path) {
                    if let Ok(fmt) = detect_format_from_reader(file_handle) {
                        fmt_opt = Some(fmt);
                        if matches!(fmt, ImageFormat::Gif | ImageFormat::Webp | ImageFormat::Png) {
                            if let Ok(Some(anim)) =
                                decode_animation_with_limits(&bytes, fmt.into(), &decode_limits)
                            {
                                let dims = Dimensions::new(anim.width, anim.height);
                                if args.json {
                                    json_entries.push(serde_json::json!({
                                        "path": file.path,
                                        "format": fmt.as_str(),
                                        "animated": true,
                                        "frames": anim.frames.len(),
                                        "dimensions": {
                                            "width": dims.width,
                                            "height": dims.height,
                                            "pixel_count": dims.pixel_count(),
                                        },
                                        "color_type": "Rgba8",
                                        "byte_size": bytes.len(),
                                    }));
                                } else {
                                    println!("Image: {}", file.path.display());
                                    println!(
                                        "  Format:       {} (animated, {} frames)",
                                        fmt,
                                        anim.frames.len()
                                    );
                                    println!(
                                        "  Dimensions:   {}x{} ({} pixels)",
                                        dims.width,
                                        dims.height,
                                        dims.pixel_count()
                                    );
                                    println!("  Color:        Rgba8 (4 channels)");
                                    println!("  Size:         {} bytes", bytes.len());
                                    println!();
                                }
                                continue;
                            }
                        }
                    }
                }
                let _ = fmt_opt;

                match analyze_image(&bytes, Some(&limits)) {
                    Ok(analysis) => {
                        if args.json {
                            json_entries.push(serde_json::json!({
                                "path": file.path,
                                "format": analysis.profile.format.as_str(),
                                "animated": false,
                                "dimensions": {
                                    "width": analysis.profile.dimensions.width,
                                    "height": analysis.profile.dimensions.height,
                                    "pixel_count": analysis.profile.dimensions.pixel_count(),
                                },
                                "color_type": format!("{:?}", analysis.profile.color_type),
                                "byte_size": analysis.profile.byte_size,
                                "characteristics": {
                                    "has_transparency": analysis.characteristics.has_transparency,
                                    "is_grayscale": analysis.characteristics.is_grayscale,
                                    "entropy_hint": analysis.characteristics.entropy_hint,
                                },
                                "metadata": {
                                    "has_exif": analysis.profile.metadata.has_exif,
                                    "has_icc": analysis.profile.metadata.has_icc,
                                    "has_xmp": analysis.profile.metadata.has_xmp,
                                }
                            }));
                        } else {
                            println!("Image: {}", file.path.display());
                            println!("  Format:       {}", analysis.profile.format);
                            println!(
                                "  Dimensions:   {}x{} ({} pixels)",
                                analysis.profile.dimensions.width,
                                analysis.profile.dimensions.height,
                                analysis.profile.dimensions.pixel_count()
                            );
                            println!(
                                "  Color:        {:?} ({} channels)",
                                analysis.profile.color_type,
                                analysis.profile.color_type.channels()
                            );
                            println!("  Size:         {} bytes", analysis.profile.byte_size);
                            println!(
                                "  Transparency: {}",
                                analysis.characteristics.has_transparency
                            );
                            println!("  Grayscale:    {}", analysis.characteristics.is_grayscale);
                            println!(
                                "  Entropy:      {:.2} bits/byte",
                                analysis.characteristics.entropy_hint
                            );
                            println!(
                                "  Metadata:     EXIF: {}, ICC: {}, XMP: {}",
                                analysis.profile.metadata.has_exif,
                                analysis.profile.metadata.has_icc,
                                analysis.profile.metadata.has_xmp
                            );
                            println!();
                        }
                    }
                    Err(e) => {
                        failed_count += 1;
                        if args.json {
                            json_entries.push(serde_json::json!({
                                "path": file.path,
                                "error": e.to_string(),
                            }));
                        } else {
                            eprintln!("Error inspecting {}: {e}", file.path.display());
                        }
                    }
                }
            }
            Err(e) => {
                failed_count += 1;
                if args.json {
                    json_entries.push(serde_json::json!({
                        "path": file.path,
                        "error": e.to_string(),
                    }));
                } else {
                    eprintln!("Error inspecting {}: {e}", file.path.display());
                }
            }
        }
    }

    if args.json {
        if is_single_file {
            if let Some(first) = json_entries.first() {
                println!("{}", serde_json::to_string_pretty(first)?);
            }
        } else {
            println!("{}", serde_json::to_string_pretty(&json_entries)?);
        }
    }

    if failed_count > 0 {
        return Err(format!("inspection encountered {failed_count} error(s)").into());
    }

    Ok(())
}

fn run_verify(args: VerifyArgs) -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = args
        .manifest
        .unwrap_or_else(|| PathBuf::from(".imgopt-manifest.json"));

    let store = ManifestStore::new(&manifest_path);
    let manifest = store.load()?;

    println!("Verifying Manifest: {}", manifest_path.display());
    println!("  Schema Version:    {}", manifest.schema_version);
    println!("  Optimizer Version: {}", manifest.optimizer_version);
    println!("  Total Entries:     {}", manifest.entries.len());

    let storage_client = if args.check_storage {
        let fallback_bucket = args.bucket.clone().or_else(|| {
            manifest
                .entries
                .values()
                .find_map(|e| e.storage.as_ref().map(|s| s.bucket.clone()))
        });
        let s3_config = S3StorageConfig::from_env_with_cli_overrides(
            fallback_bucket,
            args.endpoint,
            None,
            None,
            args.force_path_style,
            None,
        )
        .map_err(|e| format!("storage configuration required for --check-storage: {e}"))?;
        Some(S3Client::new(s3_config)?)
    } else {
        None
    };

    let mut invalid_count = 0usize;
    let mut verified_count = 0usize;
    struct PendingStorageCheck {
        bucket: String,
        object_key: String,
        expected_size: u64,
        expected_etag: Option<String>,
        is_completed_status: bool,
    }
    let mut storage_checks: Vec<BatchTask<PendingStorageCheck>> = Vec::new();

    for (sha, entry) in &manifest.entries {
        if sha.is_empty() || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
            eprintln!("  [INVALID] Malformed SHA-256 key: {sha}");
            invalid_count += 1;
            continue;
        }
        if entry.source.sha256 != *sha {
            eprintln!("  [INVALID] SHA-256 key mismatch for entry: {sha}");
            invalid_count += 1;
            continue;
        }

        if matches!(
            entry.status,
            EntryStatus::Failed | EntryStatus::Pending | EntryStatus::Skipped
        ) {
            eprintln!(
                "  [INVALID] Entry {sha} is in non-completed status: {:?}",
                entry.status
            );
            invalid_count += 1;
            continue;
        }

        let Some(ref opt) = entry.optimization else {
            eprintln!("  [INVALID] Missing optimization record for entry: {sha}");
            invalid_count += 1;
            continue;
        };
        if opt.optimization_key.is_empty() {
            eprintln!("  [INVALID] Empty optimization key for entry: {sha}");
            invalid_count += 1;
            continue;
        }

        let is_completed_status =
            entry.status == EntryStatus::Uploaded || entry.status == EntryStatus::Verified;

        if storage_client.is_some() {
            let Some(ref st) = entry.storage else {
                eprintln!("  [INVALID] Missing storage record for entry: {sha}");
                invalid_count += 1;
                continue;
            };
            let bucket = args.bucket.as_deref().unwrap_or(&st.bucket).to_string();
            storage_checks.push(BatchTask::new(
                &st.object_key,
                PendingStorageCheck {
                    bucket,
                    object_key: st.object_key.clone(),
                    expected_size: opt.output_size,
                    expected_etag: st.etag.clone(),
                    is_completed_status,
                },
            ));
        } else if is_completed_status {
            verified_count += 1;
        }
    }

    if let Some(ref client) = storage_client {
        let batch_opts = BatchOptions {
            concurrency: args.concurrency.unwrap_or(8).max(1),
            stop_on_error: false,
        };
        let (results, _) = execute_batch_scoped(storage_checks, &batch_opts, |_, chk| {
            let outcome = match client.head_object(&chk.bucket, &chk.object_key) {
                Ok(Some(meta)) => {
                    if meta.byte_size != chk.expected_size {
                        Err(format!(
                            "  [MISMATCH] Size mismatch for object {}: expected {} bytes, got {} bytes",
                            chk.object_key, chk.expected_size, meta.byte_size
                        ))
                    } else if let (Some(ref expected_etag), Some(ref actual_etag)) =
                        (&chk.expected_etag, &meta.etag)
                    {
                        let norm_exp = expected_etag.trim_matches('"');
                        let norm_act = actual_etag.trim_matches('"');
                        if !norm_exp.is_empty() && !norm_act.is_empty() && norm_exp != norm_act {
                            Err(format!(
                                "  [MISMATCH] ETag mismatch for object {}: expected {norm_exp}, got {norm_act}",
                                chk.object_key
                            ))
                        } else {
                            Ok(chk.is_completed_status)
                        }
                    } else {
                        Ok(chk.is_completed_status)
                    }
                }
                Ok(None) => Err(format!(
                    "  [MISSING] Object not found in storage: {}/{}",
                    chk.bucket, chk.object_key
                )),
                Err(e) => Err(format!(
                    "  [ERROR] Storage check error for {}: {e}",
                    chk.object_key
                )),
            };
            Ok(outcome)
        });

        for r in results {
            match r.output {
                Ok(Ok(is_completed)) => {
                    if is_completed {
                        verified_count += 1;
                    }
                }
                Ok(Err(msg)) => {
                    eprintln!("{msg}");
                    invalid_count += 1;
                }
                Err(e) => {
                    eprintln!("  [ERROR] {e}");
                    invalid_count += 1;
                }
            }
        }
    }

    println!("\nVerification Summary:");
    println!("  Valid/Verified entries: {verified_count}");
    println!("  Invalid entries:        {invalid_count}");

    if invalid_count > 0 {
        return Err(format!("manifest verification failed with {invalid_count} errors").into());
    }

    Ok(())
}

fn run_benchmark_cmd(args: BenchmarkArgs) -> Result<(), Box<dyn std::error::Error>> {
    let config = build_optimization_config(&args.opt, &args.format)?;
    let scan_opts = args.filter.to_scan_options();

    let concurrency = args
        .filter
        .concurrency
        .unwrap_or_else(|| BatchOptions::default().concurrency)
        .max(1);
    let report = run_benchmark_with_options(&args.dataset, &config, concurrency, &scan_opts)?;

    if let Some(ref json_path) = args.json {
        let json_str = report.to_json_pretty()?;
        write_atomically(json_path, json_str.as_bytes())?;
    }

    println!("\nBenchmark Summary:");
    println!("  Total cases:        {}", report.summary.total_cases);
    println!("  Successful cases:   {}", report.summary.successful_cases);
    println!("  Failed cases:       {}", report.summary.failed_cases);
    println!(
        "  Total original:     {} bytes",
        report.summary.total_original_bytes
    );
    println!(
        "  Total optimized:    {} bytes",
        report.summary.total_optimized_bytes
    );
    println!(
        "  Total saved:        {} bytes ({:.2}%)",
        report.summary.total_saved_bytes,
        report.summary.overall_savings_ratio * 100.0
    );
    println!(
        "  Average quality:    {:.4}",
        report.summary.average_quality_score
    );
    println!(
        "  Elapsed time:       {} ms",
        report.summary.total_elapsed_millis
    );

    if report.summary.total_cases == 0 {
        return Err(format!("no supported images found at '{}'", args.dataset.display()).into());
    }
    if args.fail_on_error && report.summary.failed_cases > 0 {
        return Err(format!(
            "benchmark finished with {} failed case(s)",
            report.summary.failed_cases
        )
        .into());
    }
    if report.summary.failed_cases > 0 && report.summary.successful_cases == 0 {
        return Err("all benchmark cases failed".into());
    }

    Ok(())
}

fn run_gc(args: GcArgs) -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = args
        .manifest
        .unwrap_or_else(|| PathBuf::from(".imgopt-manifest.json"));
    let store = ManifestStore::new(&manifest_path);
    let manifest = store.load()?;

    let fallback_bucket = args.bucket.clone().or_else(|| {
        manifest
            .entries
            .values()
            .find_map(|e| e.storage.as_ref().map(|s| s.bucket.clone()))
    });

    let s3_config = S3StorageConfig::from_env_with_cli_overrides(
        fallback_bucket,
        args.endpoint,
        args.prefix,
        None,
        args.force_path_style,
        None,
    )
    .map_err(|e| format!("storage configuration required for gc: {e}"))?;

    let bucket = s3_config.bucket.clone();
    let scan_prefix = match s3_config.key_prefix.as_deref() {
        Some(p) if !p.trim().is_empty() => {
            format!("{}/images/", p.trim().trim_matches('/'))
        }
        _ => "images/".to_string(),
    };
    let client = S3Client::new(s3_config)?;

    let active_keys: HashSet<String> = manifest
        .entries
        .values()
        .filter_map(|e| e.storage.as_ref())
        .filter(|st| st.bucket == bucket)
        .map(|st| st.object_key.clone())
        .collect();

    let remote_objects = client.list_objects(&bucket, Some(&scan_prefix))?;
    let mut orphan_keys = Vec::new();
    let mut orphan_bytes = 0u64;

    for obj in &remote_objects {
        if !active_keys.contains(&obj.object_key) {
            orphan_keys.push(obj.object_key.clone());
            orphan_bytes = orphan_bytes.saturating_add(obj.byte_size);
        }
    }

    let mut deleted_keys = Vec::new();
    let mut delete_errors = Vec::new();
    if args.delete {
        for key in &orphan_keys {
            match client.delete_object(&bucket, key) {
                Ok(()) => deleted_keys.push(key.clone()),
                Err(e) => delete_errors.push(format!("{key}: {e}")),
            }
        }
    }

    if let Some(ref json_path) = args.json {
        let report = serde_json::json!({
            "bucket": bucket,
            "prefix": scan_prefix,
            "dry_run": !args.delete,
            "remote_objects": remote_objects.len(),
            "active_manifest_keys": active_keys.len(),
            "orphaned_objects": orphan_keys,
            "orphaned_bytes": orphan_bytes,
            "deleted_objects": deleted_keys,
            "errors": delete_errors,
        });
        write_atomically(json_path, serde_json::to_string_pretty(&report)?.as_bytes())?;
    }

    println!(
        "GC Summary ({}):",
        if args.delete { "DELETE" } else { "DRY RUN" }
    );
    println!("  Bucket:            {bucket}");
    println!("  Scanned prefix:    {scan_prefix}");
    println!("  Remote objects:    {}", remote_objects.len());
    println!("  Active in manifest:{}", active_keys.len());
    println!(
        "  Orphaned objects:  {} ({orphan_bytes} bytes)",
        orphan_keys.len()
    );
    if args.delete {
        println!("  Deleted objects:   {}", deleted_keys.len());
    } else if !orphan_keys.is_empty() {
        for key in &orphan_keys {
            println!("  [ORPHAN] {key}");
        }
        println!("  Re-run with --delete to remove orphaned objects.");
    }

    if !delete_errors.is_empty() {
        for err in &delete_errors {
            eprintln!("  [ERROR] {err}");
        }
        return Err(format!("gc failed to delete {} object(s)", delete_errors.len()).into());
    }

    Ok(())
}

fn write_atomically(target_path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let parent = match target_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    if !parent.exists() {
        fs::create_dir_all(parent)?;
    }

    let mut tmp_file = tempfile::Builder::new()
        .prefix(".tmp-imgopt-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    tmp_file.write_all(bytes)?;
    tmp_file.as_file().sync_all()?;
    tmp_file.persist(target_path).map_err(|e| e.error)?;
    Ok(())
}
