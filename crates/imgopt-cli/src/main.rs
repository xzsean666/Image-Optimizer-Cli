use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use imgopt_core::{
    analyze_image, execute_batch_scoped, read_file_bounded, run_benchmark_with_concurrency,
    scan_path, BatchOptions, BatchTask, ImageFormat, MetadataPolicy, OptimizationConfig,
    OptimizationPreset, OptimizationService, OutputFormatPolicy, PipelineStage, ResourceLimits,
    ScanOptions, SyncItemStatus, SyncRunResult, SyncService,
};
use imgopt_manifest::{
    EntryStatus, ManifestStore, ManifestSyncAdapter, RunItemStatus, RunResult, RunResultRecord,
};
use imgopt_storage::{
    MemoryObjectStorage, ObjectStorage, RetryPolicy, S3Client, S3StorageConfig, StorageSyncAdapter,
};

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
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            continue;
        }

        let val_trimmed = v.trim();
        let val = if (val_trimmed.starts_with('"') && val_trimmed.ends_with('"') && val_trimmed.len() >= 2)
            || (val_trimmed.starts_with('\'') && val_trimmed.ends_with('\'') && val_trimmed.len() >= 2)
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

fn parse_env_file_arg() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    for i in 0..args.len() {
        if args[i] == "--" {
            break;
        }
        if args[i] == "--env-file" && i + 1 < args.len() {
            return Some(PathBuf::from(&args[i + 1]));
        }
        if let Some(val) = args[i].strip_prefix("--env-file=") {
            return Some(PathBuf::from(val));
        }
    }
    None
}

fn main() -> ExitCode {
    if let Some(explicit_env) = parse_env_file_arg() {
        if let Err(err) = load_dotenv_file(&explicit_env, true) {
            eprintln!("Error: {err}");
            return ExitCode::FAILURE;
        }
    } else if let Err(err) = load_dotenv_file(Path::new(".env"), false) {
        eprintln!("Error: {err}");
        return ExitCode::FAILURE;
    }

    let cli = Cli::parse();
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
}

#[derive(Parser, Debug)]
struct OptimizeArgs {
    /// Input file or directory to optimize
    input: PathBuf,

    /// Output destination directory or file path
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Optimization preset (balanced, maximum, aggressive)
    #[arg(long, value_enum, default_value_t = PresetArg::Balanced)]
    preset: PresetArg,

    /// Target quality (0.0 to 1.0)
    #[arg(short, long)]
    quality: Option<f64>,

    /// Encoding effort level (1 to 6)
    #[arg(short, long)]
    effort: Option<u8>,

    /// Output format policy (preserve, webp, jpeg, png)
    #[arg(long, default_value = "preserve")]
    format: String,

    /// Preserve EXIF / ICC / XMP metadata (default is strip all)
    #[arg(long)]
    preserve_metadata: bool,

    /// Max concurrency for worker pool
    #[arg(long)]
    concurrency: Option<usize>,

    /// Maximum allowed image width in pixels
    #[arg(long)]
    max_width: Option<u32>,

    /// Maximum allowed image height in pixels
    #[arg(long)]
    max_height: Option<u32>,
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

    /// Optimization preset (balanced, maximum, aggressive)
    #[arg(long, value_enum, default_value_t = PresetArg::Balanced)]
    preset: PresetArg,

    /// Target quality (0.0 to 1.0)
    #[arg(short, long)]
    quality: Option<f64>,

    /// Encoding effort level (1 to 6)
    #[arg(short, long)]
    effort: Option<u8>,

    /// Output format policy (preserve, webp, jpeg, png)
    #[arg(long, default_value = "webp")]
    format: String,

    /// Preserve EXIF / ICC / XMP metadata (default is strip all)
    #[arg(long)]
    preserve_metadata: bool,

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

    /// Max concurrency for worker pool
    #[arg(long)]
    concurrency: Option<usize>,

    /// Maximum allowed image width in pixels
    #[arg(long)]
    max_width: Option<u32>,

    /// Maximum allowed image height in pixels
    #[arg(long)]
    max_height: Option<u32>,

    /// Use mock in-memory storage for dry runs or testing without remote S3 credentials
    #[arg(long)]
    dry_run: bool,
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

#[derive(Parser, Debug)]
struct InspectArgs {
    /// Input file or directory to inspect
    input: PathBuf,

    /// Output inspection report as machine-readable JSON
    #[arg(long)]
    json: bool,
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
}

#[derive(Parser, Debug)]
struct BenchmarkArgs {
    /// Dataset directory or image file to benchmark
    dataset: PathBuf,

    /// Optimization preset (balanced, maximum, aggressive)
    #[arg(long, value_enum, default_value_t = PresetArg::Balanced)]
    preset: PresetArg,

    /// Target quality (0.0 to 1.0)
    #[arg(short, long)]
    quality: Option<f64>,

    /// Encoding effort level (1 to 6)
    #[arg(short, long)]
    effort: Option<u8>,

    /// Output format policy (preserve, webp, jpeg, png)
    #[arg(long, default_value = "preserve")]
    format: String,

    /// Preserve EXIF / ICC / XMP metadata (default is strip all)
    #[arg(long)]
    preserve_metadata: bool,

    /// Max concurrency for worker pool
    #[arg(long)]
    concurrency: Option<usize>,

    /// Maximum allowed image width in pixels
    #[arg(long)]
    max_width: Option<u32>,

    /// Maximum allowed image height in pixels
    #[arg(long)]
    max_height: Option<u32>,

    /// Path to write machine-readable benchmark JSON report
    #[arg(long)]
    json: Option<PathBuf>,
}

fn parse_format_policy(format: &str) -> Result<OutputFormatPolicy, Box<dyn std::error::Error>> {
    match format.trim().to_ascii_lowercase().as_str() {
        "preserve" | "original" => Ok(OutputFormatPolicy::PreserveOriginal),
        "webp" => Ok(OutputFormatPolicy::AllowWebp),
        "jpeg" | "jpg" => Ok(OutputFormatPolicy::Force(ImageFormat::Jpeg)),
        "png" => Ok(OutputFormatPolicy::Force(ImageFormat::Png)),
        other => Err(format!("unsupported format policy: {other}").into()),
    }
}

struct OptimizationCliOptions<'a> {
    preset: PresetArg,
    quality: Option<f64>,
    effort: Option<u8>,
    format: &'a str,
    preserve_metadata: bool,
    max_width: Option<u32>,
    max_height: Option<u32>,
}

fn build_optimization_config(
    opts: OptimizationCliOptions<'_>,
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
    }
    if opts.max_width.is_some() {
        config.max_width = opts.max_width;
    }
    if opts.max_height.is_some() {
        config.max_height = opts.max_height;
    }
    config.output_format_policy = parse_format_policy(opts.format)?;
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
    }
}

fn is_safe_relative(path: &Path) -> bool {
    !path.is_absolute()
        && !path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
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
    let config = build_optimization_config(OptimizationCliOptions {
        preset: args.preset,
        quality: args.quality,
        effort: args.effort,
        format: &args.format,
        preserve_metadata: args.preserve_metadata,
        max_width: args.max_width,
        max_height: args.max_height,
    })?;

    let service = OptimizationService::new(env!("CARGO_PKG_VERSION"));
    let scan_res = scan_path(&args.input, &ScanOptions::default())?;

    if scan_res.files.is_empty() {
        return Err(format!("no supported images found at '{}'", args.input.display()).into());
    }

    let is_single_file = scan_res.files.len() == 1 && args.input.is_file();
    let default_concurrency = BatchOptions::default().concurrency;
    let batch_opts = BatchOptions {
        concurrency: args.concurrency.unwrap_or(default_concurrency).max(1),
        stop_on_error: false,
    };

    let tasks: Vec<BatchTask<()>> = scan_res
        .files
        .iter()
        .map(|f| BatchTask::new(&f.path, ()))
        .collect();

    let (batch_results, _) = execute_batch_scoped(tasks, &batch_opts, |path, ()| {
        service.optimize_file(&path, &config)
    });

    let mut total_original_bytes = 0u64;
    let mut total_optimized_bytes = 0u64;
    let mut processed_count = 0usize;
    let mut failed_count = 0usize;
    let mut claimed_destinations = HashSet::new();

    for res in batch_results {
        let src_path = res.source_path;
        let opt_res = match res.output {
            Ok(r) => r,
            Err(e) => {
                failed_count += 1;
                eprintln!("Failed: {}: {e}", src_path.display());
                continue;
            }
        };

        let raw_dest_path = if is_single_file {
            if let Some(ref out_dest) = args.output {
                let is_dest_dir = out_dest.is_dir()
                    || out_dest.extension().is_none()
                    || out_dest.to_string_lossy().ends_with('/')
                    || out_dest.to_string_lossy().ends_with('\\');

                if is_dest_dir {
                    let orig_name = src_path.file_stem().unwrap_or_default();
                    out_dest.join(format!(
                        "{}.{}",
                        orig_name.to_string_lossy(),
                        opt_res.effective_format.extension()
                    ))
                } else {
                    out_dest.clone()
                }
            } else {
                let orig_stem = src_path.file_stem().unwrap_or_default();
                src_path.with_file_name(format!(
                    "{}-optimized.{}",
                    orig_stem.to_string_lossy(),
                    opt_res.effective_format.extension()
                ))
            }
        } else {
            let relative = src_path
                .strip_prefix(&args.input)
                .map_err(|_| {
                    format!(
                        "discovered path '{}' escaped input root '{}'",
                        src_path.display(),
                        args.input.display()
                    )
                })?;
            if !is_safe_relative(relative) {
                failed_count += 1;
                eprintln!(
                    "Failed: {}: unsafe relative path '{}'",
                    src_path.display(),
                    relative.display()
                );
                continue;
            }
            let out_base = args
                .output
                .as_ref()
                .cloned()
                .unwrap_or_else(|| PathBuf::from("optimized_output"));

            let stem = relative.file_stem().unwrap_or_default();
            let parent = relative.parent().unwrap_or_else(|| Path::new(""));
            out_base.join(parent).join(format!(
                "{}.{}",
                stem.to_string_lossy(),
                opt_res.effective_format.extension()
            ))
        };

        let dest_path = if is_single_file {
            raw_dest_path
        } else {
            resolve_unique_dest_path(raw_dest_path, &src_path, &mut claimed_destinations)
        };

        if let Err(e) = write_atomically(&dest_path, &opt_res.validated_output.bytes) {
            failed_count += 1;
            eprintln!("Failed writing {}: {e}", dest_path.display());
            continue;
        }

        total_original_bytes += opt_res.validated_output.original_size;
        total_optimized_bytes += opt_res.validated_output.byte_size;
        processed_count += 1;

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

    let overall_saved = if total_original_bytes >= total_optimized_bytes {
        i64::try_from(total_original_bytes - total_optimized_bytes).unwrap_or(i64::MAX)
    } else {
        -i64::try_from(total_optimized_bytes - total_original_bytes).unwrap_or(i64::MAX)
    };
    let overall_ratio = if total_original_bytes > 0 {
        (overall_saved as f64) / (total_original_bytes as f64) * 100.0
    } else {
        0.0
    };

    println!("\nSummary:");
    println!("  Processed: {processed_count} files");
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
    let config = build_optimization_config(OptimizationCliOptions {
        preset: args.preset,
        quality: args.quality,
        effort: args.effort,
        format: &args.format,
        preserve_metadata: args.preserve_metadata,
        max_width: args.max_width,
        max_height: args.max_height,
    })?;

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
        concurrency: args.concurrency.unwrap_or(4).max(1),
        stop_on_error: false,
    };

    let sync_result: SyncRunResult = if args.dry_run {
        let bucket = args.bucket.unwrap_or_else(|| "dry-run-bucket".into());
        let storage = MemoryObjectStorage::new();
        let storage_adapter = StorageSyncAdapter::new(
            storage,
            args.public_url_base
                .or_else(|| Some("https://dry-run.local".into())),
            args.prefix,
            RetryPolicy::default(),
        );
        let sync_service = SyncService::new(version, batch_options, &bucket);
        sync_service.run_sync(
            &args.input,
            &config,
            &storage_adapter,
            &mut manifest_adapter,
        )?
    } else {
        let s3_config = S3StorageConfig::from_env_with_overrides(
            args.bucket,
            args.endpoint,
            args.prefix,
            args.public_url_base,
        )
        .map_err(|e| format!("storage configuration failed: {e}"))?;

        let bucket = s3_config.bucket.clone();
        let key_prefix = s3_config.key_prefix.clone();
        // Only emit public URLs when an explicit public URL base is configured;
        // never persist credential-bearing presigned URLs or private API endpoints.
        let public_url_base = s3_config.public_url_base.clone();
        let s3_client = S3Client::new(s3_config)?;
        let storage_adapter = StorageSyncAdapter::new(
            s3_client,
            public_url_base,
            key_prefix,
            RetryPolicy::default(),
        );
        let sync_service = SyncService::new(version, batch_options, &bucket);
        sync_service.run_sync(
            &args.input,
            &config,
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
    for item in &sync_result.items {
        if let Some(ref err) = item.error {
            eprintln!("  - {}: {}", item.path.display(), err);
        } else if let Some(ref url) = item.remote_url {
            println!("  + {}: {}", item.path.display(), url);
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

    if sync_result.failed_files > 0
        && sync_result.optimized_files == 0
        && sync_result.skipped_files == 0
    {
        return Err("all files failed during sync".into());
    }

    Ok(())
}

fn run_inspect(args: InspectArgs) -> Result<(), Box<dyn std::error::Error>> {
    let scan_res = scan_path(&args.input, &ScanOptions::default())?;
    if scan_res.files.is_empty() {
        return Err(format!("no supported images found at '{}'", args.input.display()).into());
    }

    let limits = ResourceLimits::default();
    let is_single_file = args.input.is_file() && scan_res.files.len() == 1;
    let mut json_entries = Vec::new();
    let mut failed_count = 0usize;

    for file in &scan_res.files {
        let inspection = read_file_bounded(&file.path, limits.max_input_bytes, PipelineStage::Discovery)
            .and_then(|bytes| analyze_image(&bytes, Some(&limits)));

        match inspection {
            Ok(analysis) => {
                if args.json {
                    json_entries.push(serde_json::json!({
                        "path": file.path,
                        "format": analysis.profile.format.as_str(),
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
        let s3_config = S3StorageConfig::from_env_with_overrides(
            fallback_bucket,
            args.endpoint,
            None,
            None,
        )
        .map_err(|e| format!("storage configuration required for --check-storage: {e}"))?;
        Some(S3Client::new(s3_config)?)
    } else {
        None
    };

    let mut invalid_count = 0usize;
    let mut verified_count = 0usize;

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

        if matches!(entry.status, EntryStatus::Failed | EntryStatus::Pending) {
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

        if let Some(ref client) = storage_client {
            let Some(ref st) = entry.storage else {
                eprintln!("  [INVALID] Missing storage record for entry: {sha}");
                invalid_count += 1;
                continue;
            };
            let bucket = args.bucket.as_deref().unwrap_or(&st.bucket);
            match client.head_object(bucket, &st.object_key) {
                Ok(Some(meta)) => {
                    if meta.byte_size != opt.output_size {
                        eprintln!(
                            "  [MISMATCH] Size mismatch for object {}: expected {} bytes, got {} bytes",
                            st.object_key, opt.output_size, meta.byte_size
                        );
                        invalid_count += 1;
                        continue;
                    }
                    if let (Some(ref expected_etag), Some(ref actual_etag)) = (&st.etag, &meta.etag)
                    {
                        let norm_exp = expected_etag.trim_matches('"');
                        let norm_act = actual_etag.trim_matches('"');
                        if !norm_exp.is_empty() && !norm_act.is_empty() && norm_exp != norm_act {
                            eprintln!(
                                "  [MISMATCH] ETag mismatch for object {}: expected {norm_exp}, got {norm_act}",
                                st.object_key
                            );
                            invalid_count += 1;
                            continue;
                        }
                    }
                }
                Ok(None) => {
                    eprintln!(
                        "  [MISSING] Object not found in storage: {bucket}/{}",
                        st.object_key
                    );
                    invalid_count += 1;
                    continue;
                }
                Err(e) => {
                    eprintln!("  [ERROR] Storage check error for {}: {e}", st.object_key);
                    invalid_count += 1;
                    continue;
                }
            }
        }

        if entry.status == EntryStatus::Uploaded || entry.status == EntryStatus::Verified {
            verified_count += 1;
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
    let config = build_optimization_config(OptimizationCliOptions {
        preset: args.preset,
        quality: args.quality,
        effort: args.effort,
        format: &args.format,
        preserve_metadata: args.preserve_metadata,
        max_width: args.max_width,
        max_height: args.max_height,
    })?;

    let concurrency = args
        .concurrency
        .unwrap_or_else(|| BatchOptions::default().concurrency)
        .max(1);
    let report = run_benchmark_with_concurrency(&args.dataset, &config, concurrency)?;

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
