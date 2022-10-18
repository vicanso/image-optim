use axum::{
    body::Full,
    http::{header, HeaderValue},
    response::{IntoResponse, Response},
};
use snafu::{ensure, ResultExt, Snafu};

use image::{codecs::avif, codecs::gif, codecs::webp, AnimationDecoder, ImageFormat, RgbaImage};
use lodepng::Bitmap;
use rgb::{ComponentBytes, RGB8, RGBA8};
use std::{
    ffi::OsStr,
    io::{BufRead, Read, Seek},
};

#[derive(Debug, Snafu)]
pub enum ImageError {
    #[snafu(display("Handle image fail, category:{category}, message:{source}"))]
    Image {
        category: String,
        source: image::ImageError,
    },
    #[snafu(display("Handle image fail, category:{category}, message:{source}"))]
    ImageQuant {
        category: String,
        source: imagequant::Error,
    },
    #[snafu(display("Handle image fail, category:{category}, message:{source}"))]
    LodePNG {
        category: String,
        source: lodepng::Error,
    },
    #[snafu(display("Handle image fail, category:mozjpeg, message:unknown"))]
    Mozjpeg {},
}

pub struct ImageErrorDetail {
    pub message: String,
    pub category: String,
}

impl ImageError {
    pub fn to_detail(&self) -> ImageErrorDetail {
        match self {
            ImageError::Image { category, source } => ImageErrorDetail {
                category: category.to_string(),
                message: source.to_string(),
            },
            ImageError::ImageQuant { category, source } => ImageErrorDetail {
                category: category.to_string(),
                message: source.to_string(),
            },
            ImageError::LodePNG { category, source } => ImageErrorDetail {
                category: category.to_string(),
                message: source.to_string(),
            },
            ImageError::Mozjpeg {} => ImageErrorDetail {
                category: "mozjpeg".to_string(),
                message: "unknown error".to_string(),
            },
        }
    }
}

type Result<T, E = ImageError> = std::result::Result<T, E>;

pub struct ImageInfo {
    // rgba像素
    pub buffer: Vec<RGBA8>,
    /// Width in pixels
    pub width: usize,
    /// Height in pixels
    pub height: usize,
}

impl From<Bitmap<RGBA8>> for ImageInfo {
    fn from(info: Bitmap<RGBA8>) -> Self {
        ImageInfo {
            buffer: info.buffer,
            width: info.width,
            height: info.height,
        }
    }
}

impl From<RgbaImage> for ImageInfo {
    fn from(img: RgbaImage) -> Self {
        let width = img.width() as usize;
        let height = img.height() as usize;
        let mut buffer = Vec::with_capacity(width * height);

        for ele in img.chunks(4) {
            buffer.push(RGBA8 {
                r: ele[0],
                g: ele[1],
                b: ele[2],
                a: ele[3],
            })
        }

        ImageInfo {
            buffer,
            width,
            height,
        }
    }
}

pub struct ImagePreview {
    pub diff: f64,
    pub data: Vec<u8>,
    pub image_type: String,
}

// 图片预览转换为response
impl IntoResponse for ImagePreview {
    fn into_response(self) -> Response {
        let mut res = Full::from(self.data).into_response();

        // 设置content type
        let result = mime_guess::from_ext(self.image_type.as_str()).first_or(mime::IMAGE_JPEG);
        if let Ok(value) = HeaderValue::from_str(result.as_ref()) {
            res.headers_mut().insert(header::CONTENT_TYPE, value);
        }

        // 图片设置为可缓存5分钟
        res.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=300"),
        );
        if let Ok(value) = HeaderValue::from_str(self.diff.to_string().as_str()) {
            res.headers_mut().insert("X-Dssim-Diff", value);
        }

        res
    }
}

// pub fn open(file: String) -> Result<ImageInfo, ImageError> {
//     let result = image::open(PathBuf::from(file))?;
//     let img = result.to_rgba8();
//     Ok(img.into())
// }

pub fn load<R: BufRead + Seek>(r: R, ext: String) -> Result<ImageInfo> {
    let format = ImageFormat::from_extension(OsStr::new(ext.as_str())).unwrap_or(ImageFormat::Jpeg);
    let result = image::load(r, format).context(ImageSnafu { category: "load" })?;
    let img = result.to_rgba8();
    Ok(img.into())
}

pub fn to_gif<R: Read>(r: R, speed: u8) -> Result<Vec<u8>> {
    let decoder = gif::GifDecoder::new(r).context(ImageSnafu {
        category: "gifDecode",
    })?;
    let frames = decoder.into_frames();

    let mut w = Vec::new();

    {
        let mut encoder = gif::GifEncoder::new_with_speed(&mut w, speed as i32);
        encoder
            .set_repeat(gif::Repeat::Infinite)
            .context(ImageSnafu {
                category: "gifSetRepeat",
            })?;
        encoder
            .try_encode_frames(frames.into_iter())
            .context(ImageSnafu {
                category: "gitEncode",
            })?;
    }

    Ok(w)
}

impl ImageInfo {
    // 转换获取rgb颜色
    fn get_rgb8(&self) -> Vec<RGB8> {
        let mut output_data: Vec<RGB8> = Vec::with_capacity(self.width * self.height);

        let input = self.buffer.clone();

        for ele in input {
            output_data.push(ele.rgb())
        }

        output_data
    }
    pub fn to_png(&self, quality: u8) -> Result<Vec<u8>> {
        let mut liq = imagequant::new();
        liq.set_quality(0, quality).context(ImageQuantSnafu {
            category: "pngSetQuality",
        })?;

        let mut img = liq
            .new_image(self.buffer.as_ref(), self.width, self.height, 0.0)
            .context(ImageQuantSnafu {
                category: "pngNewImage",
            })?;

        let mut res = liq.quantize(&mut img).context(ImageQuantSnafu {
            category: "pngQuantize",
        })?;

        res.set_dithering_level(1.0).context(ImageQuantSnafu {
            category: "pngSetLevel",
        })?;

        let (palette, pixels) = res.remapped(&mut img).context(ImageQuantSnafu {
            category: "pngRemapped",
        })?;
        let mut enc = lodepng::Encoder::new();
        enc.set_palette(&palette).context(LodePNGSnafu {
            category: "pngEncoder",
        })?;

        let buf = enc
            .encode(&pixels, self.width, self.height)
            .context(LodePNGSnafu {
                category: "pngEncode",
            })?;

        Ok(buf)
    }
    pub fn to_webp(&self, quality: u8) -> Result<Vec<u8>> {
        let mut w = Vec::new();

        let q = match quality {
            100 => webp::WebPQuality::lossless(),
            _ => webp::WebPQuality::lossy(quality),
        };
        let img = webp::WebPEncoder::new_with_quality(&mut w, q);

        img.encode(
            self.buffer.as_bytes(),
            self.width as u32,
            self.height as u32,
            image::ColorType::Rgba8,
        )
        .context(ImageSnafu {
            category: "webpEncode",
        })?;

        Ok(w)
    }
    pub fn to_avif(&self, quality: u8, speed: u8) -> Result<Vec<u8>> {
        let mut w = Vec::new();

        let img = avif::AvifEncoder::new_with_speed_quality(&mut w, speed, quality);
        img.write_image(
            self.get_rgb8().as_bytes(),
            self.width as u32,
            self.height as u32,
            image::ColorType::Rgb8,
        )
        .context(ImageSnafu {
            category: "avifEncode",
        })?;

        Ok(w)
    }
    pub fn to_mozjpeg(&self, quality: u8) -> Result<Vec<u8>> {
        let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
        comp.set_size(self.width, self.height);
        comp.set_mem_dest();
        comp.set_quality(quality as f32);
        comp.start_compress();
        comp.write_scanlines(self.get_rgb8().as_bytes());
        comp.finish_compress();

        let result = comp.data_to_vec();
        // 如果处理失败，则出错
        ensure!(result.is_ok(), MozjpegSnafu {});

        Ok(result.unwrap())
    }
}
