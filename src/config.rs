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

use async_trait::async_trait;
use ctor::ctor;
use once_cell::sync::OnceCell;
use rust_embed::RustEmbed;
use std::sync::Arc;
use std::time::Duration;
use tibba_config::Config;
use tibba_error::Error;
use tibba_hook::{Task, register_task};
use tracing::info;
use validator::Validate;

type Result<T> = std::result::Result<T, Error>;
static CONFIGS: OnceCell<Config> = OnceCell::new();

fn map_error(err: impl ToString) -> Error {
    Error::new(err).with_category("config")
}

#[derive(RustEmbed)]
#[folder = "configs/"]
struct Configs;

// BasicConfig struct defines the basic application settings
// with validation rules for each field
#[derive(Debug, Clone, Default, Validate)]
pub struct BasicConfig {
    // listen address
    pub listen: String,
    // processing limit
    #[validate(range(min = 0, max = 100000))]
    pub processing_limit: i32,
    // timeout
    pub timeout: Duration,
    // secret
    pub secret: String,
    // prefix
    pub prefix: String,
    // commit id
    pub commit_id: String,
}

static BASIC_CONFIG: OnceCell<BasicConfig> = OnceCell::new();

/// Create a new basic config, if the config is invalid, it will panic
fn new_basic_config(config: &Config) -> Result<BasicConfig> {
    let timeout = config.get_duration("timeout", Duration::from_secs(60));
    let commit_id = if let Some(data) = Configs::get("commit_id.txt") {
        std::str::from_utf8(&data.data)
            .unwrap_or_default()
            .trim()
            .to_string()
    } else {
        "--".to_string()
    };
    let basic_config = BasicConfig {
        listen: config.get_str("listen", ""),
        processing_limit: config.get_int("processing_limit", 5000) as i32,
        timeout,
        secret: config.get_str("secret", ""),
        prefix: config.get_str("prefix", ""),
        commit_id,
    };
    basic_config.validate().map_err(map_error)?;
    Ok(basic_config)
}

fn new_config() -> Result<&'static Config> {
    CONFIGS.get_or_try_init(|| {
        let category = "config";
        let mut arr = vec![];
        for name in ["default.toml", &format!("{}.toml", tibba_util::get_env())] {
            let data = Configs::get(name)
                .ok_or(map_error(format!("{name} not found")))?
                .data;
            info!(category, "load config from {}", name);
            arr.push(std::str::from_utf8(&data).unwrap_or_default().to_string());
        }

        let config = tibba_config::Config::new(
            arr.iter().map(|s| s.as_str()).collect(),
            Some("IMAGE_OPTIM"),
        )?;
        Ok(config)
    })
}

pub fn must_get_basic_config() -> &'static BasicConfig {
    BASIC_CONFIG.get().unwrap()
}

async fn init_config() -> Result<()> {
    let app_config = new_config()?;
    let basic_config = new_basic_config(&app_config.sub_config("basic"))?;
    BASIC_CONFIG
        .set(basic_config)
        .map_err(|_| map_error("basic config init failed"))?;
    Ok(())
}

pub fn must_get_config() -> &'static Config {
    new_config().unwrap()
}

struct ConfigTask;

#[async_trait]
impl Task for ConfigTask {
    async fn before(&self) -> Result<bool> {
        init_config().await?;
        Ok(true)
    }
}

// add application init before application start
#[ctor]
fn init() {
    register_task("config", Arc::new(ConfigTask));
}
