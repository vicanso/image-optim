use super::error::ImageError;
use lodepng::Bitmap;
use rgb::RGBA8;

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
}
