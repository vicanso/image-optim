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
use ctor::ctor;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::sync::Arc;
use tibba_error::Error;
use tibba_hook::{BoxFuture, Task, register_task};
use tibba_opendal::{Storage, new_opendal_storage, new_opendal_storage_from_url};
use tracing::info;

type Result<T> = std::result::Result<T, Error>;

static OPENDAL_STORAGE: OnceCell<Storage> = OnceCell::new();
static NAMED_STORAGES: OnceCell<HashMap<String, Storage>> = OnceCell::new();

/// 默认 OpenDAL 存储（来自 `IMOP__OPENDAL__URL`）。未初始化时 panic。
pub fn get_opendal_storage() -> &'static Storage {
    OPENDAL_STORAGE
        .get()
        .unwrap_or_else(|| panic!("opendal storage not initialized"))
}

/// 按名字查找命名 OpenDAL 存储（来自 `IMOP__OPENDAL__<NAME>__URL`）。
/// 名字大小写不敏感，未找到返回 `None`。
pub fn get_opendal_storage_by_name(name: &str) -> Option<&'static Storage> {
    NAMED_STORAGES
        .get()
        .and_then(|map| map.get(&name.to_lowercase()))
}

/// 扫描环境变量 `IMOP__OPENDAL__<NAME>__URL`，返回 `(小写名, URL)` 列表。
/// 排除裸 `IMOP__OPENDAL__URL`（默认存储，由 tibba-config 处理），名字段为空时跳过。
/// `__` 是层级分隔符，与 tibba-config 一致；`<NAME>` 中的单 `_` 保留为名字本身。
fn collect_named_source_envs() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (key, value) in std::env::vars() {
        if let Some(rest) = key.strip_prefix("IMOP__OPENDAL__")
            && let Some(name) = rest.strip_suffix("__URL")
            && !name.is_empty()
        {
            out.push((name.to_lowercase(), value));
        }
    }
    out
}

/// 用单个 URL 字符串构造一个 Storage。
fn build_storage_from_url(url: &str) -> Result<Storage> {
    // tibba-opendal 仅在 schema == "http" 时走 HTTP 后端；http(s):// URL 自动补上。
    let schema = if url.starts_with("http://") || url.starts_with("https://") {
        Some("http")
    } else {
        None
    };
    new_opendal_storage_from_url(url, schema).map_err(Error::new)
}

struct DalTask;

impl Task for DalTask {
    fn before(&self) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async {
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

            let mut named: HashMap<String, Storage> = HashMap::new();
            for (name, url) in collect_named_source_envs() {
                let storage = build_storage_from_url(&url)?;
                let info = storage.info();
                info!(
                    source = %name,
                    schema = ?info.scheme(),
                    full_capability = ?info.full_capability(),
                    "open dal named storage init success"
                );
                named.insert(name, storage);
            }
            NAMED_STORAGES
                .set(named)
                .map_err(|_| Error::new("set opendal named storages fail"))?;

            Ok(true)
        })
    }
    fn priority(&self) -> u8 {
        16
    }
}

#[ctor(unsafe)]
fn init() {
    register_task("dal", Arc::new(DalTask));
}
