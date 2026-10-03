// Copyright 2026 Tree xie.
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

//! Web playground: upload an image into a temp directory and try the image
//! APIs on it from the browser.
//!
//! Uploads are exposed to the regular `/images/*` endpoints through a reserved
//! named storage ([`SOURCE`]), so the page exercises exactly the same code
//! path as a production call. Files are named by the server, capped in size,
//! and removed by a scheduled job once they are older than `playground.ttl`.

use crate::config::{must_get_basic_config, must_get_config};
use axum::Json;
use axum::Router;
use axum::extract::multipart::MultipartError;
use axum::extract::{DefaultBodyLimit, Multipart};
use axum::http::{HeaderName, HeaderValue, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use bytes::Bytes;
use ctor::ctor;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::env;
use std::io;
use std::path::{self, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tibba_config::humantime_serde;
use tibba_error::Error;
use tibba_runtime::{BoxFuture, Job, Task, register_job_task, register_task};
use tibba_util::uuid;
use tokio::fs;
use tracing::{info, warn};

type Result<T, E = Error> = std::result::Result<T, E>;

/// Named storage that serves the uploads (`?source=playground`). Reserved:
/// while the playground is enabled it replaces an
/// `IMOP__OPENDAL__PLAYGROUND__URL` storage of the same name.
pub const SOURCE: &str = "playground";

const CATEGORY: &str = "playground";
const DEFAULT_DIR_NAME: &str = "image-optim-playground";
/// Every upload is stored as `pg-<uuid>.<ext>`. Cleanup and the size cap only
/// ever look at files matching this shape, so pointing `playground.dir` at a
/// directory that holds other files never deletes or counts them.
const FILE_PREFIX: &str = "pg-";
const EXTENSIONS: [&str; 6] = ["jpeg", "png", "gif", "webp", "avif", "jxl"];
/// Room for the multipart boundaries and headers around the file itself.
const MULTIPART_OVERHEAD: u64 = 64 * 1024;

const PAGE: &str = include_str!("../web/playground.html");
const PAGE_CONFIG_PLACEHOLDER: &str = "__PLAYGROUND_CONFIG__";
/// Mona Sans variable font (wdth + wght axes), subset to Latin and digits.
/// Copyright (c) 2023 GitHub, SIL Open Font License 1.1 (web/mona-sans-OFL.txt).
const FONT: &[u8] = include_bytes!("../web/mona-sans.woff2");
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; img-src 'self' blob: data:; \
     style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'; \
     base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

fn default_enabled() -> bool {
    true
}

fn default_max_upload_bytes() -> u64 {
    10 * 1024 * 1024
}

fn default_max_total_bytes() -> u64 {
    512 * 1024 * 1024
}

fn default_ttl() -> Duration {
    Duration::from_secs(3600)
}

#[derive(Debug, Clone, Deserialize)]
struct PlaygroundConfig {
    #[serde(default = "default_enabled")]
    enabled: bool,
    /// Upload directory. Empty = `<system temp dir>/image-optim-playground`.
    #[serde(default)]
    dir: String,
    /// Reject a single upload larger than this.
    #[serde(default = "default_max_upload_bytes")]
    max_upload_bytes: u64,
    /// Refuse uploads once the stored uploads add up to this many bytes.
    #[serde(default = "default_max_total_bytes")]
    max_total_bytes: u64,
    /// Uploads older than this are deleted by the cleanup job.
    #[serde(default = "default_ttl", with = "humantime_serde")]
    ttl: Duration,
}

struct Playground {
    dir: PathBuf,
    max_upload_bytes: u64,
    max_total_bytes: u64,
    ttl: Duration,
}

static PLAYGROUND: OnceCell<Playground> = OnceCell::new();
static PAGE_HTML: OnceCell<String> = OnceCell::new();

fn new_error(status: u16, message: impl ToString) -> Error {
    Error::new(message)
        .with_category(CATEGORY)
        .with_status(status)
}

fn io_error(err: io::Error) -> Error {
    new_error(500, err).with_exception(true)
}

fn multipart_error(err: MultipartError) -> Error {
    new_error(err.status().as_u16(), err.body_text())
}

/// `file://` URL of the upload directory for the [`SOURCE`] named storage;
/// `None` when the playground is disabled.
pub fn storage_url() -> Option<String> {
    PLAYGROUND
        .get()
        .map(|playground| format!("file://{}", playground.dir.display()))
}

pub fn is_enabled() -> bool {
    PLAYGROUND.get().is_some()
}

fn is_avif(data: &[u8]) -> bool {
    if data.get(4..8) != Some(b"ftyp".as_slice()) {
        return false;
    }
    // ftyp box: size(4) "ftyp" major_brand(4) minor_version(4) compatible_brands(4)*
    let size = data
        .get(..4)
        .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
        .map_or(0, u32::from_be_bytes) as usize;
    data.get(8..size.min(data.len())).is_some_and(|brands| {
        brands
            .as_chunks::<4>()
            .0
            .iter()
            .any(|brand| brand == b"avif" || brand == b"avis")
    })
}

/// Identify the image format from its magic bytes. The client-supplied file
/// name and content type are never trusted, so a non-image upload is rejected
/// here instead of being stored under an image extension.
fn sniff_ext(data: &[u8]) -> Option<&'static str> {
    const SIGNATURES: [(&[u8], &str); 6] = [
        (b"\xff\xd8\xff", "jpeg"),
        (b"\x89PNG\r\n\x1a\n", "png"),
        (b"GIF87a", "gif"),
        (b"GIF89a", "gif"),
        (b"\xff\x0a", "jxl"),
        (b"\x00\x00\x00\x0cJXL \r\n\x87\n", "jxl"),
    ];
    if let Some((_, ext)) = SIGNATURES
        .iter()
        .find(|(signature, _)| data.starts_with(signature))
    {
        return Some(ext);
    }
    if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP".as_slice()) {
        return Some("webp");
    }
    is_avif(data).then_some("avif")
}

fn is_upload_name(name: &str) -> bool {
    name.strip_prefix(FILE_PREFIX)
        .and_then(|rest| rest.rsplit_once('.'))
        .is_some_and(|(_, ext)| EXTENSIONS.contains(&ext))
}

struct Upload {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
}

async fn list_uploads(dir: &Path) -> io::Result<Vec<Upload>> {
    let mut entries = fs::read_dir(dir).await?;
    let mut uploads = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        if !entry.file_name().to_str().is_some_and(is_upload_name) {
            continue;
        }
        // metadata fails when the file was removed since the listing
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        if meta.is_file() {
            uploads.push(Upload {
                path: entry.path(),
                size: meta.len(),
                modified: meta.modified().ok(),
            });
        }
    }
    Ok(uploads)
}

/// Delete uploads older than `ttl`; returns how many files were removed.
async fn cleanup(dir: &Path, ttl: Duration) -> io::Result<usize> {
    let now = SystemTime::now();
    let mut removed = 0;
    for upload in list_uploads(dir).await? {
        let expired = upload
            .modified
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= ttl);
        if expired && fs::remove_file(&upload.path).await.is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

async fn run_cleanup() {
    let Some(playground) = PLAYGROUND.get() else {
        return;
    };
    match cleanup(&playground.dir, playground.ttl).await {
        Ok(0) => {}
        Ok(removed) => info!(category = CATEGORY, removed, "expired uploads removed"),
        Err(err) => warn!(category = CATEGORY, error = %err, "cleanup uploads failed"),
    }
}

/// Run often enough that an upload outlives its ttl by at most a quarter of
/// it, without scanning the directory more than every 10 seconds.
fn cleanup_interval(ttl: Duration) -> Duration {
    (ttl / 4).clamp(Duration::from_secs(10), Duration::from_secs(300))
}

async fn read_file_field(multipart: &mut Multipart) -> Result<Bytes> {
    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        if field.name() == Some("file") {
            return field.bytes().await.map_err(multipart_error);
        }
    }
    Err(new_error(400, "missing `file` field"))
}

#[derive(Serialize)]
struct UploadResponse {
    /// Storage path of the upload, to be used as `file=<file>&source=<source>`.
    file: String,
    source: &'static str,
    ext: &'static str,
    size: u64,
    /// Seconds until the cleanup job may delete the file.
    expires_in: u64,
}

async fn upload(mut multipart: Multipart) -> Result<Json<UploadResponse>> {
    let playground = PLAYGROUND
        .get()
        .ok_or_else(|| new_error(404, "playground is disabled"))?;
    let data = read_file_field(&mut multipart).await?;
    let size = data.len() as u64;
    if size == 0 {
        return Err(new_error(400, "file is empty"));
    }
    if size > playground.max_upload_bytes {
        return Err(new_error(
            413,
            format!(
                "file too large: {size} bytes > limit {} bytes",
                playground.max_upload_bytes
            ),
        ));
    }
    let ext = sniff_ext(&data).ok_or_else(|| {
        new_error(
            415,
            format!("unsupported image type, expected {}", EXTENSIONS.join("/")),
        )
    })?;

    let stored: u64 = list_uploads(&playground.dir)
        .await
        .map_err(io_error)?
        .iter()
        .map(|upload| upload.size)
        .sum();
    if stored.saturating_add(size) > playground.max_total_bytes {
        return Err(new_error(
            507,
            "playground storage is full, try again later",
        ));
    }

    let file = format!("{FILE_PREFIX}{}.{ext}", uuid());
    fs::write(playground.dir.join(&file), &data)
        .await
        .map_err(io_error)?;
    info!(category = CATEGORY, file, size, "image uploaded");

    Ok(Json(UploadResponse {
        file,
        source: SOURCE,
        ext,
        size,
        expires_in: playground.ttl.as_secs(),
    }))
}

fn page_html(playground: &Playground) -> String {
    let config = format!(
        r#"{{"source":"{SOURCE}","maxUploadBytes":{},"ttlSeconds":{}}}"#,
        playground.max_upload_bytes,
        playground.ttl.as_secs()
    );
    PAGE.replace(PAGE_CONFIG_PLACEHOLDER, &config)
}

async fn page() -> Result<Response> {
    let playground = PLAYGROUND
        .get()
        .ok_or_else(|| new_error(404, "playground is disabled"))?;
    let html = PAGE_HTML.get_or_init(|| page_html(playground));
    let mut res = Html(html.as_str()).into_response();
    let headers = res.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    Ok(res)
}

async fn font() -> impl IntoResponse {
    let headers: [(HeaderName, &str); 2] = [
        (header::CONTENT_TYPE, "font/woff2"),
        (header::CACHE_CONTROL, "public, max-age=604800"),
    ];
    (headers, FONT)
}

/// `/` has no content of its own, send visitors to the playground.
async fn index() -> Redirect {
    // absolute when a route prefix is configured: `<prefix>` has no trailing
    // slash, so a relative target would resolve outside of it
    match must_get_basic_config().prefix.as_deref() {
        Some(prefix) => {
            Redirect::temporary(&format!("{}/playground", prefix.trim_end_matches('/')))
        }
        None => Redirect::temporary("playground"),
    }
}

/// Routes of the playground, `None` when it is disabled.
pub fn new_playground_router() -> Option<Router> {
    let playground = PLAYGROUND.get()?;
    let body_limit = playground
        .max_upload_bytes
        .saturating_add(MULTIPART_OVERHEAD);
    let body_limit = usize::try_from(body_limit).unwrap_or(usize::MAX);
    Some(
        Router::new()
            .route("/", get(index))
            .route("/playground", get(page))
            .route("/playground/font.woff2", get(font))
            .route(
                "/playground/upload",
                post(upload).layer(DefaultBodyLimit::max(body_limit)),
            ),
    )
}

/// Resolve the upload directory; the bool tells whether it is the default
/// location inside the system temp dir.
fn resolve_dir(dir: &str) -> (PathBuf, bool) {
    let dir = dir.trim();
    if dir.is_empty() {
        return (env::temp_dir().join(DEFAULT_DIR_NAME), true);
    }
    let dir = PathBuf::from(dir);
    (path::absolute(&dir).unwrap_or(dir), false)
}

#[cfg(unix)]
async fn restrict_permissions(dir: &Path) -> io::Result<()> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dir, Permissions::from_mode(0o700)).await
}

#[cfg(not(unix))]
async fn restrict_permissions(_dir: &Path) -> io::Result<()> {
    Ok(())
}

async fn prepare_dir(dir: &Path, is_default: bool) -> Result<()> {
    let existed = fs::try_exists(dir).await.map_err(io_error)?;
    fs::create_dir_all(dir).await.map_err(io_error)?;
    if !existed {
        // uploads are other people's images: keep them away from other local users
        restrict_permissions(dir).await.map_err(io_error)?;
    }
    // The default location has a predictable name inside a world-writable
    // dir. Refuse a symlink planted there: uploads and cleanup would follow it.
    let meta = fs::symlink_metadata(dir).await.map_err(io_error)?;
    if is_default && !meta.is_dir() {
        return Err(new_error(
            500,
            format!("playground dir is not a directory: {}", dir.display()),
        ));
    }
    Ok(())
}

async fn init_playground() -> Result<bool> {
    let config = must_get_config()
        .sub_config("playground")
        .try_deserialize::<PlaygroundConfig>()?;
    if !config.enabled {
        info!(category = CATEGORY, "playground is disabled");
        return Ok(false);
    }
    if config.ttl.is_zero() || config.max_upload_bytes == 0 {
        return Err(new_error(
            500,
            "playground.ttl and playground.max_upload_bytes must be greater than 0",
        ));
    }

    let (dir, is_default) = resolve_dir(&config.dir);
    prepare_dir(&dir, is_default).await?;
    // leftovers of a previous run
    let removed = cleanup(&dir, config.ttl).await.map_err(io_error)?;

    let job = Job::new_repeated_async(cleanup_interval(config.ttl), |_, _| Box::pin(run_cleanup()))
        .map_err(Error::new)?;
    register_job_task("playground_cleanup", job);

    info!(
        category = CATEGORY,
        dir = %dir.display(),
        ttl = ?config.ttl,
        max_upload_bytes = config.max_upload_bytes,
        max_total_bytes = config.max_total_bytes,
        removed,
        "playground is enabled"
    );
    PLAYGROUND
        .set(Playground {
            dir,
            max_upload_bytes: config.max_upload_bytes,
            max_total_bytes: config.max_total_bytes,
            ttl: config.ttl,
        })
        .map_err(|_| new_error(500, "playground already initialized"))?;
    Ok(true)
}

struct PlaygroundTask;

impl Task for PlaygroundTask {
    fn before(&self) -> BoxFuture<'_, Result<bool>> {
        Box::pin(init_playground())
    }
    // after config (0), before dal and guard (16) which register the storage
    fn priority(&self) -> u8 {
        8
    }
}

#[ctor(unsafe)]
fn init() {
    register_task("playground", Arc::new(PlaygroundTask));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_ext_recognizes_supported_formats() {
        assert_eq!(sniff_ext(b"\xff\xd8\xff\xe0\x00\x10JFIF"), Some("jpeg"));
        assert_eq!(sniff_ext(b"\x89PNG\r\n\x1a\n\x00\x00"), Some("png"));
        assert_eq!(sniff_ext(b"GIF89a\x01\x00"), Some("gif"));
        assert_eq!(sniff_ext(b"RIFF\x24\x00\x00\x00WEBPVP8 "), Some("webp"));
        assert_eq!(
            sniff_ext(b"\x00\x00\x00\x1cftypavif\x00\x00\x00\x00avifmif1miaf"),
            Some("avif")
        );
        // major brand mif1, avif only listed as a compatible brand
        assert_eq!(
            sniff_ext(b"\x00\x00\x00\x1cftypmif1\x00\x00\x00\x00mif1avifmiaf"),
            Some("avif")
        );
        assert_eq!(sniff_ext(b"\xff\x0a\x00\x00"), Some("jxl"));
    }

    #[test]
    fn sniff_ext_rejects_everything_else() {
        assert_eq!(sniff_ext(b""), None);
        assert_eq!(
            sniff_ext(b"<svg xmlns=\"http://www.w3.org/2000/svg\">"),
            None
        );
        assert_eq!(sniff_ext(b"<!doctype html><script>alert(1)</script>"), None);
        assert_eq!(sniff_ext(b"RIFF\x24\x00\x00\x00WAVEfmt "), None);
        // HEIC shares the ftyp container with AVIF
        assert_eq!(
            sniff_ext(b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00heicmif1"),
            None
        );
    }

    #[test]
    fn only_own_files_count_as_uploads() {
        assert!(is_upload_name(
            "pg-0199a1b2-7c3d-7e4f-8a5b-6c7d8e9f0a1b.jpeg"
        ));
        assert!(!is_upload_name("photo.jpeg"));
        assert!(!is_upload_name("pg-notes.txt"));
        assert!(!is_upload_name("pg-noext"));
    }

    #[test]
    fn cleanup_interval_is_bounded() {
        assert_eq!(
            cleanup_interval(Duration::from_secs(3600)),
            Duration::from_secs(300)
        );
        assert_eq!(
            cleanup_interval(Duration::from_secs(120)),
            Duration::from_secs(30)
        );
        assert_eq!(
            cleanup_interval(Duration::from_secs(4)),
            Duration::from_secs(10)
        );
    }

    #[tokio::test]
    async fn cleanup_removes_only_expired_uploads() {
        let dir = env::temp_dir().join(format!("image-optim-playground-test-{}", uuid()));
        fs::create_dir_all(&dir).await.unwrap();
        let upload = dir.join("pg-a.png");
        let foreign = dir.join("keep.png");
        fs::write(&upload, b"x").await.unwrap();
        fs::write(&foreign, b"x").await.unwrap();

        // nothing is older than an hour yet
        assert_eq!(cleanup(&dir, Duration::from_secs(3600)).await.unwrap(), 0);
        assert!(fs::try_exists(&upload).await.unwrap());

        // with a zero ttl every upload is expired, foreign files stay
        assert_eq!(cleanup(&dir, Duration::ZERO).await.unwrap(), 1);
        assert!(!fs::try_exists(&upload).await.unwrap());
        assert!(fs::try_exists(&foreign).await.unwrap());

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
