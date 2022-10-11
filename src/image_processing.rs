use std::borrow::BorrowMut;
use std::{ffi::OsStr, io::Cursor};

use crate::error::HTTPError;
use image::{
    imageops::{overlay, resize, FilterType},
    load, DynamicImage, ImageFormat,
};

pub struct ProcessImage {
    di: Option<DynamicImage>,
    buffer: Option<Vec<u8>>,
    ext: Option<String>,
}

type ProcessResult = Result<ProcessImage, HTTPError>;

pub trait Process {
    fn process(&self, pi: ProcessImage) -> ProcessResult;
}

// 数据加载处理
pub struct LoaderProcess {
    data: String,
    ext: String,
    pi: ProcessImage,
}

impl LoaderProcess {
    pub fn new(data: String, ext: String) -> Self {
        LoaderProcess {
            data,
            ext,
            pi: ProcessImage {
                di: None,
                buffer: None,
                ext: None,
            },
        }
    }
    async fn fetch_data(&self) -> ProcessResult {
        let data = self.data.clone();
        let mut ext = self.ext.clone();
        let original_data = match data.starts_with("http") {
            true => {
                let resp = reqwest::get(data).await?;
                if let Some(content_type) = resp.headers().get("Content-Type") {
                    let str = content_type.to_str()?;
                    let arr: Vec<_> = str.split('/').collect();
                    if arr.len() == 2 {
                        ext = arr[1].to_string();
                    }
                }
                resp.bytes().await?.into()
            }
            _ => base64::decode(data)?,
        };
        let c = Cursor::new(original_data.clone());
        let format =
            ImageFormat::from_extension(OsStr::new(ext.clone().as_str())).ok_or_else(|| {
                HTTPError {
                    message: "not support format".to_string(),
                    category: "imageFormat".to_string(),
                    ..Default::default()
                }
            })?;

        let di = load(c, format)?;
        Ok(ProcessImage {
            di: Some(di),
            buffer: Some(original_data),
            ext: Some(ext),
        })
    }
}
impl Process for LoaderProcess {
    fn process(&self, _: ProcessImage) -> ProcessResult {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        let future = self.fetch_data();
        let result = rt.block_on(future)?;
        Ok(result)
    }
}

// 尺寸调整处理
pub struct ResizeProcess {
    width: u32,
    height: u32,
}

impl ResizeProcess {
    pub fn new(width: u32, height: u32) -> Self {
        ResizeProcess { width, height }
    }
}

impl Process for ResizeProcess {
    fn process(&self, pi: ProcessImage) -> ProcessResult {
        let mut img = pi;
        let mut w = self.width;
        let mut h = self.height;
        if w == 0 && h == 0 {
            return Ok(img);
        }
        // 如果宽或者高为0，则计算对应的宽高
        if w == 0 {
            w = self.width * h / self.height
        }
        if h == 0 {
            h = self.height * w / self.width;
        }
        if let Some(di) = img.di {
            let result = resize(&di, w, h, FilterType::Lanczos3);
            img.di = Some(DynamicImage::ImageRgba8(result))
        }
        Ok(img)
    }
}

pub enum WatermarkPosition {
    LeftTop,
    Top,
    RightTop,
    Left,
    Center,
    Right,
    LeftBottom,
    Bottom,
    RightBottom,
}

// 水印处理
pub struct WatermarkProcess {
    watermark: DynamicImage,
    position: WatermarkPosition,
    margin_left: i64,
    margin_top: i64,
}

impl WatermarkProcess {
    pub fn new(watermark: DynamicImage, position: WatermarkPosition) -> Self {
        WatermarkProcess {
            watermark,
            position,
            margin_left: 0,
            margin_top: 0,
        }
    }
}

impl Process for WatermarkProcess {
    fn process(&self, pi: ProcessImage) -> ProcessResult {
        let mut img = pi;
        if let Some(di) = img.di {
            let w = di.width();
            let h = di.height();
            let ww = self.watermark.width();
            let wh = self.watermark.height();
            let mut x: i64 = 0;
            let mut y: i64 = 0;
            match self.position {
                WatermarkPosition::Top => {
                    x = ((w - ww) >> 1) as i64;
                }
                WatermarkPosition::RightTop => {
                    x = (w - ww) as i64;
                }
                WatermarkPosition::Left => {
                    y = ((h - wh) >> 1) as i64;
                }
                WatermarkPosition::Center => {
                    x = ((w - ww) >> 1) as i64;
                    y = ((h - wh) >> 1) as i64;
                }
                WatermarkPosition::Right => {
                    x = (w - ww) as i64;
                    y = ((h - wh) >> 1) as i64;
                }
                WatermarkPosition::LeftBottom => {
                    y = (h - wh) as i64;
                }
                WatermarkPosition::Bottom => {
                    x = ((w - ww) >> 1) as i64;
                    y = (h - wh) as i64;
                }
                WatermarkPosition::RightBottom => {
                    x = (w - ww) as i64;
                    y = (h - wh) as i64;
                }
                _ => (),
            }
            x += self.margin_left;
            y += self.margin_top;
            let mut bottom: DynamicImage = di;
            overlay(&mut bottom, &self.watermark, x, y);
            img.di = Some(bottom);
        }
        Ok(img)
    }
}

// TODO 截取处理