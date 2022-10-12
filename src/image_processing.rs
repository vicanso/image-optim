use crate::error::HTTPError;
use crate::image::{to_gif, ImageInfo};
use async_trait::async_trait;
use image::{
    imageops::{crop, overlay, resize, FilterType},
    load, DynamicImage, ImageFormat,
};
use std::{ffi::OsStr, io::Cursor};
use urlencoding::decode;

#[derive(Default)]
pub struct ProcessImage {
    di: DynamicImage,
    pub buffer: Vec<u8>,
    pub ext: String,
}

impl ProcessImage {
    pub fn new() -> Self {
        Self {
            di: DynamicImage::new_rgba8(0, 0),
            buffer: Vec::new(),
            ext: "".to_string(),
        }
    }
}

type ProcessResult = Result<ProcessImage, HTTPError>;

pub async fn process_pipeline(list: Vec<Box<dyn Process>>, pi: ProcessImage) -> ProcessResult {
    let mut img = pi;
    for p in list {
        img = p.process(img).await?;
    }
    Ok(img)
}

pub async fn run(desc: String) -> ProcessResult {
    let arr = desc.split('|');
    let sep = "/";
    let mut img = ProcessImage::new();
    let he = HTTPError::new("params is invalid", "validate");
    for item in arr {
        let params: Vec<_> = item.split(sep).collect();
        if params.is_empty() {
            continue;
        }
        match params[0] {
            "load" => {
                // 参数不符合
                if params.len() < 3 {
                    return Err(he);
                }
                let ext = params[1].to_string();
                let mut data = params[2..].join(sep);
                // http的url需要decode
                if data.starts_with("http") {
                    data = decode(data.as_str())?.to_string();
                }
                img = LoaderProcess::new(data, ext).process(img).await?;
            }
            "resize" => {
                // 参数不符合
                if params.len() < 3 {
                    return Err(he);
                }
                let width = params[1].parse::<u32>()?;
                let height = params[2].parse::<u32>()?;
                img = ResizeProcess::new(width, height).process(img).await?;
            }
            _ => {}
        }
    }
    Ok(img)
}

#[async_trait]

pub trait Process {
    async fn process(&self, pi: ProcessImage) -> ProcessResult;
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
            pi: ProcessImage::new(),
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
            di: di,
            buffer: original_data,
            ext: ext,
        })
    }
}

// 图片加载
#[async_trait]

impl Process for LoaderProcess {
    async fn process(&self, _: ProcessImage) -> ProcessResult {
        let result = self.fetch_data().await?;
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

#[async_trait]
impl Process for ResizeProcess {
    async fn process(&self, pi: ProcessImage) -> ProcessResult {
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
        let result = resize(&img.di, w, h, FilterType::Lanczos3);
        img.di = DynamicImage::ImageRgba8(result);
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

#[async_trait]
impl Process for WatermarkProcess {
    async fn process(&self, pi: ProcessImage) -> ProcessResult {
        let mut img = pi;
        let di = img.di;
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
        img.di = bottom;
        Ok(img)
    }
}

//  截取处理
pub struct CropProcess {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl CropProcess {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[async_trait]
impl Process for CropProcess {
    async fn process(&self, pi: ProcessImage) -> ProcessResult {
        let mut img = pi;
        let mut r = img.di;
        let result = crop(&mut r, self.x, self.y, self.width, self.height);
        img.di = DynamicImage::ImageRgba8(result.to_image());
        Ok(img)
    }
}

// 压缩处理
pub struct OptimProcess {
    output_type: String,
    quality: u8,
    speed: u8,
}

impl OptimProcess {
    pub fn new(output_type: String, quality: u8, speed: u8) -> Self {
        Self {
            output_type,
            quality,
            speed,
        }
    }
}

#[async_trait]
impl Process for OptimProcess {
    async fn process(&self, pi: ProcessImage) -> ProcessResult {
        let mut img = pi;

        let info: ImageInfo = img.di.to_rgba8().into();
        let quality = self.quality;
        let speed = self.speed;
        let original_type = img.ext.clone();

        let original_size = img.buffer.len();
        img.ext = self.output_type.clone();

        let data = match self.output_type.as_str() {
            "gif" => {
                let c = Cursor::new(img.buffer.clone());
                to_gif(c, 10)?
            }
            _ => {
                match self.output_type.as_str() {
                    "png" => info.to_png(quality)?,
                    "avif" => info.to_avif(quality, speed)?,
                    "webp" => info.to_webp(quality)?,
                    // 其它的全部使用jpeg
                    _ => {
                        img.ext = "jpeg".to_string();
                        info.to_mozjpeg(quality)?
                    }
                }
            }
        };
        // 类型不一样
        // 或者类型一样但是数据最小
        if img.ext != original_type || data.len() < original_size {
            img.buffer = data;
        }

        Ok(img)
    }
}
