use image::{
    imageops::{overlay, resize, FilterType},
    DynamicImage,
};

use crate::error::HTTPError;

pub struct ProcessImage {
    di: Option<DynamicImage>,
    buffer: Option<Vec<u8>>,
    ext: Option<String>,
}

type ProcessResult = Result<ProcessImage, HTTPError>;

pub trait Process {
    fn process(&self, img: ProcessImage) -> ProcessResult;
}

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
