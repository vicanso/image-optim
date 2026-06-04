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
use ::metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use once_cell::sync::OnceCell;
use tracing::error;

static HANDLE: OnceCell<PrometheusHandle> = OnceCell::new();

const NAME_TASK_DURATION: &str = "image_optim_task_duration_seconds";
const NAME_INPUT_BYTES: &str = "image_optim_input_bytes";
const NAME_OUTPUT_BYTES: &str = "image_optim_output_bytes";
const NAME_DSSIM_DIFF: &str = "image_optim_dssim_diff";
const NAME_DECODE_REJECTED: &str = "image_optim_decode_rejected_total";
const NAME_PATH_REJECTED: &str = "image_optim_path_rejected_total";
const NAME_ERRORS_TOTAL: &str = "image_optim_errors_total";
const NAME_PROCESS_MEMORY_MB: &str = "image_optim_process_memory_mb";
const NAME_PROCESS_CPU_PERCENT: &str = "image_optim_process_cpu_percent";
const NAME_PROCESS_OPEN_FILES: &str = "image_optim_process_open_files";
const NAME_BUILD_INFO: &str = "image_optim_build_info";

const DURATION_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];
const BYTES_BUCKETS: &[f64] = &[
    16_384.0,
    65_536.0,
    262_144.0,
    1_048_576.0,
    4_194_304.0,
    16_777_216.0,
    33_554_432.0,
];
const DSSIM_BUCKETS: &[f64] = &[0.0001, 0.0005, 0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1];

/// Install the global Prometheus recorder with explicit per-metric buckets
/// so histograms render as `histogram` (with `_bucket`/`_sum`/`_count`),
/// not as `summary` (the default for unbucketed distributions).
///
/// Must be called once at process startup, after the basic config is loaded.
pub fn init() {
    let builder = PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full(NAME_TASK_DURATION.to_string()),
            DURATION_BUCKETS,
        )
        .and_then(|b| {
            b.set_buckets_for_metric(Matcher::Suffix("_bytes".to_string()), BYTES_BUCKETS)
        })
        .and_then(|b| {
            b.set_buckets_for_metric(Matcher::Full(NAME_DSSIM_DIFF.to_string()), DSSIM_BUCKETS)
        });

    let builder = match builder {
        Ok(b) => b,
        Err(e) => {
            error!(category = "metrics_init", "bucket matcher failed: {e}");
            return;
        }
    };

    let handle = match builder.install_recorder() {
        Ok(h) => h,
        Err(e) => {
            error!(category = "metrics_init", "install_recorder failed: {e}");
            return;
        }
    };
    let _ = HANDLE.set(handle);

    describe_histogram!(
        NAME_TASK_DURATION,
        "Wall-clock seconds spent running a full image task (cache misses only)."
    );
    describe_histogram!(
        NAME_INPUT_BYTES,
        "Source image bytes downloaded from storage before decoding."
    );
    describe_histogram!(NAME_OUTPUT_BYTES, "Output image bytes after encoding.");
    describe_histogram!(
        NAME_DSSIM_DIFF,
        "DSSIM between original and re-encoded output (pure optim only)."
    );
    describe_counter!(
        NAME_DECODE_REJECTED,
        "Requests rejected before encoding by the decode-size guards."
    );
    describe_counter!(
        NAME_PATH_REJECTED,
        "Storage paths rejected by the path-traversal guard."
    );
    describe_counter!(
        NAME_ERRORS_TOTAL,
        "Image task errors classified by error category."
    );
    describe_gauge!(
        NAME_PROCESS_MEMORY_MB,
        "RSS memory in MiB (sampled every 60s)."
    );
    describe_gauge!(NAME_PROCESS_CPU_PERCENT, "CPU percent (sampled every 60s).");
    describe_gauge!(
        NAME_PROCESS_OPEN_FILES,
        "Open file descriptors (sampled every 60s)."
    );
    describe_gauge!(NAME_BUILD_INFO, "Constant 1 with `commit` label.");

    let commit = must_get_basic_config().commit_id.clone();
    gauge!(NAME_BUILD_INFO, "commit" => commit).set(1.0);
}

/// Intern output_format label values to `&'static str` so the metrics
/// facade stores a cheap `Cow::Borrowed` instead of an owned String per
/// request. Bounds label cardinality to the set imageoptimize can actually
/// emit (see `OptimProcess::process`); any future format slips into
/// `"other"` rather than producing a new Prometheus time-series.
fn intern_format(f: &str) -> &'static str {
    match f {
        "jpg" | "jpeg" => "jpeg",
        "png" => "png",
        "webp" => "webp",
        "avif" => "avif",
        "jxl" => "jxl",
        "gif" => "gif",
        _ => "other",
    }
}

/// Intern error-category label values to `&'static str`. Error::category is
/// a String (from tibba-error), so without interning a malicious upstream
/// or transitive lib could explode label cardinality in the in-process
/// metric registry. Buckets unknown values into `"other"`, empty into
/// `"unknown"`.
fn intern_error_category(c: &str) -> &'static str {
    match c {
        "" => "unknown",
        "imageoptimize" => "imageoptimize",
        "decode_guard" => "decode_guard",
        "path_guard" => "path_guard",
        "open_dal" => "open_dal",
        "blocking_join" => "blocking_join",
        "exception" => "exception",
        "timeout" => "timeout",
        "guard" => "guard",
        "config" => "config",
        "preset" => "preset",
        _ => "other",
    }
}

pub fn record_input_bytes(n: u64) {
    histogram!(NAME_INPUT_BYTES).record(n as f64);
}

pub fn record_output_bytes(format: &str, n: u64) {
    histogram!(NAME_OUTPUT_BYTES, "output_format" => intern_format(format)).record(n as f64);
}

pub fn record_task_duration(format: &str, secs: f64) {
    histogram!(NAME_TASK_DURATION, "output_format" => intern_format(format)).record(secs);
}

pub fn record_dssim(format: &str, diff: f64) {
    if diff >= 0.0 {
        histogram!(NAME_DSSIM_DIFF, "output_format" => intern_format(format)).record(diff);
    }
}

pub fn inc_decode_rejected(reason: &'static str) {
    counter!(NAME_DECODE_REJECTED, "reason" => reason).increment(1);
}

pub fn inc_path_rejected(reason: &'static str) {
    counter!(NAME_PATH_REJECTED, "reason" => reason).increment(1);
}

pub fn inc_errors(category: &str) {
    counter!(NAME_ERRORS_TOTAL, "category" => intern_error_category(category)).increment(1);
}

pub fn set_process_metrics(memory_mb: i64, cpu_percent: i64, open_files: i64) {
    gauge!(NAME_PROCESS_MEMORY_MB).set(memory_mb as f64);
    gauge!(NAME_PROCESS_CPU_PERCENT).set(cpu_percent as f64);
    gauge!(NAME_PROCESS_OPEN_FILES).set(open_files as f64);
}

/// Encode the current registry as Prometheus text exposition.
/// Returns (content_type, body). Empty body if init() failed or was skipped.
pub fn render() -> (String, String) {
    let body = HANDLE.get().map(|h| h.render()).unwrap_or_default();
    ("text/plain; version=0.0.4".to_string(), body)
}
