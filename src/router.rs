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

use crate::image::new_image_router;
use crate::metrics;
use crate::playground::new_playground_router;
use crate::state::get_app_state;
use axum::Router;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tibba_error::Error;
use tibba_router_common::{CommonRouterParams, new_common_router};

type Result<T, E = Error> = std::result::Result<T, E>;

async fn metrics_handler() -> Response {
    let (content_type, body) = metrics::render();
    let mut res = body.into_response();
    if let Ok(value) = HeaderValue::from_str(&content_type) {
        res.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    res
}

/// tibba-router-common 0.3 replaced `/ping` with `/healthz` and `/readyz`.
/// Keep `/ping` so existing health checks (Dockerfile HEALTHCHECK, probes)
/// continue to work.
async fn ping() -> Result<&'static str> {
    if !get_app_state().is_running() {
        return Err(Error::new("Server is not running")
            .with_category("common_router")
            .with_status(503));
    }
    Ok("pong")
}

pub fn new_router() -> Result<Router> {
    let common_router = new_common_router(CommonRouterParams {
        state: get_app_state(),
        cache: None,
        readiness: None,
    });

    let router = Router::new()
        .nest("/images", new_image_router())
        .route("/metrics", get(metrics_handler))
        .route("/ping", get(ping))
        .merge(common_router);

    Ok(match new_playground_router() {
        Some(playground_router) => router.merge(playground_router),
        None => router,
    })
}
