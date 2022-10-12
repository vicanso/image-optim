use crate::error::HTTPError;
use crate::image;
use crate::image_processing::{LoaderProcess, OptimProcess, Process, ProcessImage};
use crate::response::ResponseResult;
use axum::{
    extract::Query,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

pub fn new_router() -> Router {
    let r = Router::new();

    r.route("/optim-images/preview", get(optim_image_preview))
        .route("/optim-images", post(optim_image))
}

async fn handle(params: OptimImageParams) -> Result<OptimResult, HTTPError> {
    let process_img = LoaderProcess::new(params.data.clone(), params.data_type.clone())
        .process(ProcessImage::new())
        .await?;

    let original_data = process_img.buffer.clone();
    let original_size = original_data.len();
    let process_img = OptimProcess::new(params.output_type, params.quality, params.speed)
        .process(process_img)
        .await?;

    let data = process_img.buffer;

    Ok(OptimResult {
        saving: 100 * data.len() / original_size,
        data,
        output_type: process_img.ext,
    })
}

async fn optim_image_preview(
    Query(params): Query<OptimImageParams>,
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
