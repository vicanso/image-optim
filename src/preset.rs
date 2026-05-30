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

use ctor::ctor;
use once_cell::sync::OnceCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tibba_error::Error;
use tibba_hook::{BoxFuture, Task, register_task};
use tracing::{info, warn};

type Result<T> = std::result::Result<T, Error>;

pub const SUPPORTED_OPS: &[&str] = &["optim", "resize", "fit", "watermark", "crop", "padding"];

#[derive(Debug, Clone)]
pub struct Preset {
    pub op: String,
    pub params: BTreeMap<String, String>,
}

static PRESETS: OnceCell<HashMap<String, Preset>> = OnceCell::new();

/// 按名字查找预设（大小写不敏感）。
pub fn get_preset(name: &str) -> Option<&'static Preset> {
    PRESETS.get().and_then(|m| m.get(&name.to_lowercase()))
}

/// 解析单条 `IMOP_PRESET_<NAME>=<op>&k=v&k=v...` 的值字符串。
///
/// 第一个分段必须是无 `=` 的操作名（如 `fit`）；其余分段为 `key=value` 对。
/// 重复键以最后一个为准。空键、空段忽略。
fn parse_preset_value(raw: &str) -> Result<Preset> {
    let mut op: Option<String> = None;
    let mut params: BTreeMap<String, String> = BTreeMap::new();

    for segment in raw.split('&') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        match segment.split_once('=') {
            None => {
                if op.is_some() {
                    return Err(Error::new(format!(
                        "preset value has multiple bare segments (op already set to {:?}, got {:?})",
                        op, segment
                    )));
                }
                op = Some(segment.to_lowercase());
            }
            Some((k, v)) => {
                let k = k.trim();
                if k.is_empty() {
                    continue;
                }
                params.insert(k.to_lowercase(), v.trim().to_string());
            }
        }
    }

    let op = op.ok_or_else(|| {
        Error::new("preset value missing op (expected `<op>&key=value&...`)".to_string())
    })?;

    if !SUPPORTED_OPS.contains(&op.as_str()) {
        return Err(Error::new(format!(
            "preset op {:?} not in supported set {:?}",
            op, SUPPORTED_OPS
        )));
    }

    Ok(Preset { op, params })
}

/// 扫描环境变量构造预设注册表。无效条目记录 warn，不阻断启动。
fn build_registry() -> HashMap<String, Preset> {
    let mut out: HashMap<String, Preset> = HashMap::new();
    for (key, value) in std::env::vars() {
        let Some(name) = key.strip_prefix("IMOP_PRESET_") else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let name = name.to_lowercase();
        match parse_preset_value(&value) {
            Ok(preset) => {
                info!(
                    name = %name,
                    op = %preset.op,
                    params = ?preset.params,
                    "preset registered"
                );
                out.insert(name, preset);
            }
            Err(err) => {
                warn!(
                    name = %name,
                    raw = %value,
                    error = %err,
                    "skip invalid preset"
                );
            }
        }
    }
    out
}

struct PresetTask;

impl Task for PresetTask {
    fn before(&self) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async {
            let registry = build_registry();
            let count = registry.len();
            PRESETS
                .set(registry)
                .map_err(|_| Error::new("set preset registry fail"))?;
            info!(count, "preset registry init success");
            Ok(true)
        })
    }
    fn priority(&self) -> u8 {
        16
    }
}

#[ctor(unsafe)]
fn init() {
    register_task("preset", Arc::new(PresetTask));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_op_and_params() {
        let p = parse_preset_value("fit&width=300&quality=70").expect("parse");
        assert_eq!(p.op, "fit");
        assert_eq!(p.params.get("width"), Some(&"300".to_string()));
        assert_eq!(p.params.get("quality"), Some(&"70".to_string()));
    }

    #[test]
    fn rejects_missing_op() {
        let err = parse_preset_value("width=300").expect_err("should fail");
        assert!(err.to_string().contains("missing op"));
    }

    #[test]
    fn rejects_unknown_op() {
        let err = parse_preset_value("teleport&width=300").expect_err("should fail");
        assert!(err.to_string().contains("not in supported set"));
    }

    #[test]
    fn rejects_two_bare_segments() {
        let err = parse_preset_value("fit&resize&width=300").expect_err("should fail");
        assert!(err.to_string().contains("multiple bare segments"));
    }

    #[test]
    fn key_lowercased_value_preserved() {
        let p = parse_preset_value("padding&Color=%23ffffff&Width=1000").expect("parse");
        assert_eq!(p.op, "padding");
        assert_eq!(p.params.get("color"), Some(&"%23ffffff".to_string()));
        assert_eq!(p.params.get("width"), Some(&"1000".to_string()));
    }
}
