use super::error::ImageError;
use image::{codecs::avif, codecs::webp};
use lodepng::Bitmap;
use rgb::{ComponentBytes, RGB8, RGBA8};

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

        let img = avif::AvifEncoder::new_with_speed_quality(&mut w, 1, quality);
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
