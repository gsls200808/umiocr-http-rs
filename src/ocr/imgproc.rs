use std::io::Cursor;

use anyhow::{anyhow, Result};
use base64::Engine;

/// 纯 Rust OCR 管线使用的 RGB 图像（H*W*3，行优先）。
/// det/cls/rec 的归一化均为 (v/255-0.5)/0.5，三通道常数相同，因此不区分 BGR/RGB。
#[derive(Debug, Clone)]
pub struct RgbImage {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

impl RgbImage {
    pub fn new(w: usize, h: usize) -> Self {
        let w = w.max(1);
        let h = h.max(1);
        Self {
            w,
            h,
            data: vec![0; w * h * 3],
        }
    }

    #[inline]
    pub fn at(&self, x: isize, y: isize, c: usize) -> u8 {
        let x = x.clamp(0, self.w as isize - 1) as usize;
        let y = y.clamp(0, self.h as isize - 1) as usize;
        self.data[(y * self.w + x) * 3 + c]
    }
}

/// 解码请求里的 base64 图像（可带 data:image/...;base64, 前缀）。
pub fn decode_base64_image(b64: &str) -> Result<RgbImage> {
    let mut s = b64.trim();
    if let Some(i) = s.find(',') {
        if s[..i].trim().to_ascii_lowercase().starts_with("data:") {
            s = &s[i + 1..];
        }
    }
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let raw = base64::engine::general_purpose::STANDARD
        .decode(cleaned.as_bytes())
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(cleaned.as_bytes()))
        .map_err(|e| anyhow!("{e}"))?;

    let img = ::image::load_from_memory(&raw).map_err(|e| anyhow!("{e}"))?;
    let rgb = img.to_rgb8();
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    Ok(RgbImage {
        w,
        h,
        data: rgb.into_raw(),
    })
}

/// 把 RgbImage 编码为 JPEG 字节。
pub fn encode_jpeg(img: &RgbImage, quality: u8) -> Result<Vec<u8>> {
    let buffer = ::image::RgbImage::from_raw(img.w as u32, img.h as u32, img.data.clone())
        .ok_or_else(|| anyhow!("构造图像失败"))?;
    let mut out = Vec::new();
    let mut cursor = Cursor::new(&mut out);
    let mut encoder = ::image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, quality);
    encoder
        .encode_image(&buffer)
        .map_err(|e| anyhow!("JPEG 编码失败: {e}"))?;
    Ok(out)
}

#[inline]
fn clamp_u8(v: f64) -> u8 {
    v.clamp(0.0, 255.0).round() as u8
}

/// 双线性采样，边界按 BORDER_REPLICATE 处理。
#[inline]
fn sample(p: &RgbImage, fx: f64, fy: f64, c: usize) -> f64 {
    let x0 = fx.floor();
    let y0 = fy.floor();
    let wx = fx - x0;
    let wy = fy - y0;
    let x0 = x0 as isize;
    let y0 = y0 as isize;
    let v00 = p.at(x0, y0, c) as f64;
    let v10 = p.at(x0 + 1, y0, c) as f64;
    let v01 = p.at(x0, y0 + 1, c) as f64;
    let v11 = p.at(x0 + 1, y0 + 1, c) as f64;
    (1.0 - wx) * (1.0 - wy) * v00
        + wx * (1.0 - wy) * v10
        + (1.0 - wx) * wy * v01
        + wx * wy * v11
}

/// 半像素对齐双线性缩放（与 OpenCV INTER_LINEAR 一致）。
pub fn resize(src: &RgbImage, dst_w: usize, dst_h: usize) -> RgbImage {
    let dst_w = dst_w.max(1);
    let dst_h = dst_h.max(1);
    let mut dst = RgbImage::new(dst_w, dst_h);
    let sx = src.w as f64 / dst_w as f64;
    let sy = src.h as f64 / dst_h as f64;
    for y in 0..dst_h {
        let mut fy = (y as f64 + 0.5) * sy - 0.5;
        fy = fy.clamp(0.0, (src.h - 1) as f64);
        for x in 0..dst_w {
            let mut fx = (x as f64 + 0.5) * sx - 0.5;
            fx = fx.clamp(0.0, (src.w - 1) as f64);
            let i = (y * dst_w + x) * 3;
            for c in 0..3 {
                dst.data[i + c] = clamp_u8(sample(src, fx, fy, c));
            }
        }
    }
    dst
}

/// 转为 NCHW 的 float32 数据，归一化 (v/255-0.5)/0.5。
pub fn to_chw(img: &RgbImage) -> Vec<f32> {
    let mut out = vec![0f32; 3 * img.h * img.w];
    let plane = img.h * img.w;
    for c in 0..3 {
        let base = c * plane;
        for y in 0..img.h {
            for x in 0..img.w {
                out[base + y * img.w + x] =
                    (((img.data[(y * img.w + x) * 3 + c] as f64) / 255.0 - 0.5) / 0.5) as f32;
            }
        }
    }
    out
}

pub fn rotate180(p: &RgbImage) -> RgbImage {
    let mut out = RgbImage::new(p.w, p.h);
    for y in 0..p.h {
        for x in 0..p.w {
            let s = (y * p.w + x) * 3;
            let d = ((p.h - 1 - y) * p.w + (p.w - 1 - x)) * 3;
            out.data[d..d + 3].copy_from_slice(&p.data[s..s + 3]);
        }
    }
    out
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    (dx * dx + dy * dy).sqrt()
}

/// 按检测框四边形裁剪文本行（对应 RapidOCR get_rotate_crop_image）。
/// 用双线性四边形插值近似透视变换；对近似矩形的文本框差异很小。
pub fn warp_crop(src: &RgbImage, quad: &[[f64; 2]; 4]) -> RgbImage {
    let mut w = dist(quad[0], quad[1]).max(dist(quad[2], quad[3])) as usize;
    let mut h = dist(quad[0], quad[3]).max(dist(quad[1], quad[2])) as usize;
    w = w.max(1);
    h = h.max(1);
    let mut dst = RgbImage::new(w, h);
    let (p0, p1, p2, p3) = (quad[0], quad[1], quad[2], quad[3]);
    for y in 0..h {
        let v = (y as f64 + 0.5) / h as f64;
        for x in 0..w {
            let u = (x as f64 + 0.5) / w as f64;
            let fx = (1.0 - u) * (1.0 - v) * p0[0]
                + u * (1.0 - v) * p1[0]
                + u * v * p2[0]
                + (1.0 - u) * v * p3[0]
                - 0.5;
            let fy = (1.0 - u) * (1.0 - v) * p0[1]
                + u * (1.0 - v) * p1[1]
                + u * v * p2[1]
                + (1.0 - u) * v * p3[1]
                - 0.5;
            let i = (y * w + x) * 3;
            for c in 0..3 {
                dst.data[i + c] = clamp_u8(sample(src, fx, fy, c));
            }
        }
    }
    if h as f64 / w as f64 >= 1.5 {
        rotate180(&dst)
    } else {
        dst
    }
}

/// 把裁剪图缩放到 [3, h, w] 并归一化，右侧补 0（对应 resize_norm_img）。
pub fn resize_norm(crop: &RgbImage, h: usize, w: usize) -> Vec<f32> {
    let mut resized_w = (48.0 * crop.w as f64 / crop.h as f64).ceil() as usize;
    if resized_w > w {
        resized_w = w;
    }
    resized_w = resized_w.max(1);
    let r = resize(crop, resized_w, h);
    let mut out = vec![0f32; 3 * h * w];
    for c in 0..3 {
        let base = c * h * w;
        for y in 0..h {
            for x in 0..resized_w {
                out[base + y * w + x] =
                    (((r.data[(y * resized_w + x) * 3 + c] as f64) / 255.0 - 0.5) / 0.5) as f32;
            }
        }
    }
    out
}
