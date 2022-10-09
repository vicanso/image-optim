use crate::error::HTTPError;
use crate::image;
use crate::response::ResponseResult;
use axum::{routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

pub fn new_router() -> Router {
    let r = Router::new();

    r.route("/optim-images/preview", post(optim_image_preview))
        .route("/optim-images", post(optim_image))
}

async fn handle(params: OptimImageParams) -> Result<OptimResult, HTTPError> {
    // 如果data是http:// 则通过http方式读取数据
    let mut data_type = params.data_type;
    let result = match params.data.starts_with("http") {
        true => {
            let resp = reqwest::get(params.data).await?;
            if let Some(content_type) = resp.headers().get("Content-Type") {
                let str = content_type.to_str()?;
                let arr: Vec<_> = str.split('/').collect();
                if arr.len() == 2 {
                    data_type = arr[1].to_string();
                }
            }
            resp.bytes().await?.into()
        }
        _ => base64::decode(params.data)?,
    };
    let original_size = result.len();
    let c = Cursor::new(result);
    let quality = params.quality;
    let speed = params.speed;
    let mut output_type = params.output_type.clone();

    let data = match data_type.as_str() {
        // gif单独处理
        "gif" => {
            output_type = "gif".to_string();
            image::to_gif(c, 10)?
        }
        _ => {
            let info = image::load(c, data_type)?;
            match params.output_type.as_str() {
                "png" => info.to_png(quality)?,
                "avif" => info.to_avif(quality, speed)?,
                "webp" => info.to_webp(quality)?,
                //
                _ => {
                    output_type = "jpeg".to_string();
                    info.to_mozjpeg(quality)?
                }
            }
        }
    };
    Ok(OptimResult {
        saving: 100 * data.len() / original_size,
        data,
        output_type,
    })
}

async fn optim_image_preview(
    Json(params): Json<OptimImageParams>,
) -> ResponseResult<image::ImagePreview> {
    let result = handle(params).await?;

    Ok(image::ImagePreview {
        data: result.data,
        image_type: result.output_type,
    })
}

async fn optim_image(
    Json(params): Json<OptimImageParams>,
) -> ResponseResult<Json<OptimImageResult>> {
    let result = handle(params).await?;
    Ok(Json(OptimImageResult {
        saving: result.saving,
        data: base64::encode(result.data),
        output_type: result.output_type,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OptimImageParams {
    data: String,
    data_type: String,
    output_type: String,
    quality: u8,
    speed: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OptimImageResult {
    data: String,
    output_type: String,
    saving: usize,
}

struct OptimResult {
    data: Vec<u8>,
    output_type: String,
    saving: usize,
}
