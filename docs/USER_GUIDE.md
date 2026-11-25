# Image Optimizer CLI (`imgopt`) 用户使用手册

`imgopt` 是一个基于 Rust 构建的高性能、内容寻址与确定性图片压缩优化 CLI 工具。专为**“最少的图片质量损失、最大的图片压缩率”**而设计，支持本地批量压缩以及 S3 兼容对象存储（Cloudflare R2 / AWS S3 / MinIO）的增量同步。

---

## 目录
1. [快速下载与安装](#1-快速下载与安装)
2. [核心功能与设计亮点](#2-核心功能与设计亮点)
3. [快速上手（常用场景）](#3-快速上手常用场景)
4. [子命令详细参考](#4-子命令详细参考)
   - [4.1 `imgopt optimize`（本地压缩）](#41-imgopt-optimize本地压缩)
   - [4.2 `imgopt sync`（云存储增量同步）](#42-imgopt-sync云存储增量同步)
   - [4.3 `imgopt inspect`（图片元数据与特征探查）](#43-imgopt-inspect图片元数据与特征探查)
   - [4.4 `imgopt verify`（清单与存储校验）](#44-imgopt-verify清单与存储校验)
   - [4.5 `imgopt benchmark`（基准性能测试）](#45-imgopt-benchmark基准性能测试)
5. [预设模式与画质控制（Presets）](#5-预设模式与画质控制presets)
6. [云存储配置与环境变量](#6-云存储配置与环境变量)
7. [常见问题与最佳实践 (FAQ)](#7-常见问题与最佳实践-faq)

---

## 1. 快速下载与安装

### 方式 A：直接下载预编译二进制（推荐，开箱即用）
无需安装 Rust 或配置开发环境，直接前往 GitHub Releases 页面下载适合您操作系统的压缩包：

| 操作系统 | 架构 | 下载文件名 |
| :--- | :--- | :--- |
| **Linux** | x86_64 (64位) | `imgopt-linux-x86_64.tar.gz` |
| **Linux** | ARM64 / aarch64 | `imgopt-linux-aarch64.tar.gz` |
| **macOS** | Apple Silicon (M1/M2/M3/M4) | `imgopt-macos-aarch64.tar.gz` |
| **macOS** | Intel x86_64 | `imgopt-macos-x86_64.tar.gz` |
| **Windows** | x86_64 (64位) | `imgopt-windows-x86_64.zip` |

**Linux / macOS 解压与安装示例**：
```bash
# 1. 解压下载的压缩包
tar -xzvf imgopt-linux-x86_64.tar.gz

# 2. 移动到系统 PATH 路径（可选）
sudo mv imgopt /usr/local/bin/

# 3. 验证安装
imgopt --version
```

### 方式 B：从源码编译
```bash
# 克隆仓库
git clone https://github.com/xzsean666/Image-Optimizer-Cli.git
cd Image-Optimizer-Cli

# 编译 Release 版本
cargo build --release

# 二进制文件位于 target/release/imgopt
./target/release/imgopt --help
```

---

## 2. 核心功能与设计亮点

* **三格式深度优化**：
  - **JPEG**：采用最优 Huffman 统计表、渐进式扫描（Progressive Scans）与自适应 4:2:0 色度抽样，减小 20%~35% 体积。
  - **PNG**：集成 **OxiPNG** 引擎，支持多 Filter 策略穷举与 Deflate 深度压缩，完全无损削减 40%+。
  - **WebP**：支持 Lossy 与 Lossless 智能自适应决策，高压缩率同时保留极高感官细节。
* **高精度二分率失真寻优**：基于单调 SSIM 感知质量模型，以 1 的质量精度（1..100）精准锁定满足画质要求的最低文件体积。
* **SHA-256 内容寻址去重**：基于文件内容流式哈希生成唯一指纹，文件名与路径变动不会重复压缩或重复上传。
* **S3/R2 增量同步与断点续传**：带指数退避重试，上传前后校验哈希，原子写入 Manifest 清单，故障自动隔离。

---

## 3. 快速上手（常用场景）

### 场景 1：压缩单张图片并转换为 WebP
```bash
imgopt optimize photo.jpg -o photo.webp --format webp
```

### 场景 2：批量压缩整个图片目录（保持原格式无损/高保真）
```bash
imgopt optimize ./images -o ./dist --format preserve --preset balanced
```

### 场景 3：极限体积压缩（适用于网站静态资源与缩略图）
```bash
imgopt optimize ./images -o ./dist --format webp --preset aggressive
```

### 场景 4：同步本地图片库到 Cloudflare R2 / AWS S3 并生成 CDN 链接
```bash
# 1. 配置环境变量（或写入 .env 文件）
export IMGOPT_S3_BUCKET="my-assets-bucket"
export IMGOPT_S3_ENDPOINT="https://<account_id>.r2.cloudflarestorage.com"
export IMGOPT_S3_ACCESS_KEY_ID="<your_key_id>"
export IMGOPT_S3_SECRET_ACCESS_KEY="<your_secret_key>"
export IMGOPT_S3_PUBLIC_URL_BASE="https://cdn.example.com"

# 2. 执行同步
imgopt sync ./images --manifest ./images/.manifest.json --preset balanced
```

---

## 4. 子命令详细参考

### 4.1 `imgopt optimize`（本地压缩）
对单张图片或指定目录进行就地或目标目录压缩，不涉及远程存储。

```bash
imgopt optimize <INPUT> [OPTIONS]
```

#### 参数选项：
| 参数 | 缩写 | 默认值 | 描述 |
| :--- | :--- | :--- | :--- |
| `<INPUT>` | — | *(必填)* | 输入文件路径或图片目录路径 |
| `--output <PATH>` | `-o` | 无 | 输出文件路径或目标输出目录 |
| `--preset <PRESET>` | — | `balanced` | 优化预设：`balanced`, `maximum`, `aggressive` |
| `--quality <F64>` | `-q` | 预设默认 | 显式指定目标 SSIM 画质分数（`0.0` 至 `1.0`，例如 `0.95`） |
| `--effort <U8>` | `-e` | 预设默认 | 编码耗力级别（`1` 至 `6`，数值越大压缩率越高、耗时越长） |
| `--format <POLICY>` | — | `preserve` | 格式策略：`preserve`（保持原格式）、`webp`（优先转 WebP）、`jpeg`、`png` |
| `--preserve-metadata`| — | `false` | 是否保留 EXIF / ICC / XMP 元数据（默认剔除以最大化瘦身） |
| `--concurrency <N>` | — | CPU 核数 | 最大并发处理线程数 |

---

### 4.2 `imgopt sync`（云存储增量同步）
递归扫描图片目录，自动去重、压缩，并发上传至 S3 兼容对象存储，并原子提交 Manifest 清单。

```bash
imgopt sync <INPUT> [OPTIONS]
```

#### 参数选项：
| 参数 | 默认值 | 描述 |
| :--- | :--- | :--- |
| `<INPUT>` | *(必填)* | 待同步的输入目录或文件 |
| `--manifest <PATH>` | `<INPUT>/.imgopt-manifest.json` | 持久化 JSON 清单路径 |
| `--json <PATH>` | 无 | 输出机器可读的运行结果摘要 JSON 文件 |
| `--dry-run` | `false` | 演练模式：仅在内存中模拟，不上传到真实 S3 |
| `--bucket <NAME>` | 环境变量 | 覆盖 S3 存储桶名称 |
| `--endpoint <URL>` | 环境变量 | 覆盖 S3 / R2 API 端点地址 |
| `--prefix <PREFIX>` | 无 | 远程对象存储 Key 前缀（如 `assets/`） |
| `--public-url-base <URL>`| 环境变量 | 自定义 CDN 域名（如 `https://cdn.mysite.com`） |
| `--concurrency <N>` | `4` | 最大并发处理与上传线程数 |
| `--preset <PRESET>` | `balanced` | 优化预设（`balanced` / `maximum` / `aggressive`） |
| `--format <POLICY>` | `webp` | 输出格式策略（默认优先转为现代 `webp`） |

---

### 4.3 `imgopt inspect`（图片元数据与特征探查）
只读探查图片的真实格式、尺寸、通道、信息熵（Entropy）及元数据，不修改任何文件。

```bash
# 人类可读文本格式输出
imgopt inspect sample.jpg

# JSON 格式输出（适合脚本处理）
imgopt inspect sample.png --json
```

**输出示例**：
```text
Image: sample.jpg
  Format:       jpeg
  Dimensions:   1920x1080 (2073600 pixels)
  Color:        Rgb8 (3 channels)
  Size:         452120 bytes
  Transparency: false
  Grayscale:    false
  Entropy:      7.89 bits/byte
  Metadata:     EXIF: true, ICC: false, XMP: false
```

---

### 4.4 `imgopt verify`（清单与存储校验）
校验本地 Manifest 清单的完整性、哈希确定性以及远程对象存储的一致性。

```bash
# 仅校验本地 Manifest 语法与记录条目
imgopt verify --manifest manifest.json

# 深度校验：同时向远程 S3 发送 HEAD 请求验证对象是否存在且 ETag 匹配
imgopt verify --manifest manifest.json --check-storage
```

---

### 4.5 `imgopt benchmark`（基准性能测试）
在指定图片数据集上运行基准测试，统计吞吐量、平均压缩率、SSIM 画质与耗时分布。

```bash
imgopt benchmark ./test_dataset --preset balanced --json benchmark_report.json
```

---

## 5. 预设模式与画质控制（Presets）

`imgopt` 内置 3 档经过工业级调优的预设：

| 预设模式 | 目标 SSIM | Effort 级别 | 特点与适用场景 |
| :--- | :--- | :--- | :--- |
| **`maximum`**<br>(极致画质) | **$\ge 0.97$** | `6` (最高) | 适合摄影作品集、高保真大图、印刷前预览，体积缩减约 **30% ~ 50%**，人眼无法分辨差异。 |
| **`balanced`**<br>(推荐默认) | **$\ge 0.92$** | `4` (均衡) | 适合 Web 内容分发、博客图片、电商产品图，体积缩减 **50% ~ 70%**，速度与画质最佳平衡点。 |
| **`aggressive`**<br>(激进瘦身) | **$\ge 0.85$** | `3` (快速) | 适合移动端缩略图、长列表展示、低带宽场景，体积缩减 **75% ~ 85%+**。 |

> **自定义参数覆盖**：您随时可以使用 `-q / --quality 0.94` 或 `-e / --effort 5` 单独覆盖预设中的具体数值。

---

## 6. 云存储配置与环境变量

为了避免在命令行中重复传递凭证，`imgopt` 支持自动读取当前目录下的 `.env` 文件或系统环境变量：

### `.env` 示例文件（推荐）
在运行目录下创建 `.env` 文件：
```env
# S3 兼容存储配置 (以 Cloudflare R2 为例)
IMGOPT_S3_BUCKET="my-bucket-name"
IMGOPT_S3_ENDPOINT="https://<ACCOUNT_ID>.r2.cloudflarestorage.com"
IMGOPT_S3_ACCESS_KEY_ID="a1b2c3d4e5f6g7h8..."
IMGOPT_S3_SECRET_ACCESS_KEY="1234567890abcdef..."

# 自定义 CDN 域名绑定
IMGOPT_S3_PUBLIC_URL_BASE="https://cdn.example.com"
```

### 指定自定义环境文件：
```bash
imgopt --env-file /path/to/prod.env sync ./images
```

---

## 7. 常见问题与最佳实践 (FAQ)

### Q1：为什么有些 PNG 压缩后并没有变小？
* `imgopt` 采用严格的安全保真策略。如果输入图片本身已经被高度压缩，或者转换策略无法进一步缩减体积，`imgopt` 会自动保留原始最小版本，绝不产生反向膨胀。

### Q2：如何实现透明背景图片的安全压缩？
* 如果包含透明通道（Alpha Channel），请使用 `--format webp` 或 `--format preserve`。WebP 支持完美的透明通道有损/无损压缩；若强制转为 JPEG，透明区域将按工业标准自动平铺为白色背景。

### Q3：`sync` 中断后重新运行会重复上传吗？
* 不会。`imgopt` 具有**幂等性**。每次同步会先读取 Manifest 清单，只有新增文件、内容发生变化或压缩参数调整过的图片才会重新处理，已同步文件直接秒级跳过。
