use axum::http::{header, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::{BoxError, Json};
use serde::Serialize;
use tracing::error;

#[derive(Debug, Clone, Serialize)]
pub struct HTTPError {
    pub message: String,
    pub category: String,
    pub status: u16,
}
pub type HTTPResult<T> = Result<T, HTTPError>;

impl HTTPError {
    pub fn new(message: &str, category: &str) -> Self {
        Self {
            message: message.to_string(),
            category: category.to_string(),
            status: 400,
        }
    }
    pub fn new_with_category_status(message: &str, category: &str, status: u16) -> Self {
        Self {
            message: message.to_string(),
            category: category.to_string(),
            status,
        }
    }
}
impl Default for HTTPError {
    fn default() -> Self {
        HTTPError {
            message: "".to_string(),
            category: "".to_string(),
            // 默认使用400为状态码
            status: 400,
        }
    }
}
impl IntoResponse for HTTPError {
    fn into_response(self) -> Response {
        let status = match StatusCode::from_u16(self.status) {
            Ok(status) => status,
            Err(_) => StatusCode::BAD_REQUEST,
        };
        // 对于出错设置为no-cache
        let mut res = Json(self).into_response();
        res.headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        (status, res).into_response()
    }
}

impl From<std::string::FromUtf8Error> for HTTPError {
    fn from(error: std::string::FromUtf8Error) -> Self {
        HTTPError {
            message: error.to_string(),
            category: "from_utf8".to_string(),
            ..Default::default()
        }
    }
}

pub async fn handle_error(
    // `Method` and `Uri` are extractors so they can be used here
    method: Method,
    uri: Uri,
    // the last argument must be the error itself
    err: BoxError,
) -> HTTPError {
    error!("method:{}, uri:{}, error:{}", method, uri, err.to_string());
    if err.is::<tower::timeout::error::Elapsed>() {
        return HTTPError::new_with_category_status("Request took too long", "timeout", 408);
    }
    HTTPError::new(&err.to_string(), "exception")
}
