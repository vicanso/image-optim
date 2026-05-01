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

use crate::config::must_get_config;
use crate::dal::get_opendal_storage;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use cached::proc_macro::cached;
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
                speed: 3,
                max_age: default_max_age(),
                auto_output_types: vec![],
            })
    })
}

fn map_err(err: impl ToString) -> Error {
    Error::new(err).with_category("imageoptimize")
}

async fn load_image(file: &str) -> Result<ProcessImage> {
    let ext = file.split('.').next_back().unwrap_or("jpeg");
    let buffer = get_opendal_storage().read(file).await?;
    ProcessImage::new(buffer.to_vec(), ext).map_err(map_err)
}

/// 文件路径或内容型任务参数，用作缓存 key（须实现 Hash + Eq）。
/// f32 字段以字符串形式存储（如 "1.5"），避免 f32 不实现 Hash 的问题。
#[derive(Default, Clone, PartialEq, Eq, Hash, Debug)]
pub struct ImageTaskParams {
    pub file: String,
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
}

#[derive(Default, Clone, Debug)]
pub struct ImageTaskResult {
    pub buffer: Vec<u8>,
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

#[cached(size = 1000, time = 1800, result = true, sync_writes = "by_key")]
pub async fn run_image_task(params: ImageTaskParams) -> Result<(ImageTaskResult, bool)> {
    let optim_config = get_default_optim_params();
    let mut output_type = params.output_type;
    let mut cache_private = false;
    if let Some(auto_output_type) = params.auto_output_type {
        output_type = Some(auto_output_type);
        cache_private = true;
    }
    let mut img = load_image(&params.file).await?;
    let output_type = output_type.unwrap_or(img.ext.clone());
    let quality = params
        .quality
        .unwrap_or_else(|| optim_config.quality_for(&output_type));

    let mut tasks: Vec<Vec<String>> = Vec::with_capacity(16);
    let mut should_add_diff_task = true;

    if let Some(watermark_path) = params.watermark {
        let watermark_data = get_opendal_storage().read(&watermark_path).await?;
        let watermark_b64 = STANDARD.encode(watermark_data.to_vec());
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

    if should_add_diff_task {
        tasks.push(new_diff_task());
    }

    img = run_with_image(img, tasks).await.map_err(map_err)?;
    let buffer = img.get_buffer().map_err(map_err)?.to_vec();
    Ok((
        ImageTaskResult {
            buffer,
            original_size: img.original_size,
            ext: img.ext,
            diff: img.diff,
        },
        cache_private,
    ))
}

/// 按 ops 顺序执行流水线，不走缓存。
/// 若 ops 中不含 Optim，自动在末尾追加默认编码步骤。
pub async fn run_image_pipeline(data: Vec<u8>, ext: &str, ops: Vec<Op>) -> Result<ImageTaskResult> {
    let optim_config = get_default_optim_params();
    let mut img = ProcessImage::new(data, ext).map_err(map_err)?;
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

    img = run_with_image(img, tasks).await.map_err(map_err)?;
    let buffer = img.get_buffer().map_err(map_err)?.to_vec();
    Ok(ImageTaskResult {
        buffer,
        original_size: img.original_size,
        ext: img.ext,
        diff: img.diff,
    })
}
