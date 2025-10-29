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

use crate::config::must_get_config;
use async_trait::async_trait;
use ctor::ctor;
use once_cell::sync::OnceCell;
use std::sync::Arc;
use tibba_error::Error;
use tibba_hook::{Task, register_task};
use tibba_opendal::{Storage, new_opendal_storage};
use tracing::info;

type Result<T> = std::result::Result<T, Error>;

static OPENDAL_STORAGE: OnceCell<Storage> = OnceCell::new();

pub fn get_opendal_storage() -> &'static Storage {
    // init opendal storage is checked in init function
    OPENDAL_STORAGE
        .get()
        .unwrap_or_else(|| panic!("opendal storage not initialized"))
}

struct DalTask;

#[async_trait]
impl Task for DalTask {
    async fn before(&self) -> Result<bool> {
        let app_config = must_get_config();
        let storage = new_opendal_storage(&app_config.sub_config("opendal"))?;
        let info = storage.info();
        OPENDAL_STORAGE
            .set(storage)
            .map_err(|_| Error::new("set opendal storage fail"))?;

        info!(
            schema = ?info.scheme(),
            full_capability = ?info.full_capability(),
            "open dal storage init success"
        );
        Ok(true)
    }
    fn priority(&self) -> u8 {
        16
    }
}

#[ctor]
fn init() {
    register_task("dal", Arc::new(DalTask));
}
