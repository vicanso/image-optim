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
use crate::preset::get_preset;
use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::Query;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::str::FromStr;
use tibba_error::Error;
use tibba_util::QueryParams;
use validator::{Validate, ValidationError};

type Result<T, E = Error> = std::result::Result<T, E>;

/// `serde_urlencoded` only accepts JSON-literal booleans, but query strings
/// carry every value as a string. Bridge the two by accepting the common
/// truthy/falsy spellings for `bool` query fields.
fn lenient_bool<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<bool, D::Error> {
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = bool;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a bool or the strings 1/true/yes/on / 0/false/no/off")
        }
        fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<bool, E> {
            Ok(v)
        }
        fn visit_str<E: de::Error>(self, s: &str) -> std::result::Result<bool, E> {
            match s.to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => Ok(true),
                "" | "0" | "false" | "no" | "off" => Ok(false),
                other => Err(E::custom(format!("invalid bool: {other}"))),
            }
        }
    }
    d.deserialize_any(V)
}

/// `Option<bool>` flavour: missing key → `None`, otherwise lenient bool.
fn lenient_optional_bool<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<bool>, D::Error> {
    lenient_bool(d).map(Some)
}

static ACCEPT_IMAGE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"image/([^,;]+)").expect("invalid regex"));

/// `Content-Type` per output format. Built once; lookup → clone is a
/// HeaderValue Bytes ref-count bump. Covers every format imageoptimize
/// can actually emit (see OptimProcess); unknown ext falls back to
/// `image/jpeg`, matching the imageoptimize encoder's own fallback.
static CONTENT_TYPES: Lazy<HashMap<&'static str, HeaderValue>> = Lazy::new(|| {
    let mut m = HashMap::with_capacity(8);
    let jpeg = HeaderValue::from_static("image/jpeg");
    m.insert("jpeg", jpeg.clone());
    m.insert("jpg", jpeg);
    m.insert("png", HeaderValue::from_static("image/png"));
    m.insert("webp", HeaderValue::from_static("image/webp"));
    m.insert("avif", HeaderValue::from_static("image/avif"));
    m.insert("jxl", HeaderValue::from_static("image/jxl"));
    m.insert("gif", HeaderValue::from_static("image/gif"));
    m
});

static CONTENT_TYPE_FALLBACK: Lazy<HeaderValue> =
    Lazy::new(|| HeaderValue::from_static("image/jpeg"));

/// `Cache-Control` precomputed from `optim.max_age`. Built on first request
/// (after config is loaded); subsequent requests pay only an Arc bump.
static CACHE_PUBLIC: Lazy<HeaderValue> = Lazy::new(|| {
    let max_age = get_default_optim_params().max_age.as_secs();
    HeaderValue::from_str(&format!("public, max-age={max_age}"))
        .expect("cache-control header value")
});

static CACHE_PRIVATE: Lazy<HeaderValue> = Lazy::new(|| {
    let max_age = get_default_optim_params().max_age.as_secs();
    HeaderValue::from_str(&format!("private, max-age={max_age}"))
        .expect("cache-control header value")
});

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
        let headers = res.headers_mut();

        let ct = CONTENT_TYPES
            .get(img.ext.as_str())
            .cloned()
            .unwrap_or_else(|| CONTENT_TYPE_FALLBACK.clone());
        headers.insert(header::CONTENT_TYPE, ct);

        let cc = if self.cache_private {
            CACHE_PRIVATE.clone()
        } else {
            CACHE_PUBLIC.clone()
        };
        headers.insert(header::CACHE_CONTROL, cc);

        if img.diff >= 0.0f64
            && let Ok(value) = HeaderValue::from_str(&format!("{:.2}", img.diff))
        {
            headers.insert("X-Dssim-Diff", value);
        }
        if let Ok(value) = HeaderValue::from_str(ratio.to_string().as_str()) {
            headers.insert("X-Ratio", value);
        }

        res
    }
}

fn x_output_type(output_type: &str) -> Result<(), ValidationError> {
    if [
        "jpeg",
        "jpg",
        "png",
        "webp",
        "avif",
        "jxl",
        AUTO_OUTPUT_TYPE,
    ]
    .contains(&output_type)
    {
        return Ok(());
    }
    Err(ValidationError::new("output_type").with_message("invalid output type".into()))
}

/// 所有 GET 端点共享的图像调整参数，通过 `#[serde(flatten)]` 嵌入各端点结构体。
#[derive(Debug, Deserialize, Clone, Default)]
struct AdjustParams {
    /// OpenDAL 存储源名，对应 `IMOP__OPENDAL__<NAME>__URL`；未设置时使用默认存储。
    source: Option<String>,
    /// 旋转角度：90 / 180 / 270
    rotate: Option<u16>,
    /// 翻转方向：h / horizontal / v / vertical
    flip: Option<String>,
    /// 转换为灰度图
    #[serde(default, deserialize_with = "lenient_bool")]
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
    #[serde(default, deserialize_with = "lenient_bool")]
    strip: bool,
    /// 画布扩展宽度（像素），与 padding_height 配合使用
    padding_width: Option<u32>,
    /// 画布扩展高度（像素）
    padding_height: Option<u32>,
    /// 画布填充色，十六进制（如 #ffffff 或 #ffffff80），默认透明
    padding_color: Option<String>,
    /// 是否计算 DSSIM 并设置 X-Dssim-Diff 响应头。默认 true；客户端不关心
    /// 时显式 `?diff=false` 可跳过纯优化路径上的二次编解码（AVIF/JXL 提速
    /// 尤其明显）。
    #[serde(default, deserialize_with = "lenient_optional_bool")]
    diff: Option<bool>,
}

impl AdjustParams {
    fn apply_to(&self, p: &mut ImageTaskParams) {
        p.source = self
            .source
            .as_ref()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty());
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
        p.skip_diff = matches!(self.diff, Some(false));
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

// ── preset ───────────────────────────────────────────────────────────────────

fn bad_request<S: Into<String>>(msg: S) -> Error {
    Error::new(msg.into())
        .with_category("preset")
        .with_status(400)
}

/// 从字符串解析为类型 T，失败时返回 400。空字符串视为缺省（None）。
fn parse_opt<T: FromStr>(raw: Option<&String>, key: &str) -> Result<Option<T>>
where
    T::Err: std::fmt::Display,
{
    match raw {
        None => Ok(None),
        Some(v) if v.is_empty() => Ok(None),
        Some(v) => v
            .parse::<T>()
            .map(Some)
            .map_err(|e| bad_request(format!("invalid `{key}`: {e}"))),
    }
}

fn parse_bool(raw: Option<&String>) -> bool {
    matches!(
        raw.map(|s| s.to_lowercase()).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

/// 把 `merged` 中的通用调整字段（rotate/flip/gray/...）写入 task。
fn apply_adjust(task: &mut ImageTaskParams, merged: &BTreeMap<String, String>) -> Result<()> {
    task.source = merged
        .get("source")
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    task.rotate = parse_opt::<u16>(merged.get("rotate"), "rotate")?;
    task.flip = merged.get("flip").cloned().filter(|s| !s.is_empty());
    task.gray = parse_bool(merged.get("gray"));
    task.sharpen = merged.get("sharpen").cloned().filter(|s| !s.is_empty());
    task.blur = merged.get("blur").cloned().filter(|s| !s.is_empty());
    task.brighten = parse_opt::<i32>(merged.get("brighten"), "brighten")?;
    task.contrast = merged.get("contrast").cloned().filter(|s| !s.is_empty());
    task.strip = parse_bool(merged.get("strip"));
    task.padding_width = parse_opt::<u32>(merged.get("padding_width"), "padding_width")?;
    task.padding_height = parse_opt::<u32>(merged.get("padding_height"), "padding_height")?;
    task.padding_color = merged
        .get("padding_color")
        .cloned()
        .filter(|s| !s.is_empty());
    // Default-true semantics: skip DSSIM only on explicit falsey value.
    task.skip_diff = matches!(
        merged.get("diff").map(|s| s.to_lowercase()).as_deref(),
        Some("0" | "false" | "no" | "off")
    );
    Ok(())
}

fn validate_output_type(merged: &BTreeMap<String, String>) -> Result<Option<String>> {
    let Some(raw) = merged.get("output_type") else {
        return Ok(None);
    };
    if raw.is_empty() {
        return Ok(None);
    }
    if ![
        "jpeg",
        "jpg",
        "png",
        "webp",
        "avif",
        "jxl",
        AUTO_OUTPUT_TYPE,
    ]
    .contains(&raw.as_str())
    {
        return Err(bad_request(format!("invalid `output_type`: {raw}")));
    }
    Ok(Some(raw.clone()))
}

/// 按 `op` 从合并后的参数表构造 ImageTaskParams。请求侧覆盖预设侧已在调用前完成。
fn build_task(
    op: &str,
    merged: &BTreeMap<String, String>,
    headers: &HeaderMap,
) -> Result<ImageTaskParams> {
    let file = merged
        .get("file")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad_request("missing `file`"))?
        .clone();
    if file.len() < 5 {
        return Err(bad_request("`file` length must be >= 5"));
    }

    let output_type = validate_output_type(merged)?;
    let quality = parse_opt::<u8>(merged.get("quality"), "quality")?;
    let auto_output_type = get_auto_output_type(&output_type, headers);

    let mut task = ImageTaskParams {
        file,
        output_type,
        quality,
        auto_output_type,
        ..Default::default()
    };

    match op {
        "optim" => {}
        "resize" => {
            let w = parse_opt::<u32>(merged.get("width"), "width")?.unwrap_or(0);
            let h = parse_opt::<u32>(merged.get("height"), "height")?.unwrap_or(0);
            if w == 0 && h == 0 {
                return Err(bad_request("resize: `width` and `height` cannot both be 0"));
            }
            task.width = Some(w);
            task.height = Some(h);
        }
        "fit" => {
            let w = parse_opt::<u32>(merged.get("width"), "width")?.unwrap_or(0);
            let h = parse_opt::<u32>(merged.get("height"), "height")?.unwrap_or(0);
            if w == 0 && h == 0 {
                return Err(bad_request("fit: `width` and `height` cannot both be 0"));
            }
            task.fit_width = Some(w);
            task.fit_height = Some(h);
        }
        "watermark" => {
            let wm = merged
                .get("watermark")
                .filter(|s| !s.is_empty())
                .ok_or_else(|| bad_request("watermark: missing `watermark`"))?
                .clone();
            if wm.len() < 5 {
                return Err(bad_request("watermark: `watermark` length must be >= 5"));
            }
            task.watermark = Some(wm);
            task.position = merged.get("position").cloned().filter(|s| !s.is_empty());
            task.margin_left = parse_opt::<i32>(merged.get("margin_left"), "margin_left")?;
            task.margin_top = parse_opt::<i32>(merged.get("margin_top"), "margin_top")?;
        }
        "crop" => {
            let w = parse_opt::<u32>(merged.get("width"), "width")?
                .ok_or_else(|| bad_request("crop: missing `width`"))?;
            let h = parse_opt::<u32>(merged.get("height"), "height")?
                .ok_or_else(|| bad_request("crop: missing `height`"))?;
            task.width = Some(w);
            task.height = Some(h);
            task.x = Some(parse_opt::<u32>(merged.get("x"), "x")?.unwrap_or(0));
            task.y = Some(parse_opt::<u32>(merged.get("y"), "y")?.unwrap_or(0));
        }
        "padding" => {
            let w = parse_opt::<u32>(merged.get("width"), "width")?
                .ok_or_else(|| bad_request("padding: missing `width`"))?;
            let h = parse_opt::<u32>(merged.get("height"), "height")?
                .ok_or_else(|| bad_request("padding: missing `height`"))?;
            task.padding_width = Some(w);
            task.padding_height = Some(h);
            task.padding_color = merged.get("color").cloned().filter(|s| !s.is_empty());
        }
        other => return Err(bad_request(format!("unknown preset op: {other}"))),
    }

    apply_adjust(&mut task, merged)?;
    Ok(task)
}

async fn preset(
    Query(req): Query<BTreeMap<String, String>>,
    headers: HeaderMap,
) -> Result<ImagePreview> {
    let name = req
        .get("preset")
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad_request("missing `preset`"))?;

    let preset =
        get_preset(&name).ok_or_else(|| bad_request(format!("preset not found: {name}")))?;

    // 合并：先放预设默认值，再用请求参数覆盖（请求侧 > 预设侧）。
    let mut merged = preset.params.clone();
    for (k, v) in req {
        if k == "preset" {
            continue;
        }
        merged.insert(k.to_lowercase(), v);
    }

    let task = build_task(&preset.op, &merged, &headers)?;
    Ok(run_image_task(task).await?.into())
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
        .route("/preset", get(preset))
        .route("/process", post(process))
        .route("/command", get(command))
}
