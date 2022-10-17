use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use http::HeaderValue;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct HTTPError {
    pub message: String,
    pub category: String,
    pub status: u16,
}

impl HTTPError {
    pub fn new(message: &str, category: &str) -> Self {
        Self {
            message: message.to_string(),
            category: category.to_string(),
            status: 400,
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
            category: "fromUtf8".to_string(),
            ..Default::default()
        }
    }
}
