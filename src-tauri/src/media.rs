//! Screenshot helpers: downscale and re-encode as JPEG so images stay small for the model
//! (and the conversation file), and remember the scale to map coordinates back.

use base64::Engine;
use image::ImageReader;
use std::io::Cursor;

pub const MAX_WIDTH: u32 = 1280;
pub const MAX_HEIGHT: u32 = 900;

#[derive(Debug, Clone)]
pub struct Shot {
    /// Base64 JPEG.
    pub data: String,
    pub width: u32,
    pub height: u32,
    /// Original pixels per image pixel (≥ 1).
    pub scale: f64,
}

/// Decode any PNG/JPEG, fit it in MAX_WIDTH×MAX_HEIGHT, encode as JPEG.
pub fn compress(bytes: &[u8]) -> Result<Shot, String> {
    let img = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?.decode().map_err(|e| format!("can't decode screenshot: {e}"))?;
    let (w, h) = (img.width(), img.height());
    let scale = (f64::from(w) / f64::from(MAX_WIDTH)).max(f64::from(h) / f64::from(MAX_HEIGHT)).max(1.0);
    let img = if scale > 1.0 {
        img.resize((f64::from(w) / scale).round() as u32, (f64::from(h) / scale).round() as u32, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 72).encode_image(&rgb).map_err(|e| e.to_string())?;
    Ok(Shot { data: base64::engine::general_purpose::STANDARD.encode(&out), width: rgb.width(), height: rgb.height(), scale })
}

pub fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD.decode(s.trim()).map_err(|e| e.to_string())
}

pub fn data_url(shot: &Shot) -> String {
    format!("data:image/jpeg;base64,{}", shot.data)
}

impl Shot {
    pub fn image(&self) -> crate::agent::tools::Image {
        crate::agent::tools::Image { media_type: "image/jpeg".into(), data: self.data.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_screens_are_downscaled() {
        let img = image::RgbImage::from_pixel(2560, 1440, image::Rgb([30, 60, 90]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let s = compress(&png).unwrap();
        assert_eq!((s.width, s.height), (1280, 720));
        assert!((s.scale - 2.0).abs() < 1e-9);
        let jpeg = b64_decode(&s.data).unwrap();
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
        assert!(compress(b"not an image").is_err());
    }
}
