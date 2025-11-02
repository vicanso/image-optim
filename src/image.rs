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

use crate::dal::get_opendal_storage;
use crate::image_task::{
    AUTO_OUTPUT_TYPE, ImageTaskParams, get_default_optim_params, run_image_task,
};
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use imageoptimize::ProcessImage;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashSet;
use tibba_error::Error;
use tibba_util::QueryParams;
use validator::{Validate, ValidationError};

type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Default)]
struct ImagePreview {
    image: ProcessImage,
    cache_private: bool,
}
impl From<ProcessImage> for ImagePreview {
    fn from(image: ProcessImage) -> Self {
        Self {
            image,
            cache_private: false,
        }
    }
}
impl From<(ProcessImage, bool)> for ImagePreview {
    fn from((image, cache_private): (ProcessImage, bool)) -> Self {
        Self {
            image,
            cache_private,
        }
    }
}

// 图片预览转换为response
impl IntoResponse for ImagePreview {
    fn into_response(self) -> Response {
        let img = self.image;
        let buffer = match img.get_buffer() {
            Ok(buffer) => buffer,
            Err(e) => {
                return map_err(e).into_response();
            }
        };
        let ratio = (100 * buffer.len() / img.original_size).max(1);
        let mut res = Body::from(buffer).into_response();

        // 设置content type
        let result = mime_guess::from_ext(&img.ext).first_or(mime::IMAGE_JPEG);
        if let Ok(value) = HeaderValue::from_str(result.as_ref()) {
            res.headers_mut().insert(header::CONTENT_TYPE, value);
        }

        let max_age = get_default_optim_params().max_age.as_secs();

        // 图片设置为缓存
        let cache_type = if self.cache_private {
            "private"
        } else {
            "public"
        };
        if let Ok(value) =
            HeaderValue::from_str(format!("{cache_type}, max-age={max_age}").as_str())
        {
            res.headers_mut().insert(header::CACHE_CONTROL, value);
        }
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
    if ["jpeg", "jpg", "png", "webp", "avif", AUTO_OUTPUT_TYPE].contains(&output_type) {
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

fn map_err(err: impl ToString) -> Error {
    Error::new(err).with_category("imageoptimize")
}

fn get_auto_output_type(output_type: &Option<String>, headers: &HeaderMap) -> Option<String> {
    let Some(output_type) = output_type else {
        return None;
    };
    if output_type != AUTO_OUTPUT_TYPE {
        return None;
    }
    if let Ok(re) = Regex::new(r"image/([^,;]+)") {
        let optim_config = get_default_optim_params();
        let auto_output_types = &optim_config.auto_output_types;
        let accept = headers
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
            return Some(format.clone());
        }
    }
    None
}

async fn optim(
    QueryParams(params): QueryParams<OptimParams>,
    headers: HeaderMap,
) -> Result<ImagePreview> {
    let auto_output_type = get_auto_output_type(&params.output_type, &headers);
    let preview = run_image_task(ImageTaskParams {
        file: params.file,
        output_type: params.output_type,
        quality: params.quality,
        auto_output_type,
        ..Default::default()
    })
    .await?;

    Ok(preview.into())
}

fn validate_resize_params(params: &ResizeParams) -> Result<(), ValidationError> {
    if params.width == 0 && params.height == 0 {
        return Err(ValidationError::new("width_height")
            .with_message("width and height cannot both be 0".into()));
    }
    Ok(())
}

#[derive(Debug, Deserialize, Clone, Validate)]
#[validate(schema(function = "validate_resize_params"))]
struct ResizeParams {
    #[validate(length(min = 5))]
    file: String,
    quality: Option<u8>,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    #[validate(custom(function = "x_output_type"))]
    output_type: Option<String>,
}

async fn resize(
    QueryParams(params): QueryParams<ResizeParams>,
    headers: HeaderMap,
) -> Result<ImagePreview> {
    let auto_output_type = get_auto_output_type(&params.output_type, &headers);
    let preview = run_image_task(ImageTaskParams {
        file: params.file,
        output_type: params.output_type,
        quality: params.quality,
        width: Some(params.width),
        height: Some(params.height),
        auto_output_type,
        ..Default::default()
    })
    .await?;

    Ok(preview.into())
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
    #[validate(custom(function = "x_output_type"))]
    output_type: Option<String>,
}

async fn watermark(
    QueryParams(params): QueryParams<WatermarkParams>,
    headers: HeaderMap,
) -> Result<ImagePreview> {
    let auto_output_type = get_auto_output_type(&params.output_type, &headers);
    let watermark = get_opendal_storage().read(&params.watermark).await?;
    let watermark = STANDARD.encode(watermark.to_vec());
    let preview = run_image_task(ImageTaskParams {
        file: params.file,
        auto_output_type,
        watermark: Some(watermark),
        position: params.position,
        margin_left: params.margin_left,
        margin_top: params.margin_top,
        quality: params.quality,
        output_type: params.output_type,
        ..Default::default()
    })
    .await?;

    Ok(preview.into())
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
    #[validate(custom(function = "x_output_type"))]
    output_type: Option<String>,
}

async fn crop(
    QueryParams(params): QueryParams<CropParams>,
    headers: HeaderMap,
) -> Result<ImagePreview> {
    let auto_output_type = get_auto_output_type(&params.output_type, &headers);
    let preview = run_image_task(ImageTaskParams {
        file: params.file,
        x: Some(params.x),
        y: Some(params.y),
        width: Some(params.width),
        height: Some(params.height),
        quality: params.quality,
        output_type: params.output_type,
        auto_output_type,
        ..Default::default()
    })
    .await?;

    Ok(preview.into())
}

async fn command() -> Result<String> {
    let command = r#"## API 接口说明

基于存储的图片处理服务提供了以下 REST API 接口，所有接口通过 GET 请求并使用 Query 参数传递。

### 1. 图片优化 (`/images/optim`)

对存储中的图片进行压缩优化，可选择转换图片格式。

**请求方式**: `GET /images/optim`

**Query 参数**:
- `file` (必填): 存储中的图片文件路径，最小长度 5 个字符
- `output_type` (可选): 输出图片格式，支持 `jpeg`、`png`、`webp`、`avif`，默认保持原格式
- `quality` (可选): 图片压缩质量，范围 0-100，默认值为配置中的 `optim.quality`（默认 80）

**返回头部**:
- `Content-Type`: 对应的图片 MIME 类型
- `Cache-Control`: `public, max-age=2592000` (30天缓存)
- `X-Dssim-Diff`: 压缩后与原图的差异值（人眼感知差异）
- `X-Ratio`: 压缩率百分比

**示例**:
```bash
# 优化图片为 webp 格式，质量 75
curl "http://127.0.0.1:3000/images/optim?file=images/photo.jpg&output_type=webp&quality=75"

# 优化图片保持原格式
curl "http://127.0.0.1:3000/images/optim?file=images/photo.png"
```

---

### 2. 图片缩放 (`/images/resize`)

调整存储中图片的尺寸，支持等比例缩放。

**请求方式**: `GET /images/resize`

**Query 参数**:
- `file` (必填): 存储中的图片文件路径，最小长度 5 个字符
- `width` (可选): 目标宽度（像素），默认 0
- `height` (可选): 目标高度（像素），默认 0
- `quality` (可选): 图片压缩质量，默认值为配置中的 `optim.quality`（默认 80）
- `output_type` (可选): 输出图片格式，支持 `jpeg`、`png`、`webp`、`avif`，默认保持原格式

**注意事项**:
- `width` 和 `height` 不能同时为 0
- 当 `width` 为 0 时，根据 `height` 等比例计算宽度
- 当 `height` 为 0 时，根据 `width` 等比例计算高度
- 缩放后会自动进行图片优化处理

**示例**:
```bash
# 缩放图片宽度为 800px，高度等比例调整
curl "http://127.0.0.1:3000/images/resize?file=images/photo.jpg&width=800"

# 缩放图片到指定尺寸 1024x768
curl "http://127.0.0.1:3000/images/resize?file=images/photo.jpg&width=1024&height=768&quality=85"
```

---

### 3. 图片水印 (`/images/watermark`)

为存储中的图片添加水印。

**请求方式**: `GET /images/watermark`

**Query 参数**:
- `file` (必填): 存储中的图片文件路径，最小长度 5 个字符
- `watermark` (必填): 存储中的水印图片路径，最小长度 5 个字符
- `position` (可选): 水印位置，默认为空（具体位置由 imageoptimize 库决定）
- `margin_left` (可选): 水印左边距（像素），默认 0
- `margin_top` (可选): 水印上边距（像素），默认 0
- `quality` (可选): 图片压缩质量，默认值为配置中的 `optim.quality`（默认 80）
- `output_type` (可选): 输出图片格式，支持 `jpeg`、`png`、`webp`、`avif`，默认保持原格式

**说明**:
- 水印图片会被 Base64 编码后传递给图片处理库
- 添加水印后会自动进行图片优化处理

**示例**:
```bash
# 添加水印到右下角
curl "http://127.0.0.1:3000/images/watermark?file=images/photo.jpg&watermark=watermarks/logo.png&position=rightBottom"

# 添加水印并指定边距
curl "http://127.0.0.1:3000/images/watermark?file=images/photo.jpg&watermark=watermarks/logo.png&margin_left=20&margin_top=20&quality=90"
```

---

### 4. 图片裁剪 (`/images/crop`)

按指定区域裁剪图片。

**请求方式**: `GET /images/crop`

**Query 参数**:
- `file` (必填): 存储中的图片文件路径，最小长度 5 个字符
- `x` (可选): 裁剪起始点 X 坐标（像素），默认 0
- `y` (可选): 裁剪起始点 Y 坐标（像素），默认 0
- `width` (必填): 裁剪宽度（像素）
- `height` (必填): 裁剪高度（像素）
- `quality` (可选): 图片压缩质量，默认值为配置中的 `optim.quality`（默认 80）
- `output_type` (可选): 输出图片格式，支持 `jpeg`、`png`、`webp`、`avif`，默认保持原格式

**说明**:
- 裁剪后会自动进行图片优化处理
- 坐标从图片左上角 (0, 0) 开始

**示例**:
```bash
# 从 (100, 100) 位置裁剪 500x500 的区域
curl "http://127.0.0.1:3000/images/crop?file=images/photo.jpg&x=100&y=100&width=500&height=500"

# 从左上角裁剪 800x600 的区域
curl "http://127.0.0.1:3000/images/crop?file=images/photo.jpg&width=800&height=600&quality=85"
```
"#;
    Ok(command.to_string())
}

pub fn new_image_router() -> Router {
    Router::new()
        .route("/optim", get(optim))
        .route("/resize", get(resize))
        .route("/watermark", get(watermark))
        .route("/crop", get(crop))
        .route("/command", get(command))
}
