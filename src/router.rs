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

use crate::config::must_get_basic_config;
use crate::dal::get_opendal_storage;
use crate::image::ImagePreview;
use crate::state::get_app_state;
use axum::Router;
use axum::routing::get;
use imageoptimize::{PROCESS_DIFF, PROCESS_OPTIM, ProcessImage, run_with_image};
use serde::Deserialize;
use tibba_error::Error;
use tibba_router_common::{CommonRouterParams, new_common_router};
use tibba_util::QueryParams;
use validator::{Validate, ValidationError};

type Result<T, E = Error> = std::result::Result<T, E>;

fn x_output_type(output_type: &str) -> Result<(), ValidationError> {
    if ["jepg", "jpg", "png", "webp", "avif"].contains(&output_type) {
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

async fn optim(QueryParams(params): QueryParams<OptimParams>) -> Result<ImagePreview> {
    let ext = params.file.split('.').next_back().unwrap_or("jpeg");
    let output_type = params.output_type.unwrap_or(ext.to_string());
    let quality = params.quality.unwrap_or(80);
    let buffer = get_opendal_storage().read(&params.file).await?;
    let img = ProcessImage::new(buffer.to_vec(), ext).map_err(map_err)?;
    let img = run_with_image(
        img,
        vec![
            vec![
                PROCESS_OPTIM.to_string(),
                output_type,
                quality.to_string(),
                "3".to_string(),
            ],
            vec![PROCESS_DIFF.to_string()],
        ],
    )
    .await
    .map_err(map_err)?;
    let buffer = img.get_buffer().map_err(map_err)?;
    let ratio = 100 * buffer.len() / img.original_size;

    Ok(ImagePreview {
        diff: img.diff,
        ratio,
        data: buffer,
        image_type: ext.to_string(),
    })
}

pub fn new_router() -> Result<Router> {
    let basic_config = must_get_basic_config();
    let common_router = new_common_router(CommonRouterParams {
        state: get_app_state(),
        secret: basic_config.secret.clone(),
        cache: None,
        commit_id: basic_config.commit_id.clone(),
    });

    Ok(Router::new()
        .route("/image-optim", get(optim))
        .merge(common_router))
}
