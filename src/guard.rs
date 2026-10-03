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
use crate::metrics;
use crate::playground;
use ctor::ctor;
use once_cell::sync::OnceCell;
use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::env;
use std::fmt;
use std::sync::Arc;
use tibba_error::Error;
use tibba_runtime::{BoxFuture, Task, register_task};
use tracing::{info, warn};

type Result<T> = std::result::Result<T, Error>;

/// Wrapper that accepts either a TOML sequence of strings or a single CSV
/// string (so env vars like `IMOP__GUARD__DEFAULT_PREFIX_ALLOWLIST=a/,b/`
/// deserialize the same as `default_prefix_allowlist = ["a/","b/"]` in TOML).
#[derive(Default, Debug, Clone)]
pub struct StringOrVec(pub Vec<String>);

impl<'de> Deserialize<'de> for StringOrVec {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = StringOrVec;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a list of strings or a comma-separated string")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> std::result::Result<StringOrVec, E> {
                Ok(StringOrVec(
                    s.split(',')
                        .map(|x| x.trim().to_string())
                        .filter(|x| !x.is_empty())
                        .collect(),
                ))
            }
            fn visit_string<E: de::Error>(self, s: String) -> std::result::Result<StringOrVec, E> {
                self.visit_str(&s)
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<StringOrVec, A::Error> {
                let mut out = Vec::new();
                while let Some(s) = seq.next_element::<String>()? {
                    out.push(s);
                }
                Ok(StringOrVec(out))
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(Deserialize, Default, Debug)]
pub struct GuardConfig {
    /// Applies to the default storage AND to any named storage that has no
    /// explicit entry in `source_prefix_allowlist`. Empty = unrestricted.
    #[serde(default)]
    pub default_prefix_allowlist: StringOrVec,
    /// Explicit per-named-storage overrides. Map key is the source name
    /// (lowercased). Explicit empty list = unrestricted for that source.
    #[serde(default)]
    pub source_prefix_allowlist: HashMap<String, StringOrVec>,
}

#[derive(Debug)]
pub struct GuardRegistry {
    default: Vec<String>,
    by_source: HashMap<String, Vec<String>>,
}

static REGISTRY: OnceCell<GuardRegistry> = OnceCell::new();

/// Normalize a raw prefix list: drop empties, trim whitespace, auto-append a
/// trailing `/` (with a warn so misconfigured prefixes are loud at startup).
fn normalize(raw: Vec<String>) -> Vec<String> {
    raw.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.ends_with('/') {
                s
            } else {
                warn!(
                    category = "guard",
                    prefix = %s,
                    "prefix missing trailing '/'; auto-appending"
                );
                format!("{s}/")
            }
        })
        .collect()
}

fn build_registry(cfg: GuardConfig) -> GuardRegistry {
    let default = normalize(cfg.default_prefix_allowlist.0);
    let by_source: HashMap<String, Vec<String>> = cfg
        .source_prefix_allowlist
        .into_iter()
        .map(|(name, list)| (name.to_lowercase(), normalize(list.0)))
        .collect();
    GuardRegistry { default, by_source }
}

const SOURCE_ALLOWLIST_ENV_PREFIX: &str = "IMOP__GUARD__SOURCE_PREFIX_ALLOWLIST__";

/// Lowercased names of the sources whose
/// `IMOP__GUARD__SOURCE_PREFIX_ALLOWLIST__<NAME>` is set to an empty string.
/// tibba-config ignores empty env values, so this documented "explicitly
/// unrestricted" opt-out never reaches `GuardConfig` and has to be picked up
/// from the environment directly.
fn unrestricted_sources(vars: impl Iterator<Item = (String, String)>) -> Vec<String> {
    vars.filter(|(_, value)| value.is_empty())
        .filter_map(|(key, _)| {
            key.strip_prefix(SOURCE_ALLOWLIST_ENV_PREFIX)
                .filter(|name| !name.is_empty())
                .map(str::to_lowercase)
        })
        .collect()
}

fn load() -> Result<()> {
    let app_config = must_get_config();
    let cfg = app_config
        .sub_config("guard")
        .try_deserialize::<GuardConfig>()
        .unwrap_or_default();
    let mut registry = build_registry(cfg);
    for name in unrestricted_sources(env::vars()) {
        registry.by_source.insert(name, Vec::new());
    }
    // playground uploads are named by the server and live in their own
    // directory, the allowlist of the real storages must not apply to them
    if playground::is_enabled() {
        registry
            .by_source
            .insert(playground::SOURCE.to_string(), Vec::new());
    }
    info!(
        category = "guard",
        default_prefixes = ?registry.default,
        source_overrides = ?registry.by_source.keys().collect::<Vec<_>>(),
        "prefix allowlist initialized"
    );
    REGISTRY
        .set(registry)
        .map_err(|_| Error::new("guard registry already initialized").with_category("guard"))?;
    Ok(())
}

/// Enforce the configured prefix allowlist for `(source, path)`.
///
/// Lookup rule: explicit `source_prefix_allowlist[name]` if present, otherwise
/// fall back to `default_prefix_allowlist`. Empty list = unrestricted.
/// `path` must already be sanitised (no `..`, no absolute, no backslash) —
/// allowlist matching assumes a clean relative path.
pub fn enforce_prefix(source: Option<&str>, path: &str) -> Result<()> {
    let registry = match REGISTRY.get() {
        Some(r) => r,
        // Init runs as a before-task, so a missing registry only happens in
        // unit tests where guard::load() isn't invoked. Treat as unrestricted
        // there instead of blanket-rejecting all requests.
        None => return Ok(()),
    };

    let key = source.map(|s| s.to_lowercase());
    let list = match key.as_deref() {
        Some(k) => registry.by_source.get(k).unwrap_or(&registry.default),
        None => &registry.default,
    };
    if list.is_empty() {
        return Ok(());
    }
    if list.iter().any(|p| path.starts_with(p.as_str())) {
        return Ok(());
    }
    metrics::inc_path_rejected("not_in_allowlist");
    Err(Error::new(format!(
        "path not in allowlist (source={}): {path}",
        source.unwrap_or("default"),
    ))
    .with_category("path_guard")
    .with_status(400))
}

struct GuardTask;
impl Task for GuardTask {
    fn before(&self) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async {
            load()?;
            Ok(true)
        })
    }
    fn priority(&self) -> u8 {
        16
    }
}

#[ctor(unsafe)]
fn init() {
    register_task("guard", Arc::new(GuardTask));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_trims_and_appends_slash() {
        let out = normalize(vec![
            "users".into(),
            " thumbs/ ".into(),
            "".into(),
            "  ".into(),
            "logos/".into(),
        ]);
        assert_eq!(out, vec!["users/", "thumbs/", "logos/"]);
    }

    #[test]
    fn unrestricted_sources_only_picks_empty_overrides() {
        let vars = [
            ("IMOP__GUARD__SOURCE_PREFIX_ALLOWLIST__USERS", ""),
            ("IMOP__GUARD__SOURCE_PREFIX_ALLOWLIST__THUMBS", "sm/,md/"),
            ("IMOP__GUARD__SOURCE_PREFIX_ALLOWLIST__", ""),
            ("IMOP__GUARD__DEFAULT_PREFIX_ALLOWLIST", ""),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()));
        assert_eq!(unrestricted_sources(vars), vec!["users"]);
    }

    #[test]
    fn prefix_match_respects_directory_boundary() {
        let list = vec!["users/".to_string()];
        assert!(list.iter().any(|p| "users/a.png".starts_with(p.as_str())));
        assert!(!list.iter().any(|p| "usersx/a.png".starts_with(p.as_str())));
    }
}
