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
use axum::http::HeaderMap;
use imageoptimize::{
    ProcessImage, new_crop_task, new_diff_task, new_optim_task, new_resize_task,
    new_watermark_task, run_with_image,
};
use once_cell::sync::OnceCell;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashSet;
use std::time::Duration;
use tibba_config::humantime_serde;
use tibba_error::Error;

type Result<T, E = Error> = std::result::Result<T, E>;

pub const AUTO_OUTPUT_TYPE: &str = "auto";

fn default_qualtiy() -> u8 {
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
    #[serde(default = "default_qualtiy")]
    pub quality: u8,
    #[serde(default = "default_speed")]
    pub speed: u8,
    #[serde(default = "default_max_age", with = "humantime_serde")]
    pub max_age: Duration,
    pub auto_output_types: Vec<String>,
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

#[derive(Default)]
pub struct ImageTaskParams {
    pub file: String,
    pub output_type: Option<String>,
    pub quality: Option<u8>,
    pub headers: HeaderMap,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub watermark: Option<String>,
    pub position: Option<String>,
    pub margin_left: Option<i32>,
    pub margin_top: Option<i32>,
    pub x: Option<u32>,
    pub y: Option<u32>,
}

pub async fn run_image_task(params: ImageTaskParams) -> Result<(ProcessImage, bool)> {
    let optim_config = get_default_optim_params();
    let mut output_type = params.output_type;
    let mut cache_private = false;
    if output_type == Some(AUTO_OUTPUT_TYPE.to_string())
        && let Ok(re) = Regex::new(r"image/([^,;]+)")
    {
        let auto_output_types = &optim_config.auto_output_types;
        let accept = params
            .headers
            .get("accept")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();

        let mut formats_set: HashSet<&str> = re
            .captures_iter(accept)
            .filter_map(|cap| cap.get(1).map(|m| m.as_str()))
            .collect();
        // 此两类图片，浏览器均支持
        formats_set.insert("png");
        formats_set.insert("jpeg");
        if let Some(format) = auto_output_types
            .iter()
            .find(|item| formats_set.contains(item.as_str()))
        {
            output_type = Some(format.clone());
        }
        cache_private = true;
    }
    let quality = params.quality.unwrap_or(optim_config.quality);
    let mut img = load_image(&params.file).await?;

    let output_type = output_type.unwrap_or(img.ext.clone());

    let mut tasks = Vec::with_capacity(4);
    let mut should_add_diff_task = true;

    if let Some(watermark) = params.watermark {
        tasks.push(new_watermark_task(
            &watermark,
            &params.position.unwrap_or_default(),
            params.margin_left.unwrap_or_default(),
            params.margin_top.unwrap_or_default(),
        ));
        // 增加水印则图片已经发生了变化，因此不需要计算差异
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
        // 裁剪则图片已经发生了变化，因此不需要计算差异
        should_add_diff_task = false;
    }

    if params.width.is_some() || params.height.is_some() {
        let width = params.width.unwrap_or_default();
        let height = params.height.unwrap_or_default();
        let (w, h) = img.get_size();
        let width = if width == 0 { w * height / h } else { width };

        let height = if height == 0 { h * width / w } else { height };
        tasks.push(new_resize_task(width, height));

        // 由于图片的宽高有变化，因此不需要计算差异
        should_add_diff_task = false;
    }

    tasks.push(new_optim_task(&output_type, quality, optim_config.speed));

    if should_add_diff_task {
        tasks.push(new_diff_task());
    }

    img = run_with_image(img, tasks).await.map_err(map_err)?;
    Ok((img, cache_private))
}
