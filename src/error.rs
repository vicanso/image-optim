use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ImageError {
    pub message: String,
    pub category: String,
}

impl From<imagequant::Error> for ImageError {
    fn from(error: imagequant::Error) -> Self {
        ImageError {
            message: error.to_string(),
            category: "imagequant".to_string(),
        }
    }
}
impl From<lodepng::Error> for ImageError {
    fn from(error: lodepng::Error) -> Self {
        ImageError {
            message: error.to_string(),
            category: "lodepng".to_string(),
        }
    }
}
impl From<image::ImageError> for ImageError {
    fn from(error: image::ImageError) -> Self {
        ImageError {
            message: error.to_string(),
            category: "image".to_string(),
        }
    }
}
impl From<std::string::String> for ImageError {
    fn from(message: std::string::String) -> Self {
        ImageError {
            message,
            category: "unknown".to_string(),
        }
    }
}
