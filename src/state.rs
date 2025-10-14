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

use super::config::must_get_basic_config;
use async_trait::async_trait;
use ctor::ctor;
use once_cell::sync::{Lazy, OnceCell};
use std::sync::Arc;
use std::time::Duration;
use tibba_error::Error;
use tibba_hook::{Task, register_task};
use tibba_performance::get_process_system_info;
use tibba_scheduler::{Job, register_job_task};
use tibba_state::AppState;
use tibba_util::is_production;
use tokio::sync::RwLock;
use tracing::info;

type Result<T> = std::result::Result<T, Error>;

static STATE: OnceCell<AppState> = OnceCell::new();

#[derive(Debug, Default)]
struct Performance {
    refresh_count: u32,
    memory_usage_mb: u32,
    cpu_usage: u16,
    cpu_time: u64,
    open_files: usize,
    written_mb: u32,
    read_mb: u32,
}
static PERFORMANCE: Lazy<RwLock<Performance>> = Lazy::new(|| RwLock::new(Performance::default()));

pub fn get_app_state() -> &'static AppState {
    STATE.get_or_init(|| {
        let basic_config = must_get_basic_config();
        AppState::new(
            basic_config.processing_limit,
            basic_config.commit_id.clone(),
        )
    })
}

async fn update_performance() {
    let pid = std::process::id() as usize;
    let process_system_info = get_process_system_info(pid);

    let mb = 1024 * 1024;
    let mut data = PERFORMANCE.write().await;
    data.refresh_count += 1;
    data.memory_usage_mb = (process_system_info.memory_usage / mb) as u32;
    data.cpu_usage = process_system_info.cpu_usage as u16;
    data.cpu_time = process_system_info.cpu_time - data.cpu_time;
    data.open_files = process_system_info.open_files.unwrap_or(0);
    data.written_mb = (process_system_info.written_bytes / mb) as u32;
    data.read_mb = (process_system_info.read_bytes / mb) as u32;
    info!(
        category = "application_performance",
        memory_usage = data.memory_usage_mb,
        cpu_usage = data.cpu_usage,
        cpu_time = data.cpu_time,
        open_files = data.open_files,
        written_mb = data.written_mb,
        read_mb = data.read_mb,
    );
}

struct StateTask;
#[async_trait]
impl Task for StateTask {
    async fn before(&self) -> Result<bool> {
        let job = Job::new_repeated_async(Duration::from_secs(60), move |_, _| {
            Box::pin(update_performance())
        })
        .map_err(Error::new)?;
        register_job_task("application_performance", job);
        Ok(true)
    }
    fn priority(&self) -> u8 {
        u8::MAX
    }
}

struct StopAppTask;
#[async_trait]
impl Task for StopAppTask {
    async fn after(&self) -> Result<bool> {
        if !is_production() {
            return Ok(false);
        }
        // set flag --> wait x seconds
        get_app_state().stop();
        tokio::time::sleep(Duration::from_secs(10)).await;
        Ok(true)
    }
}

#[ctor]
fn init() {
    register_task("state", Arc::new(StateTask));
    register_task("stop_app", Arc::new(StopAppTask));
}
