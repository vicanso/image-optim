use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ImageError {
    message: String,
    category: String,
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
