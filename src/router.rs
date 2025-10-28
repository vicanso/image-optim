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
use crate::image::new_image_router;
use crate::state::get_app_state;
use axum::Router;
use tibba_error::Error;
use tibba_router_common::{CommonRouterParams, new_common_router};

type Result<T, E = Error> = std::result::Result<T, E>;

pub fn new_router() -> Result<Router> {
    let basic_config = must_get_basic_config();
    let common_router = new_common_router(CommonRouterParams {
        state: get_app_state(),
        cache: None,
        commit_id: basic_config.commit_id.clone(),
    });

    Ok(Router::new()
        .nest("/images", new_image_router())
        .merge(common_router))
}
