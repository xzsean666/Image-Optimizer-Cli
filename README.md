# Image Optimizer (`imgopt`)

一个高性能、确定性且幂等的 Rust 图像优化引擎与命令行工具，支持**本地批量压缩与尺寸缩放**以及 **S3 兼容云存储增量同步与清理**。

你可以通过 **两种方式** 使用本项目：
1. **作为独立命令行工具（`imgopt`）** —— 从 GitHub Releases 下载预编译二进制文件、通过 `cargo install --git` 直接安装，或从源码编译。
2. **作为 Rust 库包（`imgopt` 及工作区子 crate）** —— 在其他 Rust 项目的 `Cargo.toml` 中通过 GitHub 直接引入调用。

---

## 核心特性

- **五格式原生压缩、转换与动图处理**：
  - **JPEG**：最优霍夫曼表、渐进式扫描、自适应 4:4:4 / 4:2:0 色度抽样。
  - **PNG**：支持 **OxiPNG** 深度无损压缩以及基于 **ExoQuant** 的有损调色板量化（PNG8），大幅缩减真彩/半透明 PNG 体积。
  - **WebP**：支持有损与无损自适应编码、Alpha 透明通道及 RIFF 元数据复用。
  - **AVIF**：内置纯 Rust AVIF 编解码器（`ravif` + `avif-parse` + `rav1d`），支持有损/近无损及透明通道，无需外部 `nasm` / `libavif`。
  - **GIF 与动图**：支持静态/动态 GIF、动态 WebP 与 APNG 的帧序列解码、缩放、互转（如 GIF 转动态 WebP）与防膨胀透传。
- **高画质缩放与广色域色彩管理**：
  - 集成 SIMD 加速的 **Lanczos3** 缩放（`fast_image_resize`），支持 `scale-down`（等比缩小不放大）、`contain`、`cover`（居中裁剪）和 `exact` 四种模式。
  - 内置轻量色彩管理（`qcms`）：在默认剔除 ICC 元数据时自动将 Display P3 / AdobeRGB 等广色域图片转换为 **sRGB** 防止偏色；也支持 `--preserve-icc` 仅保留色彩配置并剥离 EXIF/XMP 隐私数据。
- **客观感知画质寻优（SSIM / PSNR）**：基于分块结构相似性指数（**SSIM**）与峰值信噪比（**PSNR**）对编码候选进行二分检索与排序；若重编码后体积未缩小，自动启用**防膨胀回退（No-Bloat Fallback）**保留最优小体积版本。
- **魔数（Magic Bytes）格式探测与过滤**：不依赖文件扩展名直接探测真实格式，支持 `--include` / `--exclude` glob 路径过滤，并可分析尺寸、色彩通道、透明通道、灰度特征、信息熵及元数据。
- **基于内容寻址的去重与断点续传**：采用流式 **SHA-256** 内容哈希结合标准化优化参数生成确定性 `OptimizationKey`，配合原子写入与系统级文件锁的 JSON 清单，同时支持 `sync` 云端同步和 `optimize --manifest` 本地增量缓存跳过。
- **安全且幂等的云存储同步与垃圾回收**：原生实现 AWS SigV4 签名与 TLS 客户端（支持 **Cloudflare R2**、**AWS S3**、**MinIO**）；上传默认附带不可变强缓存头 `Cache-Control: public, max-age=31536000, immutable` 并执行 `HEAD` 校验；支持 `verify` 并发核验以及 `gc` 清理云端孤儿对象。
- **高并发、弹性重试与内存保护**：内置有界并发工作池与共享解码内存预算（`MemoryBudget`），支持瞬态错误指数退避重试及防恶意大图/解压炸弹限制（默认最大输入 `100 MB`、最大像素 `64 MP`、最大宽高 `16384 px`）。
- **完整工具链**：提供 `optimize`、`sync`、`inspect`、`verify`、`benchmark`、`gc` 六大子命令。

---

## 工作区模块架构（Workspace Crates）

```text
crates/imgopt            统一 Rust 库门面（重导出 core、codecs、quality，按需启用 manifest/storage）
crates/imgopt-core       领域模型、魔数探测、目录扫描/glob过滤、SHA-256 哈希、图像分析、候选寻优、内存预算、同步与基准测试
crates/imgopt-codecs     JPEG、PNG（无损+有损量化）、WebP、AVIF、GIF/动图编解码器、Lanczos3 缩放与 ICC->sRGB 色彩转换
crates/imgopt-quality    客观画质评估指标（分块 SSIM、感知亮度 SSIM 与 PSNR）
crates/imgopt-manifest   原子持久化清单存储、OS 文件锁、内容去重与断点续传决策、运行结果 JSON 模型
crates/imgopt-storage    S3 兼容对象存储客户端（SigV4、Cache-Control、ListObjectsV2）、指数退避重试与 URL 构建
crates/imgopt-cli        命令行接口与二进制程序入口（`imgopt`）
```

---

## 相关文档

- 📖 **[中文用户使用手册](docs/USER_GUIDE.md)**：包含完整命令指南、参数参考、实战场景与常见问题解答。
- 🏗️ **[系统架构规范 (Architecture Specification)](docs/ARCHITECTURE.md)**：系统分层架构、数据流与模块边界设计。
- ⚙️ **[技术规范 (Technical Specification)](docs/SPEC.md)**：算法细节、哈希/键生成规范与质量评估标准。

---

## 方式一：安装与构建 CLI 工具（`imgopt`）

### 1. 下载预编译二进制文件（推荐）
直接前往 [GitHub Releases](https://github.com/xzsean666/Image-Optimizer-Cli/releases) 下载对应平台的可执行压缩包：

- **Linux**：`imgopt-linux-x86_64.tar.gz` / `imgopt-linux-aarch64.tar.gz`
- **macOS**：`imgopt-macos-aarch64.tar.gz`（Apple Silicon） / `imgopt-macos-x86_64.tar.gz`（Intel）
- **Windows**：`imgopt-windows-x86_64.zip`

每个发布压缩包均附带对应的 `.sha256` 校验文件。

### 2. 通过 Cargo 从 GitHub 直接安装
```bash
# 安装最新 main 分支版本
cargo install --git https://github.com/xzsean666/Image-Optimizer-Cli.git

# 或指定特定的发布标签（推荐）
cargo install --git https://github.com/xzsean666/Image-Optimizer-Cli.git --tag v0.1.0
```

### 3. 从源码构建
```bash
git clone https://github.com/xzsean666/Image-Optimizer-Cli.git
cd Image-Optimizer-Cli
cargo build --release
```
编译生成的可执行文件位于 `target/release/imgopt`。

---

## 方式二：作为 Rust 库包引入项目

其他 Rust 项目可以在 `Cargo.toml` 中直接从 GitHub 引入 `imgopt`（或按需单独引入 `imgopt-core` 等子 crate）。

### 1. 在 `Cargo.toml` 中添加依赖

**标准用法（本地压缩、编解码器、SSIM/PSNR 画质评估、有界并发批处理）：**
```toml
[dependencies]
imgopt = { git = "https://github.com/xzsean666/Image-Optimizer-Cli.git" }

# 推荐：锁定具体 tag 或 commit
# imgopt = { git = "https://github.com/xzsean666/Image-Optimizer-Cli.git", tag = "v0.1.0" }
```

**启用 Manifest 去重与 S3 兼容云存储同步：**
```toml
[dependencies]
imgopt = { git = "https://github.com/xzsean666/Image-Optimizer-Cli.git", features = ["full"] }
```

`imgopt` 支持的 Feature Flags：
- `default = []` —— 包含核心优化管道（`imgopt::core`）、编解码器（`imgopt::codecs`）和画质指标（`imgopt::quality`）。
- `manifest` —— 启用 `imgopt::manifest`（`imgopt-manifest`）。
- `storage` —— 启用 `imgopt::storage`（`imgopt-storage`）。
- `full` —— 同时启用 `manifest` 与 `storage`。

### 2. Rust API 调用示例

#### 优化单张图片文件或内存字节
```rust
use imgopt::{
    ImageFormat, MetadataPolicy, OptimizationConfig, OptimizationPreset,
    OptimizationService, OutputFormatPolicy,
};

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let service = OptimizationService::new(imgopt::VERSION);

    // 使用预设配置，或按需自定义画质目标、缩放尺寸与输出格式策略
    let mut config = OptimizationConfig::from_preset(OptimizationPreset::Balanced);
    config.output_format_policy = OutputFormatPolicy::Force(ImageFormat::Webp);
    config.metadata_policy = MetadataPolicy::StripAll;

    // 1) 从文件路径优化
    let file_result = service.optimize_file("photo.png", &config)?;
    std::fs::write("photo.webp", &file_result.validated_output.bytes)?;

    // 2) 或直接对内存中的字节流优化
    let raw_bytes = std::fs::read("photo.png")?;
    let mem_result = service.optimize_bytes(&raw_bytes, &config)?;
    println!(
        "Optimized: {:?}, {} bytes, SSIM={:.4}",
        mem_result.effective_format,
        mem_result.validated_output.byte_size,
        mem_result.quality_score
    );

    Ok(())
}
```

#### 图像分析与目录有界并发批量优化
```rust
use imgopt::{
    analyze_image, execute_batch_scoped, scan_path, BatchOptions, BatchTask,
    OptimizationConfig, OptimizationPreset, OptimizationService, ScanOptions,
};

fn batch_example() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let service = OptimizationService::new(imgopt::VERSION);
    let config = OptimizationConfig::from_preset(OptimizationPreset::Maximum);

    // 在优化前分析原始图像字节
    let bytes = std::fs::read("photo.jpg")?;
    let analysis = analyze_image(&bytes, None)?;
    println!(
        "Dimensions: {}x{}",
        analysis.profile.dimensions.width, analysis.profile.dimensions.height
    );

    // 递归扫描目录并执行有界并发批量优化
    let scan = scan_path("./images", &ScanOptions::default())?;
    let tasks: Vec<BatchTask<()>> = scan
        .files
        .into_iter()
        .map(|f| BatchTask::new(f.path, ()))
        .collect();

    let (_results, summary) = execute_batch_scoped(
        tasks,
        &BatchOptions::default(),
        |path, ()| service.optimize_file(&path, &config),
    );
    println!("Succeeded: {}, Failed: {}", summary.succeeded, summary.failed);
    Ok(())
}
```

---

## CLI 命令行使用指南

### 1. 本地优化（`optimize`）
支持单文件/目录批量压缩、等比缩放、格式自动择优（`auto` / `webp` / `avif`）、glob 过滤以及本地 Manifest 增量缓存：
```bash
# 使用默认 balanced 预设压缩并自动在原格式、WebP、AVIF 中选择最小输出
imgopt optimize ./photos --output ./dist --format auto

# 等比缩小大图（宽度上限 1920px，不放大），保留 ICC 色彩配置并输出 JSON 报告
imgopt optimize ./photos --output ./dist --resize-width 1920 --preserve-icc --json opt.json

# 启用本地 Manifest 增量缓存并按 glob 过滤目录
imgopt optimize ./photos --output ./dist --manifest .local-manifest.json --exclude "raw/**"
```

### 2. 云存储增量同步（`sync`）
递归扫描目录、自动基于 SHA-256 去重、压缩优化、并发上传至 S3/R2（默认附带不可变 `Cache-Control` 并执行上传后校验），原子更新 Manifest：
```bash
# Dry-run 演练模式（无需真实 S3 凭证）
imgopt sync ./photos --dry-run --manifest manifest.json --json result.json

# 生产环境同步至 Cloudflare R2 / AWS S3（读取 .env 或 --env-file）
imgopt --env-file .env.prod sync ./photos --format auto --resize-width 2048 --fail-on-error --json sync-report.json
```

### 3. 图像探查（`inspect`）
只读分析格式（含动图与帧数）、分辨率、色彩通道、透明通道、灰度特征、信息熵及 EXIF/ICC/XMP 元数据：
```bash
imgopt inspect photo.jpg
imgopt inspect ./photos --include "**/*.png" --json
```

### 4. 清单与远程存储校验（`verify`）
校验本地 Manifest 完整性，并支持多线程并发向远程 S3 发起 `HEAD` 请求核对对象存在性、大小与 ETag：
```bash
imgopt verify --manifest manifest.json
imgopt verify --manifest manifest.json --check-storage --concurrency 16
```

### 5. 云端孤儿对象垃圾回收（`gc`）
扫描远程 S3 前缀下未在当前 Manifest 引用的冗余对象（默认为安全演练模式，传入 `--delete` 才真正删除）：
```bash
# 先预览将被清理的孤儿对象
imgopt gc --manifest manifest.json --prefix assets

# 确认无误后执行删除并输出 JSON 报告
imgopt gc --manifest manifest.json --prefix assets --delete --json gc-report.json
```

### 6. 基准性能测试（`benchmark`）
```bash
imgopt benchmark ./dataset --preset maximum --format auto --json benchmark_report.json
```

---

## 预设与配置优先级

### 内置优化预设（`--preset`）
| 预设名称 | 默认目标 SSIM | 默认编码强度 (`--effort`) | 适用场景 |
|---|---|---|---|
| `maximum` | `0.97` | `6` | 摄影作品、高清展示图等追求极高画质保真的场景 |
| `balanced`（默认） | `0.92` | `4` | Web 静态资源、博客、电商图，体积与画质的最佳平衡 |
| `aggressive` | `0.85` | `3` | 缩略图、移动端低带宽等极致瘦身场景 |

### 参数与环境变量参考
支持通过命令行参数、环境变量或 `.env`（及全局参数 `--env-file <PATH>`）配置，优先级为：**CLI 参数 > 进程环境变量 > `.env` 文件 > 默认值**。

| CLI 参数 | 环境变量 | 默认值 | 说明 |
|---|---|---|---|
| `--bucket` | `IMGOPT_S3_BUCKET` / `AWS_S3_BUCKET` | — | S3 存储桶名称 |
| `--endpoint` | `IMGOPT_S3_ENDPOINT` / `AWS_ENDPOINT_URL` | AWS S3 标准端点 | 自定义 S3 / R2 / MinIO 端点 URL |
| — | `IMGOPT_S3_REGION` / `AWS_REGION` | `us-east-1` | S3 区域（R2 通常填 `auto`） |
| — | `IMGOPT_S3_ACCESS_KEY_ID` / `AWS_ACCESS_KEY_ID` | — | S3 访问密钥 ID |
| — | `IMGOPT_S3_SECRET_ACCESS_KEY` / `AWS_SECRET_ACCESS_KEY` | — | S3 访问密钥 Secret |
| `--force-path-style` | `IMGOPT_S3_FORCE_PATH_STYLE` | `true` | 是否使用 Path-Style URL |
| `--prefix` | `IMGOPT_S3_KEY_PREFIX` / `IMGOPT_S3_PREFIX` | — | 远程对象键前缀 |
| `--public-url-base` | `IMGOPT_PUBLIC_URL_BASE` / `IMGOPT_S3_PUBLIC_URL_BASE` | — | 公开 CDN 基础域名 |
| `--cache-control` | `IMGOPT_S3_CACHE_CONTROL` | `public, max-age=31536000, immutable` | 上传对象的 `Cache-Control` 响应头 |
| `--preset` | — | `balanced` | 优化预设：`balanced`、`maximum`、`aggressive` |
| `--quality` / `-q` | — | 随预设 | 目标 SSIM 画质分数（`0.0` ~ `1.0`） |
| `--effort` / `-e` | — | 随预设 | 编码耗力级别（`1` ~ `6`） |
| `--format` | — | `preserve`（`sync` 为 `webp`） | `preserve`、`webp`、`avif`、`auto`、`jpeg`、`png`、`gif`、`force-webp`、`force-avif` |
| `--resize-width` / `--resize-height` | — | — | 目标缩放宽高（像素） |
| `--resize-mode` | — | `scale-down` | 缩放模式：`scale-down`、`contain`、`cover`、`exact` |
| `--preserve-metadata` | — | `false` | 保留全部 EXIF / ICC / XMP 元数据 |
| `--preserve-icc` | — | `false` | 仅保留 ICC 色彩配置，剥离 EXIF / XMP |
| `--no-srgb-convert` | — | `false` | 禁用剥离 ICC 时的自动 sRGB 转换 |
| `--include` / `--exclude` | — | — | Glob 文件路径过滤（可多次指定） |
| `--concurrency` | — | CPU 核数上限 16（`sync` 为 `4`） | 并发工作线程数 |
| `--max-width` / `--max-height` | — | `16384` | 允许的最大输入宽高（像素） |

---

## 自动化发布（GitHub Actions）

仓库内置 [`.github/workflows/release.yml`](.github/workflows/release.yml)，支持通过手动触发（`workflow_dispatch`）自动交叉编译各平台 CLI 二进制文件、生成 `.sha256` 校验文件并发布到 GitHub Releases。

### 支持的发布目标平台
- `x86_64-unknown-linux-gnu`（`imgopt-linux-x86_64.tar.gz`）
- `aarch64-unknown-linux-gnu`（`imgopt-linux-aarch64.tar.gz`，通过 `cross` 交叉编译）
- `x86_64-apple-darwin`（`imgopt-macos-x86_64.tar.gz`）
- `aarch64-apple-darwin`（`imgopt-macos-aarch64.tar.gz`）
- `x86_64-pc-windows-msvc`（`imgopt-windows-x86_64.zip`）

### 触发发布流程
1. **通过 GitHub 网页端**：
   进入 **Actions -> Release -> Run workflow**，填写发布标签（例如 `v0.1.1`），点击 **Run workflow**。
2. **通过 GitHub CLI（`gh`）**：
   ```bash
   gh workflow run release.yml -f tag=v0.1.1
   ```

---

## 架构与工程文档

- [系统架构规范 (Architecture Specification)](docs/ARCHITECTURE.md)
- [技术规范 (Technical Specification)](docs/SPEC.md)
- [架构决策记录 (ADRs)](docs/DECISIONS.md)
- [构建与验证指南 (Build and Verification Guide)](docs/BUILD.md)
- [任务清单 (Task Backlog)](docs/tasks/TASKS.md)

---

## 开源协议

本项目采用双协议授权，你可以任选 **Apache License 2.0** 或 **MIT License**。
