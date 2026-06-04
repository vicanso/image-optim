// Copyright 2025 Tree xie.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::config::{must_get_basic_config, must_get_config};
use crate::dal::{get_opendal_storage, get_opendal_storage_by_name};
use crate::guard;
use crate::metrics;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use cached::macros::cached;
use imageoptimize::{
    ProcessImage, new_blur_task, new_brighten_task, new_contrast_task, new_crop_task,
    new_diff_task, new_fit_task, new_flip_task, new_gray_task, new_optim_task, new_padding_task,
    new_resize_task, new_rotate_task, new_sharpen_task, new_strip_task, new_watermark_task,
    run_with_image,
};
use once_cell::sync::OnceCell;
use serde::Deserialize;
use std::time::Duration;
use tibba_config::humantime_serde;
use tibba_error::Error;

type Result<T, E = Error> = std::result::Result<T, E>;

pub const AUTO_OUTPUT_TYPE: &str = "auto";

fn default_quality() -> u8 {
    80
}

fn default_speed() -> u8 {
    3
}

fn default_max_age() -> Duration {
    Duration::from_secs(2592000)
}

#[derive(Deserialize)]
pub struct OptimConfig {
    #[serde(default = "default_quality")]
    pub quality: u8,
    pub quality_jpeg: Option<u8>,
    pub quality_png: Option<u8>,
    pub quality_webp: Option<u8>,
    pub quality_avif: Option<u8>,
    pub quality_jxl: Option<u8>,
    #[serde(default = "default_speed")]
    pub speed: u8,
    #[serde(default = "default_max_age", with = "humantime_serde")]
    pub max_age: Duration,
    pub auto_output_types: Vec<String>,
}

impl OptimConfig {
    pub fn quality_for(&self, format: &str) -> u8 {
        match format {
            "jpeg" | "jpg" => self.quality_jpeg.unwrap_or(self.quality),
            "png" => self.quality_png.unwrap_or(self.quality),
            "webp" => self.quality_webp.unwrap_or(self.quality),
            "avif" => self.quality_avif.unwrap_or(self.quality),
            "jxl" => self.quality_jxl.unwrap_or(self.quality),
            _ => self.quality,
        }
    }
}

static OPTIM_CONFIG: OnceCell<OptimConfig> = OnceCell::new();

pub fn get_default_optim_params() -> &'static OptimConfig {
    OPTIM_CONFIG.get_or_init(|| {
        let app_config = must_get_config();
        app_config
            .sub_config("optim")
            .try_deserialize::<OptimConfig>()
            .unwrap_or(OptimConfig {
                quality: 80,
                quality_jpeg: None,
                quality_png: None,
                quality_webp: None,
                quality_avif: None,
                quality_jxl: None,
                speed: 3,
                max_age: default_max_age(),
                auto_output_types: vec![],
            })
    })
}

fn map_err(err: impl ToString) -> Error {
    Error::new(err).with_category("imageoptimize")
}

/// Drive `run_with_image` on a dedicated blocking thread so the encode/decode
/// pipeline (libjxl, rav1e/AVIF, mozjpeg, image-rs resize/sharpen — all sync
/// CPU-bound) can't starve the tokio I/O workers. imageoptimize 0.5.3 only
/// inserts `block_in_place` when its `bin` feature is enabled, and that
/// feature drags in clap/glob/num_cpus we don't want, so we wrap it here.
async fn run_image_blocking(image: ProcessImage, tasks: Vec<Vec<String>>) -> Result<ProcessImage> {
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || handle.block_on(run_with_image(image, tasks)))
        .await
        .map_err(|e| Error::new(e).with_category("blocking_join"))?
        .map_err(map_err)
}

/// 根据 source 返回对应的 OpenDAL 存储；未指定时返回默认存储；指定但未找到时返回 400 错误。
fn resolve_storage(source: Option<&str>) -> Result<&'static tibba_opendal::Storage> {
    match source {
        None => Ok(get_opendal_storage()),
        Some(name) => get_opendal_storage_by_name(name).ok_or_else(|| {
            Error::new(format!("opendal source not found: {name}"))
                .with_category("imageoptimize")
                .with_status(400)
        }),
    }
}

fn reject_too_large<S: Into<String>>(reason: &'static str, msg: S) -> Error {
    metrics::inc_decode_rejected(reason);
    Error::new(msg.into())
        .with_category("decode_guard")
        .with_status(413)
}

fn reject_path(reason: &'static str, path: &str) -> Error {
    metrics::inc_path_rejected(reason);
    Error::new(format!("invalid file path ({reason}): {path}"))
        .with_category("path_guard")
        .with_status(400)
}

/// Reject paths that could escape the storage root or smuggle separators past
/// downstream code. Runs ahead of any OpenDAL call — the local `file://` backend
/// joins requested paths to its root without canonicalising, so `../../etc/passwd`
/// would otherwise escape. Cheap defence-in-depth for HTTP/S3 backends too.
fn sanitize_storage_path(path: &str) -> Result<()> {
    if path.is_empty() {
        return Err(reject_path("empty", path));
    }
    if path.contains('\0') {
        return Err(reject_path("null_byte", path));
    }
    if path.contains('\\') {
        return Err(reject_path("backslash", path));
    }
    if path.starts_with('/') {
        return Err(reject_path("absolute", path));
    }
    if path.split('/').any(|seg| seg == "..") {
        return Err(reject_path("parent_segment", path));
    }
    Ok(())
}

async fn load_image(file: &str, source: Option<&str>) -> Result<ProcessImage> {
    sanitize_storage_path(file)?;
    guard::enforce_prefix(source, file)?;
    let ext = file.split('.').next_back().unwrap_or("jpeg");
    let basic = must_get_basic_config();
    let storage = resolve_storage(source)?;

    // Pre-check the object size via stat() so we can 413 oversized sources
    // without paying the download. S3 and HTTP backends serve this with a
    // HEAD; the local filesystem backend turns it into a stat(2) — both are
    // negligible compared with reading a multi-MiB body. Backends that don't
    // implement stat (or services that omit content-length) report 0 here,
    // and we fall through to the post-read guard below.
    if basic.max_source_bytes > 0
        && let Ok(meta) = storage.stat(file).await
    {
        let declared = meta.content_length();
        if declared > 0 && declared > basic.max_source_bytes {
            return Err(reject_too_large(
                "bytes",
                format!(
                    "source too large: {declared} bytes (stat) > limit {} bytes",
                    basic.max_source_bytes
                ),
            ));
        }
    }

    let buffer = storage.read(file).await?;
    let bytes_len = buffer.len() as u64;
    metrics::record_input_bytes(bytes_len);

    if basic.max_source_bytes > 0 && bytes_len > basic.max_source_bytes {
        return Err(reject_too_large(
            "bytes",
            format!(
                "source too large: {bytes_len} bytes > limit {} bytes",
                basic.max_source_bytes
            ),
        ));
    }

    let img = ProcessImage::new(buffer.to_vec(), ext).map_err(map_err)?;

    if basic.max_source_pixels > 0 {
        let (w, h) = img.get_size();
        let pixels = (w as u64).saturating_mul(h as u64);
        if pixels > basic.max_source_pixels {
            return Err(reject_too_large(
                "pixels",
                format!(
                    "source too large: {pixels} pixels ({w}x{h}) > limit {}",
                    basic.max_source_pixels
                ),
            ));
        }
    }

    Ok(img)
}

/// 文件路径或内容型任务参数，用作缓存 key（须实现 Hash + Eq）。
/// f32 字段以字符串形式存储（如 "1.5"），避免 f32 不实现 Hash 的问题。
#[derive(Default, Clone, PartialEq, Eq, Hash, Debug)]
pub struct ImageTaskParams {
    pub file: String,
    /// OpenDAL 存储源名（来自 `IMOP__OPENDAL__<NAME>__URL`）；未设置时使用默认存储。
    pub source: Option<String>,
    pub output_type: Option<String>,
    pub quality: Option<u8>,
    // resize: width/height 同时为 0 时不触发
    pub width: Option<u32>,
    pub height: Option<u32>,
    // fit: 缩放至不超过指定尺寸，保持比例，不放大
    pub fit_width: Option<u32>,
    pub fit_height: Option<u32>,
    // watermark: 水印文件路径
    pub watermark: Option<String>,
    pub position: Option<String>,
    pub margin_left: Option<i32>,
    pub margin_top: Option<i32>,
    // crop
    pub x: Option<u32>,
    pub y: Option<u32>,
    pub auto_output_type: Option<String>,
    // 图像调整参数
    pub rotate: Option<u16>,
    pub flip: Option<String>,
    pub gray: bool,
    pub sharpen: Option<String>, // "sigma" 或 "sigma,threshold"
    pub blur: Option<String>,    // sigma 字符串
    pub brighten: Option<i32>,
    pub contrast: Option<String>, // f32 字符串
    pub strip: bool,
    pub padding_width: Option<u32>,
    pub padding_height: Option<u32>,
    pub padding_color: Option<String>,
    /// When true, skip the DSSIM diff task on the pure-optim path. DSSIM
    /// requires re-encoding then re-decoding the output to compare against
    /// the original — for AVIF/JXL that's effectively doubling encode cost.
    /// Default false (=compute) preserves prior behaviour; client opts out
    /// via `?diff=false`.
    pub skip_diff: bool,
}

#[derive(Default, Clone, Debug)]
pub struct ImageTaskResult {
    /// Encoded image bytes. Backed by `bytes::Bytes` so `cached` LRU hits
    /// clone the buffer with an Arc bump instead of a full memcpy — at
    /// ~1k cached requests/sec on a 200 KiB output that saves ~200 MiB/s
    /// of pointless memory traffic. Body::from(Bytes) is also zero-copy
    /// all the way to hyper.
    pub buffer: Bytes,
    pub original_size: usize,
    pub ext: String,
    pub diff: f64,
}

/// POST /images/process 流水线操作，按数组顺序依次执行。
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Op {
    Fit {
        width: u32,
        height: u32,
    },
    Resize {
        width: u32,
        height: u32,
    },
    Crop {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    Rotate {
        deg: u16,
    },
    Flip {
        dir: String,
    },
    Gray,
    Sharpen {
        sigma: f32,
        #[serde(default)]
        threshold: i32,
    },
    Blur {
        sigma: f32,
    },
    Brighten {
        value: i32,
    },
    Contrast {
        value: f32,
    },
    Strip,
    Padding {
        width: u32,
        height: u32,
        #[serde(default)]
        color: String,
    },
    Watermark {
        data: String,
        #[serde(default)]
        position: String,
        #[serde(default)]
        margin_left: i32,
        #[serde(default)]
        margin_top: i32,
    },
    Optim {
        output_type: Option<String>,
        quality: Option<u8>,
    },
}

/// Cache watermark bytes (base64-encoded) keyed by `(source, path)`.
/// Watermark paths are nearly always a small fixed set (logos / brand marks),
/// so a 200-entry LRU with 30-min TTL gets very high hit rate and skips both
/// the storage read AND the per-request base64 encode. Caller is responsible
/// for running `sanitize_storage_path` + `guard::enforce_prefix` first so we
/// never cache a value for an attacker-controlled path.
#[cached(
    size = 200,
    ttl = 1800,
    result = true,
    sync_writes = "by_key",
    key = "(Option<String>, String)",
    convert = r#"{ (source.map(str::to_owned), path.to_owned()) }"#
)]
async fn load_watermark_b64(source: Option<&str>, path: &str) -> Result<String> {
    let bytes = resolve_storage(source)?.read(path).await?;
    Ok(STANDARD.encode(bytes.to_vec()))
}

#[cached(size = 1000, ttl = 1800, result = true, sync_writes = "by_key")]
pub async fn run_image_task(params: ImageTaskParams) -> Result<(ImageTaskResult, bool)> {
    let started = std::time::Instant::now();
    match run_image_task_inner(params).await {
        Ok((result, private)) => {
            let fmt = result.ext.as_str();
            metrics::record_task_duration(fmt, started.elapsed().as_secs_f64());
            metrics::record_output_bytes(fmt, result.buffer.len() as u64);
            metrics::record_dssim(fmt, result.diff);
            Ok((result, private))
        }
        Err(e) => {
            metrics::inc_errors(&e.category);
            Err(e)
        }
    }
}

async fn run_image_task_inner(params: ImageTaskParams) -> Result<(ImageTaskResult, bool)> {
    let optim_config = get_default_optim_params();
    let mut output_type = params.output_type;
    let mut cache_private = false;
    if let Some(auto_output_type) = params.auto_output_type {
        output_type = Some(auto_output_type);
        cache_private = true;
    }
    let source = params.source.as_deref();
    let mut img = load_image(&params.file, source).await?;
    let output_type = output_type.unwrap_or(img.ext.clone());
    let quality = params
        .quality
        .unwrap_or_else(|| optim_config.quality_for(&output_type));

    let mut tasks: Vec<Vec<String>> = Vec::with_capacity(16);
    let mut should_add_diff_task = true;

    if let Some(watermark_path) = params.watermark {
        sanitize_storage_path(&watermark_path)?;
        guard::enforce_prefix(source, &watermark_path)?;
        let watermark_b64 = load_watermark_b64(source, &watermark_path).await?;
        tasks.push(new_watermark_task(
            &watermark_b64,
            &params.position.unwrap_or_default(),
            params.margin_left.unwrap_or_default(),
            params.margin_top.unwrap_or_default(),
        ));
        should_add_diff_task = false;
    }

    if let Some(x) = params.x
        && let Some(y) = params.y
    {
        tasks.push(new_crop_task(
            x,
            y,
            params.width.unwrap_or_default(),
            params.height.unwrap_or_default(),
        ));
        should_add_diff_task = false;
    }

    if params.width.is_some() || params.height.is_some() {
        let width = params.width.unwrap_or_default();
        let height = params.height.unwrap_or_default();
        let (w, h) = img.get_size();
        let width = if width == 0 { w * height / h } else { width };
        let height = if height == 0 { h * width / w } else { height };
        tasks.push(new_resize_task(width, height));
        should_add_diff_task = false;
    }

    if params.fit_width.is_some() || params.fit_height.is_some() {
        tasks.push(new_fit_task(
            params.fit_width.unwrap_or_default(),
            params.fit_height.unwrap_or_default(),
        ));
        should_add_diff_task = false;
    }

    if let Some(deg) = params.rotate {
        tasks.push(new_rotate_task(deg));
        should_add_diff_task = false;
    }

    if let Some(dir) = &params.flip {
        tasks.push(new_flip_task(dir));
        should_add_diff_task = false;
    }

    if params.gray {
        tasks.push(new_gray_task());
        should_add_diff_task = false;
    }

    if let Some(val) = params.brighten {
        tasks.push(new_brighten_task(val));
        should_add_diff_task = false;
    }

    if let Some(val) = &params.contrast
        && let Ok(v) = val.parse::<f32>()
    {
        tasks.push(new_contrast_task(v));
        should_add_diff_task = false;
    }

    if let Some(val) = &params.sharpen {
        let mut parts = val.splitn(2, ',');
        if let Some(sigma_str) = parts.next()
            && let Ok(sigma) = sigma_str.parse::<f32>()
        {
            let threshold = parts
                .next()
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(0);
            tasks.push(new_sharpen_task(sigma, threshold));
            should_add_diff_task = false;
        }
    }

    if let Some(val) = &params.blur
        && let Ok(sigma) = val.parse::<f32>()
    {
        tasks.push(new_blur_task(sigma));
        should_add_diff_task = false;
    }

    if params.padding_width.is_some() || params.padding_height.is_some() {
        tasks.push(new_padding_task(
            params.padding_width.unwrap_or_default(),
            params.padding_height.unwrap_or_default(),
            params.padding_color.as_deref().unwrap_or_default(),
        ));
        should_add_diff_task = false;
    }

    tasks.push(new_optim_task(&output_type, quality, optim_config.speed));

    if params.strip {
        tasks.push(new_strip_task());
    }

    // DSSIM is only meaningful on the pure-optim path (no transforms), and
    // even there clients can opt out with `?diff=false` to skip the second
    // encode/decode round-trip — expensive for AVIF/JXL.
    let measure_diff = should_add_diff_task && !params.skip_diff;
    if measure_diff {
        tasks.push(new_diff_task());
    }

    img = run_image_blocking(img, tasks).await?;
    let buffer = Bytes::from(img.get_buffer().map_err(map_err)?.to_vec());
    // -1.0 sentinel signals "not measured" to into_response (so the
    // X-Dssim-Diff header is omitted) and to record_dssim (whose `>= 0.0`
    // guard skips the histogram observation).
    let diff = if measure_diff { img.diff } else { -1.0 };
    Ok((
        ImageTaskResult {
            buffer,
            original_size: img.original_size,
            ext: img.ext,
            diff,
        },
        cache_private,
    ))
}

/// 按 ops 顺序执行流水线，不走缓存。
/// 若 ops 中不含 Optim，自动在末尾追加默认编码步骤。
pub async fn run_image_pipeline(data: Vec<u8>, ext: &str, ops: Vec<Op>) -> Result<ImageTaskResult> {
    let started = std::time::Instant::now();
    let basic = must_get_basic_config();
    let bytes_len = data.len() as u64;
    metrics::record_input_bytes(bytes_len);
    if basic.max_source_bytes > 0 && bytes_len > basic.max_source_bytes {
        return Err(reject_too_large(
            "bytes",
            format!(
                "source too large: {bytes_len} bytes > limit {} bytes",
                basic.max_source_bytes
            ),
        ));
    }

    let optim_config = get_default_optim_params();
    let mut img = ProcessImage::new(data, ext).map_err(map_err)?;

    if basic.max_source_pixels > 0 {
        let (w, h) = img.get_size();
        let pixels = (w as u64).saturating_mul(h as u64);
        if pixels > basic.max_source_pixels {
            return Err(reject_too_large(
                "pixels",
                format!(
                    "source too large: {pixels} pixels ({w}x{h}) > limit {}",
                    basic.max_source_pixels
                ),
            ));
        }
    }
    let original_ext = img.ext.clone();

    let mut tasks: Vec<Vec<String>> = Vec::with_capacity(ops.len() + 1);
    let mut has_optim = false;

    for op in ops {
        match op {
            Op::Fit { width, height } => tasks.push(new_fit_task(width, height)),
            Op::Resize { width, height } => tasks.push(new_resize_task(width, height)),
            Op::Crop {
                x,
                y,
                width,
                height,
            } => tasks.push(new_crop_task(x, y, width, height)),
            Op::Rotate { deg } => tasks.push(new_rotate_task(deg)),
            Op::Flip { dir } => tasks.push(new_flip_task(&dir)),
            Op::Gray => tasks.push(new_gray_task()),
            Op::Sharpen { sigma, threshold } => tasks.push(new_sharpen_task(sigma, threshold)),
            Op::Blur { sigma } => tasks.push(new_blur_task(sigma)),
            Op::Brighten { value } => tasks.push(new_brighten_task(value)),
            Op::Contrast { value } => tasks.push(new_contrast_task(value)),
            Op::Strip => tasks.push(new_strip_task()),
            Op::Padding {
                width,
                height,
                color,
            } => tasks.push(new_padding_task(width, height, &color)),
            Op::Watermark {
                data,
                position,
                margin_left,
                margin_top,
            } => tasks.push(new_watermark_task(
                &data,
                &position,
                margin_left,
                margin_top,
            )),
            Op::Optim {
                output_type,
                quality,
            } => {
                let fmt = output_type.as_deref().unwrap_or(&original_ext);
                let q = quality.unwrap_or_else(|| optim_config.quality_for(fmt));
                tasks.push(new_optim_task(fmt, q, optim_config.speed));
                has_optim = true;
            }
        }
    }

    if !has_optim {
        tasks.push(new_optim_task(
            &original_ext,
            optim_config.quality_for(&original_ext),
            optim_config.speed,
        ));
    }

    img = run_image_blocking(img, tasks).await?;
    let buffer = Bytes::from(img.get_buffer().map_err(map_err)?.to_vec());
    metrics::record_output_bytes(&img.ext, buffer.len() as u64);
    metrics::record_task_duration(&img.ext, started.elapsed().as_secs_f64());
    metrics::record_dssim(&img.ext, img.diff);
    Ok(ImageTaskResult {
        buffer,
        original_size: img.original_size,
        ext: img.ext,
        diff: img.diff,
    })
}
