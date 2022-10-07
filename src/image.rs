use super::error::ImageError;
use image::{codecs::webp, codecs::avif};
use lodepng::Bitmap;
use rgb::{ComponentBytes, RGBA8};

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
           _ =>  webp::WebPQuality::lossy(quality),
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
        img.write_image(self.buffer.as_bytes(), self.width as u32, self.height as u32, image::ColorType::Rgba8)?;

        Ok(w)
    }
}
