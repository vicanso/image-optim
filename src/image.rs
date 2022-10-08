use super::error::ImageError;
use image::{codecs::avif, codecs::webp, ImageFormat, RgbaImage};
use lodepng::Bitmap;
use rgb::{ComponentBytes, RGB8, RGBA8};
use std::{
    ffi::OsStr,
    io::{BufRead, Seek},
    path::PathBuf,
};

pub struct ImageInfo {
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

pub fn open(file: String) -> Result<ImageInfo, ImageError> {
    let result = image::open(PathBuf::from(file))?;
    let img = result.to_rgba8();
    Ok(img.into())
}

pub fn load<R: BufRead + Seek>(r: R, ext: String) -> Result<ImageInfo, ImageError> {
    let format = ImageFormat::from_extension(OsStr::new(ext.as_str()))
        .ok_or_else(|| "not support format".to_string())?;
    let result = image::load(r, format)?;
    let img = result.to_rgba8();
    Ok(img.into())
}

impl ImageInfo {
    fn get_rgb8(&self) -> Vec<RGB8> {
        let mut output_data: Vec<RGB8> = Vec::with_capacity(self.width * self.height);

        let input = self.buffer.clone();

        for ele in input {
            output_data.push(ele.rgb())
        }

        output_data
    }
    pub fn to_png(&self, quality: u8) -> Result<Vec<u8>, ImageError> {
        let mut liq = imagequant::new();
        liq.set_quality(0, quality)?;

        let mut img = liq.new_image(self.buffer.as_ref(), self.width, self.height, 0.0)?;

        let mut res = liq.quantize(&mut img)?;

        res.set_dithering_level(1.0)?;

        let (palette, pixels) = res.remapped(&mut img)?;
        let mut enc = lodepng::Encoder::new();
        enc.set_palette(&palette)?;

        let buf = enc.encode(&pixels, self.width, self.height)?;

        Ok(buf)
    }
    pub fn to_webp(&self, quality: u8) -> Result<Vec<u8>, ImageError> {
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
        )?;

        Ok(w)
    }
    pub fn to_avif(&self, quality: u8) -> Result<Vec<u8>, ImageError> {
        let mut w = Vec::new();

        let img = avif::AvifEncoder::new_with_speed_quality(&mut w, 10, quality);
        img.write_image(
            self.buffer.as_bytes(),
            self.width as u32,
            self.height as u32,
            image::ColorType::Rgba8,
        )?;

        Ok(w)
    }
    pub fn to_mozjpeg(&self, quality: u8) -> Result<Vec<u8>, ImageError> {
        let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
        comp.set_size(self.width, self.height);
        comp.set_mem_dest();
        comp.set_quality(quality as f32);
        comp.start_compress();
        comp.write_scanlines(self.get_rgb8().as_bytes());
        comp.finish_compress();

        comp.data_to_vec().map_err(|()| ImageError {
            message: "unknown".to_string(),
            category: "mozjepg".to_string(),
        })
    }
}
