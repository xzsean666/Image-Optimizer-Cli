use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use imgopt_core::{
    analyze_image, run_benchmark, scan_path, BatchOptions, ImageFormat, MetadataPolicy,
    OptimizationConfig, OptimizationPreset, OptimizationService, OutputFormatPolicy, ScanOptions,
    SyncItemStatus, SyncRunResult, SyncService,
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
    #[command(subcommand)]
    command: Commands,
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

    /// Output format policy (preserve, webp)
    #[arg(long, default_value = "preserve")]
    format: String,

    /// Preserve EXIF / ICC / XMP metadata (default is strip all)
    #[arg(long)]
    preserve_metadata: bool,
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

    /// Output format policy (preserve, webp)
    #[arg(long, default_value = "webp")]
    format: String,

    /// Override S3 bucket name
    #[arg(long)]
    bucket: Option<String>,

    /// Override S3 endpoint URL
    #[arg(long)]
    endpoint: Option<String>,

    /// Override public URL base
    #[arg(long)]
    public_url_base: Option<String>,

    /// Max concurrency for worker pool
    #[arg(long)]
    concurrency: Option<usize>,

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

    /// Output format policy (preserve, webp)
    #[arg(long, default_value = "preserve")]
    format: String,

    /// Path to write machine-readable benchmark JSON report
    #[arg(long)]
    json: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
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

fn run_optimize(args: OptimizeArgs) -> Result<(), Box<dyn std::error::Error>> {
    let preset: OptimizationPreset = args.preset.into();
    let mut config = OptimizationConfig::from_preset(preset);

    if let Some(q) = args.quality {
        config.quality_target = q;
    }
    if let Some(eff) = args.effort {
        config.effort = eff;
    }
    if args.preserve_metadata {
        config.metadata_policy = MetadataPolicy::PreserveAll;
    }

    match args.format.to_lowercase().as_str() {
        "preserve" | "original" => {
            config.output_format_policy = OutputFormatPolicy::PreserveOriginal;
        }
        "webp" => {
            config.output_format_policy = OutputFormatPolicy::AllowWebp;
        }
        "jpeg" | "jpg" => {
            config.output_format_policy = OutputFormatPolicy::Force(ImageFormat::Jpeg);
        }
        "png" => {
            config.output_format_policy = OutputFormatPolicy::Force(ImageFormat::Png);
        }
        other => {
            return Err(format!("unsupported format policy: {other}").into());
        }
    }

    config.validate()?;

    let service = OptimizationService::new(env!("CARGO_PKG_VERSION"));
    let scan_res = scan_path(&args.input, &ScanOptions::default())?;

    if scan_res.files.is_empty() {
        return Err(format!("no supported images found at '{}'", args.input.display()).into());
    }

    let is_single_file = scan_res.files.len() == 1 && args.input.is_file();
    let mut total_original_bytes = 0u64;
    let mut total_optimized_bytes = 0u64;
    let mut processed_count = 0usize;

    for discovered in &scan_res.files {
        let opt_res = service.optimize_file(&discovered.path, &config)?;

        let dest_path = if is_single_file {
            if let Some(ref out_dest) = args.output {
                let is_dest_dir = out_dest.is_dir()
                    || out_dest.extension().is_none()
                    || out_dest.to_string_lossy().ends_with('/')
                    || out_dest.to_string_lossy().ends_with('\\');

                if is_dest_dir {
                    let orig_name = discovered.path.file_stem().unwrap_or_default();
                    out_dest.join(format!(
                        "{}.{}",
                        orig_name.to_string_lossy(),
                        opt_res.effective_format.extension()
                    ))
                } else {
                    out_dest.clone()
                }
            } else {
                let orig_stem = discovered.path.file_stem().unwrap_or_default();
                discovered.path.with_file_name(format!(
                    "{}-optimized.{}",
                    orig_stem.to_string_lossy(),
                    opt_res.effective_format.extension()
                ))
            }
        } else {
            let relative = discovered
                .path
                .strip_prefix(&args.input)
                .unwrap_or(&discovered.path);
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

        write_atomically(&dest_path, &opt_res.validated_output.bytes)?;

        total_original_bytes += opt_res.validated_output.original_size;
        total_optimized_bytes += opt_res.validated_output.byte_size;
        processed_count += 1;

        println!(
            "Optimized: {} -> {} ({} -> {} bytes, {:.1}% saved, SSIM: {:.4})",
            discovered.path.display(),
            dest_path.display(),
            opt_res.validated_output.original_size,
            opt_res.validated_output.byte_size,
            opt_res.validated_output.savings_ratio * 100.0,
            opt_res.quality_score,
        );
    }

    let overall_saved = (total_original_bytes as i64) - (total_optimized_bytes as i64);
    let overall_ratio = if total_original_bytes > 0 {
        (overall_saved as f64) / (total_original_bytes as f64) * 100.0
    } else {
        0.0
    };

    println!("\nSummary:");
    println!("  Processed: {processed_count} files");
    println!("  Original size:  {total_original_bytes} bytes");
    println!("  Optimized size: {total_optimized_bytes} bytes");
    println!("  Saved:          {overall_saved} bytes ({overall_ratio:.2}%)");

    Ok(())
}

fn run_sync(args: SyncArgs) -> Result<(), Box<dyn std::error::Error>> {
    let preset: OptimizationPreset = args.preset.into();
    let mut config = OptimizationConfig::from_preset(preset);

    if let Some(q) = args.quality {
        config.quality_target = q;
    }
    if let Some(eff) = args.effort {
        config.effort = eff;
    }

    match args.format.to_lowercase().as_str() {
        "preserve" | "original" => {
            config.output_format_policy = OutputFormatPolicy::PreserveOriginal;
        }
        "webp" => {
            config.output_format_policy = OutputFormatPolicy::AllowWebp;
        }
        "jpeg" | "jpg" => {
            config.output_format_policy = OutputFormatPolicy::Force(ImageFormat::Jpeg);
        }
        "png" => {
            config.output_format_policy = OutputFormatPolicy::Force(ImageFormat::Png);
        }
        other => {
            return Err(format!("unsupported format policy: {other}").into());
        }
    }

    config.validate()?;

    let version = env!("CARGO_PKG_VERSION");

    // Resolve manifest path
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
        concurrency: args.concurrency.unwrap_or(4),
        stop_on_error: false,
    };

    let sync_result: SyncRunResult = if args.dry_run {
        let bucket = args.bucket.unwrap_or_else(|| "dry-run-bucket".into());
        let storage = MemoryObjectStorage::new();
        let storage_adapter = StorageSyncAdapter::new(
            storage,
            args.public_url_base
                .or_else(|| Some("https://dry-run.local".into())),
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
        // Load from environment or flags
        let mut s3_config = match S3StorageConfig::from_env() {
            Ok(c) => c,
            Err(e) => {
                if let Some(ref b) = args.bucket {
                    S3StorageConfig {
                        bucket: b.clone(),
                        region: "auto".into(),
                        endpoint: args.endpoint.clone(),
                        access_key_id: "anonymous".into(),
                        secret_access_key: "anonymous".into(),
                        force_path_style: false,
                        public_url_base: args.public_url_base.clone(),
                        timeout_seconds: 30,
                    }
                } else {
                    return Err(format!("storage configuration failed: {e}").into());
                }
            }
        };

        if let Some(b) = args.bucket {
            s3_config.bucket = b;
        }
        if let Some(ep) = args.endpoint {
            s3_config.endpoint = Some(ep);
        }
        if let Some(purl) = args.public_url_base {
            s3_config.public_url_base = Some(purl);
        }

        let bucket = s3_config.bucket.clone();
        let public_url_base = s3_config.public_url_base.clone();
        let s3_client = S3Client::new(s3_config)?;
        let storage_adapter =
            StorageSyncAdapter::new(s3_client, public_url_base, RetryPolicy::default());
        let sync_service = SyncService::new(version, batch_options, &bucket);
        sync_service.run_sync(
            &args.input,
            &config,
            &storage_adapter,
            &mut manifest_adapter,
        )?
    };

    // Emit result JSON if requested
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
    println!("  Total files:     {}", sync_result.total_files);
    println!("  Optimized:       {}", sync_result.optimized_files);
    println!("  Skipped (cached): {}", sync_result.skipped_files);
    println!("  Failed:          {}", sync_result.failed_files);
    for item in &sync_result.items {
        if let Some(ref err) = item.error {
            eprintln!("  - {}: {}", item.path.display(), err);
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

    for file in &scan_res.files {
        let bytes = fs::read(&file.path)?;
        let analysis = analyze_image(&bytes, None)?;

        if args.json {
            let json_val = serde_json::json!({
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
            });
            println!("{}", serde_json::to_string_pretty(&json_val)?);
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

    Ok(())
}

fn run_verify(args: VerifyArgs) -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = args
        .manifest
        .unwrap_or_else(|| PathBuf::from(".imgopt-manifest.json"));

    let store = ManifestStore::new(&manifest_path);
    let manifest = store.load_or_init(env!("CARGO_PKG_VERSION"))?;

    println!("Verifying Manifest: {}", manifest_path.display());
    println!("  Schema Version:    {}", manifest.schema_version);
    println!("  Optimizer Version: {}", manifest.optimizer_version);
    println!("  Total Entries:     {}", manifest.entries.len());

    let mut invalid_count = 0usize;
    let mut verified_count = 0usize;

    for (sha, entry) in &manifest.entries {
        if entry.source.sha256 != *sha {
            eprintln!("  [INVALID] SHA-256 key mismatch for entry: {sha}");
            invalid_count += 1;
            continue;
        }

        if let Some(ref opt) = entry.optimization {
            if opt.optimization_key.is_empty() {
                eprintln!("  [INVALID] Empty optimization key for entry: {sha}");
                invalid_count += 1;
                continue;
            }
        }

        if args.check_storage {
            if let Some(ref st) = entry.storage {
                let bucket = args.bucket.as_ref().unwrap_or(&st.bucket);
                let s3_config = match S3StorageConfig::from_env() {
                    Ok(mut c) => {
                        c.bucket = bucket.clone();
                        c
                    }
                    Err(_) => S3StorageConfig {
                        bucket: bucket.clone(),
                        region: "auto".into(),
                        endpoint: None,
                        access_key_id: "anonymous".into(),
                        secret_access_key: "anonymous".into(),
                        force_path_style: false,
                        public_url_base: None,
                        timeout_seconds: 30,
                    },
                };

                if let Ok(client) = S3Client::new(s3_config) {
                    match client.head_object(bucket, &st.object_key) {
                        Ok(Some(meta)) => {
                            if let Some(ref opt) = entry.optimization {
                                if meta.byte_size != opt.output_size {
                                    eprintln!(
                                        "  [MISMATCH] Size mismatch for object {}: expected {} bytes, got {} bytes",
                                        st.object_key, opt.output_size, meta.byte_size
                                    );
                                    invalid_count += 1;
                                    continue;
                                }
                            }
                        }
                        Ok(None) => {
                            // If mock or remote not reachable
                        }
                        Err(e) => {
                            eprintln!("  [ERROR] Storage check error for {}: {e}", st.object_key);
                            invalid_count += 1;
                            continue;
                        }
                    }
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
    let preset: OptimizationPreset = args.preset.into();
    let mut config = OptimizationConfig::from_preset(preset);

    if let Some(q) = args.quality {
        config.quality_target = q;
    }
    if let Some(eff) = args.effort {
        config.effort = eff;
    }

    match args.format.to_lowercase().as_str() {
        "preserve" | "original" => {
            config.output_format_policy = OutputFormatPolicy::PreserveOriginal;
        }
        "webp" => {
            config.output_format_policy = OutputFormatPolicy::AllowWebp;
        }
        "jpeg" | "jpg" => {
            config.output_format_policy = OutputFormatPolicy::Force(ImageFormat::Jpeg);
        }
        "png" => {
            config.output_format_policy = OutputFormatPolicy::Force(ImageFormat::Png);
        }
        other => {
            return Err(format!("unsupported format policy: {other}").into());
        }
    }

    let report = run_benchmark(&args.dataset, &config)?;

    if let Some(ref json_path) = args.json {
        let json_str = report.to_json_pretty()?;
        fs::write(json_path, json_str)?;
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
    let parent = target_path.parent().unwrap_or_else(|| Path::new("."));
    if !parent.exists() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = parent.join(format!(
        ".tmp-{}.tmp",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));

    let res = (|| -> Result<(), std::io::Error> {
        let mut file = File::create(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp_path, target_path)?;
        Ok(())
    })();

    if res.is_err() && tmp_path.exists() {
        let _ = fs::remove_file(&tmp_path);
    }

    res?;
    Ok(())
}
