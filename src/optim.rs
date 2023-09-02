use crate::error::HTTPError;
use crate::image_processing::{run, PROCESS_LOAD, PROCESS_OPTIM};
use crate::images;
use crate::response::ResponseResult;
use axum::extract::{Query, RawQuery};
use axum::routing::get;
use axum::{Json, Router};
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use urlencoding::decode;

pub fn new_router() -> Router {
    let optim_images = Router::new().route("/", get(optim_image_preview).post(optim_image));
    let pipe_line = Router::new()
        .route("/", get(pipeline_image))
        .route("/preview", get(pipeline_image_preview));

    Router::new()
        .nest("/optim-images", optim_images)
        .nest("/pipeline-images", pipe_line)
}

async fn handle(params: OptimImageParams) -> Result<OptimResult, HTTPError> {
    let desc = params.description();
    pipeline(desc).await
}

async fn pipeline(desc: Vec<Vec<String>>) -> Result<OptimResult, HTTPError> {
    let process_img = run(desc).await?;

    let data = process_img.get_buffer()?;
    let mut ratio = 0;
    if process_img.original_size > 0 {
        ratio = 100 * data.len() / process_img.original_size;
    }

    Ok(OptimResult {
        diff: process_img.diff,
        ratio,
        data,
        output_type: process_img.ext,
    })
}

async fn optim_image_preview(
    Query(params): Query<OptimImageParams>,
) -> ResponseResult<images::ImagePreview> {
    let result = handle(params).await?;

    Ok(images::ImagePreview {
        ratio: result.ratio,
        diff: result.diff,
        data: result.data,
        image_type: result.output_type,
    })
}

async fn optim_image(
    Json(params): Json<OptimImageParams>,
) -> ResponseResult<Json<OptimImageResult>> {
    let result = handle(params).await?;
    Ok(Json(OptimImageResult {
        diff: result.diff,
        ratio: result.ratio,
        data: general_purpose::STANDARD.encode(result.data),
        output_type: result.output_type,
    }))
}

fn convert_query_to_desc(query: Option<String>) -> Result<Vec<Vec<String>>, HTTPError> {
    let desc = query.ok_or_else(|| HTTPError::new("params is null", "validate"))?;
    let sep = "&";
    let arr = desc.split(sep);
    let mut result = Vec::new();
    for str in arr {
        let items: Vec<_> = str.split('=').collect();
        if items.len() != 2 {
            continue;
        }
        let value = decode(items[1])?.to_string();
        let mut params = vec![items[0].to_string()];
        for p in value.split('|') {
            params.push(p.to_string());
        }
        result.push(params);
    }
    Ok(result)
}

async fn pipeline_image(RawQuery(query): RawQuery) -> ResponseResult<Json<OptimImageResult>> {
    let desc = convert_query_to_desc(query)?;

    let result = pipeline(desc).await?;

    Ok(Json(OptimImageResult {
        diff: result.diff,
        ratio: result.ratio,
        data: general_purpose::STANDARD.encode(result.data),
        output_type: result.output_type,
    }))
}
async fn pipeline_image_preview(RawQuery(query): RawQuery) -> ResponseResult<images::ImagePreview> {
    let desc = convert_query_to_desc(query)?;

    let result = pipeline(desc).await?;
    Ok(images::ImagePreview {
        ratio: result.ratio,
        diff: result.diff,
        data: result.data,
        image_type: result.output_type,
    })
}

#[derive(Deserialize)]
struct OptimImageParams {
    data: String,
    data_type: Option<String>,
    output_type: Option<String>,
    quality: Option<u8>,
    speed: Option<u8>,
}
impl OptimImageParams {
    // to processing description string
    pub fn description(self) -> Vec<Vec<String>> {
        let load_process = vec![
            PROCESS_LOAD.to_string(),
            self.data,
            self.data_type.unwrap_or_default(),
        ];

        let quality = self.quality.unwrap_or(80);
        let speed = self.speed.unwrap_or(3);

        let optim_process = vec![
            PROCESS_OPTIM.to_string(),
            self.output_type.unwrap_or_default(),
            quality.to_string(),
            speed.to_string(),
        ];

        let arr = vec![load_process, optim_process];

        arr
    }
}

#[derive(Serialize)]
struct OptimImageResult {
    diff: f64,
    data: String,
    output_type: String,
    ratio: usize,
}

struct OptimResult {
    diff: f64,
    data: Vec<u8>,
    output_type: String,
    ratio: usize,
}
