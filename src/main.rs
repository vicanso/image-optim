use std::{fs, path::Path};

mod error;
mod image;

fn main() {
    let png_i = lodepng::decode32_file("/Users/xieshuzhou/Downloads/icons/open.png").unwrap();

    let img_info: image::ImageInfo = png_i.into();
    let buf = img_info.to_png(100).unwrap();
    println!("{}", buf.len());

    fs::write(Path::new("buf.png"), buf).unwrap();
}
