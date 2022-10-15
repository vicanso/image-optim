use crate::image::HandleError;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
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
        (status, Json(self)).into_response()
    }
}

impl From<HandleError> for HTTPError {
    fn from(err: HandleError) -> Self {
        HTTPError {
            message: err.to_string(),
            category: "image".to_string(),
            status: 500,
            ..Default::default()
        }
    }
}
impl From<base64::DecodeError> for HTTPError {
    fn from(error: base64::DecodeError) -> Self {
        HTTPError {
            message: error.to_string(),
            category: "base64".to_string(),
            ..Default::default()
        }
    }
}
impl From<reqwest::Error> for HTTPError {
    fn from(error: reqwest::Error) -> Self {
        HTTPError {
            message: error.to_string(),
            category: "reqwest".to_string(),
            ..Default::default()
        }
    }
}
impl From<reqwest::header::ToStrError> for HTTPError {
    fn from(error: reqwest::header::ToStrError) -> Self {
        HTTPError {
            message: error.to_string(),
            category: "reqwest".to_string(),
            ..Default::default()
        }
    }
}

impl From<image::ImageError> for HTTPError {
    fn from(error: image::ImageError) -> Self {
        HTTPError {
            message: error.to_string(),
            category: "image".to_string(),
            ..Default::default()
        }
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
impl From<std::num::ParseIntError> for HTTPError {
    fn from(error: std::num::ParseIntError) -> Self {
        HTTPError {
            message: error.to_string(),
            category: "parseInt".to_string(),
            ..Default::default()
        }
    }
}
