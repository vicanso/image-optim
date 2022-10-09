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

async fn handle(params: OptimImageParams) -> Result<(Vec<u8>, String), HTTPError> {
    let result = base64::decode(params.data)?;
    let c = Cursor::new(result);
    let quality = params.quality;
    let speed = params.speed;
    let mut output_type = params.output_type.clone();

    let data = match params.data_type.as_str() {
        // gif单独处理
        "gif" => {
            output_type = "gif".to_string();
            image::to_gif(c, 10)?
        }
        _ => {
            let info = image::load(c, params.data_type)?;
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
    Ok((data, output_type))
}

async fn optim_image_preview(
    Json(params): Json<OptimImageParams>,
) -> ResponseResult<image::ImagePreview> {
    let (data, output_type) = handle(params).await?;

    Ok(image::ImagePreview {
        data,
        image_type: output_type,
    })
}

async fn optim_image(
    Json(params): Json<OptimImageParams>,
) -> ResponseResult<Json<OptimImageResult>> {
    let (data, output_type) = handle(params).await?;
    Ok(Json(OptimImageResult {
        data: base64::encode(data),
        output_type,
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
}
