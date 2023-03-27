use crate::error::HTTPError;
use crate::images::{avif_decode, to_gif, ImageError, ImageInfo};
use crate::{task_local::*, tl_info};
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use dssim::Dssim;
use image::imageops::grayscale;
use image::{
    imageops::{crop, overlay, resize, FilterType},
    load, DynamicImage, ImageFormat, RgbaImage,
};
use lru::LruCache;
use once_cell::sync::OnceCell;
use rgb::FromSlice;
use snafu::{ensure, ResultExt, Snafu};
use std::{env, ffi::OsStr, io::Cursor, num::NonZeroUsize, sync::Mutex, vec};
use urlencoding::decode;

fn get_default_quality() -> u8 {
    static OPTIM_QUALITY: OnceCell<u8> = OnceCell::new();
    let result = OPTIM_QUALITY.get_or_init(|| -> u8 {
        let quality = env::var("OPTIM_QUALITY").unwrap_or_else(|_| "90".to_string());
        quality.parse::<u8>().unwrap_or(90)
    });
    result.to_owned()
}

fn get_default_speed() -> u8 {
    static OPTIM_SPEED: OnceCell<u8> = OnceCell::new();
    let result = OPTIM_SPEED.get_or_init(|| -> u8 {
        let speed = env::var("OPTIM_SPEED").unwrap_or_else(|_| "3".to_string());
        speed.parse::<u8>().unwrap_or(3)
    });
    result.to_owned()
}

fn is_disable_dssim() -> bool {
    static OPTIM_DISABLE_DSSIM: OnceCell<bool> = OnceCell::new();
    let result = OPTIM_DISABLE_DSSIM.get_or_init(|| -> bool {
        let disable = env::var("OPTIM_DISABLE_DSSIM").unwrap_or_else(|_| "".to_string());
        disable == "1"
    });
    result.to_owned()
}

fn get_alias() -> &'static String {
    static OPTIM_ALIAS: OnceCell<String> = OnceCell::new();
    OPTIM_ALIAS.get_or_init(|| -> String {
        let prefix = "OPTIM_ALIAS_";
        let mut arr = Vec::new();
        for (key, value) in env::vars() {
            if !key.starts_with(prefix) {
                continue;
            }
            let k = key[prefix.len()..].to_string();
            if k.is_empty() {
                continue;
            }
            arr.push(format!("{}={}", k, value));
        }
        arr.join(" ")
    })
}

fn get_lru_cache() -> &'static Mutex<LruCache<String, ProcessImage>> {
    static CACHE: OnceCell<Mutex<LruCache<String, ProcessImage>>> = OnceCell::new();
    CACHE.get_or_init(|| {
        let c = LruCache::new(NonZeroUsize::new(10).unwrap());
        Mutex::new(c)
    })
}

fn get_image_cache(key: String) -> Option<ProcessImage> {
    if let Ok(mut c) = get_lru_cache().lock() {
        if let Some(result) = c.get(&key) {
            return Some(result.clone());
        }
    }
    None
}

fn set_image_cache(key: String, v: &ProcessImage) {
    // TODO 缓存是否设置有效期
    if let Ok(mut c) = get_lru_cache().lock() {
        // 失败忽略
        c.put(key, v.clone());
    }
}

#[derive(Debug, Snafu)]
pub enum ImageProcessingError {
    #[snafu(display("Process image fail, message:{message}"))]
    ParamsInvalid { message: String },
    #[snafu(display("{source}"))]
    Reqwest { source: reqwest::Error },
    #[snafu(display("{source}"))]
    HTTPHeaderToStr { source: http::header::ToStrError },
    #[snafu(display("{source}"))]
    Base64Decode { source: base64::DecodeError },
    #[snafu(display("{source}"))]
    Image { source: image::ImageError },
    #[snafu(display("{source}"))]
    Images { source: ImageError },
    #[snafu(display("{source}"))]
    ParseInt { source: std::num::ParseIntError },
    #[snafu(display("{source}"))]
    FromUtf { source: std::string::FromUtf8Error },
}
type Result<T, E = ImageProcessingError> = std::result::Result<T, E>;

#[derive(Default, Clone)]
pub struct ProcessImage {
    original: Option<RgbaImage>,
    di: DynamicImage,
    pub diff: f64,
    pub original_size: usize,
    buffer: Vec<u8>,
    pub ext: String,
}
impl From<ImageProcessingError> for HTTPError {
    fn from(err: ImageProcessingError) -> Self {
        match err {
            ImageProcessingError::Images { source } => {
                let detail = source.to_detail();
                HTTPError {
                    status: 500,
                    category: detail.category,
                    message: detail.message,
                }
            }
            _ => HTTPError {
                status: 400,
                category: "".to_string(),
                message: err.to_string(),
            },
        }
    }
}

impl ProcessImage {
    pub fn new() -> Self {
        Self {
            original_size: 0,
            di: DynamicImage::new_rgba8(0, 0),
            buffer: Vec::new(),
            ext: "".to_string(),
            ..Default::default()
        }
    }
    pub fn get_buffer(&self) -> Result<Vec<u8>> {
        if self.buffer.is_empty() {
            let mut bytes: Vec<u8> = Vec::new();
            let format =
                ImageFormat::from_extension(self.ext.as_str()).unwrap_or(ImageFormat::Jpeg);
            self.di
                .write_to(&mut Cursor::new(&mut bytes), format)
                .context(ImageSnafu {})?;
            Ok(bytes)
        } else {
            Ok(self.buffer.clone())
        }
    }
    fn support_dssim(&self) -> bool {
        !is_disable_dssim() && self.ext != IMAGE_TYPE_GIF
    }
    fn get_diff(&self) -> f64 {
        // 如果无数据
        if self.original.is_none() {
            return -1.0;
        }
        // 如果是gif或者禁用了dssim
        if !self.support_dssim() {
            return -1.0;
        }
        // 已确保一定有数据
        let original = self.original.clone().unwrap();
        // 如果宽高不一致，则不比对
        if original.width() != self.di.width() || original.height() != self.di.height() {
            return -1.0;
        }
        let width = original.width() as usize;
        let height = original.height() as usize;
        let attr = Dssim::new();
        let gp1 = attr
            .create_image_rgba(original.as_raw().as_rgba(), width, height)
            .unwrap();
        let gp2 = attr
            .create_image_rgba(self.di.clone().to_rgba8().as_raw().as_rgba(), width, height)
            .unwrap();
        let (diff, _) = attr.compare(&gp1, gp2);
        let value: f64 = diff.into();
        // 放大1千倍
        value * 1000.0
    }
}

pub const PROCESS_LOAD: &str = "load";
pub const PROCESS_RESIZE: &str = "resize";
pub const PROCESS_OPTIM: &str = "optim";
pub const PROCESS_CROP: &str = "crop";
pub const PROCESS_GRAY: &str = "gray";
pub const PROCESS_WATERMARK: &str = "watermark";

const IMAGE_TYPE_GIF: &str = "gif";
const IMAGE_TYPE_PNG: &str = "png";
const IMAGE_TYPE_AVIF: &str = "avif";
const IMAGE_TYPE_WEBP: &str = "webp";
const IMAGE_TYPE_JPEG: &str = "jpeg";

pub async fn run(tasks: Vec<Vec<String>>) -> Result<ProcessImage> {
    let mut img = ProcessImage::new();
    let he = ParamsInvalidSnafu {
        message: "params is invalid",
    };
    let alias = get_alias();
    let alias_list = alias.split(' ');
    let mut alias_replacements = Vec::new();
    for item in alias_list {
        let kv: Vec<_> = item.split('=').collect();
        if kv.len() != 2 {
            continue;
        }
        alias_replacements.push(kv);
    }
    let replacements = &alias_replacements;

    for params in tasks {
        if params.len() < 2 {
            continue;
        }
        let mut sub_params = Vec::with_capacity(params.len() - 1);
        for (i, param) in params.iter().enumerate() {
            if i == 0 {
                continue;
            }
            // 替换alias
            let mut value = param.clone();
            for replacement in replacements {
                value = value.replace(replacement[0], replacement[1]);
            }
            sub_params.push(value);
        }
        let task = &params[0];
        tl_info!(task, "processing {:?}", sub_params,);
        match task.as_str() {
            PROCESS_LOAD => {
                let data = sub_params[0].to_string();
                let mut ext = "".to_string();
                if sub_params.len() >= 2 {
                    ext = sub_params[1].to_string();
                }
                img = LoaderProcess::new(data, ext).process(img).await?;
                img.original = Some(img.di.to_rgba8().clone())
            }
            PROCESS_RESIZE => {
                // 参数不符合
                ensure!(sub_params.len() >= 2, he);
                let width = sub_params[0].parse::<u32>().context(ParseIntSnafu {})?;
                let height = sub_params[1].parse::<u32>().context(ParseIntSnafu {})?;
                img = ResizeProcess::new(width, height).process(img).await?;
            }
            PROCESS_GRAY => {
                img = GrayProcess::new().process(img).await?;
            }
            PROCESS_OPTIM => {
                // 参数不符合
                ensure!(!sub_params.is_empty(), he);
                let output_type = sub_params[0].to_string();
                let mut quality = get_default_quality();
                if sub_params.len() > 1 {
                    quality = sub_params[1].parse::<u8>().context(ParseIntSnafu {})?;
                }

                let mut speed = get_default_speed();
                if sub_params.len() > 2 {
                    speed = sub_params[2].parse::<u8>().context(ParseIntSnafu {})?;
                }

                img = OptimProcess::new(output_type, quality, speed)
                    .process(img)
                    .await?;
            }
            PROCESS_CROP => {
                // 参数不符合
                ensure!(sub_params.len() >= 4, he);
                let x = sub_params[0].parse::<u32>().context(ParseIntSnafu {})?;
                let y = sub_params[1].parse::<u32>().context(ParseIntSnafu {})?;
                let width = sub_params[2].parse::<u32>().context(ParseIntSnafu {})?;
                let height = sub_params[3].parse::<u32>().context(ParseIntSnafu {})?;
                img = CropProcess::new(x, y, width, height).process(img).await?;
            }
            PROCESS_WATERMARK => {
                // 参数不符合
                ensure!(!sub_params.is_empty(), he);
                let url = decode(sub_params[0].as_str())
                    .context(FromUtfSnafu {})?
                    .to_string();
                let mut position = WatermarkPosition::RightBottom;
                if sub_params.len() > 1 {
                    position = WatermarkPosition::from_str(sub_params[1].as_str());
                }
                let mut margin_left = 0;
                if sub_params.len() > 2 {
                    margin_left = sub_params[2].parse::<i64>().context(ParseIntSnafu {})?;
                }
                let mut margin_top = 0;
                if sub_params.len() > 3 {
                    margin_top = sub_params[3].parse::<i64>().context(ParseIntSnafu {})?;
                }
                // 读取缓存
                let watermark = if let Some(img) = get_image_cache(url.clone()) {
                    img
                } else {
                    let img = LoaderProcess::new(url.clone(), "".to_string())
                        .process(ProcessImage::new())
                        .await?;

                    set_image_cache(url, &img);
                    img
                };
                let pro = WatermarkProcess::new(watermark.di, position, margin_left, margin_top);
                img = pro.process(img).await?;
            }
            _ => {}
        }
        tl_info!(task, "processing done");
    }
    img.diff = img.get_diff();
    Ok(img)
}

#[async_trait]

pub trait Process {
    async fn process(&self, pi: ProcessImage) -> Result<ProcessImage>;
}

// 数据加载处理
pub struct LoaderProcess {
    data: String,
    ext: String,
}

impl LoaderProcess {
    pub fn new(data: String, ext: String) -> Self {
        LoaderProcess { data, ext }
    }
    async fn fetch_data(&self) -> Result<ProcessImage> {
        let data = self.data.clone();
        let mut ext = self.ext.clone();
        let original_data = match data.starts_with("http") {
            true => {
                let resp = reqwest::get(data).await.context(ReqwestSnafu {})?;
                if let Some(content_type) = resp.headers().get("Content-Type") {
                    let str = content_type.to_str().context(HTTPHeaderToStrSnafu {})?;
                    let arr: Vec<_> = str.split('/').collect();
                    if arr.len() == 2 {
                        ext = arr[1].to_string();
                    }
                }
                resp.bytes().await.context(ReqwestSnafu {})?.into()
            }
            _ => general_purpose::STANDARD_NO_PAD
                .decode(data.as_bytes())
                .context(Base64DecodeSnafu {})?,
        };
        let c = Cursor::new(original_data.clone());
        let format = ImageFormat::from_extension(OsStr::new(ext.clone().as_str()));

        ensure!(
            format.is_some(),
            ParamsInvalidSnafu {
                message: "format is not support".to_string(),
            }
        );

        let di = load(c, format.unwrap()).context(ImageSnafu {})?;
        Ok(ProcessImage {
            original_size: original_data.len(),
            di,
            buffer: original_data,
            ext,
            ..Default::default()
        })
    }
}

// 图片加载
#[async_trait]

impl Process for LoaderProcess {
    async fn process(&self, _: ProcessImage) -> Result<ProcessImage> {
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
    async fn process(&self, pi: ProcessImage) -> Result<ProcessImage> {
        let mut img = pi;
        let mut w = self.width;
        let mut h = self.height;
        if w == 0 && h == 0 {
            return Ok(img);
        }
        let width = img.di.width();
        let height = img.di.height();
        // 如果宽或者高为0，则计算对应的宽高
        if w == 0 {
            w = width * h / height;
        }
        if h == 0 {
            h = height * w / width;
        }
        let result = resize(&img.di, w, h, FilterType::Lanczos3);
        img.buffer = vec![];
        img.di = DynamicImage::ImageRgba8(result);
        Ok(img)
    }
}

pub struct GrayProcess {}

impl GrayProcess {
    pub fn new() -> Self {
        GrayProcess {}
    }
}

#[async_trait]
impl Process for GrayProcess {
    async fn process(&self, pi: ProcessImage) -> Result<ProcessImage> {
        let mut img = pi;
        img.di = DynamicImage::ImageLuma8(grayscale(&img.di));
        img.buffer = vec![];
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

impl WatermarkPosition {
    fn from_str(s: &str) -> Self {
        match s {
            "leftTop" => WatermarkPosition::LeftTop,
            "top" => WatermarkPosition::Top,
            "rightTop" => WatermarkPosition::RightTop,
            "left" => WatermarkPosition::Left,
            "center" => WatermarkPosition::Center,
            "right" => WatermarkPosition::Right,
            "leftBottom" => WatermarkPosition::LeftBottom,
            "bottom" => WatermarkPosition::Bottom,
            _ => WatermarkPosition::RightBottom,
        }
    }
}

// 水印处理
pub struct WatermarkProcess {
    watermark: DynamicImage,
    position: WatermarkPosition,
    margin_left: i64,
    margin_top: i64,
}

impl WatermarkProcess {
    pub fn new(
        watermark: DynamicImage,
        position: WatermarkPosition,
        margin_left: i64,
        margin_top: i64,
    ) -> Self {
        WatermarkProcess {
            watermark,
            position,
            margin_left,
            margin_top,
        }
    }
}

#[async_trait]
impl Process for WatermarkProcess {
    async fn process(&self, pi: ProcessImage) -> Result<ProcessImage> {
        let mut img = pi;
        let di = img.di;
        let w = di.width() as i64;
        let h = di.height() as i64;
        let ww = self.watermark.width() as i64;
        let wh = self.watermark.height() as i64;
        let mut x: i64 = 0;
        let mut y: i64 = 0;
        match self.position {
            WatermarkPosition::Top => {
                x = (w - ww) >> 1;
            }
            WatermarkPosition::RightTop => {
                x = w - ww;
            }
            WatermarkPosition::Left => {
                y = (h - wh) >> 1;
            }
            WatermarkPosition::Center => {
                x = (w - ww) >> 1;
                y = (h - wh) >> 1;
            }
            WatermarkPosition::Right => {
                x = w - ww;
                y = (h - wh) >> 1;
            }
            WatermarkPosition::LeftBottom => {
                y = h - wh;
            }
            WatermarkPosition::Bottom => {
                x = (w - ww) >> 1;
                y = h - wh;
            }
            WatermarkPosition::RightBottom => {
                x = w - ww;
                y = h - wh;
            }
            _ => (),
        }
        x += self.margin_left;
        y += self.margin_top;
        let mut bottom: DynamicImage = di;
        overlay(&mut bottom, &self.watermark, x, y);
        img.buffer = vec![];
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
    async fn process(&self, pi: ProcessImage) -> Result<ProcessImage> {
        let mut img = pi;
        let mut r = img.di;
        let result = crop(&mut r, self.x, self.y, self.width, self.height);
        img.di = DynamicImage::ImageRgba8(result.to_image());
        img.buffer = vec![];
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
    async fn process(&self, pi: ProcessImage) -> Result<ProcessImage> {
        let mut img = pi;

        let info: ImageInfo = img.di.to_rgba8().into();
        let quality = self.quality;
        let speed = self.speed;
        let original_type = img.ext.clone();

        let original_size = img.buffer.len();
        let mut output_type = self.output_type.clone();
        // 如果未指定输出，则保持原有
        if output_type.is_empty() {
            output_type = original_type.clone();
        }

        img.ext = output_type.clone();

        let data = match output_type.as_str() {
            IMAGE_TYPE_GIF => {
                let c = Cursor::new(img.buffer.clone());
                to_gif(c, 10).context(ImagesSnafu {})?
            }
            _ => {
                match output_type.as_str() {
                    IMAGE_TYPE_PNG => info.to_png(quality).context(ImagesSnafu {})?,
                    IMAGE_TYPE_AVIF => info.to_avif(quality, speed).context(ImagesSnafu {})?,
                    IMAGE_TYPE_WEBP => info.to_webp(quality).context(ImagesSnafu {})?,
                    // 其它的全部使用jpeg
                    _ => {
                        img.ext = IMAGE_TYPE_JPEG.to_string();
                        info.to_mozjpeg(quality).context(ImagesSnafu {})?
                    }
                }
            }
        };
        // 类型不一样
        // 或者类型一样但是数据最小
        // 或者无原始数据
        if img.ext != original_type || data.len() < original_size || original_size == 0 {
            img.buffer = data;
            // 支持dssim再根据数据生成image
            // 否则无此必要
            if img.support_dssim() {
                // image 的avif decoder有问题
                // 暂使用其它模块
                if img.ext == IMAGE_TYPE_AVIF {
                    img.di = avif_decode(&img.buffer).context(ImagesSnafu {})?;
                } else {
                    let c = Cursor::new(&img.buffer);
                    let format = ImageFormat::from_extension(OsStr::new(img.ext.as_str()));
                    img.di = load(c, format.unwrap()).context(ImageSnafu {})?;
                }
            }
        }

        Ok(img)
    }
}
