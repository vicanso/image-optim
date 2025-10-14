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

use axum::body::Body;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};

pub struct ImagePreview {
    pub diff: f64,
    pub ratio: usize,
    pub data: Vec<u8>,
    pub image_type: String,
}

// 图片预览转换为response
impl IntoResponse for ImagePreview {
    fn into_response(self) -> Response {
        let mut res = Body::from(self.data).into_response();

        // 设置content type
        let result = mime_guess::from_ext(self.image_type.as_str()).first_or(mime::IMAGE_JPEG);
        if let Ok(value) = HeaderValue::from_str(result.as_ref()) {
            res.headers_mut().insert(header::CONTENT_TYPE, value);
        }

        // 图片设置为缓存30天
        res.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=2592000"),
        );
        if let Ok(value) = HeaderValue::from_str(&format!("{:.2}", self.diff)) {
            res.headers_mut().insert("X-Dssim-Diff", value);
        }
        if let Ok(value) = HeaderValue::from_str(self.ratio.to_string().as_str()) {
            res.headers_mut().insert("X-Ratio", value);
        }

        res
    }
}
