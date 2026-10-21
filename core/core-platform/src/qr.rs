//! 二维码 / 条码识别（core-platform）。
//!
//! 基于 rxing（ZXing 的 Rust 移植，纯 Rust 无原生依赖）。
//!
//! 设计取舍：
//! - **多码识别**：用 [`rxing::GenericMultipleBarcodeReader`] 做四象限递归，
//!   屏幕截图里同时出现多个二维码 / 条码是常见场景（例如一张网页上有
//!   登录码 + 收款码 + 商品码），递归还会顺带处理"码只占画面一小块"的情况；
//! - **降采样保护**：长边超过 [`MAX_DECODE_EDGE`] 时先等比降采样再识别。
//!   4K 截图（830 万像素）直接跑象限递归是秒级，且 QR 模块在 2048 长边下
//!   仍有上千倍分辨率余量，降采样不损失识别率；
//! - **坐标回溯**：返回的定位点已换算回**原图像素坐标**，供前端画高亮框。

use anyhow::{Context, Result};
use image::{DynamicImage, RgbaImage, imageops};
use rxing::{
    common::HybridBinarizer,
    multi::{GenericMultipleBarcodeReader, MultipleBarcodeReader},
    BinaryBitmap, BufferedImageLuminanceSource, DecodeHints, MultiFormatReader, RXingResult,
};
use snappin_core_algo::RgbaBuffer;

/// 解码前允许的最长边（像素）。超过则先等比降采样。
pub const MAX_DECODE_EDGE: u32 = 2048;

/// 一个识别到的码。
#[derive(Debug, Clone)]
pub struct DecodedCode {
    /// 码内容（URL / 文本 / 数字串）。
    pub text: String,
    /// 格式名（`QR_CODE` / `EAN_13` / `DATA_MATRIX` …）。
    pub format: String,
    /// 码的定位点，**原图像素坐标**。
    /// 二维码通常是 3 个（finder pattern），DataMatrix 4 个。
    pub points: Vec<(f32, f32)>,
}

/// 识别截图缓冲中的全部二维码 / 条码。
///
/// 没有任何码时返回空 `Vec`（不是错误）——前端据此判断"没识别到"。
pub fn decode_codes(buf: &RgbaBuffer) -> Result<Vec<DecodedCode>> {
    let image = to_dynamic_image(buf)?;
    let (image, scale) = shrink_to_limit(image);
    let luminance = BufferedImageLuminanceSource::new(image);
    let mut bitmap = BinaryBitmap::new(HybridBinarizer::new(luminance));
    let mut reader = GenericMultipleBarcodeReader::new(MultiFormatReader::default());

    match reader.decode_multiple_with_hints(&mut bitmap, &decode_hints()) {
        Ok(results) => Ok(results.iter().map(|r| to_decoded(r, scale)).collect()),
        Err(e) => {
            // rxing 用 NotFoundException 表示"整张图里没有码"，这是正常结果而非故障。
            if matches!(e, rxing::Exceptions::NotFoundException(_)) {
                return Ok(Vec::new());
            }
            Err(anyhow::anyhow!("decode failed: {e}"))
        }
    }
}

/// RGBA 缓冲转 `image` 动态图（rxing 的亮度源只认它）。
fn to_dynamic_image(buf: &RgbaBuffer) -> Result<DynamicImage> {
    let image = RgbaImage::from_raw(buf.width, buf.height, buf.data.clone())
        .context("rgba buffer has invalid length")?;
    Ok(DynamicImage::ImageRgba8(image))
}

/// 等比缩小到 [`MAX_DECODE_EDGE`] 以内。
///
/// 返回 `(图, 缩放倍数)`；缩放倍数是 `原图宽 / 新图宽`，用于把识别到的
/// 定位点换算回原图坐标。
fn shrink_to_limit(image: DynamicImage) -> (DynamicImage, f32) {
    let (w, h) = (image.width(), image.height());
    let longest = w.max(h);
    if longest <= MAX_DECODE_EDGE {
        return (image, 1.0);
    }
    let ratio = MAX_DECODE_EDGE as f32 / longest as f32;
    let nw = ((w as f32 * ratio).round() as u32).max(1);
    let nh = ((h as f32 * ratio).round() as u32).max(1);
    // 最近邻：保住二维码黑白模块的硬边缘，避免插值产生灰边导致二值化失败。
    let scaled = imageops::resize(&image, nw, nh, imageops::FilterType::Nearest);
    (DynamicImage::ImageRgba8(scaled), w as f32 / nw as f32)
}

/// 解码用的 hints。
///
/// 只有一项偏离默认值：[`DecodeHints::AlsoInverted`]。
/// 深色模式 / 反色网页截图里的码是"白模块 + 黑底"，rxing 默认不尝试反相，
/// 而截图工具拿到反色码是常态（用户截一张深色网站），所以显式打开。
fn decode_hints() -> DecodeHints {
    DecodeHints {
        AlsoInverted: Some(true),
        ..DecodeHints::default()
    }
}

/// rxing 结果转对外结构，并把定位点从"降采样后坐标"换算回原图坐标。
fn to_decoded(result: &RXingResult, scale: f32) -> DecodedCode {
    DecodedCode {
        text: result.getText().to_string(),
        format: format!("{:?}", result.getBarcodeFormat()),
        points: result
            .getPoints()
            .iter()
            .map(|p| (p.x * scale, p.y * scale))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};
    use rxing::{BarcodeFormat, EncodeHints, Writer};

    /// 用 rxing 自带的编码器生成一张二维码位图，再放大成 RGBA 图。
    ///
    /// 返回 `(图, 模块数)`；模块数让测试能精确推算 finder 点的预期位置。
    ///
    /// 为什么用编码器而不是硬编码位图：二维码内容/掩码/纠错由库自己算，
    /// 测试只关心"编出来的能解回来"，不会因为手写位图有误而误判。
    fn qr_bitmap(text: &str, scale: u32, margin: u32) -> (RgbaImage, u32) {
        let hints = EncodeHints {
            Margin: Some(margin.to_string()),
            ..EncodeHints::default()
        };
        let matrix = rxing::MultiFormatWriter
            .encode_with_hints(text, &BarcodeFormat::QR_CODE, 0, 0, &hints)
            .expect("encode qr");
        // 编码器按内容自动定尺寸；用 getEnclosingRectangle 反推实际模块数。
        let [left, top, w, h] = matrix
            .getEnclosingRectangle()
            .expect("qr matrix should have modules");
        let side = w.max(h);
        let full = side + 2 * margin;
        let mut img = RgbImage::from_pixel(full, full, Rgb([255, 255, 255]));
        for y in 0..full {
            for x in 0..full {
                // matrix 坐标 = 像素坐标 - margin（left/top 即 quiet zone 起点）
                if matrix.get(x + left, y + top) {
                    img.put_pixel(x, y, Rgb([0, 0, 0]));
                }
            }
        }
        let img = imageops::resize(
            &img,
            full * scale,
            full * scale,
            imageops::FilterType::Nearest,
        );
        let mut rgba = RgbaImage::new(full * scale, full * scale);
        for (x, y, px) in img.enumerate_pixels() {
            rgba.put_pixel(x, y, image::Rgba([px[0], px[1], px[2], 255]));
        }
        (rgba, side)
    }

    fn to_buffer(img: &RgbaImage) -> RgbaBuffer {
        RgbaBuffer::new(img.width(), img.height(), img.clone().into_raw())
            .expect("valid buffer")
    }

    #[test]
    fn decodes_qr_round_trip() {
        let (img, _modules) = qr_bitmap("https://www.wciee.com/exhibitor/register", 6, 4);
        let buf = to_buffer(&img);
        let codes = decode_codes(&buf).expect("decode should not error");
        assert_eq!(codes.len(), 1, "expected exactly one code, got {codes:?}");
        assert_eq!(codes[0].text, "https://www.wciee.com/exhibitor/register");
        assert_eq!(codes[0].format, "QR_CODE");
        // rxing 会报 3 个 finder + 1 个 alignment 点，共 4 个。
        assert!(codes[0].points.len() >= 3, "qr should report >=3 points");
        // 定位点应落在图像范围内（原图坐标，未缩放）
        for (x, y) in &codes[0].points {
            assert!(*x >= 0.0 && *x <= img.width() as f32);
            assert!(*y >= 0.0 && *y <= img.height() as f32);
        }
    }

    #[test]
    fn decodes_two_codes_in_one_image() {
        let (a, _) = qr_bitmap("LEFT-CODE", 4, 4);
        let (b, _) = qr_bitmap("RIGHT-CODE", 4, 4);
        let (w, h) = (a.width() + b.width() + 40, a.height().max(b.height()));
        let mut canvas = RgbaImage::from_pixel(w, h, image::Rgba([255, 255, 255, 255]));
        image::imageops::replace(&mut canvas, &a, 10, 10);
        image::imageops::replace(&mut canvas, &b, (10 + a.width() + 20) as i64, 10);
        let buf = to_buffer(&canvas);

        let mut texts: Vec<String> = decode_codes(&buf)
            .expect("decode should not error")
            .into_iter()
            .map(|c| c.text)
            .collect();
        texts.sort();
        assert_eq!(texts, vec!["LEFT-CODE", "RIGHT-CODE"]);
    }

    #[test]
    fn no_code_returns_empty_vec() {
        let img = RgbaImage::from_pixel(240, 240, image::Rgba([255, 255, 255, 255]));
        let buf = to_buffer(&img);
        let codes = decode_codes(&buf).expect("blank image is not an error");
        assert!(codes.is_empty());
    }

    #[test]
    fn large_image_is_downscaled_and_points_map_back() {
        // 同一张码渲染两个尺寸：
        // - 小图 < MAX_DECODE_EDGE，不走降采样；
        // - 大图 > MAX_DECODE_EDGE，必然走降采样分支。
        // 若定位点忘了换算回原图坐标，大图测得的点会停留在降采样坐标系里，
        // 与"小图点 × 尺寸比"对不上——这正是本测试要卡住的回归。
        let (small, _) = qr_bitmap("DOWNSCALE-CHECK", 40, 4);
        let (large, _) = qr_bitmap("DOWNSCALE-CHECK", 160, 4);
        assert!(small.width() < MAX_DECODE_EDGE, "control image must not downscale");
        assert!(large.width() > MAX_DECODE_EDGE, "test image must downscale");

        let small_codes = decode_codes(&to_buffer(&small)).expect("decode should not error");
        let large_codes = decode_codes(&to_buffer(&large)).expect("decode should not error");
        assert_eq!(small_codes.len(), 1);
        assert_eq!(large_codes.len(), 1);
        assert_eq!(large_codes[0].text, "DOWNSCALE-CHECK");

        let ratio = large.width() as f32 / small.width() as f32;
        let small_max = small_codes[0]
            .points
            .iter()
            .map(|(x, _)| *x)
            .fold(f32::MIN, f32::max);
        let large_max = large_codes[0]
            .points
            .iter()
            .map(|(x, _)| *x)
            .fold(f32::MIN, f32::max);
        let expected = small_max * ratio;
        assert!(
            (large_max - expected).abs() < 8.0,
            "large point {large_max} should equal small {small_max} × {ratio} = {expected}: {large_codes:?}"
        );
        assert!(large_max <= large.width() as f32);
    }

    #[test]
    fn inverted_qr_is_still_decoded() {
        // 深色模式截图里二维码可能是白底黑码的反相；rxing 默认会尝试 inverted。
        let (img, _) = qr_bitmap("INVERTED-OK", 5, 4);
        let mut inverted = img.clone();
        for px in inverted.pixels_mut() {
            px[0] = 255 - px[0];
            px[1] = 255 - px[1];
            px[2] = 255 - px[2];
        }
        let buf = to_buffer(&inverted);
        let codes = decode_codes(&buf).expect("decode should not error");
        assert!(
            codes.iter().any(|c| c.text == "INVERTED-OK"),
            "inverted qr should still decode: {codes:?}"
        );
    }
}
