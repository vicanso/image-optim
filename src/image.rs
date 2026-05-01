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

use crate::image_task::{
    AUTO_OUTPUT_TYPE, ImageTaskParams, ImageTaskResult, Op, get_default_optim_params,
    run_image_pipeline, run_image_task,
};
use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tibba_error::Error;
use tibba_util::QueryParams;
use validator::{Validate, ValidationError};

type Result<T, E = Error> = std::result::Result<T, E>;

static ACCEPT_IMAGE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"image/([^,;]+)").expect("invalid regex"));

struct ImagePreview {
    image: ImageTaskResult,
    cache_private: bool,
}

impl From<(ImageTaskResult, bool)> for ImagePreview {
    fn from((image, cache_private): (ImageTaskResult, bool)) -> Self {
        Self {
            image,
            cache_private,
        }
    }
}

impl IntoResponse for ImagePreview {
    fn into_response(self) -> Response {
        let img = self.image;
        let buffer = img.buffer;

        let ratio = (100 * buffer.len() / img.original_size).max(1);
        let mut res = Body::from(buffer).into_response();

        let result = mime_guess::from_ext(&img.ext).first_or(mime::IMAGE_JPEG);
        if let Ok(value) = HeaderValue::from_str(result.as_ref()) {
            res.headers_mut().insert(header::CONTENT_TYPE, value);
        }

        let max_age = get_default_optim_params().max_age.as_secs();
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

/// 所有 GET 端点共享的图像调整参数，通过 `#[serde(flatten)]` 嵌入各端点结构体。
#[derive(Debug, Deserialize, Clone, Default)]
struct AdjustParams {
    /// 旋转角度：90 / 180 / 270
    rotate: Option<u16>,
    /// 翻转方向：h / horizontal / v / vertical
    flip: Option<String>,
    /// 转换为灰度图
    #[serde(default)]
    gray: bool,
    /// USM 锐化，格式：sigma 或 sigma,threshold（如 1.0 或 1.0,5）
    sharpen: Option<String>,
    /// 高斯模糊 sigma（如 2.0）
    blur: Option<String>,
    /// 亮度调整，正数增亮，负数减暗
    brighten: Option<i32>,
    /// 对比度调整（如 1.5），浮点数字符串
    contrast: Option<String>,
    /// 剥离 EXIF 元数据（不重新编码）
    #[serde(default)]
    strip: bool,
    /// 画布扩展宽度（像素），与 padding_height 配合使用
    padding_width: Option<u32>,
    /// 画布扩展高度（像素）
    padding_height: Option<u32>,
    /// 画布填充色，十六进制（如 #ffffff 或 #ffffff80），默认透明
    padding_color: Option<String>,
}

impl AdjustParams {
    fn apply_to(&self, p: &mut ImageTaskParams) {
        p.rotate = self.rotate;
        p.flip = self.flip.clone();
        p.gray = self.gray;
        p.sharpen = self.sharpen.clone();
        p.blur = self.blur.clone();
        p.brighten = self.brighten;
        p.contrast = self.contrast.clone();
        p.strip = self.strip;
        p.padding_width = self.padding_width;
        p.padding_height = self.padding_height;
        p.padding_color = self.padding_color.clone();
    }
}

fn get_auto_output_type(output_type: &Option<String>, headers: &HeaderMap) -> Option<String> {
    let Some(output_type) = output_type else {
        return None;
    };
    if output_type != AUTO_OUTPUT_TYPE {
        return None;
    }
    let optim_config = get_default_optim_params();
    let auto_output_types = &optim_config.auto_output_types;
    let accept = headers
        .get("accept")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    let mut formats_set: HashSet<&str> = ACCEPT_IMAGE_RE
        .captures_iter(accept)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str()))
        .collect();
    formats_set.insert("png");
    formats_set.insert("jpeg");
    auto_output_types
        .iter()
        .find(|item| formats_set.contains(item.as_str()))
        .cloned()
}

// ── optim ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone, Validate)]
struct OptimParams {
    #[validate(length(min = 5))]
    file: String,
    #[validate(custom(function = "x_output_type"))]
    output_type: Option<String>,
    quality: Option<u8>,
    #[serde(flatten)]
    adjust: AdjustParams,
}

async fn optim(
    QueryParams(params): QueryParams<OptimParams>,
    headers: HeaderMap,
) -> Result<ImagePreview> {
    let auto_output_type = get_auto_output_type(&params.output_type, &headers);
    let mut task = ImageTaskParams {
        file: params.file,
        output_type: params.output_type,
        quality: params.quality,
        auto_output_type,
        ..Default::default()
    };
    params.adjust.apply_to(&mut task);
    Ok(run_image_task(task).await?.into())
}

// ── resize ───────────────────────────────────────────────────────────────────

fn validate_resize_params(p: &ResizeParams) -> Result<(), ValidationError> {
    if p.width == 0 && p.height == 0 {
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
    #[serde(flatten)]
    adjust: AdjustParams,
}

async fn resize(
    QueryParams(params): QueryParams<ResizeParams>,
    headers: HeaderMap,
) -> Result<ImagePreview> {
    let auto_output_type = get_auto_output_type(&params.output_type, &headers);
    let mut task = ImageTaskParams {
        file: params.file,
        output_type: params.output_type,
        quality: params.quality,
        width: Some(params.width),
        height: Some(params.height),
        auto_output_type,
        ..Default::default()
    };
    params.adjust.apply_to(&mut task);
    Ok(run_image_task(task).await?.into())
}

// ── fit ──────────────────────────────────────────────────────────────────────

fn validate_fit_params(p: &FitParams) -> Result<(), ValidationError> {
    if p.width == 0 && p.height == 0 {
        return Err(ValidationError::new("width_height")
            .with_message("width and height cannot both be 0".into()));
    }
    Ok(())
}

#[derive(Debug, Deserialize, Clone, Validate)]
#[validate(schema(function = "validate_fit_params"))]
struct FitParams {
    #[validate(length(min = 5))]
    file: String,
    quality: Option<u8>,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    #[validate(custom(function = "x_output_type"))]
    output_type: Option<String>,
    #[serde(flatten)]
    adjust: AdjustParams,
}

async fn fit(
    QueryParams(params): QueryParams<FitParams>,
    headers: HeaderMap,
) -> Result<ImagePreview> {
    let auto_output_type = get_auto_output_type(&params.output_type, &headers);
    let mut task = ImageTaskParams {
        file: params.file,
        output_type: params.output_type,
        quality: params.quality,
        fit_width: Some(params.width),
        fit_height: Some(params.height),
        auto_output_type,
        ..Default::default()
    };
    params.adjust.apply_to(&mut task);
    Ok(run_image_task(task).await?.into())
}

// ── watermark ────────────────────────────────────────────────────────────────

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
    #[serde(flatten)]
    adjust: AdjustParams,
}

async fn watermark(
    QueryParams(params): QueryParams<WatermarkParams>,
    headers: HeaderMap,
) -> Result<ImagePreview> {
    let auto_output_type = get_auto_output_type(&params.output_type, &headers);
    let mut task = ImageTaskParams {
        file: params.file,
        auto_output_type,
        watermark: Some(params.watermark),
        position: params.position,
        margin_left: params.margin_left,
        margin_top: params.margin_top,
        quality: params.quality,
        output_type: params.output_type,
        ..Default::default()
    };
    params.adjust.apply_to(&mut task);
    Ok(run_image_task(task).await?.into())
}

// ── crop ─────────────────────────────────────────────────────────────────────

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
    #[serde(flatten)]
    adjust: AdjustParams,
}

async fn crop(
    QueryParams(params): QueryParams<CropParams>,
    headers: HeaderMap,
) -> Result<ImagePreview> {
    let auto_output_type = get_auto_output_type(&params.output_type, &headers);
    let mut task = ImageTaskParams {
        file: params.file,
        x: Some(params.x),
        y: Some(params.y),
        width: Some(params.width),
        height: Some(params.height),
        quality: params.quality,
        output_type: params.output_type,
        auto_output_type,
        ..Default::default()
    };
    params.adjust.apply_to(&mut task);
    Ok(run_image_task(task).await?.into())
}

// ── padding ──────────────────────────────────────────────────────────────────

fn validate_padding_params(p: &PaddingParams) -> Result<(), ValidationError> {
    if p.width == 0 && p.height == 0 {
        return Err(ValidationError::new("width_height")
            .with_message("width and height cannot both be 0".into()));
    }
    Ok(())
}

#[derive(Debug, Deserialize, Clone, Validate)]
#[validate(schema(function = "validate_padding_params"))]
struct PaddingParams {
    #[validate(length(min = 5))]
    file: String,
    width: u32,
    height: u32,
    color: Option<String>,
    quality: Option<u8>,
    #[validate(custom(function = "x_output_type"))]
    output_type: Option<String>,
    #[serde(flatten)]
    adjust: AdjustParams,
}

async fn padding(
    QueryParams(params): QueryParams<PaddingParams>,
    headers: HeaderMap,
) -> Result<ImagePreview> {
    let auto_output_type = get_auto_output_type(&params.output_type, &headers);
    let mut task = ImageTaskParams {
        file: params.file,
        output_type: params.output_type,
        quality: params.quality,
        padding_width: Some(params.width),
        padding_height: Some(params.height),
        padding_color: params.color,
        auto_output_type,
        ..Default::default()
    };
    params.adjust.apply_to(&mut task);
    Ok(run_image_task(task).await?.into())
}

// ── process (POST pipeline) ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, Validate)]
struct ProcessJsonParams {
    #[validate(length(min = 1))]
    data: String,
    #[validate(length(min = 1))]
    ext: String,
    #[serde(default)]
    ops: Vec<Op>,
}

#[derive(Serialize)]
struct ProcessJsonResponse {
    data: String,
    ext: String,
    ratio: usize,
    diff: f64,
}

async fn process(Json(params): Json<ProcessJsonParams>) -> Result<Json<ProcessJsonResponse>> {
    params.validate().map_err(Error::new)?;
    let image_data = STANDARD.decode(&params.data).map_err(Error::new)?;
    let result = run_image_pipeline(image_data, &params.ext, params.ops).await?;

    let ratio = (100 * result.buffer.len() / result.original_size).max(1);
    Ok(Json(ProcessJsonResponse {
        data: STANDARD.encode(&result.buffer),
        ext: result.ext,
        ratio,
        diff: result.diff,
    }))
}

// ── command ──────────────────────────────────────────────────────────────────

async fn command() -> Result<String> {
    let command = r#"## API 接口说明

图片处理服务提供以下 REST API 接口。

---

### 公共可选参数（所有 GET 接口均支持）

| 参数 | 说明 | 示例 |
|------|------|------|
| `output_type` | 输出格式：`jpeg` `png` `webp` `avif` `auto` | `output_type=webp` |
| `quality` | 压缩质量 0-100，默认 80 | `quality=85` |
| `rotate` | 旋转角度：`90` `180` `270` | `rotate=90` |
| `flip` | 翻转：`h`/`horizontal` 或 `v`/`vertical` | `flip=h` |
| `gray` | 转灰度图 | `gray=true` |
| `sharpen` | USM 锐化，格式：`sigma` 或 `sigma,threshold` | `sharpen=1.0` |
| `blur` | 高斯模糊 sigma | `blur=2.0` |
| `brighten` | 亮度调整，正数增亮/负数减暗 | `brighten=20` |
| `contrast` | 对比度，浮点字符串 | `contrast=1.5` |
| `strip` | 剥离 EXIF 元数据 | `strip=true` |
| `padding_width` | 画布扩展宽度（像素） | `padding_width=1000` |
| `padding_height` | 画布扩展高度（像素） | `padding_height=1000` |
| `padding_color` | 填充色，十六进制，默认透明 | `padding_color=%23ffffff` |

---

### 1. 图片优化 (`GET /images/optim`)

压缩优化存储中的图片，可选格式转换。

**必填参数**: `file`（存储路径，最小长度 5）

```bash
curl "http://127.0.0.1:3000/images/optim?file=photo.jpg&output_type=webp&quality=75"
curl "http://127.0.0.1:3000/images/optim?file=photo.jpg&strip=true&sharpen=1.0"
```

---

### 2. 图片缩放 (`GET /images/resize`)

等比或指定尺寸缩放，其中一边为 0 时按另一边等比计算。

**必填参数**: `file`；`width` 和 `height` 不能同时为 0

```bash
curl "http://127.0.0.1:3000/images/resize?file=photo.jpg&width=800"
curl "http://127.0.0.1:3000/images/resize?file=photo.jpg&width=800&rotate=90"
```

---

### 3. 适应缩放 (`GET /images/fit`)

缩放至不超过指定尺寸，保持比例，不放大原图。与 `resize` 的区别：若图片已在边界内则不做任何操作。

**必填参数**: `file`；`width` 和 `height` 不能同时为 0

```bash
curl "http://127.0.0.1:3000/images/fit?file=photo.jpg&width=800&height=600"
```

---

### 4. 图片水印 (`GET /images/watermark`)

叠加存储中的水印图片。

**必填参数**: `file`、`watermark`（水印存储路径）

**可选参数**: `position`、`margin_left`、`margin_top`

```bash
curl "http://127.0.0.1:3000/images/watermark?file=photo.jpg&watermark=logo.png&position=rightBottom"
```

---

### 5. 图片裁剪 (`GET /images/crop`)

按矩形区域裁剪，坐标原点为左上角。

**必填参数**: `file`、`width`、`height`

**可选参数**: `x`（默认 0）、`y`（默认 0）

```bash
curl "http://127.0.0.1:3000/images/crop?file=photo.jpg&x=100&y=100&width=500&height=500"
```

---

### 6. 画布填充 (`GET /images/padding`)

将图片居中并扩展画布至指定尺寸。

**必填参数**: `file`、`width`、`height`（不能同时为 0）

**可选参数**: `color`（填充色，默认透明）

```bash
curl "http://127.0.0.1:3000/images/padding?file=photo.jpg&width=1000&height=1000&color=%23ffffff"
```

---

### 7. 流水线处理（Base64）(`POST /images/process`)

以 JSON 提交 Base64 图片，按 `ops` 数组顺序执行任意组合操作，返回 Base64 结果。

**请求体** (`Content-Type: application/json`):
- `data` (必填): Base64 编码的图片
- `ext` (必填): 原始格式扩展名，如 `jpg`
- `ops` (可选): 操作数组，不含 `optim` 时自动追加默认编码

**支持的操作类型**（`type` 字段）:

| type | 参数 |
|------|------|
| `fit` | `width`, `height` |
| `resize` | `width`, `height` |
| `crop` | `x`, `y`, `width`, `height` |
| `rotate` | `deg`（90/180/270） |
| `flip` | `dir`（h/v） |
| `gray` | — |
| `sharpen` | `sigma`, `threshold`（默认 0） |
| `blur` | `sigma` |
| `brighten` | `value` |
| `contrast` | `value` |
| `strip` | — |
| `padding` | `width`, `height`, `color`（默认透明） |
| `watermark` | `data`（Base64）, `position`, `margin_left`, `margin_top` |
| `optim` | `output_type`, `quality` |

**响应字段**: `data`（Base64）、`ext`、`ratio`（压缩率）、`diff`（DSSIM，仅纯优化时有效）

```bash
curl -X POST http://127.0.0.1:3000/images/process \
  -H "Content-Type: application/json" \
  -d '{
    "data": "<base64>",
    "ext": "jpg",
    "ops": [
      {"type": "fit", "width": 800, "height": 600},
      {"type": "sharpen", "sigma": 1.0},
      {"type": "optim", "output_type": "webp", "quality": 80}
    ]
  }'
```
"#;
    Ok(command.to_string())
}

// ── router ───────────────────────────────────────────────────────────────────

pub fn new_image_router() -> Router {
    Router::new()
        .route("/optim", get(optim))
        .route("/resize", get(resize))
        .route("/fit", get(fit))
        .route("/watermark", get(watermark))
        .route("/crop", get(crop))
        .route("/padding", get(padding))
        .route("/process", post(process))
        .route("/command", get(command))
}
