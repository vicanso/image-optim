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
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use imageoptimize::{
    ProcessImage, new_crop_task, new_diff_task, new_optim_task, new_resize_task,
    new_watermark_task, run_with_image,
};
use once_cell::sync::OnceCell;
use serde::Deserialize;
use tibba_error::Error;
use tibba_util::QueryParams;
use validator::{Validate, ValidationError};

type Result<T, E = Error> = std::result::Result<T, E>;

struct OptimConfig {
    quality: u8,
    speed: u8,
}

static OPTIM_CONFIG: OnceCell<OptimConfig> = OnceCell::new();

fn get_default_optim_params() -> (u8, u8) {
    let config = OPTIM_CONFIG.get_or_init(|| {
        let app_config = must_get_config();
        let config = app_config.sub_config("optim");
        OptimConfig {
            quality: config.get_int("quality", 80) as u8,
            speed: config.get_int("speed", 3) as u8,
        }
    });
    (config.quality, config.speed)
}

#[derive(Default)]
struct ImagePreview(ProcessImage);

// 图片预览转换为response
impl IntoResponse for ImagePreview {
    fn into_response(self) -> Response {
        let img = self.0;
        let buffer = match img.get_buffer() {
            Ok(buffer) => buffer,
            Err(e) => {
                return map_err(e).into_response();
            }
        };
        let ratio = 100 * buffer.len() / img.original_size;
        let mut res = Body::from(buffer).into_response();

        // 设置content type
        let result = mime_guess::from_ext(&img.ext).first_or(mime::IMAGE_JPEG);
        if let Ok(value) = HeaderValue::from_str(result.as_ref()) {
            res.headers_mut().insert(header::CONTENT_TYPE, value);
        }

        // 图片设置为缓存30天
        res.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=2592000"),
        );
        if img.diff >= 0.0f64
            && let Ok(value) = HeaderValue::from_str(&format!("{:.2}", img.diff))
        {
            res.headers_mut().insert("X-Dssim-Diff", value);
        }
        if let Ok(value) = HeaderValue::from_str(ratio.to_string().as_str()) {
            res.headers_mut().insert("X-Ratio", value);
        }

        res
    }
}

fn x_output_type(output_type: &str) -> Result<(), ValidationError> {
    if ["jpeg", "jpg", "png", "webp", "avif"].contains(&output_type) {
        return Ok(());
    }
    Err(ValidationError::new("output_type").with_message("invalid output type".into()))
}

#[derive(Debug, Deserialize, Clone, Validate)]
struct OptimParams {
    #[validate(length(min = 5))]
    file: String,
    #[validate(custom(function = "x_output_type"))]
    output_type: Option<String>,
    quality: Option<u8>,
}

fn map_err(err: imageoptimize::ImageProcessingError) -> Error {
    Error::new(err).with_category("imageoptimize")
}
async fn load_image(file: &str) -> Result<ProcessImage> {
    let ext = file.split('.').next_back().unwrap_or("jpeg");
    let buffer = get_opendal_storage().read(file).await?;
    ProcessImage::new(buffer.to_vec(), ext).map_err(map_err)
}

async fn optim(QueryParams(params): QueryParams<OptimParams>) -> Result<ImagePreview> {
    let (default_qualtiy, default_speed) = get_default_optim_params();
    let quality = params.quality.unwrap_or(default_qualtiy);
    let mut img = load_image(&params.file).await?;
    let output_type = params.output_type.unwrap_or(img.ext.clone());
    img = run_with_image(
        img,
        vec![
            new_optim_task(&output_type, quality, default_speed),
            new_diff_task(),
        ],
    )
    .await
    .map_err(map_err)?;

    Ok(ImagePreview(img))
}

#[derive(Debug, Deserialize, Clone, Validate)]
struct ResizeParams {
    #[validate(length(min = 5))]
    file: String,
    quality: Option<u8>,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

async fn resize(QueryParams(params): QueryParams<ResizeParams>) -> Result<ImagePreview> {
    let (default_qualtiy, default_speed) = get_default_optim_params();
    let mut img = load_image(&params.file).await?;
    if params.width == 0 && params.height == 0 {
        return Err(Error::new("width and height can not be 0").with_category("validate"));
    }
    let quality = params.quality.unwrap_or(default_qualtiy);
    let (w, h) = img.get_size();
    let width = if params.width == 0 {
        w * params.height / h
    } else {
        params.width
    };
    let ext = img.ext.clone();

    let height = if params.height == 0 {
        h * params.width / w
    } else {
        params.height
    };
    img = run_with_image(
        img,
        vec![
            new_resize_task(width, height),
            new_optim_task(&ext, quality, default_speed),
        ],
    )
    .await
    .map_err(map_err)?;

    Ok(ImagePreview(img))
}

#[derive(Debug, Deserialize, Clone, Validate)]
struct WatermarkParams {
    #[validate(length(min = 5))]
    file: String,
    #[validate(length(min = 5))]
    watermark: String,
    position: Option<String>,
    margin_left: Option<i32>,
    margin_top: Option<i32>,
    quality: Option<u8>,
}

async fn watermark(QueryParams(params): QueryParams<WatermarkParams>) -> Result<ImagePreview> {
    let (default_qualtiy, default_speed) = get_default_optim_params();
    let watermark = get_opendal_storage().read(&params.watermark).await?;
    let watermark = STANDARD.encode(watermark.to_vec());
    let mut img = load_image(&params.file).await?;
    let ext = img.ext.clone();
    let quality = params.quality.unwrap_or(default_qualtiy);

    img = run_with_image(
        img,
        vec![
            new_watermark_task(
                &watermark,
                &params.position.unwrap_or_default(),
                params.margin_left.unwrap_or_default(),
                params.margin_top.unwrap_or_default(),
            ),
            new_optim_task(&ext, quality, default_speed),
        ],
    )
    .await
    .map_err(map_err)?;

    Ok(ImagePreview(img))
}

#[derive(Debug, Deserialize, Clone, Validate)]
struct CropParams {
    #[validate(length(min = 5))]
    file: String,
    #[serde(default)]
    x: u32,
    #[serde(default)]
    y: u32,
    width: u32,
    height: u32,
    quality: Option<u8>,
}

async fn crop(QueryParams(params): QueryParams<CropParams>) -> Result<ImagePreview> {
    let (default_qualtiy, default_speed) = get_default_optim_params();
    let mut img = load_image(&params.file).await?;
    let ext = img.ext.clone();
    let quality = params.quality.unwrap_or(default_qualtiy);
    img = run_with_image(
        img,
        vec![
            new_crop_task(params.x, params.y, params.width, params.height),
            new_optim_task(&ext, quality, default_speed),
        ],
    )
    .await
    .map_err(map_err)?;
    Ok(ImagePreview(img))
}

pub fn new_image_router() -> Router {
    Router::new()
        .route("/optim", get(optim))
        .route("/resize", get(resize))
        .route("/watermark", get(watermark))
        .route("/crop", get(crop))
}
